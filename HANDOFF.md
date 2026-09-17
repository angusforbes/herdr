# HANDOFF — herdr-italics fork

For whoever comes next. This is Angus's fork of Herdr (`herdrdev/herdr`), installed as
`~/.local/bin/herdr` and serving his live workspace. The Obsidian project folder is
`~/Obsidian/Projects/Herdr/` (design notes per feature); durable engineering notes live here.
Add a section per session; don't rewrite others'.

## Build & deploy (works as of 2026-09-15)

```sh
cargo build --release                      # or: mise x zig@0.15.2 -- cargo build --release
cp target/release/herdr ~/.local/bin/herdr.new && mv -f ~/.local/bin/herdr.new ~/.local/bin/herdr
herdr server live-handoff --import-exe ~/.local/bin/herdr
```

- Plain `cp` over the running binary fails with "Text file busy" — use the mv.
- Without `--import-exe` the server re-execs its own now-deleted path and the handoff fails.
- Handoff preserves panes/tabs/sessions but **drops `report-metadata` tokens** (agent names).
  Keystone's `herdr-name-keeper.service` re-publishes them within ~10 s.
- Back up the previous binary first (`~/.local/bin/herdr.before-<feature>`).
- `which herdr` is the stock `/usr/bin/herdr`; use `~/.local/bin/herdr config check` to validate
  config against the fork (stock complains about the fork-only `index` sidebar token).
- 16 tests fail on a clean tree (detect::manifest*, headless keybinding, workspace id). Compare
  `git stash; cargo test` vs after before blaming yourself.

## Sessions

### Room workspace presentation memory (uncommitted)

- Workspace-only navigation restores that stable workspace ID's last room/terminal surface. Inactive `RoomPresentation` values retain composer draft, editor/history/undo, transcript cache and scroll; transient selection/drag and refresh timestamp are cleared on departure.
- Explicit pane/agent/tab focus hides only the destination room, after saving the source presentation. Esc and room toggle remember terminal view while retaining the draft. Workspace deletion/pane death evict closed IDs; reorder keeps stable ownership. Pane-move completion/recovery also reconciles presentation.
- TUI-only, in-memory cache in `AppState`; no room runtime/API semantics or restart persistence changed. Existing clipboard/header, sidebar and reload-cache work preserved. No commit or deployment.
- Focused validation: 72 `room_`, 286 `workspace`, and 2 `tab_focus` tests passed before adding the pane-death regression; final rerun recorded by the implementing agent. Tests cover independent drafts/history/scroll, cross-workspace explicit focus, Esc/toggle on both input routes, reorder/close and fresh replacement IDs. Tool shims require `mise x just@1.58.0 zig@0.15.2 aqua:nextest-rs/nextest/cargo-nextest@0.9.144 -- just test-one <filter>`.

### 2026-09-14 — Halyard (claude-fable-5-1): sidebar space selection (`09a042c`)

**What:** workspace circles in the left sidebar are selection indicators. `●` = the space's agents
appear in the agent panel, `○` = hidden. Colour is per-workspace and stable (derived from the
workspace id via `decode_public_number`, 8-colour palette). Agent rows keep their state glyph
(◐ × ✓ ○) but are coloured to match their space. `alt+space` toggles the active space (or the
sidebar-highlighted one in navigate mode). All selected by default.

**Where:**
- `src/app/state.rs` — `deselected_workspace_ids`, `workspace_selected()`, `toggle_workspace_selection()`,
  `toggle_current_workspace_selection()`, `workspace_color()`.
- `src/ui/sidebar.rs` — `workspace_selection_icon()`, `agent_state_icon()`; the filter is one
  `retain` in `agent_panel_entries_with_runtimes` (so `all_agent_panel_entries` is still unfiltered).
- Keybind plumbing: `config/model.rs`, `config/keybinds.rs`, `app/input/navigate.rs`
  (`NavigateAction::ToggleWorkspaceSelection`), `ui/keybind_help.rs`, `main.rs` doc comment.
- Tests: `app/actions.rs::workspace_selection_tests`; snapshot tests in `ui/sidebar.rs`,
  `ui.rs`, `ui/tab_surface.rs` updated (the tab_surface test pins `workspaces[0].id = "w1"` because
  the digest now depends on the colour).

**Decisions:** colour keyed by id not list index so it doesn't shift when a space closes. Filtering
happens before `apply_agent_view` so priority sort and view overrides compose. Not persisted
(see below).

**Not done / known gaps:**
- Selection resets on server restart. To persist: add the set to `persist/snapshot.rs`
  (`Snapshot` + raw struct + `from_state`), `restore.rs`, `server/handoff.rs`, and `app/mod.rs`
  startup — expect ~6 struct literals in tests to update.
- Space rows lost the aggregate-state colour (yellow working / red blocked). If wanted back,
  colour the glyph by state only when blocked, keep the selection circle otherwise.
- No mouse toggle on the circle. `app/input/sidebar.rs` handles workspace-row clicks; a click on
  column 1 of a space row could call `toggle_workspace_selection(ws_idx)`.
- `mobile.rs` and `search_pane.rs` still use the old `state_icon` colouring — intentional for now.

Also config-only, same session: `previous_agent`/`next_agent` bound to `alt+shift+up/down` and
`prefix+up/down` in `~/.config/herdr/config.toml`.

### 2026-09-16 — Selvedge: panel focus and global search shortcuts

