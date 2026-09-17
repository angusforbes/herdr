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
