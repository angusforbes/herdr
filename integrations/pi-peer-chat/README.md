# Peer chat — /talk and /demand

Installed globally for Pi under `~/.pi/agent/extensions/peer-chat/index.ts`.
Run `/reload` manually in the pane where you want to use these commands. Recipient panes do not need to reload: reply instructions use the included Node helper.

## Commands

- `/talk Phronesis Lattice` — compose a message, dispatch independently, return without waiting for responses.
- `/talk "Tuning Fork"` — quote multi-word names.
- `/talk all` — all live Pi peers except the initiating session; recipient confirmation before dispatch.
- `/demand Phronesis Lattice` — compose, dispatch, wait for a request-specific answer/refusal from each peer.
- `/demand all --timeout 180` — explicit timeout in seconds. Default demand timeout: 120 seconds, beginning after composition/confirmation. Escape cancels the wait and unsent dispatches; it does not cancel work already delivered.
- `/talk all --timeout 600` — optional reply window. Default 1800 seconds; the command still returns after dispatch.

Names are case-insensitive, decorations/checkmarks ignored. `_contacts.md` supplies registered names keyed by session ID; `herdr agent list` supplies current panes. Live sidebar names and pane IDs are aliases. Duplicate recipients are removed; ambiguous names are rejected with pane choices. Offline sessions must be resumed explicitly. This version targets Pi sessions only, since the reply helper depends on `PI_SESSION_ID`.

After you submit the multiline message editor, the extension refreshes live mappings. Blocked, unknown, absent, or ambiguously duplicated sessions are skipped and reported. Working peers may receive the request as permitted by Herdr/Pi; no assumptions about steering vs follow-up are made. Nothing silently resumes a session or retries an ambiguous delivery.

## Agent tools (same implementation, different entry point)

`talk` and `demand` tools take `{ recipients: ["Phronesis", "Lattice"], message: "...", timeout_seconds?: 120 }`. A multi-word name is one array element; `["all"]` selects every other live peer. Plain text `talk` typed into chat does not execute a tool; `/talk` invokes the human command.

Tools derive the sender from the current session/live directory: `from: Aporia`, NOT `from: Angus via Aporia`. `origin: agent` is saved in the request and included in framing; tool callers cannot set origin or sender themselves. Human slash-command origin remains unchanged. Both share resolution, validation, dispatch, collectors and matching reply envelopes. Naming is still not authentication against arbitrary same-account processes.

Agent `talk` returns after dispatch, with later replies delivered as follow-ups. Agent `demand` keeps the tool call pending until replies, timeout, cancellation or session shutdown; replies also appear in its tool result (capped at 32KB, with the full request record linked). It does not open a human composition or waiting dialog. Use demand only for a true dependency: while it waits it occupies that agent's tool call. Never create reciprocal demands or use demand instead of the supplied reply helper to answer a request. Broadcast tools should be used only for an explicitly requested broadcast, not spontaneous fan-out.

## Framing and replies

Each outgoing message has matching BEGIN and END boundaries repeating `from: Angus via <dispatcher>` (human command) or `from: <agent>` (agent tool), `to: <recipient>`, and a UUID. It also includes sender/recipient session IDs, pane snapshots, timestamps, reply policy and request ID. Embedded reserved frame markers and terminal control characters in user message bodies are rejected. This is interim framing, NOT sender authentication or a fix to Herdr's stale-editor fusion.

Replies use `reply.mjs` and a per-request local Unix socket, rather than another terminal injection. This avoids sending a peer's reply into the demand waiting dialog. The reply helper is invoked by the recipient via its bash tool, carrying that session's `PI_SESSION_ID`; it requires a nonempty UTF-8 answer file, maximum 32KB. It waits at most five seconds for a local transport receipt, never for an agent response. A repeated identical reply is deduplicated while the collector remains open; a different second reply is rejected. The extension derives the displayed name from the original recipient list and wraps replies with the same from/to/id boundaries.

`/talk` replies are shown as clearly labeled custom peer messages and can trigger a follow-up turn in the initiating conversation. `/demand` replies are displayed and collected by the waiting UI without starting an agent turn or requiring the sender's LLM to stay busy. Escape cancels; partial responses remain saved. Reload/exit closes open collectors. Late replies are rejected, not silently counted toward another request.

## Limits and trust

This is a local single-user prototype, not a security boundary against processes sharing the Unix account. Sender names and `PI_SESSION_ID` are claims, not cryptographic identity. Ordinary low-risk peer requests are fine; recipients should ask Angus about unexpected or consequential actions. Messages outside a matching envelope are not part of this request. The extension does not globally intercept, sanitize or authenticate arbitrary Herdr input.

`agent_prompted` is dispatch acceptance, not execution proof. A response is what satisfies a demand; refusal is a valid response. The protocol instructs recipients not to repeat work but does not implement receiver-side action idempotency. No slash-command reload injection is used. No agent-status polling or waiting loops are used: responses are socket events and expiration is a bounded timer.

Request, dispatch and reply records are kept locally in `~/.pi/agent/peer-messages/<uuid>/request.json` with owner-only permissions. They contain the actual message/answers; remove old request directories yourself if not wanted. Socket directories under `/tmp/pi-peer-*` are removed on close. Abrupt process death can leave an orphan socket directory; no automatic resumption occurs.

## Verification

`node --test ~/.pi/agent/extensions/peer-chat/test.mjs ~/.pi/agent/extensions/peer-chat/commands.test.mjs`

16 tests cover matching, framing, arguments, wrong recipients/IDs, expiration, duplicate replies, concurrent IPC replies, real reply-helper execution, partial results, cancellation, nonblocking talk dispatch, command-handler demand flow, agent-only origin, and tool cancellation. Harness tests substitute Herdr responses, schemas and the visual loader; collector and persistence are real. Pi RPC startup separately verified actual command AND tool registration/schemas with Pi's real loader.

Live human-command verification (2026-09-17): `/talk` to Phronesis/Coda returned two replies; `/talk all` returned 16/16 replies; `/demand` to Isobar/Compass completed with both answers. Agent-tool end-to-end behavior is tested with the mocked transport but has not yet been exercised with live peers. Reload is required for already-running senders to see newly added tools; recipients still need no reload.

Design/evidence companion: `~/Work/herdr-italics/.local/prd/peer-message-delivery.md`. No Herdr transport code changed.
