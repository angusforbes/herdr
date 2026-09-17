//! Room client presentation and input; shared facts are in Workspace::room.
pub(crate) mod editor;
pub(crate) mod selection;
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
    pub editor: editor::Editor,
    pub composer_rows: Vec<editor::Row>,
    pub composer_area: ratatui::layout::Rect,
    pub composer_cursor: Option<ratatui::layout::Position>,
    pub transcript_area: ratatui::layout::Rect,
    pub transcript_begin: usize,
    pub selection: Option<selection::Selection>,
    pub members: Vec<Member>,
    pub receivers: Vec<crate::room_delivery::ReceiverStatus>,
    pub delivery_summary: String,
    pub recipient: Option<Member>,
    pub status: String,
    pub scroll: usize,
    refreshed: Option<std::time::Instant>,
}

fn delivery_summary(deliveries: &[crate::room_delivery::DeliveryStatus]) -> String {
    let Some(sequence) = deliveries.last().map(|d| d.request_sequence) else {
        return "No runtime deliveries (restart never replays history)".into();
    };
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for delivery in deliveries.iter().filter(|d| d.request_sequence == sequence) {
        use crate::room_delivery::Status::*;
        let label = match delivery.status {
            Queued => "queued",
            Claimed => "claimed",
            Submitted => "submitted",
            Replied => "replied",
            Unanswered => "unanswered",
            Failed => "failed",
            Unavailable => "unavailable",
            Expired => "expired",
        };
        *counts.entry(label).or_default() += 1;
    }
    let summary = counts
        .iter()
        .map(|(label, count)| format!("{count} {label}"))
        .collect::<Vec<_>>()
        .join(" · ");
    let detail = deliveries
        .iter()
        .rev()
        .filter(|d| d.request_sequence == sequence)
        .find_map(|d| {
            d.detail
                .as_ref()
                .map(|detail| format!(" · {}: {detail}", d.recipient.name))
        })
        .unwrap_or_default();
    let earlier_pending = deliveries
        .iter()
        .filter(|d| {
            d.request_sequence != sequence
                && matches!(
                    d.status,
                    crate::room_delivery::Status::Queued
                        | crate::room_delivery::Status::Claimed
                        | crate::room_delivery::Status::Submitted
                )
        })
        .count();
    let backlog = if earlier_pending > 0 {
        format!("Earlier requests: {earlier_pending} pending · ")
    } else {
        String::new()
    };
    format!("{backlog}#{sequence}: {summary}{detail}")
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
        self.room_ui.selection = None;
        self.room_ui.composer_cursor = None;
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
        self.room_ui.editor.insert(&mut self.room_ui.composer, text);
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
        self.cleanup_room_delivery(crate::room_delivery::now());
        if let Some(index) = self.state.active {
            let members = crate::room::members(&self.state, index);
            let workspace = &self.state.workspaces[index].id;
            let receivers = self.room_delivery.receivers(workspace, &members);
            let deliveries = self.room_delivery.deliveries(workspace);
            let delivery_summary = delivery_summary(&deliveries);
            if members != self.state.room_ui.members
                || receivers != self.state.room_ui.receivers
                || delivery_summary != self.state.room_ui.delivery_summary
            {
                self.state.room_ui.members = members;
                self.state.room_ui.receivers = receivers;
                self.state.room_ui.delivery_summary = delivery_summary;
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
            KeyCode::Up if key.modifiers.is_empty() => {
                self.state.room_ui.scroll = self.state.room_ui.scroll.saturating_add(1);
            }
            KeyCode::Down if key.modifiers.is_empty() => {
                self.state.room_ui.scroll = self.state.room_ui.scroll.saturating_sub(1);
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Backspace
            | KeyCode::Delete
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL =>
            {
                let ui = &mut self.state.room_ui;
                let control = key.modifiers == KeyModifiers::CONTROL;
                match key.code {
                    KeyCode::Left if control => ui.editor.word_left(&ui.composer),
                    KeyCode::Left => ui.editor.left(&ui.composer),
                    KeyCode::Right if control => ui.editor.word_right(&ui.composer),
                    KeyCode::Right => ui.editor.right(&ui.composer),
                    KeyCode::Home => ui.editor.home(&ui.composer, control),
                    KeyCode::End => ui.editor.end(&ui.composer, control),
                    KeyCode::Backspace => ui.editor.backspace(&mut ui.composer, control),
                    KeyCode::Delete => ui.editor.delete(&mut ui.composer),
                    _ => {}
                }
            }
            KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                let ui = &mut self.state.room_ui;
                ui.editor.backspace(&mut ui.composer, true);
            }
            KeyCode::Char('c' | 'C')
                if key.modifiers == KeyModifiers::CONTROL
                    || key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                if let Some(text) = self
                    .state
                    .room_ui
                    .selection
                    .as_ref()
                    .and_then(|s| s.text(&self.state.room_ui.transcript_lines))
                {
                    self.state.request_clipboard_write = Some(text.into_bytes());
                    self.dispatch_pending_clipboard_write();
                }
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
                                "Recipient changed; selection cleared to all. Tab to select one again.".into();
                            None
                        }
                    },
                };
                self.state.room_ui.recipient = next.cloned();
            }
            KeyCode::Enter if key.modifiers == KeyModifiers::SHIFT => {
                self.state.insert_room_text("\n");
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.post_room_composer(),
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
                        queued,
                        unavailable,
                        ..
                    },
                ..
            }) => {
                self.state.room_ui.composer.clear();
                self.state.room_ui.editor = editor::Editor::default();
                self.state.room_ui.scroll = 0;
                self.state.room_ui.status = format!(
                    "#{sequence} {persistence} · {queued} queued · {unavailable} unavailable"
                );
                self.state.room_ui.refreshed = None;
                self.refresh_room_members();
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
        // Own the whole gesture, even outside the viewport/chrome. Never turn
        // transcript drags into tab presses, pane selection or terminal input.
        if self.state.room_active()
            && self.state.room_ui.selection.is_some_and(|s| s.dragging)
            && matches!(
                mouse.kind,
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
            )
        {
            let ui = &mut self.state.room_ui;
            if let (Some(end), Some(selection)) = (
                selection::hit(
                    &ui.transcript_lines,
                    ui.transcript_begin,
                    ui.transcript_area,
                    position,
                ),
                &mut ui.selection,
            ) {
                selection.end = end;
                selection.dragging = mouse.kind != MouseEventKind::Up(MouseButton::Left);
            }
            return true;
        }
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
                MouseEventKind::Down(MouseButton::Left) => {
                    self.state.mode = Mode::Terminal;
                    let ui = &mut self.state.room_ui;
                    ui.selection = None;
                    if ui.transcript_area.contains(position) {
                        ui.selection = selection::hit(
                            &ui.transcript_lines,
                            ui.transcript_begin,
                            ui.transcript_area,
                            position,
                        )
                        .map(|point| selection::Selection {
                            anchor: point,
                            end: point,
                            dragging: true,
                        });
                    } else if ui.composer_area.contains(position) {
                        // A batched paste/key may precede this click before the
                        // next frame. Recompute this bounded buffer, never index
                        // stale byte offsets into a newly edited draft.
                        let rows = editor::rows(&ui.composer, ui.composer_area.width as usize);
                        let row =
                            ui.editor.top + position.y.saturating_sub(ui.composer_area.y) as usize;
                        if let Some(row) = rows.get(row) {
                            ui.editor.cursor = row.start
                                + editor::byte_at_column(
                                    &ui.composer[row.start..row.end],
                                    position.x.saturating_sub(ui.composer_area.x) as usize,
                                );
                        } else {
                            ui.editor.cursor = ui.composer.len();
                        }
                    }
                }
                _ => {}
            }
            return true;
        }
        false
    }
}
