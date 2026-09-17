import test from "node:test";
import assert from "node:assert/strict";
import net from "node:net";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { RoomReceiver, registerRoomLifecycle } from "./pi/receiver.mjs";
import { socketCall } from "./pi/transport.mjs";

const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
function fixture(envOverrides = {}) {
  const member = { pane_id: "w1.p1", terminal_id: "t1", session: "Path:/tmp/live.jsonl" };
  const ctx = { mode: "tui", hasUI: true, idle: true, pending: false, file: "/tmp/live.jsonl",
    isIdle() { return this.idle; }, hasPendingMessages() { return this.pending; },
    sessionManager: { getSessionFile: () => ctx.file, getSessionId: () => "live" },
    ui: { setStatus: (_id, text) => statuses.push(text), notify: () => {} } };
  const statuses = [], requests = [], sent = [], timers = new Map(), queue = [];
  let calls = 0, maxCalls = 0, timerId = 0, claimHook, replyHook, registerHook, reportHook;
  const pi = { sendMessage(message, options) { sent.push({ message, options }); ctx.idle = false; } };
  const f = { ctx, member, requests, sent, timers, statuses, queue, pi,
    setClaim: fn => { claimHook = fn; }, setReply: fn => { replyHook = fn; }, setRegister: fn => { registerHook = fn; },
    setReport: fn => { reportHook = fn; },
    get maxCalls() { return maxCalls; },
    delivery(n = 1) { return { delivery_id: `d${n}`, workspace_id: "w1", request_sequence: n, recipient: { ...member }, text: `Question ${n}`, expires_unix: 2000 }; },
  };
  f.receiver = new RoomReceiver(pi, { HERDR_ROOM_ENABLED: "1", HERDR_SOCKET_PATH: "/tmp/explicit.sock", HERDR_PANE_ID: member.pane_id, HERDR_WORKSPACE_ID: "w1", PI_SESSION_ID: "wrong-parent", ...envOverrides }, {
    now: () => 1000000,
    schedule: fn => { timers.set(++timerId, fn); return timerId; }, cancel: id => timers.delete(id),
    call: async (_socket, method, params) => {
      calls++; maxCalls = Math.max(maxCalls, calls);
      requests.push({ socket: _socket, method, params });
      try {
        if (method === "workspace.list") return { workspaces: [{ workspace_id: "w1" }] };
        if (method === "room.get") return { members: [member] };
        if (method === "room.delivery.register") return registerHook ? await registerHook(params) : { receiver_id: "r1", server_epoch: "e1" };
        if (method === "room.delivery.claim") return claimHook ? await claimHook(params) : { delivery: params.ready ? queue.shift() ?? null : null };
        if (method === "room.delivery.report") return reportHook ? await reportHook(params) : { accepted: true };
        if (method === "room.reply") return replyHook ? await replyHook(params) : { persistence: "saved", sequence: 10 };
        throw new Error("unexpected " + method);
      } finally { calls--; }
    },
  });
  f.tick = async () => {
    const next = timers.entries().next().value;
    assert.ok(next, "self-scheduled timer exists"); timers.delete(next[0]);
    // Use tick directly to await the same deterministic lifecycle.
    await f.receiver.tick();
  };
  return f;
}

test("busy, pending and modal agents heartbeat but never claim; attribution and followUp", async () => {
  const f = fixture(); await f.receiver.start(f.ctx); f.queue.push(f.delivery());
  f.ctx.idle = false; await f.tick();
  f.ctx.idle = true; f.ctx.pending = true; await f.tick();
  f.ctx.pending = false; f.receiver.modal(f.ctx, true); await f.tick();
  assert.deepEqual(f.requests.filter(r => r.method.endsWith("claim")).map(r => r.params.ready), [false, false, false]);
  assert.equal(f.sent.length, 0);
  f.receiver.modal(f.ctx, false); await f.tick();
  assert.equal(f.sent.length, 1);
  assert.deepEqual(f.sent[0].options, { triggerTurn: true, deliverAs: "followUp" });
  assert.match(f.sent[0].message.content, /BEGIN ROOM QUESTION.*"sender":"human"/);
  assert.match(f.sent[0].message.content, /END ROOM QUESTION/);
  assert.equal(f.requests.find(r => r.method.endsWith("register")).params.session, f.member.session);
  f.receiver.stop(); assert.equal(f.timers.size, 0);
});

