# Rooms prototype (local fork; not released)

Each **workspace**, not a grouped worktree space, owns a human-owned room. Desktop chrome pins `room` before terminal tabs. It is not a terminal tab, has no pane or PTY, and does not change terminal tab IDs or numbering. A terminal tab remains required by existing workspace invariants.

**Saved human questions now queue runtime-only delivery to online Pi receivers.** The UI always addresses all current workspace agents once. The neutral scripting API also supports an explicit exact-session recipient. Other agents, unidentified sessions and offline Pi receivers are visibly unavailable, not silently routed through terminal input. There is no PTY injection, coordinator, collector, round loop or reply-to-reply fanout. Reads, opening/joining, registration and restart never trigger inference or replay delivery. The Pi adapter records one deterministic arrival notice per agent session per room; spontaneous agent posts are shared but never dispatched. A separately enabled polling Pi adapter must claim the question and request its model turn; `queued` or `submitted` is not proof of a model answer. Automated disposable real-model and owned-PTY handoff checks are described below; human physical UI validation and production approval remain outstanding.

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

## Composer revision: Pi-style editing (fork candidate, not deployed)

This section specifies the current composer controls. Reference: installed Pi **0.85.1** embedded `editor.ts`, `word-navigation.ts`, `kill-ring.ts`, `undo-stack.ts`, complete installed keybindings/TUI/terminal-setup docs, and version-tagged editor/word/history fixtures. This is a Rust implementation of the principal defaults, **not an assertion of exact Pi/ICU parity**. No transport or automatic-membership behavior changes.

- **Up/Down** move through wrapped visual editor rows, preserving a UTF-16 preferred offset snapped to whole graphemes. At the top, Up first moves a nonempty draft to logical line start, then browses older sent prompts. At the bottom, Down moves to line end or newer history. History retains 100 trimmed successful sends, newest first, consecutive duplicates omitted. Older entries open at start; newer entries at end. Advancing past newest restores the original draft and cursor. Editing/paste/undo/composer clicks leave history; keyboard cursor movement does not. History is retained per workspace presentation for this server lifetime, never seeded from agent replies or persisted.
- **Enter** submits expanded, trimmed text. **Shift+Enter / Ctrl+J** insert LF. Backslash immediately before Enter removes one backslash and inserts LF instead of sending. **Ctrl+Enter / Alt+Enter are unbound**. Failed validation/persistence preserves text, paste registry and cursor and does not enter history. Success clears draft/undo, retains kill ring/history, and follows latest transcript.
- **Left / Ctrl+B**, **Right / Ctrl+F** move by grapheme. **Home / Ctrl+Home / Ctrl+A** and **End / Ctrl+End / Ctrl+E** are current *logical line* boundaries, not document boundaries. **Alt+Left / Ctrl+Left / Alt+B**, **Alt+Right / Ctrl+Right / Alt+F** skip whitespace then move to the other edge of a word or punctuation run; newline boundaries cross alone. **Ctrl+] / Ctrl+Alt+]** then a printable character jump forward/backward.
- **Backspace / Shift+Backspace** and **Delete / Shift+Delete / Ctrl+D** delete graphemes. **Ctrl+W / Alt+Backspace**, **Alt+D / Alt+Delete** kill previous/next word; **Ctrl+U / Ctrl+K** kill to logical line start/end, including the adjoining LF when at the boundary. Consecutive kills prepend/append in a kill ring. **Ctrl+Y** yanks; **Alt+Y** rotates after yank. Ordinary deletion is not a kill. **Ctrl-minus** undoes; Ctrl+Z and Ctrl+Backspace are not editor defaults. Typing coalesces until whitespace/movement; paste is one undo operation; undo snapshots include cursor and paste registry, including the pre-history draft.
- Clipboard text uses **Ctrl+V** or **Ctrl+Shift+V** and existing bracketed paste; no image support. CRLF/CR become LF, tabs become **four spaces**, control characters other than LF are filtered. A pasted path beginning `/`, `~`, or `.` receives a separating space after an ASCII word character. More than 10 lines or 1000 UTF-16 units fold into registered `[paste #N …]` markers. Left/right/word/delete/kill/mouse/undo respect registered atoms; typed lookalikes stay literal. The **8192-byte cap applies to expanded payload**, with only complete input graphemes accepted; markers cannot bypass it. Sending expands only registered ranges.
- Two horizontal rules, no side borders/title/padding; wrap width is available width minus one cursor cell. Word-aware wrapping retains whitespace, supports CJK break opportunities and grapheme-wraps long words. Exact-width lines do not gain an empty cursor row. Input grows from one row to `max(5, floor(terminalHeight × .3))`, clamped to available geometry; border indicators show hidden rows. The active grapheme (or end blank) is inverse video. Stream cursor metadata retains the IME position with hardware visibility false; real terminal IME behavior has not been physically verified.
- Room-specific routing is intentional: plain **PgUp/PgDn** scroll transcript five rows; **Ctrl+PgUp/PgDn** move the editor by its maximum page size; wheel scrolls transcript three rows. Home/End remain editor keys, unlike fullscreen Pi's transcript routing. **Tab is unbound and consumed** (no recipient selection or hidden-pane focus); panel-focus shortcuts, room toggle and **Ctrl+C / Ctrl+Shift+C transcript selection copy** remain intact. Old key-release leases still finish only at their original PTY.

