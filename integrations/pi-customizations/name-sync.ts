/**
 * name-sync — one `/name` sets the whole identity, and never touches subagents.
 *
 * Pi's built-in `/name` only sets the session display label. This extension
 * mirrors that into the Herdr sidebar + tab via `herdr-name` (which writes both
 * together), so session label, sidebar and tab stay in sync. It also nudges the
 * agent to pick an icon/colour and post its room introduction.
 *
 * Guards (both cheap, no shelling out):
 *  - Session start reads `getHeader().parentSession`. If this session has a
 *    parent (a subagent, or a fork) name-sync stays completely inert — it must
 *    not repaint the pane its parent is living in.
 *  - A zero-cost `#<8hex>` pattern backstop drops the auto-generated subagent
 *    session names (`general-purpose#a10f8089`) in case an event from a child
 *    session is ever delivered to a top-level instance.
 *
 * Deliberately NOT here (removed after they destabilised sessions on 2026-09-18):
 *  - no `input` event hook (it ran on every keystroke),
 *  - no `herdr pane list` shell-out (it ran on every rename).
 * The only child process spawned is `herdr-name`, and only when a top-level
 * agent is actually renamed.
 *
 * `/name` forms: `Blaze` literal · `"Blaze Runner"` quoted literal ·
 * `?`/`? <hint>` choose-from-context · a bare multi-word phrase is a hint.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(execFile);

type Style = { icon?: string; color?: string; bold?: boolean; italic?: boolean; dim?: boolean };

export default function nameSync(pi: ExtensionAPI) {
  let selfSet: string | null = null; // swallow the echo from our own setSessionName
  let inert = false; // set at session_start: this session has a parent -> do nothing
  let currentName: string | null = null;

  const looksLikeSubagentLabel = (n: string) => /#[0-9a-f]{8}$/i.test(n);

  const detectParent = (ctx: any): boolean => {
    try {
      const p = (ctx?.sessionManager?.getHeader?.() as any)?.parentSession;
      return typeof p === "string" && p.length > 0;
    } catch {
      return false;
    }
  };

  pi.on("session_start", async (_event: any, ctx: any) => {
    inert = detectParent(ctx);
    try {
      currentName = pi.getSessionName?.() ?? null;
    } catch {
      currentName = null;
    }
  });

  const styledArgs = (name: string, s: Style): string[] => {
    const a = [s.icon ? `${s.icon} ${name}` : name];
    if (s.color) a.push("--fg", s.color);
    if (s.bold) a.push("--bold");
    if (s.italic) a.push("--italic");
    if (s.dim) a.push("--dim");
    return a;
  };

  // setLabel=false when Pi's built-in /name already set the session label and we
  // only need to sync the sidebar/tab.
  async function publish(name: string, style: Style, ctx: any, setLabel: boolean): Promise<boolean> {
    const clean = name.trim();
    if (!clean || clean.includes("\n") || clean.length > 60) {
      ctx?.ui?.notify?.("A name must be 1..60 characters on a single line.", "warning");
      return false;
    }
    if (setLabel) {
      selfSet = clean;
      try {
        pi.setSessionName(clean);
      } catch { /* label is best-effort */ }
    }
    try {
      await run("herdr-name", styledArgs(clean, style), { env: process.env });
      currentName = clean;
      ctx?.ui?.notify?.(`Renamed to "${clean}" — sidebar, tab and session label are in sync.`, "info");
      return true;
    } catch (err: any) {
      ctx?.ui?.notify?.(
        `Label is "${clean}", but herdr-name failed (${err?.message ?? err}). Run: herdr-name '${styledArgs(clean, style).join(" ")}'`,
        "warning",
      );
      return false;
    }
  }

  function renameNudge(prev: string | null, next: string, hadIcon: boolean, ctx: any) {
    const styleAsk = hadIcon
      ? ""
      : `Give yourself a fitting icon and colour by calling rename_self with name "${next}" plus an icon and a #rrggbb colour. `;
    const intro = prev && prev !== next
      ? `Then post one short room notice with room_post, led by your icon, e.g. "\ud83d\udd27 ${prev} is now ${next}."`
      : `Then post one short room_post introduction led by your icon, e.g. "Hi, I'm \ud83d\udd27 ${next}."`;
    try {
      if (ctx?.isIdle?.() ?? true) pi.sendUserMessage(`You are now "${next}". ${styleAsk}${intro}`);
      else pi.sendUserMessage(`You are now "${next}". ${styleAsk}${intro}`, { deliverAs: "followUp" });
    } catch { /* nudge is best-effort */ }
  }

  function chooseNudge(ctx: any, hint?: string) {
    const basis = hint
      ? `Base it on this hint: "${hint}".`
      : "Base it on what this session is about; if that is unclear, ask me the topic first.";
    try {
      pi.sendUserMessage(
        `Choose a concise room name for yourself (usually one word). ${basis} ` +
          `Then call rename_self with the chosen name plus a fitting icon and a #rrggbb colour, ` +
          `and post a one-line room_post introduction led by that icon.`,
      );
    } catch { /* best-effort */ }
  }

  function revertLabel() {
    if (currentName != null) {
      selfSet = currentName;
      try {
        pi.setSessionName(currentName);
      } catch { /* best-effort */ }
    }
  }

  pi.on("session_info_changed", async (event: any, ctx: any) => {
    if (inert) return; // subagent or fork: never touch the pane's identity
    const name = event?.name?.trim();
    if (selfSet !== null && name === selfSet) {
      selfSet = null; // our own echo
      return;
    }
    selfSet = null;
    if (!name) return; // cleared
    if (looksLikeSubagentLabel(name)) return; // backstop vs stray subagent events
    if (name === "?" || name === "??") {
      revertLabel();
      return chooseNudge(ctx);
    }
    if (name.startsWith("? ")) {
      revertLabel();
      return chooseNudge(ctx, name.slice(2).trim());
    }
    const quoted = name.match(/^(["'])(.+)\1$/);
    if (quoted) {
      const inner = quoted[2].trim();
      const prev = currentName;
      if (await publish(inner, {}, ctx, true)) renameNudge(prev, inner, false, ctx);
      return;
    }
    if (/\s/.test(name)) return chooseNudge(ctx, name); // multi-word = hint
    // single-token literal: Pi already set the label; just sync sidebar/tab.
    const prev = currentName;
    if (await publish(name, {}, ctx, false) && name !== prev) renameNudge(prev, name, false, ctx);
  });

  pi.registerTool({
    name: "rename_self",
    label: "Rename self",
    description:
      "Change your own identity: sets the Pi session label and the Herdr sidebar/tab name together, " +
      "with an optional icon and colour. Subagent/child sessions are refused. Use sparingly; do not " +
      "rename an agent with an established identity without the user's ok, and post a room_post notice after.",
    promptSnippet: "Rename yourself (session label + Herdr identity together)",
    parameters: Type.Object(
      {
        name: Type.String({ minLength: 1, maxLength: 60, description: "New name (single line)." }),
        icon: Type.Optional(Type.String({ maxLength: 8, description: "Optional leading icon/emoji." })),
        color: Type.Optional(Type.String({ pattern: "^#[0-9a-fA-F]{6}$", description: "Optional #rrggbb sidebar colour." })),
        bold: Type.Optional(Type.Boolean()),
        italic: Type.Optional(Type.Boolean()),
        dim: Type.Optional(Type.Boolean()),
        reason: Type.Optional(Type.String({ maxLength: 200, description: "Why you are renaming (not displayed)." })),
      },
      { additionalProperties: false },
    ),
    execute: async (_id: string, p: any, _signal: any, _update: any, ctx: any) => {
      if (inert) {
        return {
          content: [{ type: "text", text: "Refused: this is a subagent/child session; it must not change the parent agent's identity." }],
          details: { applied: false, refused: "has-parent" },
        };
      }
      const ok = await publish(p.name, p, ctx, true);
      return {
        content: [{
          type: "text",
          text: ok
            ? `Identity is now "${String(p.name).trim()}" (session label + Herdr sidebar/tab). Post a one-line room_post notice if peers knew you by another name.`
            : `Could not fully apply "${p.name}" — see the warning.`,
        }],
        details: { name: String(p.name).trim(), applied: ok },
      };
    },
  });
}
