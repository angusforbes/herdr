import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
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
  const receiver = new RoomReceiver(pi, env);
  registerRoomLifecycle(pi, receiver);
  pi.registerTool({
    name: "room_reply",
    label: "Room reply",
    description: "Save one answer or refusal to the currently delivered human room question. Maximum 8192 UTF-8 bytes. Never retry an uncertain submission. Socket and author identity are fixed by the receiver, not tool arguments.",
    promptSnippet: "Answer the active shared room question once",
    parameters: Type.Object({ delivery_id: Type.String({ minLength: 1 }), text: Type.String({ minLength: 1, maxLength: 8192 }) }, { additionalProperties: false }),
    execute: (_id, params, signal, _update, ctx) => receiver.reply(params, ctx, signal),
  });
}