**Known differences / bounded scope:** Rust UAX #29 segmentation is not ICU dictionary segmentation (e.g. Han words are per-character rather than Pi's `你好`/`世界`); non-ASCII punctuation, CJK break classifications and some emoji/resize sticky-column edge cases are not exhaustively equivalent. Marker IDs are monotonic until submit (no Pi backspace renumbering); positional registration intentionally prevents a typed copy of an existing ID from expanding. Killed folded content yanks as expanded text, not a restored folded marker; normal undo restores folding/registry. Narrow split-marker vertical navigation is safe/atomic but not exhaustively matched to Pi's continuation-row behavior; cursor inverses the displayed grapheme, not an entire marker. Undo and kill ring retain at most 100 entries each (Pi's are unbounded); C1 controls are filtered as well as C0. No custom Pi keybinding configuration, images, external editor, slash commands, completion, thinking borders or orchestration. No new dependencies or pane-scaled work; geometry remains outside pure rendering, transcript wrapping remains incremental. Physical TUI/IME testing remains outstanding.

## UI

- Click `room`, or use **Ctrl+Alt+R** from terminal/Spaces/Agents/Search focus.
- **Esc**, Ctrl+Alt+R, or clicking a terminal tab returns to terminals. Normal terminal tab indices and wheel navigation remain unchanged. Workspace navigation restores that workspace's last room/terminal view. Explicit tab/pane/agent focus returns the destination workspace to terminals without erasing the source workspace's view. Panel-focus shortcuts remain available.
- The room is a normal group conversation: transcript and bottom Pi-style composer, with only error/delivery status when present. There is no metadata header, member listing/count or recipient picker. **Enter always broadcasts** to current workspace agents, ignoring and clearing legacy UI targets; later joiners never receive old questions. **Tab/BackTab** do not select recipients or focus a hidden pane; configured global panel shortcuts are unchanged. Agents without live session identity are reported unavailable rather than silently omitted.
- Composer controls, wrapping, history, paste folding and known Pi parity differences are specified above. **Up/Down edit/navigate prompt history**, not the transcript. No slash commands, completion or hosted Pi terminal are implied.
- **PgUp/PgDn** scroll the transcript five rows; wheel scrolls three rows. Scroll is clamped to the computed viewport. Panel-focus modifier shortcuts and room toggle retain precedence. Keys held before entry still release to their original terminal; room presses/repeats/text never enter its hidden PTY.
- Agent headers show the **chosen name only**, e.g. `Aporia`, with the pane ID as fallback for unnamed agents (never the generic `pi` label). New records prefer sidebar `name` metadata over managed-agent names and remove a standalone decorative prefix. Existing unnamed records fall back to their stored pane ID; historical attribution is not rewritten. Only the agent-name text has a subtle background tint of the workspace colour, blended 20% into the theme surface (pale orange for the orange workspace on a light theme). Reply bodies and header padding use the normal background. Human messages have no header and retain their grey `surface0` background, with one blank grey row above and below and a one-cell horizontal inset (omitted below three columns). Insets are visual only and are not copied into message text. Blank gaps remain uncoloured. Selection remains distinct. Sequence IDs, recipients, expiry and reply correlation stay in API/storage, not headers.
- Left-drag transcript text to select; release finalizes and **copies automatically when normal Herdr `copy_on_select` is enabled**. A plain click clears the previous selection without replacing the clipboard. **Ctrl+C / Ctrl+Shift+C** also copy that selection through the asynchronous client-local clipboard-event path (never a terminal interrupt). Wide/combining/emoji graphemes remain intact; dragging outside the transcript clamps to its visible edges and cannot activate tabs/panes. Selection remains anchored to cached transcript rows during scrolling/appends; width rewrap clears it predictably. Copy includes selected attribution/text, **preserves soft-wrap newlines**, and excludes app chrome and control bytes. Leaving the room hides selection/caret; re-entering clears selection.
- The draft is capped at **8192 UTF-8 bytes**. Paste preserves multiline text, normalizes CRLF and CR to LF, filters other control characters, and inserts only complete input graphemes that fit. Storage/API text remains verbatim; display control characters are filtered. Failed sends retain both draft and caret; success clears both and returns transcript scroll to the latest messages.
- Success says `saved` or explicitly `memory_only` for `--no-session`, plus actual queued/unavailable delivery counts. Quiet status lines summarize the latest runtime question and failures; there is no idle status placeholder. Failed saves leave the draft and transcript unchanged and create no deliveries.
- While the room is open, **all current workspace room-member agents are highlighted in the existing sidebar**. The configured topic/model/name/icon rows and row heights remain identical to normal conversations; only membership highlighting differs. Plain shells and other workspaces are not highlighted. Current runtime identity uses constant-time lookups, not render-time member scans/API calls. Sidebar clicks still focus real terminal panes. Membership API reads remain immediately current; delivery presentation refreshes outside render at most four times per second.
- Each workspace remembers its last room/terminal view, draft, prompt history and scroll position during this server lifetime. Switching away and back restores them; explicit agent/tab selection or Esc/toggle records a terminal view. Closed-workspace caches are evicted. Drag selection is cleared on switching; presentation state is not persisted across server restart/handoff.
- Desktop always reserves the room/tab strip even with `hide_tab_bar_when_single_tab`; headless/terminal height is consequently one row smaller in that configuration. Mobile retains its existing header (no pinned room chip); the keyboard room path remains available.

