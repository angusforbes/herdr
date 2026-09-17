import net from 'node:net';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { acceptReply } from './core.mjs';

export async function startCollector(request, storeRoot, { onReply = () => {}, onClose = () => {} } = {}) {
  fs.mkdirSync(storeRoot, { recursive: true, mode: 0o700 });
  const dir = path.join(storeRoot, request.id);
  fs.mkdirSync(dir, { mode: 0o700 });
  const socketDir = fs.mkdtempSync(path.join(os.tmpdir(), 'pi-peer-'));
  fs.chmodSync(socketDir, 0o700);
  request.socket = path.join(socketDir, 'reply.sock');
  const requestPath = path.join(dir, 'request.json');
  let timer, closed = false, dispatchDone = false;
  const sockets = new Set();
  const persist = () => {
    fs.writeFileSync(requestPath + '.tmp', JSON.stringify(request, null, 2) + '\n', { mode: 0o600 });
    fs.renameSync(requestPath + '.tmp', requestPath);
  };
  let resolveFinished;
  const finished = new Promise(resolve => { resolveFinished = resolve; });
  const close = reason => {
    if (closed) return;
    closed = true;
    clearTimeout(timer);
    request.status = reason;
    request.closedAt = new Date().toISOString();
    persist();
    server.close();
    // A reply's acknowledgment is flushed before close() is scheduled.
    for (const socket of sockets) socket.end();
    fs.rmSync(socketDir, { recursive: true, force: true });
    resolveFinished(request);
    onClose(request);
  };
  const maybeFinish = () => {
    if (dispatchDone && request.recipients.every(p => request.replies[p.session] || ['failed', 'skipped'].includes(request.dispatch[p.session]?.state))) {
      close(Object.keys(request.replies).length === request.recipients.length ? 'completed' : 'partial');
    }
  };
  const server = net.createServer(socket => {
    sockets.add(socket); socket.setTimeout(5000, () => socket.destroy());
    socket.on('close', () => sockets.delete(socket)); socket.on('error', () => {});
    let buffer = '', bytes = 0, handled = false;
    socket.setEncoding('utf8');
    socket.on('data', chunk => {
      if (handled) return;
      bytes += Buffer.byteLength(chunk); buffer += chunk;
      if (bytes > 40000) { handled = true; socket.end(JSON.stringify({ ok: false, error: 'Reply too large.' }) + '\n'); return; }
      if (!buffer.includes('\n')) return;
      handled = true;
      try {
        const result = acceptReply(request, JSON.parse(buffer.slice(0, buffer.indexOf('\n'))));
        persist();
        socket.end(JSON.stringify({ ok: true, duplicate: result.duplicate }) + '\n', () => {
          if (!result.duplicate) onReply(request, result.peer);
          maybeFinish();
        });
      } catch (e) { socket.end(JSON.stringify({ ok: false, error: e.message }) + '\n'); }
    });
  });
  try {
    await new Promise((resolve, reject) => { server.once('error', reject); server.listen(request.socket, resolve); });
    persist();
  } catch (e) {
    server.close(); fs.rmSync(socketDir, { recursive: true, force: true }); throw e;
  }
  timer = setTimeout(() => close('timed-out'), Math.max(1, Date.parse(request.expires) - Date.now()));
  return {
    request, requestPath, finished, close,
    recordDispatch(session, state, detail) {
      if (closed) return;
      request.dispatch[session] = { state, detail, at: new Date().toISOString() }; persist();
    },
    finishDispatch() { dispatchDone = true; maybeFinish(); },
  };
}
