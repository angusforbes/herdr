import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { stripTypeScriptTypes } from 'node:module';
import { RoomReceiver, registerRoomLifecycle } from './pi/receiver.mjs';
import { contextualRoomTransport, boundedMessages, visibleRoomQuestion } from './pi/room-context.mjs';

// Execute the actual fresh entrypoint with native cached adapters, stub only
// host/UI/schema dependencies. Real Pi loader coverage is in test_pi_startup.py.
const source = stripTypeScriptTypes(readFileSync(new URL('./pi/index.ts', import.meta.url), 'utf8'))
  .replace(/^import .*;\s*$/gm, '').replace('export default function', 'return function');
const load = new Function('process', 'Type', 'Box', 'Text', 'contextualRoomTransport', 'boundedMessages', 'visibleRoomQuestion', 'RoomReceiver', 'registerRoomLifecycle', source);
const Type = {
  Object: (properties, options) => ({ properties, ...options }),
  String: options => options, Integer: options => options, Optional: value => value,
};
function fixture({ callOverride, arrivals = new Map(), session = 'Path:/private/session', workspace = 'w1', terminal = 't1' } = {}) {
  const requests = [], sent = [], notices = [], handlers = new Map(), tools = new Map();
  let receiver;
  const member = { pane_id: 'w1:p1', terminal_id: terminal, session, name: 'Ada', agent: 'pi' };
  const ctx = { mode: 'tui', hasUI: true, isIdle: () => true, hasPendingMessages: () => false,
    sessionManager: { getSessionFile: () => session.startsWith('Path:') ? session.slice(5) : undefined, getSessionId: () => session.slice(3) },
    ui: { setStatus() {}, notify: text => notices.push(text) } };
  const call = async (socket, method, params, options) => {
    requests.push({ socket, method, params, options });
    const override = await callOverride?.(method, params, options);
    if (override !== undefined) return override;
    if (method === 'room.get') return { members: [member] };
    if (method === 'room.delivery.register') return { receiver_id: 'r', server_epoch: 'e' };
    if (method === 'room.delivery.claim') return { delivery: null };
    if (method === 'room.agent.post') {
      if (params.arrival) {
        const key = JSON.stringify([params.workspace_id, 'pi', params.session]);
        if (!arrivals.has(key)) arrivals.set(key, arrivals.size + 1);
        return { persistence: 'saved', sequence: arrivals.get(key) };
      }
      return { persistence: 'saved', sequence: 10 };
    }
    throw new Error(`Unexpected method ${method}`);
  };
  class Receiver extends RoomReceiver {
    constructor(pi, env, options) {
      super(pi, env, { ...options, schedule: () => 0, cancel() {} }); receiver = this;
    }
  }
  const pi = { on: (name, handler) => handlers.set(name, handler), registerTool: tool => tools.set(tool.name, tool),
    registerCommand() {}, registerMessageRenderer() {}, sendMessage: (...args) => sent.push(args), sendUserMessage: (...args) => sent.push(args) };
  load({ env: { HERDR_SOCKET_PATH: '/explicit.sock', HERDR_PANE_ID: member.pane_id, HERDR_WORKSPACE_ID: workspace } }, Type, null, null,
    pi => contextualRoomTransport(pi, { call }), boundedMessages, visibleRoomQuestion, Receiver, registerRoomLifecycle)(pi);
  return { receiver, requests, sent, notices, tools, ctx, arrivals, member,
    start: () => handlers.get('session_start')({}, ctx), stop: () => handlers.get('session_shutdown')(),
    post: (params, signal) => tools.get('room_post').execute('call', params, signal, undefined, ctx) };
}

test('startup posts one deterministic arrival within serialized registration without a model turn', async () => {
  const f = fixture();
  assert.equal(f.requests.length, 0, 'factory must not open sockets');
  await f.start();
  assert.deepEqual(f.requests.map(r => r.method), ['room.get', 'room.delivery.register', 'room.agent.post']);
  const arrival = f.requests.at(-1);
  assert.equal(arrival.socket, '/explicit.sock');
  assert.deepEqual(arrival.params, { workspace_id: 'w1', pane_id: 'w1:p1', terminal_id: 't1', session: f.member.session, text: 'Joined the room.', arrival: true });
  await f.receiver.tick(); await f.receiver.tick();
  assert.equal(f.requests.filter(r => r.method === 'room.agent.post').length, 1);
  assert.equal(f.sent.length, 0);
  assert.ok(f.receiver.run.receiver); assert.ok(!f.receiver.run.frozen);
});

test('fresh TS factory with cached native adapters attempts arrival, server dedups reload and terminal handoff', async () => {
  const arrivals = new Map();
  for (const terminal of ['t1', 't1', 'replacement']) {
    const f = fixture({ arrivals, terminal }); await f.start(); f.stop();
    assert.equal(f.sent.length, 0); assert.equal(arrivals.size, 1);
  }
  for (const options of [{ session: 'Id:new-session' }, { workspace: 'other-room' }]) {
    const f = fixture({ arrivals, ...options }); await f.start(); f.stop();
  }
  assert.equal(arrivals.size, 3);
});