Implemented and activated: Ctrl+Tab / Ctrl+Shift+Tab cycle Spaces → Agents → terminal → open Search; Ctrl+Alt+Shift+Up/Down visit search results while preserving keyboard focus; Ctrl+' focuses query; Ctrl+/ toggles Keyword/AI. Six configurable defaults, modal guards, Agents arrow navigation and focus styling. Core helper: `src/app/input/panel_focus.rs`; TUI-only `navigate_agents` flag in AppState.

Receipt and caveats: `.local/prd/panel-focus-shortcuts.md`. User-facing companion: `~/Obsidian/Projects/Herdr/Panel focus and search shortcuts.md`.

Final targeted tests: 199 passed. Full suite: 3505 passed / 4 failed, all four independently reproduced on pristine HEAD `575cab9`. Existing config-reference failure also reproduced. Release built and live-handoff succeeded; 7 workspaces / 23 panes / 13 agents retained with original pane assignments and active pane. Existing handoff regenerates internal terminal IDs. Human physical-key/UI verification remains pending. Backup `~/.local/bin/herdr.before-panel-shortcuts-20260916-000157`.

### 2026-09-17 — Aporia: peer-message delivery evidence (no code changes)

Observed separator-free message fusion, including two overlapping CLI sends whose complete payloads remained intact in one received turn. This does not establish universal atomicity or distinguish steering from follow-up. Reload remained unverified; stale cross-sender text was separately reported by Lattice. Durable findings, caveats, proposed requirements for human multi-peer conversations, and archived test receipts: `.local/prd/peer-message-delivery.md` and `.local/prd/peer-message-evidence/`. No transport fix, deployment, or new communication command implemented.

### 2026-09-17 — Rooms implementation writer: passive working prototype (uncommitted)

Scope: `/home/agf/Work/herdr-rooms`, branch `angus/rooms`, approved base `7178f2e`. No agents spawned, commits, pushes, deployment, live handoff, edits to `herdr-italics`, or global Pi extension changes. Parent review/integration remains required. The requested no-subagents constraint meant no roundtable was run here.

**Working slice:** shared workspace-owned room transcript + neutral JSON API, pinned desktop `room` slot separate from terminal tabs, isolated room input/composer, dynamic member list, one explicit recipient, direct manual reply helper, bounded immutable attributed history, sequence correlation/expiry/reply deduplication, persisted workspace association and messages. Room has no PTY or agent. Terminal IDs, root-pane/layout invariants and baseline customizations remain intact. **Outbound prompting is deliberately disabled**; addressed records are manual-pull requests, not successful agent dispatch. No room operation starts inference or rebroadcasts replies.

**Files (exact):**
- New core/model/API: `src/room.rs`, `src/api/schema/rooms.rs`, `src/app/api/rooms.rs`.
- New presentation/tests: `src/app/room.rs`, `src/app/room/tests.rs`, `src/ui/room.rs`.
- API wiring: `src/api/mod.rs`, `src/api/schema.rs`, `src/api/schema/response.rs`, `src/api/server.rs`, `src/app/api.rs`, `src/main.rs`.
- State/storage: `src/workspace.rs`, `src/app/state.rs`, `src/app/session.rs`, `src/persist.rs`, `src/persist/io.rs`, `src/persist/snapshot.rs`, `src/persist/restore.rs`.
- UI/input seams: `src/ui.rs`, `src/ui/tabs.rs`, `src/ui/tab_surface.rs`, `src/app/actions.rs`, `src/app/mod.rs`, `src/app/input/mod.rs`, `src/app/input/mouse.rs`, `src/app/input/terminal.rs`, `src/app/input/panel_focus.rs`.
- Geometry/benchmark coverage: `tests/detach_reattach.rs`, `src/server/render_stream.rs`.
- Standalone helper/disposable runner/tests: `integrations/room/room.py`, `integrations/room/disposable.py`, `integrations/room/test_room.py`.
- Docs/schema: `docs/next/rooms-prototype.md`, `docs/next/api/herdr-api.schema.json`, this `HANDOFF.md`. Local receipt: `.local/prd/rooms-implementation.md`.

**Persistence:** `WorkspaceSnapshot.room` defaults for old snapshots; normal restore and handoff restore copy the same data. Explicit room writes join any earlier snapshot writer, validate/save a candidate synchronously, and only then publish it. Errors retain prior state/sequence and report `room_save_failed`; `--no-session` reports `memory_only`. Uses existing atomic replacement writer, **not fsync/power-loss durability**. No filesystem work in rendering or pane loops. `room.read` is bounded by sequence/page; EventHub is not storage. Render transcript wrapping is incremental, cached by width and append count, and draws only visible cached lines.