test("multiple questions: reply does not unlock until settled; no history/fanout loop", async () => {
  const f = fixture(); await f.receiver.start(f.ctx); f.queue.push(f.delivery(1), f.delivery(2));
  await f.tick();
  await f.receiver.reply({ delivery_id: "d1", text: "First reply" }, f.ctx);
  f.ctx.idle = true; await f.tick(); assert.equal(f.sent.length, 1);
  await f.receiver.settled(f.ctx); await f.tick(); assert.equal(f.sent.length, 2);
  f.ctx.idle = true; await f.receiver.settled(f.ctx);
  assert.equal(f.requests.filter(r => r.params.outcome === "unanswered").length, 1);
  for (let n = 0; n < 4; n++) await f.tick();
  assert.equal(f.sent.length, 2); assert.equal(f.maxCalls, 1);
  assert.equal(f.requests.filter(r => r.method === "room.read" || r.method === "room.post").length, 0);
  f.receiver.stop();
});

test("claim response after becoming busy/modal/expired is reported failed, never injected", async () => {
  for (const change of [f => { f.ctx.idle = false; }, f => f.receiver.modal(f.ctx, true), f => { f.receiver.started(f.ctx); f.ctx.idle = true; }, f => { f.receiver.modal(f.ctx, true); f.receiver.modal(f.ctx, false); }, f => { f.queue[0].expires_unix = 0; }]) {
    const f = fixture(); await f.receiver.start(f.ctx); f.queue.push(f.delivery());
    const pending = deferred(); f.setClaim(async () => { await pending.promise; return { delivery: f.queue.shift() }; });
    const tick = f.tick(); await Promise.resolve(); change(f); pending.resolve(); await tick;
    assert.equal(f.sent.length, 0);
    assert.ok(f.requests.some(r => r.params.outcome === "failed"));
    f.receiver.stop();
  }
});

test("unrelated settlement during an idle claim does not discard the question", async () => {
  const f = fixture(); await f.receiver.start(f.ctx);
  const pending = deferred(); f.setClaim(() => pending.promise);
  const ticking = f.tick(); await Promise.resolve();
  await f.receiver.settled(f.ctx);
  pending.resolve({ delivery: f.delivery() }); await ticking;
  assert.equal(f.sent.length, 1);
  assert.equal(f.requests.filter(r => r.params.outcome === "failed").length, 0);
  f.receiver.stop();
});

test("undispatched job cannot settle as unanswered even when failure report is lost", async () => {
  const f = fixture(); await f.receiver.start(f.ctx);
  const expired = f.delivery(); expired.expires_unix = 0;
  f.queue.push(expired);
  const pending = deferred(), entered = deferred();
  f.setReport(() => { entered.resolve(); return pending.promise; });
  const ticking = f.tick(); await entered.promise;
  await f.receiver.settled(f.ctx);
  pending.reject(new Error("lost failure report")); await ticking;
  await f.receiver.settled(f.ctx);
  assert.equal(f.sent.length, 0);
  assert.equal(f.receiver.run.active, undefined);
  assert.equal(f.receiver.run.frozen, true);
  assert.deepEqual(f.requests.filter(r => r.method === "room.delivery.report").map(r => r.params.outcome), ["failed"]);
  f.receiver.stop();
});

test("reload cancels old generation; stale claim/settled/reply callbacks cannot overwrite new job", async () => {
  const f = fixture(); await f.receiver.start(f.ctx);
  const pending = deferred(); f.setClaim(() => pending.promise);
  const tick = f.tick(); await Promise.resolve();
  f.receiver.stop();
  const started = f.receiver.start(f.ctx);
  pending.resolve({ delivery: f.delivery(1) }); await tick; await started;
  assert.equal(f.sent.length, 0);
  f.setClaim(null); f.queue.push(f.delivery(2)); await f.tick();
  const reply = deferred(); f.setReply(() => reply.promise);
  const writing = f.receiver.reply({ delivery_id: "d2", text: "old" }, f.ctx);
  const failed = assert.rejects(writing, /not confirmed/);
  await Promise.resolve(); f.receiver.stop(); f.ctx.file = "/tmp/replacement.jsonl"; f.member.session = "Path:/tmp/replacement.jsonl";
  const restarting = f.receiver.start(f.ctx);
  reply.resolve({ persistence: "saved", sequence: 90 }); await failed; await restarting;
  f.setReply(null); f.ctx.idle = true; f.queue.push(f.delivery(3)); await f.tick();
  assert.equal(f.receiver.run.active.delivery_id, "d3"); assert.equal(f.receiver.run.active.replied, false);
  assert.equal(f.maxCalls, 1); f.receiver.stop();
});

