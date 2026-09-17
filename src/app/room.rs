//! Room client presentation and input; shared facts are in Workspace::room.
#[cfg(test)]
mod tests;
use crate::{
    app::{App, AppState, Mode},
    input::TerminalKey,
    room::Member,
};
use crossterm::event::{
    KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

#[derive(Default)]
pub struct RoomPresentation {
    pub workspace: Option<String>,
    pub visible: bool,
    pub transcript_lines: Vec<String>,
    pub transcript_width: u16,
    pub transcript_messages: usize,
    pub composer: String,
    pub members: Vec<Member>,
    pub recipient: Option<Member>,
    pub status: String,
    pub scroll: usize,
    refreshed: Option<std::time::Instant>,
}

impl AppState {
    pub(crate) fn room_active(&self) -> bool {
        self.room_ui.visible
            && self
                .active
                .and_then(|i| self.workspaces.get(i))
                .is_some_and(|ws| self.room_ui.workspace.as_deref() == Some(ws.id.as_str()))
    }

    pub(crate) fn select_room(&mut self) {
        let Some(index) = self.active else { return };
        let Some(ws) = self.workspaces.get(index) else {
            return;
        };
        if self.room_ui.workspace.as_deref() != Some(ws.id.as_str()) {
            self.room_ui = RoomPresentation {
                workspace: Some(ws.id.clone()),
                ..Default::default()
            };
        }
        self.room_ui.visible = true;
        self.room_ui.members = crate::room::members(self, index);
        self.room_ui.refreshed = None;
        self.mode = Mode::Terminal;
        self.clear_selection();
        self.drag = None;
        self.tab_presses.clear();
        self.view.pane_infos.clear();
        self.view.split_borders.clear();
    }

    pub(crate) fn insert_room_text(&mut self, text: &str) {
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            if self.room_ui.composer.len() + ch.len_utf8() > crate::room::MAX_MESSAGE_BYTES {
                break;
            }
            self.room_ui.composer.push(ch);
        }
    }
}

impl App {
    /// Bounded presentation refresh only while viewing a room, never from render.
    /// API membership is derived immediately and independently of this cache.
    pub(crate) fn refresh_room_members(&mut self) {
        if !self.state.room_active() {
            return;
        }
        let now = std::time::Instant::now();
        if self
            .state
            .room_ui
            .refreshed
            .is_some_and(|last| now.duration_since(last).as_millis() < 250)
        {
            return;
        }
        if let Some(index) = self.state.active {
            let members = crate::room::members(&self.state, index);
            if members != self.state.room_ui.members {
                self.state.room_ui.members = members;
                self.render_dirty.request_generic();
                self.render_notify.notify_one();
            }
            // Keep a vanished selection as an invalid target: never silently turn an
            // addressed request into an unaddressed post or target a replacement.
        }
        self.state.room_ui.refreshed = Some(now);
    }