**Tests, final source:**
- Full nextest: **3539 run, 3531 passed, 8 failed, 1 ignored manual benchmark**. Pristine `git archive 7178f2e` at `/tmp/herdr-rooms-baseline.JwQ6GW`: **3526 run, 3518 passed, identical 8 failed, 1 ignored**. Thirteen added tests; existing geometry/schema expectations updated only for the intended pinned room row/API additions.
- Focused `room` filter: **15 passing** (includes two renamed existing single-tab geometry tests).
- Python room helper + actual temporary server save/restart smoke: **5 passing**. Tests use shell-only disposable servers, never real model calls. The smoke verifies saved-before-ack via the actual JSON file, paging, restored sequence/transcript and stable room ID.
- UI hot-path architecture: **6 passing**. Bundled integration assets: **26 passing**. Maintenance suite: **97 passing / 1 baseline failure**, `scripts.test_config_reference_check.RealModelTests.test_preview_reference_matches_real_config_model`; reproduced separately on baseline (missing `keys.toggle_workspace_selection` docs entry).
- `cargo fmt --check` and `git diff --check`: pass. `just check` was attempted and stops at **six pre-existing Clippy errors**, independently reproduced on baseline with this machine's Rust 1.98: `chunks_exact_to_as_chunks` in `app/api/pane_graphics.rs` and `ghostty/mod.rs` (three sites), `byte_char_slices` in `server/handoff.rs`, `some_filter` in `terminal_theme.rs`, `field_reassign_with_default` in `app/search_pane.rs` test. No new Clippy errors observed. Windows lint and plugin-marketplace install/test were not run; check cannot reach them, and unrelated dependency installation was avoided.
- The eight unchanged Rust failures: `cases::protocol::plugin_list_preserves_protocol_mismatch_envelope`, `cases::protocol_guard::cli_rejects_protocol_mismatch_before_agent_wait_request`, `cases::sessions::dead_server_cli_reports_one_session_aware_json_line`, `cases::surface::api_snapshot_prints_live_session_snapshot`, `cross_area_agent_process_survives_detach_and_reattach`, `server::headless::tests::retained_pty_update_matches_full_render_frame`, `server::headless::tests::retained_pty_update_streams_cursor_only_change`, `ui::mobile::tests::distinct_agent_summary_uses_configured_symbols_for_every_state`.

**Fixed-geometry scaling (`just bench-render-scale`, release, 120×40, 1/15/50 populated panes):**
- Baseline background-workspace median µs: **336 / 346 / 367**; candidate **330 / 340 / 377**. Candidate 15-vs-1 factor **1.03×**, baseline **1.03×**; 15-pane median delta **−1.7%**.
- Baseline active-pane median µs: **320 / 398 / 459**; candidate **334 / 415 / 481**. 15-vs-1 factor **1.24×** for both; 15-pane delta **+4.3%** (single sampling run, not a release performance guarantee).
- New room-selected scenario (1000 messages, 1/15/50 populated hidden panes): **295 / 293 / 298 µs**, factors **1.00 / 0.99 / 1.01×**. No membership derivation, terminal snapshots, runtime locks or disk I/O added to pane-scaled rendering.
- No release-smoke/stable download or live-server performance testing performed; this is not a release/deployment.

**Parent run commands** (from this checkout; each is one line):

```sh
mise x zig@0.15.2 -- cargo build --locked
```

```sh
python3 integrations/room/disposable.py
```

```sh
mise x just@1.58.0 zig@0.15.2 aqua:nextest-rs/nextest/cargo-nextest@0.9.144 -- just test-one room
```

```sh
python3 -m unittest discover -s integrations/room -v
```

The disposable runner starts only this fork with temporary config/state/sockets, no inherited `HERDR_*` routes, `/bin/sh`, and agent resumption disabled; it owns and stops that server and deletes its data on exit. It prints the one-line helper command with its explicit socket. Create a workspace if the UI starts empty, click `room` or Ctrl+Alt+R; Tab selects a recipient, Enter records, Esc returns. Interactive human UI verification remains pending; server lifecycle/helper paths are tested.

**Deliberately unresolved:** no safe automatic outbound transport; no multi-recipient/group send, Pi extension install, authentication of per-agent claims, idempotent post keys, independent archived rooms after workspace deletion, draft persistence or per-workspace draft map, or mobile pinned chip. Only live-hook session identities are reply-capable; identity-only integrations still appear as members. Outstanding requests are deliberately not rebound across recreated terminal IDs on restore/handoff (old requests then cannot receive replies). The local socket is trusted, not impersonation-proof. Snapshot writes can briefly block the app; accepted means atomic file replacement, not power-loss fsync. Closing a workspace (including baseline last-pane-move removal) removes its room. A room caps at 1000 messages/8192 bytes each and rejects overflow. Single-tab-hide config now retains the desktop room strip, reducing terminal height one row. See `docs/next/rooms-prototype.md` for exact contracts and API/helper usage.

Logs retained under `/tmp/herdr-rooms-{tests-final,baseline-tests,final-clippy,baseline-clippy,maintenance,baseline-config,assets,bench-final,baseline-bench}.log`. The branch remains uncommitted for parent review.

### 2026-09-17 — Rooms narrow follow-up fixes (uncommitted)

Sole-writer follow-up on the existing prototype in `/home/agf/Work/herdr-rooms`, `angus/rooms` at `7178f2e`. No commit, deployment, spawned agent, original-fork/global modification, or persistence redesign.