test("uncertain claims freeze ready=true polling; no injection retry or tight timer loops", async () => {
  const f = fixture(); await f.receiver.start(f.ctx);
  f.setClaim(() => { throw new Error("response lost"); }); await f.tick();
  f.setClaim(() => ({ delivery: null })); for (let n = 0; n < 3; n++) await f.tick();
  assert.deepEqual(f.requests.filter(r => r.method.endsWith("claim")).map(r => r.params.ready), [true, false, false, false]);
  assert.equal(f.sent.length, 0); assert.equal(f.timers.size, 1); f.receiver.stop();
});

test("uncertain replies are never retried even if model repeats tool; settle reports unanswered", async () => {
  const f = fixture(); await f.receiver.start(f.ctx); f.queue.push(f.delivery()); await f.tick();
  f.setReply(() => { throw new Error("lost saved ack"); });
  await assert.rejects(f.receiver.reply({ delivery_id: "d1", text: "x" }, f.ctx), /not confirmed/);
  await assert.rejects(f.receiver.reply({ delivery_id: "d1", text: "x" }, f.ctx), /already attempted/);
  assert.equal(f.requests.filter(r => r.method === "room.reply").length, 1);
  f.ctx.idle = true; await f.receiver.settled(f.ctx);
  assert.ok(f.requests.some(r => r.params.outcome === "unanswered")); f.receiver.stop();
});

test("tool identity is captured, stale callbacks and mismatched delivery rejected", async () => {
  const f = fixture(); await f.receiver.start(f.ctx); f.queue.push(f.delivery()); await f.tick();
  await assert.rejects(f.receiver.reply({ delivery_id: "other", text: "x" }, f.ctx), /No matching/);
  await f.receiver.reply({ delivery_id: "d1", text: "x", pane_id: "attacker", session: "wrong", socket: "/tmp/wrong" }, f.ctx);
  const reply = f.requests.find(r => r.method === "room.reply");
  assert.deepEqual(reply.params, { workspace_id: "w1", request_sequence: 1, ...f.member, text: "x" });
  f.ctx.file = "/tmp/switched.jsonl"; f.ctx.idle = true; await f.receiver.settled(f.ctx);
  assert.equal(f.receiver.run.active.delivery_id, "d1"); f.receiver.stop();
});

test("reply and heartbeat use one in-flight socket; settle waits for saved acknowledgement", async () => {
  const f = fixture(); await f.receiver.start(f.ctx); f.queue.push(f.delivery()); await f.tick();
  const pending = deferred(); f.setReply(() => pending.promise);
  const reply = f.receiver.reply({ delivery_id: "d1", text: "yes" }, f.ctx);
  const tick = f.tick(); f.ctx.idle = true; const settled = f.receiver.settled(f.ctx);
  pending.resolve({ persistence: "saved", sequence: 30 }); await Promise.all([reply, tick, settled]);
  assert.equal(f.maxCalls, 1); assert.equal(f.receiver.run.active, undefined);
  assert.ok(!f.requests.some(r => r.params.outcome === "unanswered")); f.receiver.stop();
});

test("uncertain registration is not retried; missing workspace env uses exact live membership", async () => {
  const f = fixture(); f.setRegister(() => { throw new Error("lost registration ack"); });
  await f.receiver.start(f.ctx); for (let n = 0; n < 3; n++) await f.tick();
  assert.equal(f.requests.filter(r => r.method.endsWith("register")).length, 1);
  assert.equal(f.sent.length, 0); f.receiver.stop();
  const other = fixture(); delete other.receiver.env.HERDR_WORKSPACE_ID;
  await other.receiver.start(other.ctx);
  assert.equal(other.requests[0].method, "workspace.list");
  assert.equal(other.receiver.run.workspace, "w1"); other.receiver.stop();
});