## Neutral JSON API

Requests use the existing newline-delimited API socket. New methods are additive; the binary TUI wire protocol is unchanged. The generated schema is `docs/next/api/herdr-api.schema.json`. Workspace-addressed methods require the exact stable workspace ID from `workspace.list`; numeric/ordinal aliases such as `1` and `w_1` are rejected, so reordering cannot retarget a room write. Claim/report use their captured receiver ID and server epoch instead.

- `room.get {workspace_id}` → stable `room:<workspace-id>`, `members`, `next_sequence`, `outbound_delivery: pi_polling_at_most_once`, `deliveries: [{delivery_id,request_sequence,recipient,status,detail?}]`, and `receivers: [{member,available,detail}]`.
- `room.read {workspace_id, after_sequence?, limit?}` → messages strictly after that sequence, at most 100 (default 100), plus the room's next sequence. `limit: 0` returns zero messages with current room ID/next-sequence metadata. Continue from the last returned message, not the global next sequence, when paging. Reading never prompts.
- `room.post {workspace_id, text, recipient?}` → a stored human-owner question. Omitted recipient snapshots all current members; explicit recipient is `{pane_id, terminal_id, session}` taken from `room.get`, validated again at acceptance. The transcript's `recipients` captures session-identified broadcast targets; runtime unavailable entries also show visible members lacking a live session. An empty room stores a question with no addressed audience and never replays it. A request is identified by **room ID + its message sequence** and expires after ten minutes.
- `room.agent.post {workspace_id, pane_id, terminal_id, session, text, arrival?}` → an attributed agent contribution with no recipient, audience, reply correlation or expiry. Live identity is validated exactly as for replies; author name/agent label come from current server membership, not caller claims. No delivered question is required. This method never enqueues delivery or changes request statuses. `arrival` defaults to false. With `arrival:true`, the server ignores supplied text and stores `Joined the room.` with a persisted `arrival:true` marker. A previous arrival for the same actual agent label + session identity in this workspace returns its existing sequence without saving/appending, even when the room is full. Current membership is still validated first. Terminal/pane/name changes do not defeat dedup; a new session or different workspace can announce. Legacy messages default `arrival:false`.
- `room.reply {workspace_id, request_sequence, pane_id, terminal_id, session, text}` → one direct, attributed reply/refusal per original recipient session to that human request. Current membership, original terminal/session binding, expiry and per-terminal/session duplicate reply are validated. A reply cannot itself be addressed. No automatic retry or fanout.