**Exact changes:**
- `src/app/room.rs`, `src/app/mod.rs`, `src/app/runtime.rs`, `src/app/input/terminal.rs`: room key handling no longer consumes releases. Only lease-owned target forwarding may finish a pre-room key-down; release encoding uses the original terminal ID, not current pane focus. Room repeats cannot forward or discard that lease. Local raw input now intercepts room repeats before lease repeat routing (matching headless), and room mouse chrome works with terminal mouse capture disabled. Ctrl+Alt+R repeats do not toggle the room. Focus-loss cleanup can release owned keys while the room is active; missing runtimes never rebind to replacements. Popup precedence remains unchanged.
- `src/app/room.rs`: Tab from a stale member clears selection to none with explicit status; only the next Tab selects the first current member. Nonreplyable current members remain visible and report `no live session identity`, distinct from genuinely changed recipients.
- `src/ui/room.rs`, `src/ui.rs`: one shared room layout helper supplies render and geometry. Geometry clamps stored scroll to wrapped transcript line count minus the actual transcript viewport height, including short layouts, resizes and appends; render remains immutable.
- `src/app/api/rooms.rs`: `room.read limit=0` returns no messages with current metadata. All room methods resolve exact stable workspace IDs only; ordinal aliases cannot retarget writes after reorder.
- `src/app/actions.rs`: reproduced and fixed workspace focus leaving room visibility latched, causing the old room to reappear on a later return. Valid workspace focus now clears room visibility. API pane/agent/tab focus and focused/background pane moves are regression-tested without broad refactoring.
- `src/room.rs`: defensively skip panes missing public-number registration; explicit `Path`/`Id` session prefixes preserve existing serialized spelling. Missing request expiry remains fail-closed, now with an explicit error. The malformed-pane test deliberately constructs invalid state; **no normal/legacy restore corruption was established** (restore precomputes migrated public pane numbers).
- `src/app/room/tests.rs`, `src/ui/room.rs`, `src/room.rs`: eleven added tests beyond the original room slice. Lifecycle matrix covers local/headless, keyboard/mouse entry, composer/panel focus, repeats, literal report-event press/release bytes (`ESC[97;1:1u` / `ESC[97;1:3u`), original-target pinning, cleared ledger, no hidden new input, focus loss and runtime replacement. Other tests cover stale Tab, zero-limit metadata, stable IDs/reorder, malformed registration/session spelling, nonreplyable status, missing expiry, keyboard/wheel scroll and wrapped geometry.
- `docs/next/rooms-prototype.md`: updated input, focus, scroll, stable-ID, zero-limit and fail-closed contracts. No schema shape changed.

**Focused verification:** `just test-one room` **26 passed**; `release` **86 passed**; `repeat` **35 passed**; `popup` **31 passed**; `pane_move` **18 passed** (filters overlap; not an aggregate unique-test count). `workspace_focus` matched zero tests and exited 4; focus coverage is instead the new passing `room_api_focus_and_move_leave_the_correct_surface_active`. `cargo fmt --check` and `git diff --check` pass. Logs: `/tmp/herdr-rooms-fixes-{tests,release,repeat,popup,move,focus}.log`. Full nextest/Python/performance suites were not rerun; the parent's existing full-suite baseline comparison and smoke receipts above remain the broad evidence. All changes remain uncommitted for parent inspection.

### Parent verification after review fixes — Aporia

Inspected key-release routing and stale-recipient handling after fixes. Independently reran 26 room tests and 5 Python helper/restart tests: all passed. Full final nextest run: 3550 tests, 3542 passed, the same 8 baseline failures, 1 ignored benchmark. Log: `/tmp/rooms-parent-final-tests.log`. `git diff --check` passed. No physical TUI validation, commit of room changes, install, or deployment yet. Passive/manual-pull limitation remains: this is not yet automatic group chat.

### 2026-09-17 — roomchat-api: Rust runtime delivery/API/UI (uncommitted)

Scope: `/home/agf/Work/herdr-rooms` at local `5be9c40`, preserving the parent's uncommitted group-domain work in `src/room.rs` (only cargo-formatting it) and the user's disposable-runner mode change. This writer did not edit `integrations/room`, the original checkout or global config, spawn agents, commit, push, install or deploy. The parallel adapter writer owns integration files; parent owns actual-model/physical-UI integration.

**Implemented:** volatile `App::room_delivery` inbox, exact `room.delivery.register/claim/report` JSON contract, current live workspace/pane/terminal/session validation, 15-second receiver heartbeat TTL, one active claimed/submitted slot per receiver, mark-before-response claims (never reoffered after uncertain response), no implicit retry, reply/unanswered/failed slot release, and expiry/receiver/workspace cleanup. Registration/reads/join/restart never build work from history. Replacement receiver tokens invalidate old pending work rather than transferring it. Delivery status is runtime-only and lost on restart/handoff.

Human `room.post` now defaults to a once-captured group audience. Explicit recipients retain current-session validation and legacy single-recipient storage; broadcast storage uses the parent's `recipients` domain field and per-terminal/session reply dedup. Only successful candidate persistence creates inbox entries. All visible members get queued or explicit unavailable status (no live session, unsupported agent or offline receiver); only live Pi receivers queue. Persisted `room.reply` updates matching terminal/session delivery status, including a same-workspace pane move; failed persistence leaves status/active slot untouched. No PTY injection or network/filesystem work was added to rendering.

The room client defaults to all, shows real queued/unavailable counts, latest-request status counts/detail, and Pi receiver availability; stale explicit target and existing input/release isolation remain. Presentation updates are cached outside render (visible-room refresh, at most four times/second). Shared facts remain in the server runtime/API; only presentation projections are in `room_ui`.