test("missing preview opt-in, implicit socket or non-TUI never starts network resources", async () => {
  for (const mutate of [f => { f.receiver.enabled = false; }, f => { delete f.receiver.env.HERDR_SOCKET_PATH; }, f => { f.ctx.mode = "rpc"; }]) {
    const f = fixture(); mutate(f); await f.receiver.start(f.ctx);
    assert.equal(f.requests.length, 0); assert.equal(f.timers.size, 0);
  }
});

test("commands register while inactive; explicit enable uses current exact binding without inference", async () => {
  const f = fixture({ HERDR_ROOM_ENABLED: undefined });
  const commands = new Map(), events = new Map();
  registerRoomLifecycle({ registerCommand: (n, c) => commands.set(n, c), on: (n, h) => events.set(n, h) }, f.receiver);
  assert.deepEqual([...commands.keys()], ["room-enable", "room-disable"]);
  assert.equal(f.requests.length, 0);
  await events.get("session_start")({}, f.ctx);
  await f.receiver.tick();
  await f.receiver.settled(f.ctx);
  await assert.rejects(f.receiver.reply({ delivery_id: "d1", text: "no" }, f.ctx), /No matching/);
  assert.equal(f.requests.length, 0); assert.equal(f.timers.size, 0);
  // A captured parent session must not supply the command's identity.
  f.ctx.file = "/tmp/current-tui.jsonl"; f.member.session = "Path:/tmp/current-tui.jsonl";
  await commands.get("room-enable").handler("", f.ctx);
  const g = f.receiver.run;
  await commands.get("room-enable").handler("", f.ctx);
  assert.equal(f.receiver.run, g); // idempotent, not a retry/reset
  for (let n = 0; n < 3; n++) await f.tick();
  assert.equal(f.sent.length, 0);
  assert.equal(f.receiver.env.HERDR_ROOM_ENABLED, undefined);
  assert.ok(f.requests.every(r => r.socket === "/tmp/explicit.sock"));
  assert.equal(f.requests.find(r => r.method.endsWith("register")).params.session, f.member.session);
  assert.equal(f.requests.find(r => r.method.endsWith("register")).params.pane_id, f.member.pane_id);
  commands.get("room-disable").handler("", f.ctx);
  const count = f.requests.length;
  await f.receiver.tick(); await events.get("session_start")({}, f.ctx);
  assert.equal(f.requests.length, count); assert.equal(f.timers.size, 0);
  assert.equal(f.receiver.run, undefined);
  // /reload creates a new factory/receiver: no persistence of command opt-in.
  const fresh = fixture({ HERDR_ROOM_ENABLED: undefined });
  await fresh.receiver.start(fresh.ctx);
  assert.equal(fresh.requests.length, 0);
});

test("explicit enable cannot infer socket, pane, or non-TUI identity", async () => {
  for (const env of [{ HERDR_SOCKET_PATH: undefined }, { HERDR_SOCKET_PATH: "relative.sock" }, { HERDR_PANE_ID: undefined }]) {
    const f = fixture({ HERDR_ROOM_ENABLED: undefined, ...env });
    await f.receiver.enable(f.ctx);
    assert.equal(f.requests.length, 0); assert.equal(f.timers.size, 0);
  }
  const f = fixture({ HERDR_ROOM_ENABLED: undefined }); f.ctx.mode = "rpc";
  await assert.rejects(f.receiver.enable(f.ctx), /current Pi TUI/);
  assert.equal(f.requests.length, 0);
});

