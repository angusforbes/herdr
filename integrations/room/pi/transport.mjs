import net from "node:net";
import { randomUUID } from "node:crypto";

// Exactly one request per connection. Never reconnect/retry a possibly written request.
export function socketCall(path, method, params, { signal, timeoutMs = 2500, maxBytes = 2 * 1024 * 1024 } = {}) {
  if (typeof path !== "string" || !path.startsWith("/")) throw new Error("Room requires an explicit absolute Unix socket");
  return new Promise((resolve, reject) => {
    const id = randomUUID();
    const socket = net.createConnection(path);
    let chunks = [], bytes = 0, done = false;
    const finish = (error, result) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      socket.destroy();
      error ? reject(error) : resolve(result);
    };
    const unknown = () => finish(new Error("Room transport failed; write outcome unknown; no retry"));
    const abort = () => unknown();
    const timer = setTimeout(unknown, timeoutMs);
    signal?.addEventListener("abort", abort, { once: true });
    if (signal?.aborted) { abort(); return; }
    socket.on("connect", () => socket.write(JSON.stringify({ id, method, params }) + "\n"));
    socket.on("error", unknown);
    socket.on("end", unknown);
    socket.on("close", () => { if (!done) unknown(); });
    socket.on("data", (chunk) => {
      bytes += chunk.length;
      if (bytes > maxBytes) { unknown(); return; }
      chunks.push(chunk);
      if (!chunk.includes(10)) return;
      try {
        const raw = Buffer.concat(chunks);
        const response = JSON.parse(raw.subarray(0, raw.indexOf(10)).toString("utf8"));
        if (response?.id !== id) { unknown(); return; }
        if (response.error) {
          const error = new Error("Room server rejected request");
          error.serverRejected = true;
          finish(error);
        } else if (response.result && typeof response.result === "object") {
          finish(null, response.result);
        } else unknown();
      } catch { unknown(); }
    });
  });
}
