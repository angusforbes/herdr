//! Client presentation for exact Pi messages. Forking goes through the runtime API.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use crate::{
    api::schema::{AgentConversationParams, AgentForkParams, Method},
    app::{state::Mode, App},
    pi_conversation::{ConversationMessage, MessageRef},
    pi_fork::BranchPosition,
};

#[derive(Debug, Clone)]
pub(crate) struct ConversationPreview {
    pub target: String,
    pub messages: Vec<ConversationMessage>,
    pub selected: usize,
    pub tree_scroll: usize,
    pub text_scroll: usize,
    pub status: String,
}

impl ConversationPreview {
    pub(crate) fn selected_message(&self) -> Option<&ConversationMessage> {
        self.messages.get(self.selected)
    }
}

/// Same geometry for drawing and hit testing. Two action rows fit narrow panels.
pub(crate) fn preview_rects(area: Rect) -> (Rect, Rect, Rect, Rect) {
    let x = area.x.saturating_add(1);
    let width = area.width.saturating_sub(1);
    let controls = Rect::new(x, area.y, width, area.height.min(2));
    let remaining = area.height.saturating_sub(3);
    let tree_height = (remaining / 2).min(12);
    let tree = Rect::new(x, area.y.saturating_add(2), width, tree_height);
    let status = Rect::new(
        x,
        tree.y.saturating_add(tree.height),
        width,
        u16::from(area.height > 2),
    );
    let text = Rect::new(
        x,
        status.y.saturating_add(status.height),
        width,
        remaining.saturating_sub(tree_height),
    );
    (controls, tree, status, text)
}

impl App {
    pub(crate) fn open_conversation_preview(&mut self, target: String, reference: MessageRef) {
        let response = self.dispatch_runtime_mutation(
            "search-conversation",
            Method::AgentConversation(AgentConversationParams {
                target: target.clone(),
                query: None,
                limit: Some(500),
                offset: Some(0),
                around_entry_id: Some(reference.entry_id.clone()),
            }),
        );
        let parsed: serde_json::Value = match serde_json::from_str(&response) {
            Ok(v) => v,
            Err(_) => {
                self.state.search_pane.status = Some("conversation response unreadable".into());
                return;
            }
        };
        if let Some(error) = parsed.get("error") {
            self.state.search_pane.status = Some(
                error["message"]
                    .as_str()
                    .unwrap_or("conversation unavailable")
                    .into(),
            );
            return;
        }
        let result = &parsed["result"];
        let messages: Vec<ConversationMessage> =
            match serde_json::from_value(result["messages"].clone()) {
                Ok(m) => m,
                Err(_) => {
                    self.state.search_pane.status =
                        Some("conversation messages unavailable".into());
                    return;
                }
            };
        let Some(selected) = messages.iter().position(|m| m.reference == reference) else {
            self.state.search_pane.status =
                Some("message changed or outside preview page; search again".into());
            return;
        };
        let total = result["total"].as_u64().unwrap_or(messages.len() as u64);
        let mut status = format!("{} messages", messages.len());
        if total > messages.len() as u64 {
            status = format!("showing {} of {total} messages", messages.len());
        }
        if result["partial_tail"] == true {
            status.push_str(" · writing…");
        }
        self.state.conversation_preview = Some(ConversationPreview {
            target,
            messages,
            selected,
            tree_scroll: selected.saturating_sub(3),
            text_scroll: 0,
            status,
        });
        self.state.focus_search_pane();
    }