test("disable/re-enable cancels stale claim and reply generations without retry", async () => {
  const f = fixture({ HERDR_ROOM_ENABLED: undefined }); await f.receiver.enable(f.ctx);
  const pending = deferred(); f.setClaim(() => pending.promise);
  const ticking = f.tick(); await Promise.resolve();
  const old = f.receiver.run;
  f.receiver.disable(f.ctx);
  assert.ok(old.abort.signal.aborted); assert.equal(f.timers.size, 0);
  const enabling = f.receiver.enable(f.ctx);
  pending.resolve({ delivery: f.delivery(1) }); await ticking; await enabling;
  assert.notEqual(f.receiver.run.nonce, old.nonce); assert.equal(f.sent.length, 0);
  f.setClaim(null); f.queue.push(f.delivery(2)); await f.tick();
  const writing = deferred(); f.setReply(() => writing.promise);
  const reply = f.receiver.reply({ delivery_id: "d2", text: "uncertain" }, f.ctx);
  const failed = assert.rejects(reply, /not confirmed/); await Promise.resolve();
  f.receiver.disable(f.ctx); const reenabled = f.receiver.enable(f.ctx);
  writing.resolve({ persistence: "saved", sequence: 10 }); await failed; await reenabled;
  await assert.rejects(f.receiver.reply({ delivery_id: "d2", text: "uncertain" }, f.ctx), /No matching/);
  assert.equal(f.requests.filter(r => r.method === "room.reply").length, 1);
  assert.equal(f.receiver.run.active, undefined); assert.equal(f.maxCalls, 1);
  f.receiver.disable(f.ctx);
});

test("enable does not borrow another live member's pane or session", async () => {
  for (const mutate of [f => { f.member.session = "Path:/tmp/other.jsonl"; }, f => { f.member.pane_id = "w1.other"; }]) {
    const f = fixture({ HERDR_ROOM_ENABLED: undefined }); mutate(f);
    await f.receiver.enable(f.ctx); await f.tick();
    assert.ok(!f.requests.some(r => r.method.endsWith("register")));
    assert.equal(f.sent.length, 0); f.receiver.stop();
  }
});

test("shutdown cancels resources even if the old UI has been torn down", async () => {
  const f = fixture(); await f.receiver.start(f.ctx); const g = f.receiver.run;
  f.ctx.ui.setStatus = () => { throw new Error("stale UI"); };
  f.receiver.stop();
  assert.ok(g.abort.signal.aborted); assert.equal(f.timers.size, 0); assert.equal(f.receiver.run, undefined);
});

test("startup env opt-in still connects without a command and never changes caller env", async () => {
  const f = fixture(); await f.receiver.start(f.ctx);
  assert.equal(f.receiver.run.session, f.member.session); assert.equal(f.sent.length, 0);
  f.receiver.disable(f.ctx); assert.equal(f.receiver.env.HERDR_ROOM_ENABLED, "1");
  await f.receiver.start(f.ctx); assert.equal(f.receiver.run, undefined);
  await f.receiver.enable(f.ctx); assert.ok(f.receiver.run); f.receiver.stop();
});

async function withSocket(handler, use) {
  const dir = await mkdtemp(path.join(os.tmpdir(), "room-socket-"));
  const socket = path.join(dir, "api.sock");
  const clients = new Set();
  const server = net.createServer(s => { clients.add(s); s.on("error", () => {}); s.on("close", () => clients.delete(s)); s.once("data", b => handler(s, JSON.parse(b))); });
  await new Promise(resolve => server.listen(socket, resolve));
  try { await use(socket); } finally { for (const s of clients) s.destroy(); await new Promise(resolve => server.close(resolve)); await rm(dir, { recursive: true }); }
}

test("actual JSONL socket: id validation, errors, bounded bytes, timeout, split UTF-8", async () => {
  await withSocket((s, req) => { const b = Buffer.from(JSON.stringify({ id: req.id, result: { text: "π" } }) + "\n"); s.write(b.subarray(0, b.length - 4)); s.end(b.subarray(b.length - 4)); }, async socket => {
    assert.deepEqual(await socketCall(socket, "room.get", {}), { text: "π" });
  });
  for (const handler of [s => s.end('{"id":"wrong","result":{}}\n'), s => s.end("x".repeat(300)), s => s.end("partial"), () => {}]) {
    await withSocket(handler, async socket => { await assert.rejects(socketCall(socket, "room.reply", {}, { maxBytes: 256, timeoutMs: 30 }), /unknown/); });
  }
  await withSocket((s, req) => s.end(JSON.stringify({ id: req.id, error: { message: "do not echo server data" } }) + "\n"), async socket => { await assert.rejects(socketCall(socket, "room.reply", {}), /server rejected/); });
});
