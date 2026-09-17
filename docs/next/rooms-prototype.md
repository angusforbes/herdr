# Rooms prototype (local fork; not released)

Each **workspace**, not a grouped worktree space, owns a human-owned room. Desktop chrome pins `room` before terminal tabs. It is not a terminal tab, has no pane or PTY, and does not change terminal tab IDs or numbering. A terminal tab remains required by existing workspace invariants.

**Saved human questions now queue runtime-only delivery to online Pi receivers.** Omit the recipient to address all current agents once, or choose one exact session. Other agents, unidentified sessions and offline Pi receivers are visibly unavailable, not silently routed through terminal input. There is no PTY injection, coordinator, collector, round loop or reply-to-reply fanout. Reads, opening/joining, registration and restart never create work. A separately enabled polling Pi adapter must claim the question and request its model turn; `queued` or `submitted` is not proof of a model answer. Automated disposable real-model and owned-PTY handoff checks are described below; human physical UI validation and production approval remain outstanding.

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
- **Tab** cycles one recipient through all current members, then back to **all** (the default). Enter snapshots current members once; later joiners never receive old questions. A stale selected binding is rejected, not silently rebound. Its first Tab clears to all with an explicit status message; the next Tab selects the first current member. A selected member without live session identity is rejected; a broadcast still records that member as unavailable in runtime status.
- Type/paste/IME into the composer; **Enter** saves and queues. PgUp/PgDn and wheel scroll the transcript. Scroll is clamped during geometry computation to the actual wrapped transcript viewport. Keys held before entry still release to their original terminal; room presses/repeats/text never enter its hidden PTY. The composer is append/backspace only, capped at 8192 UTF-8 bytes; pasted control characters, including newlines, are omitted. Full API text is preserved in storage; display control characters are filtered.
- Success says `saved` or explicitly `memory_only` for `--no-session`, plus actual queued/unavailable counts. The status line summarizes the latest runtime question's delivery states and a failure reason; member rows distinguish `Pi receiver online`, offline, unsupported and missing live identity. Failed saves leave the draft and transcript unchanged and create no deliveries.
- Membership API reads are immediately current. The visible member list refreshes outside render at most four times per second while a room is visible. The list is windowed; Tab visits every member.
- The same workspace's draft survives leaving/re-entering its room. This first client stores only one workspace's draft/view at a time. Drafts, selection and scrolling are not persisted.
- Desktop always reserves the room/tab strip even with `hide_tab_bar_when_single_tab`; headless/terminal height is consequently one row smaller in that configuration. Mobile retains its existing header (no pinned room chip); the keyboard room path remains available.

## Neutral JSON API

Requests use the existing newline-delimited API socket. New methods are additive; the binary TUI wire protocol is unchanged. The generated schema is `docs/next/api/herdr-api.schema.json`. Workspace-addressed methods require the exact stable workspace ID from `workspace.list`; numeric/ordinal aliases such as `1` and `w_1` are rejected, so reordering cannot retarget a room write. Claim/report use their captured receiver ID and server epoch instead.

- `room.get {workspace_id}` → stable `room:<workspace-id>`, `members`, `next_sequence`, `outbound_delivery: pi_polling_at_most_once`, `deliveries: [{delivery_id,request_sequence,recipient,status,detail?}]`, and `receivers: [{member,available,detail}]`.
- `room.read {workspace_id, after_sequence?, limit?}` → messages strictly after that sequence, at most 100 (default 100), plus the room's next sequence. `limit: 0` returns zero messages with current room ID/next-sequence metadata. Continue from the last returned message, not the global next sequence, when paging. Reading never prompts.
- `room.post {workspace_id, text, recipient?}` → a stored human-owner question. Omitted recipient snapshots all current members; explicit recipient is `{pane_id, terminal_id, session}` taken from `room.get`, validated again at acceptance. The transcript's `recipients` captures session-identified broadcast targets; runtime unavailable entries also show visible members lacking a live session. An empty room stores a question with no addressed audience and never replays it. A request is identified by **room ID + its message sequence** and expires after ten minutes.
- `room.reply {workspace_id, request_sequence, pane_id, terminal_id, session, text}` → one direct, attributed reply/refusal per original recipient session to that human request. Current membership, original terminal/session binding, expiry and per-terminal/session duplicate reply are validated. A reply cannot itself be addressed. No automatic retry or fanout.