    pub(crate) fn handle_conversation_key(&mut self, key: KeyEvent) -> bool {
        if self.state.conversation_preview.is_none() {
            return false;
        }
        if key.kind == crossterm::event::KeyEventKind::Release {
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                self.state.conversation_preview = None;
            }
            KeyCode::Enter => self.fork_preview_message(
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT)
                {
                    BranchPosition::Continue
                } else {
                    BranchPosition::Rewrite
                },
            ),
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.fork_preview_message(BranchPosition::Rewrite)
            }
            KeyCode::Up => self.move_conversation_selection(-1),
            KeyCode::Down => self.move_conversation_selection(1),
            KeyCode::PageUp => self.scroll_conversation_text(-5),
            KeyCode::PageDown => self.scroll_conversation_text(5),
            _ => {}
        }
        true
    }

    pub(crate) fn move_conversation_selection(&mut self, delta: isize) {
        let height = usize::from(preview_rects(self.state.view.search_pane_rect).1.height).max(1);
        if let Some(p) = self.state.conversation_preview.as_mut() {
            if p.messages.is_empty() {
                return;
            }
            p.selected =
                (p.selected as isize + delta).clamp(0, p.messages.len() as isize - 1) as usize;
            p.text_scroll = 0;
            if p.selected < p.tree_scroll {
                p.tree_scroll = p.selected;
            }
            if p.selected >= p.tree_scroll + height {
                p.tree_scroll = p.selected + 1 - height;
            }
        }
    }

    pub(crate) fn scroll_conversation_text(&mut self, delta: isize) {
        if let Some(p) = self.state.conversation_preview.as_mut() {
            let max = p.selected_message().map_or(0, |m| m.text.chars().count());
            p.text_scroll = (p.text_scroll as isize + delta)
                .clamp(0, max.min(u16::MAX as usize) as isize) as usize;
        }
    }

    pub(crate) fn handle_conversation_click(&mut self, col: u16, row: u16, shift: bool) -> bool {
        if self.state.conversation_preview.is_none() {
            return false;
        }
        self.state.mode = Mode::SearchPane;
        let (controls, tree, _, _) = preview_rects(self.state.view.search_pane_rect);
        if col < controls.x || col >= controls.x.saturating_add(controls.width) {
            return true;
        }
        let x = col - controls.x;
        if row == controls.y {
            if x < 17 {
                self.state.conversation_preview = None;
            }
            return true;
        }
        if row == controls.y + 1 {
            // Only the painted buttons activate; blank panel space never forks.
            if x < 23 {
                self.fork_preview_message(if x >= 13 || shift {
                    BranchPosition::Continue
                } else {
                    BranchPosition::Rewrite
                });
            }
        } else if row >= tree.y && row < tree.y + tree.height {
            if let Some(p) = self.state.conversation_preview.as_mut() {
                let index = p.tree_scroll + usize::from(row - tree.y);
                if index < p.messages.len() {
                    p.selected = index;
                    p.text_scroll = 0;
                } else {
                    return true;
                }
            }
            if shift {
                self.fork_preview_message(BranchPosition::Continue);
            }
        }
        true
    }

    pub(crate) fn fork_preview_message(&mut self, position: BranchPosition) {
        let Some(preview) = self.state.conversation_preview.as_ref() else {
            return;
        };
        let Some(message) = preview.selected_message() else {
            return;
        };
        let supported = match position {
            BranchPosition::Rewrite => message.can_rewrite,
            BranchPosition::Continue => message.can_continue,
        };
        if !supported {
            if let Some(p) = self.state.conversation_preview.as_mut() {
                p.status = match position {
                    BranchPosition::Rewrite => "Rewrite needs a text-only user prompt".into(),
                    BranchPosition::Continue => {
                        "Cannot continue inside an unfinished tool turn".into()
                    }
                };
            }
            return;
        }
        let params = AgentForkParams {
            target: preview.target.clone(),
            reference: message.reference.clone(),
            position,
        };
        let response = self.dispatch_runtime_mutation("search-fork", Method::AgentFork(params));
        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap_or_default();
        if let Some(error) = parsed.get("error") {
            if let Some(p) = self.state.conversation_preview.as_mut() {
                p.status = error["message"].as_str().unwrap_or("fork failed").into();
            }
        } else if parsed.get("result").is_some() {
            self.state.conversation_preview = None;
            self.state.search_pane.status =
                Some("fork launch requested · original unchanged · same working directory".into());
            self.state.mode = Mode::Terminal;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn blank_space_and_border_do_not_activate_fork_buttons() {
        let mut app = crate::app::input::app_for_mouse_test();
        app.state.view.search_pane_rect = Rect::new(40, 0, 64, 25);
        let message = ConversationMessage {
            reference: MessageRef {
                session_path: "/fixture".into(),
                session_id: "session".into(),
                entry_id: "entry".into(),
                ancestry_hash: "hash".into(),
            },
            parent_message_id: None,
            role: "assistant".into(),
            text: "answer".into(),
            text_truncated: false,
            timestamp: String::new(),
            depth: 0,
            active: true,
            can_rewrite: false,
            can_continue: false,
        };
        app.state.conversation_preview = Some(ConversationPreview {
            target: "fixture".into(),
            messages: vec![message],
            selected: 0,
            tree_scroll: 0,
            text_scroll: 0,
            status: "unchanged".into(),
        });
        assert!(app.handle_conversation_click(100, 1, false));
        assert!(app.handle_conversation_click(40, 1, true));
        assert_eq!(
            app.state.conversation_preview.as_ref().unwrap().status,
            "unchanged"
        );
        app.handle_conversation_click(43, 1, false);
        assert!(app
            .state
            .conversation_preview
            .as_ref()
            .unwrap()
            .status
            .contains("text-only user"));
        app.state.conversation_preview.as_mut().unwrap().status = "unchanged".into();
        app.handle_conversation_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
        assert!(app
            .state
            .conversation_preview
            .as_ref()
            .unwrap()
            .status
            .contains("unfinished tool"));
    }
}