    pub(crate) fn handle_room_key(&mut self, key: &TerminalKey) -> bool {
        // Releases belong to the input lease ledger, even while a room is open.
        // Only that ledger knows which terminal received the original key-down.
        if key.kind == KeyEventKind::Release
            || self.state.popup_pane.is_some()
            || !matches!(
                self.state.mode,
                Mode::Terminal | Mode::Navigate | Mode::SearchPane
            )
        {
            return false;
        }
        if key.code == KeyCode::Char('r')
            && key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            if key.kind == KeyEventKind::Press {
                if self.state.room_active() {
                    self.state.room_ui.visible = false;
                } else {
                    self.state.select_room();
                }
            }
            return true;
        }
        if !self.state.room_active() || self.state.mode != Mode::Terminal {
            return false;
        }
        if self.handle_panel_shortcut(key) {
            return true;
        }
        match key.code {
            KeyCode::Esc => self.state.room_ui.visible = false,
            KeyCode::Backspace => {
                self.state.room_ui.composer.pop();
            }
            KeyCode::PageUp => {
                self.state.room_ui.scroll = self.state.room_ui.scroll.saturating_add(5)
            }
            KeyCode::PageDown => {
                self.state.room_ui.scroll = self.state.room_ui.scroll.saturating_sub(5)
            }
            KeyCode::Tab => {
                self.state.room_ui.status.clear();
                let members = &self.state.room_ui.members;
                let next = match self.state.room_ui.recipient.as_ref() {
                    None => members.first(),
                    Some(selected) => match members.iter().position(|m| m == selected) {
                        Some(index) => members.get(index + 1),
                        None => {
                            self.state.room_ui.status =
                                "Recipient changed; selection cleared to none. Tab to select again.".into();
                            None
                        }
                    },
                };
                self.state.room_ui.recipient = next.cloned();
            }
            KeyCode::Enter => self.post_room_composer(),
            KeyCode::Char('v')
                if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                if let Some(text) = crate::platform::read_clipboard_text() {
                    self.state.insert_room_text(&text);
                }
            }
            KeyCode::Char(ch)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                self.state.insert_room_text(&ch.to_string())
            }
            _ => {}
        }
        true
    }

    fn post_room_composer(&mut self) {
        let Some(index) = self.state.active else {
            return;
        };
        // Use the selected snapshot, not just its reused pane address.
        if let Some(selected) = &self.state.room_ui.recipient {
            let current = crate::room::members(&self.state, index);
            if !current.iter().any(|m| m == selected) {
                self.state.room_ui.status = "Recipient changed; select again. Nothing sent.".into();
                return;
            }
            if selected.session.is_none() {
                self.state.room_ui.status =
                    "Recipient has no live session identity. Nothing sent.".into();
                return;
            }
        }
        let response = self.dispatch_api_request(
            "room.composer",
            crate::api::schema::Method::RoomPost(crate::api::schema::RoomPostParams {
                workspace_id: self.state.workspaces[index].id.clone(),
                text: self.state.room_ui.composer.clone(),
                recipient: self.state.room_ui.recipient.as_ref().and_then(|member| {
                    member
                        .session
                        .as_ref()
                        .map(|session| crate::api::schema::RoomRecipient {
                            pane_id: member.pane_id.clone(),
                            terminal_id: member.terminal_id.clone(),
                            session: session.clone(),
                        })
                }),
            }),
        );
        match serde_json::from_str::<crate::api::schema::SuccessResponse>(&response) {
            Ok(crate::api::schema::SuccessResponse {
                result:
                    crate::api::schema::ResponseResult::RoomWritten {
                        persistence,
                        sequence,
                        ..
                    },
                ..
            }) => {
                self.state.room_ui.composer.clear();
                self.state.room_ui.scroll = 0;
                self.state.room_ui.status =
                    format!("#{sequence} {persistence}. No prompt dispatched.");
            }
            _ => {
                self.state.room_ui.status =
                    serde_json::from_str::<crate::api::schema::ErrorResponse>(&response)
                        .map(|r| r.error.message)
                        .unwrap_or_else(|_| "Room write failed".into())
            }
        }
    }

    pub(crate) fn handle_room_mouse(&mut self, mouse: MouseEvent) -> bool {
        if self.state.popup_pane.is_some()
            || !matches!(
                self.state.mode,
                Mode::Terminal | Mode::Navigate | Mode::SearchPane
            )
        {
            return false;
        }
        let position = ratatui::layout::Position::new(mouse.column, mouse.row);
        if self.state.view.room_hit_area.contains(position)
            && !matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                self.state.select_room();
            }
            return true;
        }
        if self.state.room_active() && self.state.view.terminal_area.contains(position) {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.state.room_ui.scroll = self.state.room_ui.scroll.saturating_add(3)
                }
                MouseEventKind::ScrollDown => {
                    self.state.room_ui.scroll = self.state.room_ui.scroll.saturating_sub(3)
                }
                MouseEventKind::Down(MouseButton::Left) => self.state.mode = Mode::Terminal,
                _ => {}
            }
            return true;
        }
        false
    }
}