Success for a write includes `sequence`, `persistence`, `queued`, `unavailable` and `outbound_delivery: pi_polling_at_most_once`. Replies report zero newly queued/unavailable entries. API errors include `workspace_not_found`, `invalid_recipient`, `invalid_room_post`, `invalid_room_reply`, `room_save_failed`, `room_delivery_full`, `invalid_room_receiver` and `invalid_room_delivery`.

### Volatile receiver contract

- `room.delivery.register {workspace_id,pane_id,terminal_id,session,receiver_nonce}` → `{receiver_id,server_epoch}`. Exact current Pi identity is required. Same live nonce is idempotent; another nonce invalidates the previous receiver and its pending work, never transfers/retries it. Tokens are UUIDv8 correlation IDs built with existing std/SHA-256 facilities, not authentication credentials.
- `room.delivery.claim {receiver_id,server_epoch,ready}` → `{delivery:null|{delivery_id,workspace_id,request_sequence,recipient,text,expires_unix}}`. Registration lasts 15 seconds; each valid claim renews it, including `ready:false`. `false` only heartbeats. Claim marks the job before responding: a lost response is never reoffered. One claimed/submitted job holds the receiver's slot; subsequent claims cannot replace it.
- `room.delivery.report {receiver_id,server_epoch,delivery_id,outcome,detail?}` → `{accepted:true}`. Outcomes are `submitted`, `unanswered`, `failed`; bounded detail keeps at most 512 non-control characters. Submitted retains the active slot. Unanswered/failed clears it. A successfully **persisted** `room.reply` marks the matching entry replied and clears the server slot. Late/duplicate reports cannot resurrect terminal outcomes. The adapter must independently remain busy until its turn settles, not merely until its reply tool succeeds.
- Every receiver operation revalidates current workspace/pane/terminal/session and epoch. Session change, movement, receiver expiry or replacement invalidates the token. Queued jobs then become unavailable; claimed/submitted jobs become failed with unknown outcome and no retry. Request expiry marks pending jobs expired and releases the slot. Never retry uncertain model submission automatically.
- Status values: `queued`, `claimed`, `submitted`, `replied`, `unanswered`, `failed`, `unavailable`, `expired`. Queued means server inbox only; submitted means adapter-reported dispatch, not successful inference. Unsupported/offline members remain visible.
- Runtime lives in `App`, **not** persisted `AppState`/snapshots. Restart/handoff loses all receivers, inbox and statuses; transcripts remain. Registration/read/join never rebuilds work from history. Cleanup is lazy on delivery/get/post/reply operations and visible-room refresh, with no filesystem/network work in rendering. Closed workspaces are removed at cleanup. Capacity: 1024 live receivers and 8192 delivery records globally; posts that exceed capacity are rejected before save. Status records are removed ten minutes after request expiry. No retry UI is implemented.

### Pi opt-in commands (local adapter)

The extension always registers `/room-enable`, `/room-disable` and `room_reply`, but opens **no room sockets or timers while inactive**. `room_reply` fails unless this receiver has a matching active delivery. The disposable wrapper retains startup opt-in via `HERDR_ROOM_ENABLED=1`; simply installing/loading the extension in other agents does not opt them in.

For an already-running Pi without that startup flag:

1. The approved receiver files must be in a Pi auto-discovered extension directory. This fork does not install them globally. Resource exclusions or `--no-extensions` can prevent loading.
2. In that agent's **current TUI**, run `/reload`, then `/room-enable`. These are slash commands, not model prompts. No process/session restart is required.
3. Its existing environment must already contain the **exact intended absolute `HERDR_SOCKET_PATH` and `HERDR_PANE_ID`**. No default socket, parent `PI_SESSION_ID`, or guessed pane is used. A working Herdr live-session hook and the candidate room API are required; discovery matches the current Pi session-manager identity to the live pane/terminal/session tuple. Missing prerequisites fail closed or remain waiting, never silently switch routes.

`/room-enable` opts in only this extension instance; it does not modify `process.env`, config, other Pi agents or the server. Repeating it is idempotent, not a way to retry uncertain work. `/room-disable` cancels its timer, aborts I/O and invalidates the generation immediately. It does **not** abort an already-started model turn, undo a saved reply or retry uncertain delivery. The server may show the old receiver online until its 15-second heartbeat TTL expires; re-enabling replaces its nonce and invalidates old pending work rather than replaying it.