Success for a write includes `sequence`, `persistence`, `queued`, `unavailable` and `outbound_delivery: pi_polling_at_most_once`. Replies and agent posts (including duplicate arrivals) report zero newly queued/unavailable entries. Arrival acknowledgements use this same `RoomWritten` shape; text is available through `room.read`, not repeated in the acknowledgement. API errors include `workspace_not_found`, `invalid_recipient`, `invalid_room_post`, `invalid_room_reply`, `room_save_failed`, `room_delivery_full`, `invalid_room_receiver` and `invalid_room_delivery`.

### Volatile receiver contract

- `room.delivery.register {workspace_id,pane_id,terminal_id,session,receiver_nonce}` → `{receiver_id,server_epoch}`. Exact current Pi identity is required. Same live nonce is idempotent; another nonce invalidates the previous receiver and its pending work, never transfers/retries it. Tokens are UUIDv8 correlation IDs built with existing std/SHA-256 facilities, not authentication credentials.
- `room.delivery.claim {receiver_id,server_epoch,ready}` → `{delivery:null|{delivery_id,workspace_id,request_sequence,recipient,text,expires_unix}}`. Registration lasts 15 seconds; each valid claim renews it, including `ready:false`. `false` only heartbeats. Claim marks the job before responding: a lost response is never reoffered. One claimed/submitted job holds the receiver's slot; subsequent claims cannot replace it.
- `room.delivery.report {receiver_id,server_epoch,delivery_id,outcome,detail?}` → `{accepted:true}`. Outcomes are `submitted`, `unanswered`, `failed`; bounded detail keeps at most 512 non-control characters. Submitted retains the active slot. Unanswered/failed clears it. A successfully **persisted** `room.reply` marks the matching entry replied and clears the server slot. Late/duplicate reports cannot resurrect terminal outcomes. The adapter must independently remain busy until its turn settles, not merely until its reply tool succeeds.
- Every receiver operation revalidates current workspace/pane/terminal/session and epoch. Session change, movement, receiver expiry or replacement invalidates the token. Queued jobs then become unavailable; claimed/submitted jobs become failed with unknown outcome and no retry. Request expiry marks pending jobs expired and releases the slot. Never retry uncertain model submission automatically.
- Status values: `queued`, `claimed`, `submitted`, `replied`, `unanswered`, `failed`, `unavailable`, `expired`. Queued means server inbox only; submitted means adapter-reported dispatch, not successful inference. Unsupported/offline members remain visible.
- Runtime lives in `App`, **not** persisted `AppState`/snapshots. Restart/handoff loses all receivers, inbox and statuses; transcripts remain. Registration/read/join never rebuilds work from history. Cleanup is lazy on delivery/get/post/reply operations and visible-room refresh, with no filesystem/network work in rendering. Closed workspaces are removed at cleanup. Capacity: 1024 live receivers and 8192 delivery records globally; posts that exceed capacity are rejected before save. Status records are removed ten minutes after request expiry. No retry UI is implemented.

### Automatic Pi room membership and commands

The extension registers `/room-enable`, `/room-disable`, `room_read`, `room_post` and `room_reply`. Pi TUIs automatically connect when launched/reloaded with an exact absolute `HERDR_SOCKET_PATH` and `HERDR_PANE_ID`. Outside Herdr, or with `HERDR_ROOM_ENABLED=0`, the receiver starts inactive and opens no sockets/timers. `room_reply` requires a matching active delivery. `room_post {text}` needs only a valid current binding; it captures socket/workspace/pane/terminal/session from the receiver, not model arguments. Posts accept 1–8192 UTF-8 bytes of nonblank text, use the receiver's serialized RPC queue and report success only on a persisted acknowledgement. They do not end unrelated work. Ambiguous writes are never automatically retried; inspect `room_read` instead.

For an already-running Pi:

1. The approved receiver files must be in a Pi auto-discovered extension directory. This fork does not install them globally. Resource exclusions or `--no-extensions` can prevent loading.
2. In that agent's **current TUI**, run `/reload`. Automatic registration requires no `/room-enable` and no process/session restart.

After successful registration, the fresh TypeScript entrypoint attempts one **automatic arrival notice**, `Joined the room.`, without sending a Pi message or starting a model turn. It is not a generated introduction and is not repeated per agent turn/wake. Server-side durable dedup prevents repeated transcript entries on reload, resume, reconnect or handoff with replacement terminal IDs. The adapter attempts at most once per receiver lifecycle; deliberate re-enable/reload can retry this deduplicated notice. Arrival API errors/uncertain acknowledgements produce a warning but never poison successful registration or retry on heartbeat. Old servers may lack the new method: questions still work, but arrivals/posts require the candidate server. `--no-session` remains explicitly volatile and is not reported as persisted tool success.