**Bounds/limitations:** 1024 live receiver registrations; 8192 delivery records across workspaces, checked before saving a post; records retained until ten minutes after request expiry; details capped at 512 non-control characters, nonce at 256 bytes. Cleanup is lazy on room delivery/get/post/reply operations and visible-room refresh. No persistent inbox, replay, retries, post idempotency key, per-agent security boundary, fsync guarantee, or support beyond explicitly enabled Pi polling. UUIDv8 correlation tokens use std random hash seeds plus the existing SHA-256 library (no new dependency). The receiver must continue heartbeating while busy and must not accept another turn until settled even if the reply tool has already cleared the server slot. `submitted` is an adapter report, not proof of a model response. No contract shape deviation; response types are `room_delivery_registered`, `room_delivery_claimed`, `room_delivery_reported`. JSON API additions require no binary TUI protocol bump.

**Exact files owned by this writer:**
- New: `src/room_delivery.rs`, `src/room_delivery/tests.rs`, `src/app/room/delivery_tests.rs`.
- Modified: `src/main.rs`, `src/app/mod.rs`, `src/app/api.rs`, `src/app/api/rooms.rs`, `src/app/room.rs`, `src/app/room/tests.rs`, `src/ui/room.rs`, `src/api/schema.rs`, `src/api/schema/rooms.rs`, `src/api/schema/response.rs`, `src/api/server.rs`.
- Generated/docs: `docs/next/api/herdr-api.schema.json`, `docs/next/rooms-prototype.md`, this `HANDOFF.md`.
- `src/room.rs`: cargo-format only over parent's pre-existing domain changes; no logic edits by this writer.