Runtime opt-in is not persisted: `/reload`, `/new`, `/resume` and `/fork` replace the Pi extension instance. Without the startup flag, run `/room-enable` again. With `HERDR_ROOM_ENABLED=1`, the new instance starts automatically even if the previous instance was disabled. An uncertain/stale receiver remains conservative: inspect the room before reload and explicit re-enable. Only **fresh human room posts** create delivery; enabling, registration, reading and rejoining never wake a model or replay history.

The adapter uses idle/modal gating, generation cancellation, one network operation in flight and `room_reply`, never terminal input injection. This is a trusted local integration, not an isolation/authentication boundary.

### Standalone manual helper

Use the wrapper's explicit socket, workspace ID and current pane ID in these templates:

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE get
```

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE read --after 0
```

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE post "question for all current agents"
```

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE post --to PANE "question for this session"
```

```sh
python3 integrations/room/room.py --socket SOCKET WORKSPACE reply REQUEST_SEQUENCE --pane PANE "reply or refusal"
```

`-` instead of text reads stdin. The helper queries current identity before addressed posts/replies and sends directly to Herdr's API, never an initiating Pi collector. It does not start servers or automatically retry writes. An agent may explicitly use `get/read/reply` during an already-authorized turn; installing this helper does not activate it. Manual read/reply needs no Pi receiver; automatic delivery does. The Python helper is Unix-only; core Rust code has no new platform-specific API.

## Storage and identity contracts

- `Workspace::room` is shared runtime state, included in version-3 structural snapshots and both ordinary restore and handoff capture. Old snapshots default to an empty room. No EventHub retention is used as storage.
- Writes clone/validate a candidate, join any earlier background snapshot writer, synchronously save the structural snapshot, then publish/acknowledge it. This is deliberately a bounded prototype tradeoff: explicit writes can briefly block the app; there is no disk I/O in rendering or pane loops. Save failures return errors without advancing in-memory sequence. `--no-session` explicitly reports volatile memory storage.
- `saved` means the existing session writer successfully replaced the JSON file atomically. It does **not** promise power-loss durability/fsync. A disconnect/timeout after a write has unknown outcome: read before deciding whether to retry. Posts do not yet have client-supplied idempotency keys; accepted replies are durably deduplicated by original request sequence plus terminal/session identity. Legacy messages lacking `recipients` retain their original single-recipient reply/dedup behavior.
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

Node receiver/lifecycle tests:

```sh
node --test integrations/room/test_receiver.mjs
```

Opt-in Linux installed-to-candidate real-model handoff test (private sockets/config/auth/session copies, five harmless Haiku turns, deletes private data):

```sh
python3 integrations/room/handoff_smoke.py
```

The handoff harness checks retained Pi PIDs/start times/PTYs/session IDs, a real streamed assistant message beginning before handoff and finishing afterward with the same JSONL parent, and hot-loading an initially non-opted-in Pi via `/reload` + `/room-enable` before its persisted attributed room reply. It also attaches the actual installed Herdr TUI on an owned PTY, observes handoff disconnect, **explicitly relaunches the client on that PTY**, then verifies render, input and an actual Pi response. Installed Herdr exits on handoff; automatic same-process client reconnect is **not** supported/proven. This is automated PTY/ANSI rendering evidence, not human physical-key/pixel validation, production scale or release-build verification. Cleanup authenticates/stops the imported server even after the original server Popen exits.

The earlier passive smoke test starts/stops the debug binary twice on temporary sockets and verifies saved-before-ack, read-by-sequence, stable room identity, restored transcript, and empty shell membership; it is not evidence of automatic model delivery. Current Rust tests additionally exercise broadcast capture, targeted delivery, receiver registration/replacement/heartbeat/expiry, lost-response at-most-once claims, submitted slot retention, reply/unanswered/failed release, bounded state/detail, current-session checks, restart/read/join non-replay, and save failure before dispatch/status updates. Existing legacy snapshot/reply dedup, input isolation, geometry and transcript-cache tests remain. No real model, live user session or physical UI was tested by the Rust writer. See `HANDOFF.md` for focused results and the earlier full-suite baseline/performance receipts; no new scaling work was added to pane render loops.
