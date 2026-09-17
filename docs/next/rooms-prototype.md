# Rooms prototype (local fork; not released)

Each **workspace**, not a grouped worktree space, owns a passive room. Desktop chrome pins `room` before terminal tabs. It is not a terminal tab, has no pane or PTY, and does not change terminal tab IDs or numbering. A terminal tab remains required by existing workspace invariants.

**Outbound prompting is disabled.** This is a working shared transcript/manual-pull prototype, not an agent transport implementation. Neither joining, reading, posting, replying, restoring, nor opening the room invokes `agent.prompt`, launches inference, or broadcasts prompts. There is no coordinator, collector, round loop, watcher prompting agents, or reply-to-reply fanout. Existing terminal prompt transport has not been fixed.

## Try it safely

From this checkout, build the debug binary:

```sh
mise x zig@0.15.2 -- cargo build --locked
```

Launch the disposable TUI wrapper (temporary config, state, sockets and `/bin/sh`; agent resumption disabled; stops only its owned server on exit):

```sh
python3 integrations/room/disposable.py
```

The wrapper prints a one-line API helper command containing its explicit socket. It never installs a binary, touches global Pi extensions, or connects to inherited Herdr sockets. The disposable session and its transcript are deleted on exit. The wrapper's interactive UI has not been human-validated; its underlying server/start/stop/environment path is exercised by the smoke test.

## UI

- Click `room`, or use **Ctrl+Alt+R** from terminal/Spaces/Agents/Search focus.
- **Esc**, Ctrl+Alt+R, or clicking a terminal tab returns to terminals. Normal terminal tab indices and wheel navigation remain unchanged. API workspace/tab/pane/agent focus returns to terminals rather than reviving an old room selection. Panel-focus shortcuts remain available.
- **Tab** cycles one recipient through all current members, then back to `none`. None records a human note. Identified live-hook sessions can receive an **addressed record**, which must be read manually; no prompt is dispatched. A stale selected binding is rejected, not silently rebound. Its first Tab clears to `none` with a status message; the next Tab selects the first current member.
- Type/paste/IME into the composer; **Enter** records. PgUp/PgDn and wheel scroll the transcript. Scroll is clamped during geometry computation to the actual wrapped transcript viewport. Keys held before entry still release to their original terminal; room presses/repeats/text never enter its hidden PTY. The composer is append/backspace only, capped at 8192 UTF-8 bytes; pasted control characters, including newlines, are omitted. Full API text is preserved in storage; display control characters are filtered.
- Success says `saved` or explicitly `memory_only` for `--no-session`, and `No prompt dispatched`. Failed saves leave the draft and transcript unchanged.
- Membership API reads are immediately current. The visible member list refreshes outside render at most four times per second while a room is visible. The list is windowed; Tab visits every member.
- The same workspace's draft survives leaving/re-entering its room. This first client stores only one workspace's draft/view at a time. Drafts, selection and scrolling are not persisted.
- Desktop always reserves the room/tab strip even with `hide_tab_bar_when_single_tab`; headless/terminal height is consequently one row smaller in that configuration. Mobile retains its existing header (no pinned room chip); the keyboard room path remains available.

## Neutral JSON API

Requests use the existing newline-delimited API socket. New methods are additive; the binary TUI wire protocol is unchanged. The generated schema is `docs/next/api/herdr-api.schema.json`. Every room method requires the exact stable workspace ID from `workspace.list`; numeric/ordinal aliases such as `1` and `w_1` are rejected, so reordering cannot retarget a room write.

- `room.get {workspace_id}` → stable `room:<workspace-id>`, `members`, `next_sequence`, `outbound_delivery: disabled_manual_pull_only`.
- `room.read {workspace_id, after_sequence?, limit?}` → messages strictly after that sequence, at most 100 (default 100), plus the room's next sequence. `limit: 0` returns zero messages with current room ID/next-sequence metadata. Continue from the last returned message, not the global next sequence, when paging. Reading never prompts.
- `room.post {workspace_id, text, recipient?}` → a stored human-owner record. Recipient is `{pane_id, terminal_id, session}` taken from `room.get`, validated again at acceptance. A request is identified by **room ID + its message sequence**. Addressed requests expire after ten minutes.
- `room.reply {workspace_id, request_sequence, pane_id, terminal_id, session, text}` → one direct, attributed reply/refusal to that human request. Current membership, original terminal/session binding, expiry and duplicate reply are validated. A reply cannot itself be addressed. No automatic retry or fanout.