**Focused verification:** `just test-one room`: **42 passed** (parent's 28 plus 14 delivery/API tests). Covers broadcast/target capture, later joiners/offline/unsupported visibility, claim-at-most-once, heartbeat/readiness, active/submitted slots, terminal reports, bounded detail/capacity, expiry cleanup, registration replacement, session invalidation, same-workspace reply correlation, schema request shapes, restart/read/join non-replay, both post and reply save failure, existing legacy snapshot/reply dedup and input isolation. Generated-schema test: **1 passed** after regeneration and again without update mode. `cargo fmt --check` and `git diff --check`: passed. `just lint` attempted: blocked by the same six previously documented Rust-1.98 Clippy diagnostics (three chunks_exact sites, byte_char_slices, some_filter, field_reassign_with_default); no new diagnostic appeared. No broad suite, render benchmark, actual model, live user session or physical UI run by this writer.

Commands (one line each, using HANDOFF-pinned tools):

```sh
mise x just@1.58.0 zig@0.15.2 aqua:nextest-rs/nextest/cargo-nextest@0.9.144 -- just test-one room
```

```sh
mise x just@1.58.0 zig@0.15.2 aqua:nextest-rs/nextest/cargo-nextest@0.9.144 -- just test-one generated_protocol_schema_artifact_is_current
```

```sh
mise x zig@0.15.2 -- cargo build --locked
```

Parent: use the rebuilt `target/debug/herdr` with the parallel adapter's disposable smoke launcher. Parent reports 11 Node and 7 Python adapter tests passed separately; those are not this writer's verification. Actual-model smoke and independent adapter review remain parent-owned; do not infer end-to-end success from these Rust receipts.

### Parent actual-model verification — Aporia

Rebuilt candidate from roomchat-api was exercised with two real Pi TUIs, isolated private configuration, and anthropic/claude-haiku-4-5. `python3 integrations/room/smoke.py --provider anthropic --model claude-haiku-4-5 --timeout 150` PASSED: broadcast produced two persisted attributed replies, target produced exactly one, replies arrived before room API reads, and repeated reads did not replay/fan out. Initial attempts using openai-codex/gpt-5.4-mini failed because that model is listed but rejected for this ChatGPT account; captured agent error confirmed the cause, while Haiku replied successfully. No credentials or debug sessions retained.

Parent independently reran 11 JS and 7 Python tests (all passed). Full nextest: 3566 tests, 3558 passed, same eight baseline failures, 1 skipped. Log `/tmp/roomchat-parent-tests.log`. Independent adapter/server reviews still pending at this writing. No implementation commit, deployment, or physical UI validation yet.

### Parent adapter-review resolution

Fixed reviewer cfa2fc77 findings 1/2: unrelated agent_settled does not invalidate an idle in-flight claim; undispatched jobs cannot be mislabeled unanswered, and active state is cleared even when failure-report outcome is unknown. Two new deterministic regressions pass (13 JS tests total). Actual two-Haiku smoke rerun passed again. Finding 3 (frozen receiver after uncertain submitted report even if reply later saves) remains the explicit conservative policy requiring /reload, not automatic recovery: a generic unfreeze could also clear unrelated session/heartbeat faults. Server review still pending.

### Final server review disposition — Aporia

Review 2ebb4a70 complete. Corrected delivery-capacity wording to state retained history (including completed entries) lasts up to 20 minutes, rather than promising space at request expiry. Kept the intentional bounded history cap. Room summary now prefixes earlier pending delivery count, so posting another question does not hide previous queued/claimed/submitted work; new regression test passes. Unavailable-to-Replied is intentional: direct manual replies are valid with original captured identity/expiry, even without automatic receiver support. Reviewer did not audit all wire schema or pre-existing room tests; real socket smoke and automated suite cover those separately, not an exhaustive review claim. All 43 room tests pass after fixes; debug binary rebuilt; diff check clean. Both independent reviews complete. New code remains uncommitted and undeployed pending approval.

### 2026-09-17 — Local deployment prerequisites: runtime opt-in and active/client handoff

No deployment, global config/extension/binary edits, production socket calls, commits, pushes, children or original-checkout changes. Preserved the pre-existing `disposable.py`/`test_disposable.py` appearance-copy edits and extended the existing untracked `handoff_smoke.py`.

**Adapter:** `integrations/room/pi/index.ts` always registers `room_reply` and command/lifecycle handlers. `receiver.mjs` adds instance-local `/room-enable` and `/room-disable`, retaining startup `HERDR_ROOM_ENABLED=1` compatibility. Enable uses only the inherited exact absolute socket/pane and current TUI session manager, never mutates process env, never injects on registration/read. Inactive tools reject without I/O. Repeated enable is idempotent; disable cancels timers/I/O/generation without replay, undo or model abort. Old receiver status can linger until the 15-second TTL. Reload/session replacement drops runtime opt-in; without startup flag explicit enable is needed again. With the flag, reload starts a fresh opted-in instance. Uncertain delivery remains at-most-once; replacement discards rather than retries pending work.

**Verification:** 19 Node receiver/transport/lifecycle tests and 9 Python helper/server/real-installed-Pi-loader tests pass. The new loader test uses a private fake socket, actual TUI slash commands, no model requests; startup inactive, enable, disable, reload inactive and re-enable all verified. Stop also cancels safely when an old UI throws. `git diff --check` and Python compile pass. No Rust changes or broad Rust suite rerun.

Real smoke PASS: `.local/prd/handoff-smoke-20260917-150131.json`, details in `.local/prd/live-handoff-real-pi.md`. Installed source and debug candidate unchanged from earlier receipt (both v0.8.2/protocol 20). Pi A PID **4098401**, B **4098405**, PTYs **47/50**, original session IDs and start ticks retained. A's real provider text stream began before handoff; the **same run/message** completed ~9 seconds after importer readiness, `stop`, with one original user entry and its exact assistant JSONL child ending `INFLIGHT-FINISHED`. No replacement prompt was sent to A. B started with no receiver and no env opt-in; private copy + TUI `/reload` stayed inactive, `/room-enable` registered, reads/enable caused zero agent starts, and a new targeted human post produced B's persisted `ROOM-AFTER-HANDOFF` reply. Disable/re-enable then reload reset B's runtime opt-in. Five model turns in the successful run.

**Important client limitation, now measured:** real installed Herdr client PID **4100095** on owned PTY **51** rendered and accepted input before handoff, displayed the handoff shutdown notice and exited **1**. It does **not auto-reconnect**. Explicitly launching the client again on the same private socket/PTY (PID **4101114**) restored rendering and keyboard input; `/name CLIENT-AFTER-HANDOFF` was persisted and visible, and a prompt through that client produced B's actual `AFTER-1` answer. Neither Pi restarted. The first attempt `...145959.json` failed only because it asserted unsupported automatic client reconnect; it already proved active-turn survival, cleaned up, and did not reach room enable. No client runtime fix was attempted. Automated PTY/ANSI evidence is not physical keyboard/pixel validation.

**Remaining production prerequisites:** parent diff inspection/independent review and explicit deployment approval; approved receiver installation into a discoverable path, then each intended existing Pi's TUI `/reload` + `/room-enable` with inherited correct socket/pane and live-session hook. Plan explicit Herdr client reattach (not seamless reconnect). No production pane-count/capacity inventory was taken; 64-pane handoff limit, volatile delivery loss and regenerated terminal IDs still apply. No candidate-to-candidate registered-receiver handoff, release build/scale/performance or physical UI validation. Private copied auth/config/session dirs removed; imported-server peer ownership checked and cleanup reports zero survivors for both runs.

### Room text UX candidate — uncommitted, base `e4951aa`

Sole writer in `/home/agf/Work/herdr-rooms`; no child agents, production/global/original-checkout edits, real model calls, install, live handoff, commit or push. Parent owns independent review/integration/deployment. This candidate has **not been physically UI-tested**.

**Implemented:** room-local grapheme editor and cached viewport geometry, bounded multiline composer/caret, and transcript drag selection/highlighting/copy. Shared server room facts/API/storage are unchanged; no fake terminal pane or PTY selection extraction. Copy uses `request_clipboard_write` → `dispatch_pending_clipboard_write` → existing clipboard event handling (no synchronous clipboard writes). Transcript wrapping remains incremental at fixed width; only the bounded 8192-byte composer is projected each geometry pass, outside pure render and pane-scaled loops.

**Exact controls/contracts:** unmodified Up/Down scroll history one display row even with a draft; PgUp/PgDn retain five rows, wheel three. Left/Right move by grapheme; Ctrl+Left/Right by whitespace-delimited word; Home/End logical line, Ctrl+Home/End whole buffer. Backspace/Delete remove whole graphemes; Ctrl+W/Ctrl+Backspace delete the previous word. Enter sends; Shift+Enter inserts LF (no Alt+Enter fallback added). Tab recipient cycling and panel modifier shortcuts retain precedence. Paste/IME insert at the caret; CRLF/CR normalize to LF, other controls are filtered, accepted text stops at complete input graphemes under 8192 bytes. Composer has 1–6 text rows (3–8 bordered) and follows the caret. Failed validation or persistence keeps draft+cursor; successful send resets both and history scroll.

Left drag selects only cached transcript text; release finalizes, click resets, Ctrl+C/Ctrl+Shift+C copy selected bytes without terminal interrupts. Both halves of wide glyphs snap to the glyph start; combining/emoji graphemes are never split. Out-of-area drags clamp and cannot activate chrome/panes. Selection remains anchored through scrolling/appends, clears on width rewrap and re-entry, and is hidden/cleared when the room is left. **Soft-wrap newlines are preserved in copied text**; no app chrome/control bytes are copied. Composer clicks hit the rendered grapheme; batched edits before a click cannot index stale byte ranges. Pre-entry key-release ownership remains unchanged.

**Files:** new `src/app/room/editor.rs`, `src/app/room/selection.rs`, `src/app/room/ux_tests.rs`; modified `src/app/room.rs`, `src/app/room/tests.rs`, `src/app/input/clipboard.rs` (visibility only for shared dispatch), `src/ui/room.rs`, `src/ui/tab_surface.rs`, `Cargo.toml`, `Cargo.lock`, `docs/next/rooms-prototype.md`, this file. `unicode-segmentation` is now a direct dependency using the already-locked version: editor operations need extended grapheme boundaries including LF, whereas Ratatui's styled-grapheme iterator filters control graphemes.

**Verification:** `just test-one room`: **53 passed** (43 existing + 10 new); `just test-one app::input`: **363 passed**. New deterministic tests cover local/headless App input, paste byte limits/newline normalization, grapheme editing/word movement, reversed wide/combining/emoji selection, TestBackend highlight/caret, clipboard event exact bytes and no PTY leakage, scroll retention and drag chrome ownership, resize invalidation, tiny geometry, bounded composer scrolling, successful reset and actual persistence-gate failure retaining the caret. Existing room lifecycle matrix still proves old lease releases and no hidden PTY input. **9 Python tests**, **19 Node receiver tests**, **6 UI hot-path architecture tests** pass. Python includes only disposable shell-server restart and installed-Pi loader/private fake-socket checks, no model calls. Debug `target/debug/herdr` built with `mise x zig@0.15.2 -- cargo build --locked`, ready for the disposable wrapper. Formatting/diff checks pass. `just lint` reports the same six previously documented unrelated Rust-1.98 Clippy errors, no new diagnostics; no broad Rust suite or performance/release benchmark run.

Logs: `/tmp/rooms-text-{tests,input,python,node,architecture,build,lint}.log`. Safe physical preview command (not run interactively by this writer):

```sh
python3 integrations/room/disposable.py
```

### Live room editor + automatic membership

Committed editor as 4128079 and built release. User then authorized default automatic Pi receiver joining and live update. Adapter now auto-starts only with explicit absolute Herdr socket and pane identity unless HERDR_ROOM_ENABLED=0; reload/restart recreates an enabled receiver. No history replay. 20 Node +9 Python tests pass; real two-Haiku broadcast/targeted smoke passed without HERDR_ROOM_ENABLED. Installed receiver.mjs atomically after backup, then release->release handoff returned 0 at stable sockets; 28 panes/18 agent records remain, one executable-path-matched Pi process retained its start time (not a complete per-agent PID inventory). Backups/receipt: ~/.local/state/herdr-upgrades/room-editor-20260917-160605/. Existing agents require one /reload to load new adapter. Automatic-membership source/docs/test changes remain uncommitted pending commit-message approval.

### Pi-style room composer — fork-only candidate on `4128079`

Sole writer; no children, commits/pushes, deployment/live handoff, global config/extension edits or delivery changes. Parent's pre-existing automatic-membership changes in `integrations/room/pi/receiver.mjs`, Node/Python tests and these docs are preserved. This candidate is **not installed** and has not received physical keyboard/IME verification.

Read complete installed Pi 0.85.1 keybindings/TUI/terminal-setup documentation, its embedded `editor.ts`, `word-navigation.ts`, `kill-ring.ts`, `undo-stack.ts`, and relevant version-tagged upstream word/editor/history fixtures. Added room-local visual-row navigation, UTF-16 sticky offsets with grapheme snapping, successful-send-only 100-entry history and original draft/cursor restoration; Pi logical-line/word/grapheme aliases, char jump, kill accumulation/yank/pop, bounded undo including paste registry and pre-history snapshots. Enter expands/trims; Shift+Enter/Ctrl+J newline; backslash-Enter newline; Ctrl/Alt+Enter unbound. Registered atomic paste folding, CR/LF/tab normalization, path separation and expanded 8192-byte cap are implemented; typed marker lookalikes remain literal. Failed sends preserve draft/cursor and never enter history. Clipboard selection pipeline, PTY isolation and pinned releases are untouched.

Rendering uses top/bottom rules only, word-aware whitespace-preserving wrapping with CJK break opportunities, reserved cursor cell, adaptive 1..max(5, floor(terminalHeight*.3)) rows, scroll-border indicators, inverse grapheme/end-blank cursor and hidden hardware cursor metadata with IME coordinates retained. Plain PgUp/PgDn and wheel remain transcript controls; Ctrl+PgUp/PgDn editor paging. Tab recipient, room toggle and panel shortcuts stay room-specific. Geometry is computed outside pure render; transcript cache stays incremental. Work is active-room-local, not pane-scaled; no new locks/I/O/dependencies or render-scaling profile needed.

Files: `src/app/room/editor.rs`, new `editor_tests.rs` and `parity_tests.rs`, `src/app/room.rs`, `src/app/room/tests.rs`, `ux_tests.rs`, room-only IME branches in `src/app/input/mod.rs`, `src/ui/room.rs`, room-height arguments in `src/ui.rs`, room cursor metadata in `src/ui/tab_surface.rs`; additive docs. Four old UX tests were updated for the explicitly changed Up/Down, Ctrl+Home/End, four-space tabs, folded payload lengths and adaptive-height contracts; selection/copy/release characterization stays intact.

**Verification, final source:** `just test-one room` **62 passed**; `just test-one app::input` **363 passed**; full `cargo nextest run --locked --no-fail-fast` **3586 run, 3578 passed, the same 8 documented baseline failures, 1 skipped**. Used direct full nextest because the standard recipe stops at baseline failures. `just ui-hot-path-architecture-test` **6 passed**. `cargo fmt --check`, `git diff --check`, and debug `cargo build --locked` pass. Logs `/tmp/rooms-pi-{tests,input,full,architecture,build}.log`. Parent's 20 Node/9 Python changes were not edited or rerun; no real-model/physical UI/release build or deployment tests performed.

**Honest parity limits:** UAX #29 rather than ICU dictionary word segmentation (Han per-character vs Pi word pairs); non-ASCII punctuation/CJK classifications and some emoji/resize sticky-column edge cases are not exhaustively equivalent. Marker IDs are monotonic until submit instead of Pi backspace renumbering, registration is positional to keep typed lookalikes literal, killed folded payload yanks expanded, cursor inverses a displayed grapheme rather than the entire marker, and narrow split-marker continuation navigation is not exhaustively matched. Undo/kill rings cap at 100 entries each rather than unbounded; C1 controls also filtered. No custom Pi bindings, image paste, external editor, slash completion, thinking borders or orchestrator. `docs/next/rooms-prototype.md` now has a superseding composer-controls section with exact bindings and these gaps. No claim of complete Pi parity.

### Room group UI simplification — fork-only, uncommitted

Removed room metadata/header/member-count/recipient-picker surfaces, reclaiming their rows for conversation. The Pi-style composer is unchanged; empty status occupies no rows, error/delivery status remains muted. UI Enter always posts `recipient: None` and clears any legacy presentation target. Tab/BackTab are consumed without recipient selection or hidden-pane focus; existing panel shortcuts retain precedence. Explicit targeted `room.post` scripting/API support is untouched.

The existing sidebar highlights every current active-workspace agent using constant-time pane/terminal/public-number identity lookups (no cached member-vector scan, I/O or process inspection). During room viewing agent rows show name + public pane ID; named shells and other workspaces are not highlighted. Expanded/collapsed highlighting is covered; leaving the room restores custom topic/model/name formatting and focused-pane styling exactly. Existing agent filtering/navigation and real-pane click behavior are unchanged.

Preserved prior Pi editor/input/layout and parent autojoin edits. The only `parity_tests.rs` adjustment replaces its obsolete stale-recipient send-failure fixture with the existing injected persistence failure, retaining history/undo assertions; no editor/parity rewrite. Updated equivalent obsolete UX/input expectations and added broadcast delivery/non-fanout and room/sidebar render regressions.

Verification: **66 room tests**, **141 sidebar tests**, debug build, `cargo fmt --check`, and `git diff --check` pass. A bounded debug fixed-geometry render profile (not release or baseline comparison) at 1/15/50 panes reports room medians **5674/5691/5766 µs**, 15:1 **1.003×**; background workspace **5296/5844/6129 µs**, active pane **5272/7395/10433 µs**. Release profiling/full suite were not rerun to keep this requested follow-up bounded. No known focused-test blocker; human physical UI validation remains outstanding. No commit, install, deployment, child agents or global edits.

### Parent transcript/clipboard/sidebar verification

Agent headers now contain only stored author name + pane ID; human rows have no header and highlighted background. Clipboard defect reproduced before fix: room mouse release ignored normal copy_on_select. Now copies via existing foreground-client clipboard route, with regression exercising ServerMessage::Clipboard and decoded OSC52 bytes (not proof of physical clipboard acceptance). Parent finished restoring normal configured sidebar rows/heights while retaining room-member highlighting, and reran 73 room tests + prior 141 sidebar tests successfully. Full parent suite: 3589 passed, same 8 baseline failures, 1 skipped; `/tmp/room-polish-parent-tests.log`. Workspace view-memory behavior tested by builder and parent room suite. Pending changes also include previously live-installed reload-cache loader fix with 11 Python tests verified earlier. No new deployment/commit yet.

### Workspace-tinted replies and actual chosen names (pending)

Parent implemented agent-row background (including header/padding, excluding spacers) as 20% workspace accent +80% theme surface; current light orange resolves #f7e3d9. Human grey unchanged. Agent headers now chosen name alone, pane ID fallback. Root name bug confirmed live: self-chosen names live in metadata_tokens["name"] (e.g. ∴ Aporia), while room membership read only terminal.agent_name, falling back to pi. Added narrow metadata accessor and proper precedence +decorative-prefix cleanup; historical unnamed records show stored pane IDs rather than rewriting attribution. Added metadata-name, fallback/Unicode-name, light/dark tint and exact render/copy tests. All76 room tests passed; fmt/diff checked. No deploy/commit yet. Separately user-approved grey is live via ~/.config/herdr/config.toml theme.custom.surface0=#f5f5f7 matching Pi omarchy-system panel; backup in room-polish-20260917-175813/config.before-message-grey.toml.