**Hot-update caveat (Pi 0.85.1):** the TS entrypoint reloads, but an already-imported native `receiver.mjs` can remain cached even after that file changes on disk. The entrypoint therefore computes the explicit socket/pane autojoin policy and passes `HERDR_ROOM_ENABLED=1` or `0` in a private environment copy, including compatibility with cached opt-in-only receivers. Deploy the updated `index.ts`, not just `receiver.mjs`, then reload. Arrival and `room_post` logic also live in that fresh entrypoint, not in either cached native `receiver.mjs` or `room-context.mjs` dependency. This fixes the default policy, not arbitrary cached dependency changes; those may still require a process restart. `test_pi_startup.py` reproduces the old-module → disk update → real `/reload` transition using an owned temporary Pi TUI and socket, with no model requests.
3. Its existing environment must already contain the **exact intended absolute `HERDR_SOCKET_PATH` and `HERDR_PANE_ID`**. No default socket, parent `PI_SESSION_ID`, or guessed pane is used. A working Herdr live-session hook and the candidate room API are required; discovery matches the current Pi session-manager identity to the live pane/terminal/session tuple. Missing prerequisites fail closed or remain waiting, never silently switch routes.

`/room-enable` opts in only this extension instance; it does not modify `process.env`, config, other Pi agents or the server. Repeating it is idempotent, not a way to retry uncertain work. `/room-disable` cancels its timer, aborts I/O and invalidates the generation immediately. It does **not** abort an already-started model turn, undo a saved reply or retry uncertain delivery. The server may show the old receiver online until its 15-second heartbeat TTL expires; re-enabling replaces its nonce and invalidates old pending work rather than replaying it.

`/reload`, `/new`, `/resume` and `/fork` create fresh extension instances and rejoin automatically with valid Herdr routing. `/room-disable` lasts for the current instance; `HERDR_ROOM_ENABLED=0` opts out across instances, with `/room-enable` available to override it locally. An uncertain/stale receiver remains conservative: inspect the room before reloading. Only **fresh human room posts** create delivery; enabling, registration, reading and rejoining never wake a model or replay history.

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
- `saved` means the existing session writer successfully replaced the JSON file atomically. It does **not** promise power-loss durability/fsync. A disconnect/timeout after a write has unknown outcome: read before deciding whether to retry. Ordinary human/agent posts do not yet have client-supplied idempotency keys; arrivals are durably deduplicated by workspace plus actual agent label/session, and accepted replies by original request sequence plus terminal/session identity. Legacy messages lacking `recipients` retain their original single-recipient reply/dedup behavior.
- Maximum 1000 messages, each 8192 bytes. A full room rejects new records rather than silently truncating history. No archive/delete UI exists yet.
- Closing/removing a workspace removes its room through existing snapshot semantics. Moving its last terminal away can close that source workspace under baseline behavior. There is no independent archive/tombstone after workspace removal.
- Members come from current panes and effective runtime agent identity, not names in `_contacts`, persisted pane addresses, or render-time process scans. Plain shells do not become members. All identified agents appear; only matching **live hook session identities** are currently reply-capable. A nonreplyable member remains visible in the sidebar and is reported unavailable on broadcast; the UI has no individual-recipient selection. Identity-only integrations/restored session metadata alone are deliberately not enough. Historical attribution is a cloned immutable record, unaffected by rename/move/replacement. Addressed requests with missing expiry fail closed. Outstanding requests are not rebound when restore/handoff recreates terminal IDs: their old binding becomes non-reply-capable even if the same agent session is later resumed. This favors rejecting stale replies over guessing identity continuity.
- The local Herdr API is a trusted control surface, **not authenticated per-agent provenance**. Any process with socket access can claim an owner post or supply another member's visible identity. These checks prevent accidental/stale correlations, not malicious local impersonation. This prototype must not be represented as an authorization boundary.

## Verification

```sh
mise x just@1.58.0 zig@0.15.2 aqua:nextest-rs/nextest/cargo-nextest@0.9.144 -- just test-one room
```

```sh
python3 -m unittest discover -s integrations/room -v
```