test('old API, lost arrival ack and invalid ack do not poison registration or cause retries/turns', async () => {
  for (const failure of [new Error('method unavailable'), new Error('timeout after write'), { persistence: 'memory_only', sequence: 1 }, { persistence: 'saved', sequence: 0 }]) {
    const f = fixture({ callOverride: method => {
      if (method === 'room.agent.post') { if (failure instanceof Error) throw failure; return failure; }
    } });
    await f.start(); await f.receiver.tick(); await f.receiver.tick();
    assert.ok(f.receiver.run.receiver); assert.ok(!f.receiver.run.frozen);
    assert.equal(f.requests.filter(r => r.method === 'room.agent.post').length, 1);
    assert.equal(f.sent.length, 0); assert.equal(f.notices.length, 1);
  }
});

test('room_post needs no delivery, captures identity, bounds bytes and does not terminate unrelated work', async () => {
  const f = fixture();
  await assert.rejects(f.post({ text: 'not bound' }), /No current room binding/);
  await f.start();
  assert.equal(f.receiver.run.active, undefined);
  assert.deepEqual(Object.keys(f.tools.get('room_post').parameters.properties), ['text']);
  assert.equal(f.tools.get('room_post').parameters.additionalProperties, false);
  const result = await f.post({ text: 'A contribution', workspace_id: 'forged', terminal_id: 'forged', session: 'forged', arrival: true });
  assert.equal(result.terminate, undefined);
  assert.deepEqual(result.details, { sequence: 10, workspace_id: 'w1' });
  assert.deepEqual(f.requests.at(-1).params, { workspace_id: 'w1', pane_id: 'w1:p1', terminal_id: 't1', session: f.member.session, text: 'A contribution' });
  const count = f.requests.length;
  for (const text of [' ', '界'.repeat(2731)]) await assert.rejects(f.post({ text }), /1..8192/);
  const abort = new AbortController(); abort.abort();
  await assert.rejects(f.post({ text: 'cancelled' }, abort.signal), /cancelled/);
  assert.equal(f.requests.length, count);
  assert.equal(f.sent.length, 0);
});

test('ambiguous post is sent once, not reported successful or retried by heartbeat', async () => {
  for (const failure of [new Error('lost ack'), { persistence: 'memory_only', sequence: 2 }, { persistence: 'saved', sequence: '2' }]) {
    const f = fixture({ callOverride: (method, params) => {
      if (method === 'room.agent.post' && !params.arrival) { if (failure instanceof Error) throw failure; return failure; }
    } });
    await f.start(); await assert.rejects(f.post({ text: 'uncertain' }), /do not retry/);
    await f.receiver.tick(); await f.receiver.tick();
    assert.equal(f.requests.filter(r => r.method === 'room.agent.post' && !r.params.arrival).length, 1);
    assert.equal(f.sent.length, 0);
  }
});

test('late registration from replaced generation cannot announce or consume new generation attempt', async () => {
  let release, entered;
  const registering = new Promise(resolve => { entered = resolve; });
  let registrations = 0;
  const f = fixture({ callOverride: method => {
    if (method === 'room.delivery.register' && registrations++ === 0) {
      entered(); return new Promise(resolve => { release = resolve; });
    }
  } });
  const old = f.start(); await registering;
  const fresh = f.start();
  release({ receiver_id: 'old', server_epoch: 'e' });
  await old; await fresh;
  assert.equal(f.requests.filter(r => r.method === 'room.agent.post').length, 1);
  assert.equal(f.receiver.run.receiver.receiver_id, 'r');
  assert.equal(f.sent.length, 0);
});

test('queued post from an invalidated session never reaches socket', async () => {
  const f = fixture(); await f.start();
  let release;
  f.receiver.serial = new Promise(resolve => { release = resolve; });
  const post = f.post({ text: 'queued' }); f.stop(); release();
  await assert.rejects(post, /do not retry/);
  assert.equal(f.requests.filter(r => r.method === 'room.agent.post' && !r.params.arrival).length, 0);
});

test('session change after post write is uncertain, never false success', async () => {
  let release;
  const f = fixture({ callOverride: (method, params) => method === 'room.agent.post' && !params.arrival ? new Promise(resolve => { release = resolve; }) : undefined });
  await f.start();
  const post = f.post({ text: 'racing' });
  // Flush only microtasks to let the queued write start; no wall-clock polling.
  await Promise.resolve(); await Promise.resolve();
  assert.equal(typeof release, 'function');
  f.stop(); release({ persistence: 'saved', sequence: 10 });
  await assert.rejects(post, /do not retry/);
});
