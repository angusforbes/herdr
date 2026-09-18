import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { Box, Text } from "@earendil-works/pi-tui";
import { contextualRoomTransport, boundedMessages, visibleRoomQuestion } from "./room-context.mjs";
import { RoomReceiver, registerRoomLifecycle } from "./receiver.mjs";

// Join only the explicitly supplied Herdr socket/pane. Never infer a socket,
// spawn a server, use inherited PI_SESSION_ID, or inject input into a PTY.
export default function (pi: ExtensionAPI) {
  // Pi reloads this TS factory, but native .mjs dependencies can stay cached.
  // Decide policy here so even a cached opt-in-only receiver gets the new
  // default. Do not mutate process.env; explicit opt-out survives reload.
  const env = { ...process.env };
  env.HERDR_ROOM_ENABLED = env.HERDR_ROOM_ENABLED !== "0"
    && env.HERDR_SOCKET_PATH?.startsWith("/") === true
    && typeof env.HERDR_PANE_ID === "string" && env.HERDR_PANE_ID.length > 0
    ? "1" : "0";
  const context = contextualRoomTransport(pi);
  // Keep arrival policy in this fresh TS closure: both native adapters may
  // already be cached in a Pi upgraded through /reload. This runs inside the
  // receiver's serialized RPC slot; recursively calling receiver.rpc deadlocks.
  const attemptedArrivals = new WeakSet();
  const introductionNames = new WeakMap();
  const receiver = new RoomReceiver({ ...pi, sendMessage: context.sendMessage }, env, {
    call: async (socket, method, params, options) => {
      const g = receiver.run;
      const result = await context.call(socket, method, params, options);
      if (method === "room.get" && g && receiver.valid(g)) {
        const member = result.members?.find(m => m.pane_id === env.HERDR_PANE_ID && m.session === g.session);
        if (member) introductionNames.set(g, { workspace: params.workspace_id, member });
      }
      if (method === "room.delivery.register" && !options?.signal?.aborted && result.receiver_id && result.server_epoch
          && g && receiver.valid(g) && params.session === g.session && !attemptedArrivals.has(g)) {
        attemptedArrivals.add(g);
        try {
          const known = introductionNames.get(g);
          const member = known?.workspace === params.workspace_id && known.member.terminal_id === params.terminal_id ? known.member : undefined;
          const name = member?.name?.trim();
          const text = name && name !== member.pane_id && name.toLowerCase() !== member.agent?.toLowerCase()
            ? `Hi, I'm ${name}.` : "Hi, I'm here. I'll introduce myself once I've chosen a name.";
          const ack = await context.call(socket, "room.agent.post", {
            workspace_id: params.workspace_id, pane_id: params.pane_id,
            terminal_id: params.terminal_id, session: params.session,
            // Every lifecycle/reload intentionally posts a fresh introduction.
            // The durable once-per-session arrival mode is not used here.
            text,
          }, options);
          if (ack.persistence !== "saved" || !Number.isSafeInteger(ack.sequence) || ack.sequence < 1) throw new Error("Arrival not confirmed saved");
        } catch {
          // Old/offline servers and uncertain notices must not freeze a valid
          // receiver registration. No model message, inference, or silent retry.
          try {
            if (receiver.valid(g) && g.ctx.hasUI) g.ctx.ui.notify("Room arrival not confirmed; receiver remains available. No automatic retry.", "warning");
          } catch { /* a closing UI cannot poison registration */ }
        }
      }
      return result;
    },
  });
  registerRoomLifecycle(pi, receiver);
  pi.registerMessageRenderer("room-question", (message, { outputPad }, theme) => {
    const box = new Box(outputPad, 1, text => theme.bg("userMessageBg", text));
    box.addChild(new Text(theme.fg("userMessageText", visibleRoomQuestion(message)), 0, 0));
    return box;
  });
  pi.registerTool({
    name: "room_read",
    label: "Read room",
    description: "Read attributed shared-room messages, including other agents' replies. Read-only: never triggers agents. Workspace and socket come from this Pi's room binding. Use after_sequence to page forward; limit defaults to 20, max 40. Private agent conversations are not included.",
    parameters: Type.Object({ after_sequence: Type.Optional(Type.Integer({ minimum: 0 })), limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 40 })) }, { additionalProperties: false }),
    execute: async (_id, params, signal, _update, ctx) => {
      const g = receiver.run;
      if (!g || !receiver.valid(g, ctx) || !g.workspace) throw new Error("No current room binding");
      const after = params.after_sequence ?? 0;
      const result = await receiver.rpc(g, "room.read", { workspace_id: g.workspace, after_sequence: after, limit: params.limit ?? 20 }, signal);
      if (!receiver.valid(g, ctx) || result.room_id !== `room:${g.workspace}`) throw new Error("Room binding changed");
      const messages = boundedMessages(result.messages);
      const next = messages.at(-1)?.sequence ?? after;
      return { content: [{ type: "text", text: JSON.stringify({ room: g.workspace, messages, next_after_sequence: next, more: next < result.next_sequence - 1 }) }], details: { workspace_id: g.workspace, next_after_sequence: next } };
    },
  });
  pi.registerTool({
    name: "room_post",
    label: "Post to room",
    description: "Share a spontaneous attributed contribution in the current room, without a delivered question. Maximum 8192 UTF-8 bytes. Does not prompt other agents or end your current task. Never retry an uncertain submission; inspect room_read instead. Socket and author identity are fixed by the receiver.",
    promptSnippet: "Share a bounded contribution in the current shared room",
    parameters: Type.Object({ text: Type.String({ minLength: 1, maxLength: 8192 }) }, { additionalProperties: false }),
    execute: async (_id, params, signal, _update, ctx) => {
      const g = receiver.run;
      if (!g || !receiver.valid(g, ctx) || !g.workspace || !g.member) throw new Error("No current room binding");
      if (signal?.aborted) throw new Error("Room post cancelled before submission");
      if (!params.text.trim() || Buffer.byteLength(params.text) > 8192) throw new Error("Post must contain 1..8192 UTF-8 bytes");
      try {
        const result = await receiver.rpc(g, "room.agent.post", {
          workspace_id: g.workspace, ...g.member, text: params.text,
        }, signal);
        if (!receiver.valid(g, ctx) || result.persistence !== "saved" || !Number.isSafeInteger(result.sequence) || result.sequence < 1) throw new Error("Uncertain room post acknowledgement");
        return { content: [{ type: "text", text: "Post saved in the shared room. No agents were prompted." }], details: { sequence: result.sequence, workspace_id: g.workspace } };
      } catch {
        throw new Error("Room post not confirmed; do not retry. Inspect room_read.");
      }
    },
  });
  pi.registerTool({
    name: "room_reply",
    label: "Room reply",
    description: "Save one answer or refusal to the currently delivered human room question. Maximum 8192 UTF-8 bytes. Never retry an uncertain submission. Socket and author identity are fixed by the receiver, not tool arguments.",
    promptSnippet: "Answer the active shared room question once",
    parameters: Type.Object({ delivery_id: Type.String({ minLength: 1 }), text: Type.String({ minLength: 1, maxLength: 8192 }) }, { additionalProperties: false }),
    execute: (_id, params, signal, _update, ctx) => receiver.reply(params, ctx, signal),
  });
}
