//! Global, non-modal TUI panel shortcuts. These do not change runtime identity.
use crossterm::event::{KeyCode, KeyEventKind};

use super::navigate::{ActionContext, NavigateAction};

use crate::{
    app::{state::ViewLayout, App, AppState, Mode},
    input::TerminalKey,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Panel {
    Spaces,
    Agents,
    Terminal,
    Search,
}

impl AppState {
    fn cycle_panel_focus(&mut self, reverse: bool) {
        let mut panels = Vec::with_capacity(4);
        if !self.sidebar_collapsed
            && self.view.layout != ViewLayout::Mobile
            && self.view.sidebar_rect.width > 1
            && self.view.sidebar_rect.height > 0
        {
            let (spaces, agents) = crate::ui::expanded_sidebar_sections(
                self.view.sidebar_rect,
                self.sidebar_section_split,
            );
            if spaces.height > 0 {
                panels.push(Panel::Spaces);
            }
            if agents.height >= 3 {
                panels.push(Panel::Agents);
            }
        }
        if self.active.is_some() {
            panels.push(Panel::Terminal);
        }
        if self.search_pane.visible && self.view.search_pane_rect.width > 0 {
            panels.push(Panel::Search);
        }
        if panels.is_empty() {
            return;
        }
        let current = match self.mode {
            Mode::Navigate if self.navigate_agents => Panel::Agents,
            Mode::Navigate => Panel::Spaces,
            Mode::SearchPane => Panel::Search,
            _ => Panel::Terminal,
        };
        let next = match panels.iter().position(|panel| *panel == current) {
            Some(index) if reverse => (index + panels.len() - 1) % panels.len(),
            Some(index) => (index + 1) % panels.len(),
            None if reverse => panels.len() - 1,
            None => 0,
        };
        self.navigate_agents = panels[next] == Panel::Agents;
        if self.navigate_agents {
            if let Some(entry) = crate::ui::agent_panel_entries(self)
                .into_iter()
                .find(|entry| self.is_active_pane(entry.ws_idx, entry.tab_idx, entry.pane_id))
            {
                self.ensure_agent_panel_entry_visible(entry.index - 1);
            }
        }
        self.mode = match panels[next] {
            Panel::Spaces | Panel::Agents => Mode::Navigate,
            Panel::Terminal => Mode::Terminal,
            Panel::Search => Mode::SearchPane,
        };
    }
}

impl App {
    /// Plain vertical arrows belong to the main terminal even while a sidebar
    /// accepts text/other keys. Prefix and modal controls retain their semantics.
    pub(crate) fn panel_arrow_targets_terminal(&self, key: &TerminalKey) -> bool {
        self.state.popup_pane.is_none()
            && !self.state.room_active()
            && matches!(self.state.mode, Mode::Navigate | Mode::SearchPane)
            && matches!(key.code, KeyCode::Up | KeyCode::Down)
            && key.modifiers.is_empty()
    }

    /// Shared by local TUI and attached-client input routes, before ordinary
    /// terminal forwarding or Search's Tab/text handling. Never captures dialogs,
    /// copy mode, or popups. Only the Search toggle opens a closed sidebar.
    pub(crate) fn handle_panel_shortcut(&mut self, key: &TerminalKey) -> bool {
        if self.state.popup_pane.is_some()
            || !matches!(
                self.state.mode,
                Mode::Terminal | Mode::Navigate | Mode::SearchPane | Mode::Prefix
            )
        {
            return false;
        }
        let prefix = self.state.mode == Mode::Prefix;
        let matches = |binding: &crate::config::ActionKeybinds| {
            if prefix {
                binding.matches_prefix_key(key)
            } else {
                binding.matches_direct_key(key)
            }
        };
        let kb = &self.state.keybinds;
        if matches(&kb.search_pane) {
            if key.kind != KeyEventKind::Release {
                if prefix {
                    self.state.mode = if self.state.active.is_some() {
                        Mode::Terminal
                    } else {
                        Mode::Navigate
                    };
                    self.state.navigate_agents = false;
                }
                if self.state.search_pane.visible {
                    self.cancel_ai_search();
                }
                self.state.toggle_search_pane();
            }
            return true;
        }
        let reverse = matches(&kb.focus_panel_previous);
        if reverse || matches(&kb.focus_panel_next) {
            if key.kind != KeyEventKind::Release {
                self.state.cycle_panel_focus(reverse);
            }
            return true;
        }
        let navigation = [
            (&kb.previous_workspace, NavigateAction::PreviousWorkspace),
            (&kb.next_workspace, NavigateAction::NextWorkspace),
            (&kb.previous_agent, NavigateAction::PreviousAgent),
            (&kb.next_agent, NavigateAction::NextAgent),
        ]
        .into_iter()
        .find_map(|(binding, action)| matches(binding).then_some(action));
        if let Some(action) = navigation {
            if key.kind != KeyEventKind::Release {
                let focus = self.state.mode;
                let agents = self.state.navigate_agents;
                self.execute_tui_navigate_action(action, ActionContext::Direct);
                if !prefix {
                    self.state.mode = focus;
                    self.state.navigate_agents = agents;
                }
            }
            return true;
        }
        let previous = matches(&kb.search_result_previous);
        let next = matches(&kb.search_result_next);
        // Reserve result navigation even when closed: never fall through to a
        // terminal app or an overlapping resize binding.
        if !self.state.search_pane.visible {
            return previous || next;
        }
        let input = matches(&kb.search_input);
        let mode = matches(&kb.search_mode);
        if !(previous || next || input || mode) {
            return false;
        }
        if key.kind == KeyEventKind::Release {
            return true;
        }
        if prefix {
            self.state.mode = if self.state.active.is_some() {
                Mode::Terminal
            } else {
                Mode::Navigate
            };
            self.state.navigate_agents = false;
        }
        if previous || next {
            // Visit the hit in the main pane, but retain keyboard focus so a user
            // can keep editing the query or browsing a sidebar with these keys.
            if self.state.search_pane.results_fresh() {
                let focus = self.state.mode;
                let agents = self.state.navigate_agents;
                self.state
                    .move_search_pane_selection(if previous { -1 } else { 1 });
                if let Some(flat) = self.state.search_pane.selected {
                    self.state.jump_to_search_hit(&self.terminal_runtimes, flat);
                }
                self.state.mode = focus;
                self.state.navigate_agents = agents;
            }
        } else if input {
            self.state.focus_search_pane();
            // Enter should submit the query, not activate an old selected hit.
            self.state.search_pane.selected = None;
        } else {
            let mode = self.state.search_pane.mode.toggled();
            self.switch_search_mode(mode);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::input::app_for_mouse_test, workspace::Workspace};
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::layout::Rect;

    fn app() -> App {
        let mut app = app_for_mouse_test();
        let mut config = crate::config::Config::default();
        config.keys.previous_workspace = crate::config::BindingConfig::one("alt+up");
        config.keys.next_workspace = crate::config::BindingConfig::one("alt+down");
        config.keys.previous_agent = crate::config::BindingConfig::one("shift+up");
        config.keys.next_agent = crate::config::BindingConfig::one("shift+down");
        app.state.keybinds = config.keybinds();
        app.state.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.state.active = Some(0);
        app.state.selected = 1;
        app.state.search_pane.visible = true;
        app.state.search_pane.query = "keep my query".into();
        app.state.search_pane.scroll = 4;
        app.state.view.search_pane_rect = Rect::new(80, 0, 30, 20);
        app
    }

    fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        let key = TerminalKey::new(code, modifiers);
        app.route_client_events(
            vec![
                crate::raw_input::RawInputEvent::Key(key.clone()),
                crate::raw_input::RawInputEvent::Key(key.with_kind(KeyEventKind::Release)),
            ],
            false,
        );
    }

    #[test]
    fn panel_shortcuts_cycle_both_directions_through_attached_client_route() {
        let mut app = app();
        let expected = [
            (Mode::SearchPane, false),
            (Mode::Navigate, false),
            (Mode::Navigate, true),
            (Mode::Terminal, false),
        ];
        for (mode, agents) in expected {
            press(&mut app, KeyCode::Tab, KeyModifiers::CONTROL);
            assert_eq!((app.state.mode, app.state.navigate_agents), (mode, agents));
        }
        for (mode, agents) in [
            (Mode::Navigate, true),
            (Mode::Navigate, false),
            (Mode::SearchPane, false),
            (Mode::Terminal, false),
        ] {
            press(
                &mut app,
                KeyCode::BackTab,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            );
            assert_eq!((app.state.mode, app.state.navigate_agents), (mode, agents));
        }
        assert_eq!(app.state.selected, 1);
        assert_eq!(app.state.active, Some(0));
        assert_eq!(app.state.search_pane.query, "keep my query");
        assert_eq!(app.state.search_pane.scroll, 4);
    }

    #[test]
    fn panel_shortcuts_search_toggle_works_from_every_panel() {
        for (mode, agents) in [
            (Mode::Terminal, false),
            (Mode::Navigate, false),
            (Mode::Navigate, true),
            (Mode::SearchPane, false),
        ] {
            let mut app = app();
            app.state.mode = mode;
            app.state.navigate_agents = agents;
            press(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
            assert!(
                !app.state.search_pane.visible,
                "close from {mode:?}, agents={agents}"
            );
            app.state.mode = if mode == Mode::SearchPane {
                Mode::Terminal
            } else {
                mode
            };
            app.state.navigate_agents = agents;
            press(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
            assert!(
                app.state.search_pane.visible,
                "open from {mode:?}, agents={agents}"
            );
            assert_eq!(app.state.mode, Mode::SearchPane);
            assert_eq!(app.state.search_pane.query, "keep my query");
        }
    }

    #[test]
    fn panel_shortcuts_skip_closed_collapsed_mobile_and_unrendered_panels() {
        let mut app = app();
        app.state.search_pane.visible = false;
        press(&mut app, KeyCode::Tab, KeyModifiers::CONTROL);
        assert_eq!(app.state.mode, Mode::Navigate);
        assert!(!app.state.navigate_agents);
        app.state.sidebar_collapsed = true;
        press(&mut app, KeyCode::Tab, KeyModifiers::CONTROL);
        assert_eq!(app.state.mode, Mode::Terminal);
        press(&mut app, KeyCode::Tab, KeyModifiers::CONTROL);
        assert_eq!(app.state.mode, Mode::Terminal);
        app.state.sidebar_collapsed = false;
        app.state.view.layout = ViewLayout::Mobile;
        press(&mut app, KeyCode::Tab, KeyModifiers::CONTROL);
        assert_eq!(app.state.mode, Mode::Terminal);
        app.state.search_pane.visible = true;
        app.state.view.search_pane_rect = Rect::default();
        press(&mut app, KeyCode::Tab, KeyModifiers::CONTROL);
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn panel_shortcuts_do_not_capture_modal_or_closed_search_keys() {
        let mut app = app();
        for mode in [
            Mode::Settings,
            Mode::Navigator,
            Mode::Copy,
            Mode::RenamePane,
        ] {
            app.state.mode = mode;
            for code in [KeyCode::Tab, KeyCode::Char('\''), KeyCode::Char('/')] {
                assert!(!app.handle_panel_shortcut(&TerminalKey::new(code, KeyModifiers::CONTROL)));
                assert_eq!(app.state.mode, mode);
            }
        }
        app.state.mode = Mode::Terminal;
        app.state.search_pane.visible = false;
        for code in [KeyCode::Char('\''), KeyCode::Char('/')] {
            assert!(!app.handle_panel_shortcut(&TerminalKey::new(code, KeyModifiers::CONTROL)));
        }
    }

    #[test]
    fn panel_shortcuts_modified_agents_arrows_browse_agents_and_enter_returns_to_terminal() {
        let mut app = app();
        app.state.ensure_test_terminals();
        for terminal in app.state.terminals.values_mut() {
            terminal.detected_agent = Some(crate::detect::Agent::Claude);
        }
        press(
            &mut app,
            KeyCode::BackTab,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert!(app.state.navigate_agents);
        press(&mut app, KeyCode::Down, KeyModifiers::SHIFT);
        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.mode, Mode::Navigate);
        assert!(app.state.navigate_agents);
        press(&mut app, KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(app.state.active, Some(0));
        assert!(app.state.navigate_agents);
        press(&mut app, KeyCode::Enter, KeyModifiers::empty());
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(!app.state.navigate_agents);
    }

    #[test]
    fn panel_shortcuts_space_and_agent_families_work_from_every_panel() {
        for (mode, agents) in [
            (Mode::Terminal, false),
            (Mode::Navigate, false),
            (Mode::Navigate, true),
            (Mode::SearchPane, false),
        ] {
            for modifiers in [KeyModifiers::ALT, KeyModifiers::SHIFT] {
                let mut app = app();
                app.state.ensure_test_terminals();
                for terminal in app.state.terminals.values_mut() {
                    terminal.detected_agent = Some(crate::detect::Agent::Claude);
                }
                app.state.mode = mode;
                app.state.navigate_agents = agents;
                press(&mut app, KeyCode::Down, modifiers);
                assert_eq!(app.state.active, Some(1));
                assert_eq!((app.state.mode, app.state.navigate_agents), (mode, agents));
                press(&mut app, KeyCode::Up, modifiers);
                assert_eq!(app.state.active, Some(0));
                assert_eq!((app.state.mode, app.state.navigate_agents), (mode, agents));
                assert_eq!(app.state.search_pane.query, "keep my query");
            }
        }
    }

    #[tokio::test]
    async fn panel_shortcuts_plain_arrows_reach_terminal_from_every_panel_with_repeats() {
        for (mode, agents) in [
            (Mode::Terminal, false),
            (Mode::Navigate, false),
            (Mode::Navigate, true),
            (Mode::SearchPane, false),
        ] {
            let mut app = app();
            let pane_id = app.state.workspaces[0].tabs[0].root_pane;
            let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
            app.state.workspaces[0].tabs[0]
                .runtimes
                .insert(pane_id, runtime);
            app.state.mode = mode;
            app.state.navigate_agents = agents;
            let up = TerminalKey::new(KeyCode::Up, KeyModifiers::empty());
            app.route_client_events(
                vec![
                    crate::raw_input::RawInputEvent::Key(up.clone()),
                    crate::raw_input::RawInputEvent::Key(
                        up.clone().with_kind(KeyEventKind::Repeat),
                    ),
                    crate::raw_input::RawInputEvent::Key(up.with_kind(KeyEventKind::Release)),
                ],
                false,
            );
            assert_eq!(rx.try_recv().unwrap().as_ref(), b"\x1b[A");
            assert_eq!(rx.try_recv().unwrap().as_ref(), b"\x1b[A");
            assert!(rx.try_recv().is_err());
            assert!(app.input_leases.is_empty());
            assert_eq!((app.state.mode, app.state.navigate_agents), (mode, agents));
            assert_eq!(app.state.active, Some(0));
            assert_eq!(app.state.selected, 1);
            assert_eq!(app.state.search_pane.selected, None);
            assert_eq!(app.state.search_pane.query, "keep my query");
            // Monolithic TUI routing uses the same policy.
            assert!(app
                .handle_key(TerminalKey::new(KeyCode::Down, KeyModifiers::empty()))
                .await
                .is_some());
            assert_eq!(rx.try_recv().unwrap().as_ref(), b"\x1b[B");
            assert_eq!((app.state.mode, app.state.navigate_agents), (mode, agents));
        }
    }

    #[test]
    fn panel_shortcuts_result_arrows_are_reserved_when_search_closed() {
        let mut app = app();
        app.state.search_pane.visible = false;
        for code in [KeyCode::Up, KeyCode::Down] {
            assert!(app.handle_panel_shortcut(&TerminalKey::new(code, KeyModifiers::CONTROL)));
        }
        assert!(!app.state.search_pane.visible);
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn panel_shortcuts_decode_real_csi_u_ctrl_tab_and_reverse() {
        let mut app = app();
        app.route_client_input(b"\x1b[9;5u\x1b[9;5:3u".to_vec());
        assert_eq!(app.state.mode, Mode::SearchPane);
        app.route_client_input(b"\x1b[9;6u\x1b[9;6:3u".to_vec());
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[tokio::test]
    async fn panel_shortcuts_work_through_local_tui_route() {
        let mut app = app();
        app.handle_key(TerminalKey::new(KeyCode::Tab, KeyModifiers::CONTROL))
            .await;
        assert_eq!(app.state.mode, Mode::SearchPane);
        app.handle_key(TerminalKey::new(KeyCode::Tab, KeyModifiers::CONTROL))
            .await;
        assert_eq!(app.state.mode, Mode::Navigate);
        app.handle_key(TerminalKey::new(KeyCode::Tab, KeyModifiers::CONTROL))
            .await;
        assert!(app.state.navigate_agents);
        app.handle_key(TerminalKey::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(!app.state.navigate_agents);
    }
}