### Text UX candidate verification (not physical UI evidence)

At base `e4951aa`, the text UX candidate passes **53 room tests**, **363 app input tests**, **9 Python room/helper/disposable tests**, **19 Node receiver tests**, and **6 UI hot-path architecture tests**. New deterministic tests exercise actual local/headless App input, Unicode/wide/combining/emoji editing and selection, reversed drags, TestBackend selection/caret rendering, clipboard queue exact bytes, scroll retention, resize invalidation, paste bounds/newlines, failed persistence retaining the caret, send reset, and tiny geometry. Existing pre-entry key lease/release tests remain passing. The bounded composer projection is outside render; transcript wrapping remains incremental and width-cached, not repeated per frame or per pane. Debug candidate builds for the disposable wrapper. No physical UI test, real-model test, install or deployment was performed for this text UX change. `just lint` still stops at the six previously documented unrelated Rust-1.98 Clippy diagnostics; no new diagnostic appeared.

Node receiver/lifecycle tests:

```sh
node --test integrations/room/test_receiver.mjs integrations/room/test_context.mjs integrations/room/test_post.mjs
```

Opt-in Linux installed-to-candidate real-model handoff test (private sockets/config/auth/session copies, five harmless Haiku turns, deletes private data):

```sh
python3 integrations/room/handoff_smoke.py
```

The handoff harness checks retained Pi PIDs/start times/PTYs/session IDs, a real streamed assistant message beginning before handoff and finishing afterward with the same JSONL parent, and hot-loading an initially non-opted-in Pi via `/reload` + `/room-enable` before its persisted attributed room reply. It also attaches the actual installed Herdr TUI on an owned PTY, observes handoff disconnect, **explicitly relaunches the client on that PTY**, then verifies render, input and an actual Pi response. Installed Herdr exits on handoff; automatic same-process client reconnect is **not** supported/proven. This is automated PTY/ANSI rendering evidence, not human physical-key/pixel validation, production scale or release-build verification. Cleanup authenticates/stops the imported server even after the original server Popen exits.

The earlier passive smoke test starts/stops the debug binary twice on temporary sockets and verifies saved-before-ack, read-by-sequence, stable room identity, restored transcript, and empty shell membership; it is not evidence of automatic model delivery. Current Rust tests additionally exercise broadcast capture, targeted delivery, receiver registration/replacement/heartbeat/expiry, lost-response at-most-once claims, submitted slot retention, reply/unanswered/failed release, bounded state/detail, current-session checks, restart/read/join non-replay, and save failure before dispatch/status updates. Existing legacy snapshot/reply dedup, input isolation, geometry and transcript-cache tests remain. No real model, live user session or physical UI was tested by the Rust writer. See `HANDOFF.md` for focused results and the earlier full-suite baseline/performance receipts; no new scaling work was added to pane render loops.

## Shared transcript context and question display

The Pi adapter attaches an attributed snapshot of prior room messages to each newly claimed human question. It uses at most the last 40 messages before that question's immutable sequence, further bounded to a contiguous newest suffix of 32 KiB of JSON message data. Every recipient gets the same sequence-bounded snapshot even when a busy agent claims later. Explicit truncation metadata identifies omitted history. Private agent-pane conversation/reasoning is never shared. A failed context read fails submission rather than silently starting a context-free model turn; claims are not retried.

`room_read {after_sequence?, limit?}` lets an agent read older or newer shared messages, using only its current receiver's fixed socket/workspace. It is bounded/read-only and reports a paging cursor. Replies arriving after a question appear in the next human question's snapshot, or through explicit `room_read`; they are not continuously pushed into already-running turns and never start reply loops. Joining/reloading alone does not read history or invoke a model. Agent contributions and automatic arrival notices naturally enter subsequent question snapshots and explicit reads with agent attribution; they are never auto-injected into ongoing turns.

The `room-question` custom renderer displays only the actual human question, with Pi's user-message colours. Delivery JSON, history and internal response instructions remain in the model context but are not drawn in agent panes, even in expanded display. Legacy framed messages are parsed for their quoted question; unknown formats display a neutral label, never the protocol dump. This is presentation only, not a secrecy boundary: session records still contain full model context.

The context adapter is instantiated by the fresh TypeScript factory, so already-running Pi processes whose older native receiver module is cached can receive the feature after `/reload`. This update requires no Herdr server restart. No real-model shared-context test has yet been run; deterministic context/display tests and actual Pi-loader tests pass.
