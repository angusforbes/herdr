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