Success for a write includes `sequence`, `persistence`, and disabled outbound delivery. API errors include `workspace_not_found`, `invalid_recipient`, `invalid_room_post`, `invalid_room_reply`, and `room_save_failed`.

### Standalone manual helper

Use the wrapper's explicit socket, workspace ID and current pane ID in these templates:

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE get
```

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE read --after 0
```

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE post "unaddressed human note"
```

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE post --to PANE "manually read this request"
```

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE reply REQUEST_SEQUENCE --pane PANE "reply or refusal"
```

`-` instead of text reads stdin. The helper queries current identity before addressed posts/replies and sends directly to Herdr's API, never an initiating Pi collector. It does not start servers or automatically retry writes. An agent may explicitly use `get/read/reply` during an already-authorized turn; installing this helper does not activate it. No Pi extension deployment is needed. The Python helper is Unix-only; core Rust code has no new platform-specific API.

## Storage and identity contracts

- `Workspace::room` is shared runtime state, included in version-3 structural snapshots and both ordinary restore and handoff capture. Old snapshots default to an empty room. No EventHub retention is used as storage.
- Writes clone/validate a candidate, join any earlier background snapshot writer, synchronously save the structural snapshot, then publish/acknowledge it. This is deliberately a bounded prototype tradeoff: explicit writes can briefly block the app; there is no disk I/O in rendering or pane loops. Save failures return errors without advancing in-memory sequence. `--no-session` explicitly reports volatile memory storage.
- `saved` means the existing session writer successfully replaced the JSON file atomically. It does **not** promise power-loss durability/fsync. A disconnect/timeout after a write has unknown outcome: read before deciding whether to retry. Posts do not yet have client-supplied idempotency keys; accepted replies are durably deduplicated by original request sequence.
- Maximum 1000 messages, each 8192 bytes. A full room rejects new records rather than silently truncating history. No archive/delete UI exists yet.
- Closing/removing a workspace removes its room through existing snapshot semantics. Moving its last terminal away can close that source workspace under baseline behavior. There is no independent archive/tombstone after workspace removal.
- Members come from current panes and effective runtime agent identity, not names in `_contacts`, persisted pane addresses, or render-time process scans. Plain shells do not become members. All identified agents appear; only matching **live hook session identities** are currently reply-capable. Selecting a nonreplyable member reports `no live session identity` without hiding that member. Identity-only integrations/restored session metadata alone are deliberately not enough. Historical attribution is a cloned immutable record, unaffected by rename/move/replacement. Addressed requests with missing expiry fail closed. Outstanding requests are not rebound when restore/handoff recreates terminal IDs: their old binding becomes non-reply-capable even if the same agent session is later resumed. This favors rejecting stale replies over guessing identity continuity.
- The local Herdr API is a trusted control surface, **not authenticated per-agent provenance**. Any process with socket access can claim an owner post or supply another member's visible identity. These checks prevent accidental/stale correlations, not malicious local impersonation. This prototype must not be represented as an authorization boundary.

## Verification

```sh
mise x just@1.58.0 zig@0.15.2 aqua:nextest-rs/nextest/cargo-nextest@0.9.144 -- just test-one room
```

```sh
python3 -m unittest discover -s integrations/room -v
```

The smoke test starts/stops this debug binary twice on temporary sockets and verifies saved-before-ack, read-by-sequence, stable room identity, restored transcript, and empty shell membership. Rust tests exercise Pi lifecycle reports, moves/replacements, reply correlation/expiry/dedup, input isolation with a captured runtime channel, save rollback, legacy snapshots, tab invariants, and incremental Unicode transcript layout. No real model or live user session is involved. See `HANDOFF.md` for full-suite baseline failures and performance receipts.
