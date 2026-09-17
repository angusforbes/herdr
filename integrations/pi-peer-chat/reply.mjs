#!/usr/bin/env node
// Local reply transport. The session label is checked, NOT authenticated against
// hostile processes sharing the same Unix account. No LLM or pane injection.
import fs from 'node:fs';
import net from 'node:net';
import { UUID } from './core.mjs';

try {
  const [requestPath, option, file, ...extra] = process.argv.slice(2);
  if (!requestPath || option !== '--file' || !file || extra.length) throw new Error('Usage: node reply.mjs <request.json> --file <answer.txt>');
  const request = JSON.parse(fs.readFileSync(requestPath, 'utf8'));
  const session = process.env.PI_SESSION_ID;
  if (!UUID.test(session ?? '') || !request.recipients.some(p => p.session === session)) throw new Error('PI_SESSION_ID must identify a requested recipient. Run from your own session bash tool.');
  if (request.status !== 'open' || Date.now() >= Date.parse(request.expires)) throw new Error('Request closed or expired; no reply sent.');
  if (fs.statSync(file).size > 32000) throw new Error('Reply is larger than 32KB.');
  const text = fs.readFileSync(file, 'utf8');
  if (!text.trim()) throw new Error('Reply is empty.');
  const response = await new Promise((resolve, reject) => {
    const socket = net.createConnection(request.socket);
    const timer = setTimeout(() => { socket.destroy(); reject(new Error('Reply receipt timed out; delivery unknown. Do not automatically retry.')); }, 5000);
    let buffer = '', settled = false;
    function finish(error, value) {
      if (settled) return; settled = true; clearTimeout(timer); socket.destroy();
      error ? reject(error) : resolve(value);
    }
    socket.setEncoding('utf8');
    socket.on('connect', () => socket.write(JSON.stringify({ id: request.id, session, text }) + '\n'));
    socket.on('error', e => finish(e));
    socket.on('end', () => finish(new Error('Collector closed before receipt; delivery unknown.')));
    socket.on('data', chunk => {
      buffer += chunk;
      if (buffer.length > 40000) return finish(new Error('Invalid collector response.'));
      if (buffer.includes('\n')) {
        try { finish(null, JSON.parse(buffer.slice(0, buffer.indexOf('\n')))); }
        catch (e) { finish(e); }
      }
    });
  });
  if (!response.ok) throw new Error(response.error);
  console.log(response.duplicate ? 'Identical reply already recorded; no duplicate delivered.' : 'Reply recorded by the requesting conversation. No further acknowledgment needed.');
} catch (e) { console.error(e.message); process.exitCode = 1; }
