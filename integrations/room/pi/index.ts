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
  const receiver = new RoomReceiver({ ...pi, sendMessage: context.sendMessage }, env, { call: context.call });
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
    name: "room_reply",
    label: "Room reply",
    description: "Save one answer or refusal to the currently delivered human room question. Maximum 8192 UTF-8 bytes. Never retry an uncertain submission. Socket and author identity are fixed by the receiver, not tool arguments.",
    promptSnippet: "Answer the active shared room question once",
    parameters: Type.Object({ delivery_id: Type.String({ minLength: 1 }), text: Type.String({ minLength: 1, maxLength: 8192 }) }, { additionalProperties: false }),
    execute: (_id, params, signal, _update, ctx) => receiver.reply(params, ctx, signal),
  });
}
