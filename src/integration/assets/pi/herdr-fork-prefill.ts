// Herdr's Pi 0.85.1 launch bridge. Plain JavaScript-compatible TypeScript.
// No prompt dispatch, session switching or model calls. One private receipt
// prevents obsolete draft resurrection even when the parent shell relaunches Pi.
import { readFileSync, openSync, closeSync } from "node:fs";
import { dirname, join } from "node:path";

export default function herdrForkPrefill(pi) {
  let handled = false;
  pi.on("session_start", (event, ctx) => {
    if (handled || event.reason !== "startup") return;
    handled = true;
    if (ctx.mode !== "tui" || !ctx.hasUI) return;
    const header = ctx.sessionManager.getHeader();
    const sessionFile = ctx.sessionManager.getSessionFile();
    if (!sessionFile || !header?.id) return;
    const entries = ctx.sessionManager.getBranch();
    const markerIndex = entries.findLastIndex((entry) =>
      entry.type === "custom" && entry.customType === "herdr.fork" &&
      entry.data?.schemaVersion === 1 && entry.data?.sessionId === header.id);
    const marker = entries[markerIndex];
    if (!marker || marker.data.position !== "rewrite" ||
        typeof marker.data.draftText !== "string") return;
    // Even an unconsumed ticket must not resurrect a draft after work continued.
    if (entries.slice(markerIndex + 1).some((entry) =>
      ["message", "custom_message", "compaction", "branch_summary"].includes(entry.type))) return;
    if (ctx.ui.getEditorText() !== "") return;
    try {
      const directory = dirname(sessionFile);
      const ticket = JSON.parse(readFileSync(join(directory, "herdr-prefill-ticket.json"), "utf8"));
      if (ticket.sessionId !== header.id || ticket.launchToken !== marker.data.launchToken) return;
      // Exclusive creation is the cross-process arbiter. Never remove/overwrite
      // an existing receipt and never put authorization into the pane shell env.
      const receipt = openSync(join(directory, "herdr-prefill-consumed"), "wx", 0o600);
      closeSync(receipt);
    } catch {
      return; // missing, mismatched, unwritable, or already consumed: no prefill
    }
    ctx.ui.setEditorText(marker.data.draftText);
  });
}
