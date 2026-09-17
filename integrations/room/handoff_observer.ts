// Test-only observer, copied into private smoke config. Never installed globally.
// Only lifecycle counters/identity/timestamps, no prompts, credentials or payloads.
import { appendFileSync } from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export default function (pi: ExtensionAPI) {
  const path = process.env.HANDOFF_OBSERVER_PATH;
  if (!path) throw new Error("Smoke observer needs a private evidence path");
  let run = 0, message = 0, streaming = false;
  const record = (kind, ctx, extra = {}) => appendFileSync(path, JSON.stringify({
    kind, pid: process.pid, session: ctx.sessionManager.getSessionId(),
    time_ms: Date.now(), run, message, ...extra,
  }) + "\n", { mode: 0o600 });
  pi.on("agent_start", (_event, ctx) => { run++; record("agent_start", ctx); });
  pi.on("message_start", (event, ctx) => {
    if (event.message.role !== "assistant") return;
    message++; streaming = false; record("assistant_start", ctx);
  });
  pi.on("message_update", (event, ctx) => {
    if (streaming || event.assistantMessageEvent.type !== "text_delta") return;
    streaming = true; record("first_text_delta", ctx);
  });
  pi.on("message_end", (event, ctx) => {
    if (event.message.role === "assistant") record("assistant_end", ctx, { stop: event.message.stopReason });
  });
  pi.on("agent_settled", (_event, ctx) => record("agent_settled", ctx));
}
