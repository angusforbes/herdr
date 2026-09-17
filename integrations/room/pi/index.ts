import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { RoomReceiver, registerRoomLifecycle } from "./receiver.mjs";

// Local prototype, intentionally opt-in. Never infer a socket from a default,
// spawn a server, use inherited PI_SESSION_ID, or inject input into a PTY.
export default function (pi: ExtensionAPI) {
  const receiver = new RoomReceiver(pi, process.env);
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
