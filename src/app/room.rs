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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptRole {
    Human,
    AgentHeader,
    Agent,
    Spacer,
}

impl TranscriptRole {
    /// Presentation-only inset; narrow viewports reserve all cells for text.
    pub fn inset(self, width: u16) -> usize {
        usize::from(self == Self::Human && width >= 3)
    }
}

#[derive(Default)]
pub struct RoomPresentation {
    pub workspace: Option<String>,
    pub visible: bool,
    pub transcript_lines: Vec<String>,
    /// Parallel display-row roles; rebuilt/appended together with cached text.
    pub transcript_roles: Vec<TranscriptRole>,
    pub transcript_width: u16,
    pub transcript_messages: usize,
    pub composer: String,
    pub editor: editor::Editor,
    pub composer_rows: Vec<editor::Row>,
    pub composer_max_rows: usize,
    pub composer_area: ratatui::layout::Rect,
    pub composer_cursor: Option<ratatui::layout::Position>,
    pub transcript_area: ratatui::layout::Rect,
    pub transcript_begin: usize,
    pub selection: Option<selection::Selection>,
    pub members: Vec<Member>,
    pub receivers: Vec<crate::room_delivery::ReceiverStatus>,
    pub delivery_summary: String,
    /// Legacy presentation value only; never used as a composer delivery target.
    /// Cleared on send so an older client selection cannot survive a new post.
    pub recipient: Option<Member>,
    pub status: String,
    pub scroll: usize,
    refreshed: Option<std::time::Instant>,
}

fn delivery_summary(deliveries: &[crate::room_delivery::DeliveryStatus]) -> String {
    let Some(sequence) = deliveries.last().map(|d| d.request_sequence) else {
        return String::new();
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

    /// Reconcile after workspace navigation or removal. The outgoing owner is
    /// carried by the presentation itself, so index shifts cannot misattribute it.
    pub(crate) fn sync_room_workspace(&mut self) {
        self.room_presentations
            .retain(|id, _| self.workspaces.iter().any(|ws| ws.id == *id));
        let workspace = self
            .active
            .and_then(|index| self.workspaces.get(index))
            .map(|ws| ws.id.clone());
        if self.room_ui.workspace == workspace {
            return;
        }
        let mut outgoing = std::mem::take(&mut self.room_ui);
        outgoing.selection = None;
        outgoing.composer_cursor = None;
        outgoing.refreshed = None;
        if let Some(id) = outgoing.workspace.clone() {
            if self.workspaces.iter().any(|ws| ws.id == id) {
                self.room_presentations.insert(id, outgoing);
            }
        }
        if let Some(id) = workspace {
            self.room_ui =
                self.room_presentations
                    .remove(&id)
                    .unwrap_or_else(|| RoomPresentation {
                        workspace: Some(id),
                        ..Default::default()
                    });
        }
        if self.room_active() {
            self.prepare_room_surface();
        }
    }

    pub(crate) fn select_room(&mut self) {
        self.sync_room_workspace();
        if self.room_ui.workspace.is_none() {
            return;
        }
        self.room_ui.visible = true;
        self.prepare_room_surface();
    }

    fn prepare_room_surface(&mut self) {
        let Some(index) = self.active else { return };
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
        let ui = &mut self.state.room_ui;
        if ui.editor.jump_key(&ui.composer, key) {
            return true;
        }
        if editor::handle_key(
            &mut ui.editor,
            &mut ui.composer,
            key,
            if ui.composer_area.width == 0 {
                self.state.view.terminal_area.width.saturating_sub(1).max(1) as usize
            } else {
                ui.composer_area.width.saturating_sub(1).max(1) as usize
            },
            ui.composer_max_rows,
        ) {
            return true;
        }
        match key.code {
            KeyCode::Esc => self.state.room_ui.visible = false,
            KeyCode::Char('c' | 'C')
                if key.modifiers == KeyModifiers::CONTROL
                    || key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                self.copy_room_selection();
            }
            KeyCode::PageUp if key.modifiers.is_empty() => {
                self.state.room_ui.scroll = self.state.room_ui.scroll.saturating_add(5)
            }
            KeyCode::PageDown if key.modifiers.is_empty() => {
                self.state.room_ui.scroll = self.state.room_ui.scroll.saturating_sub(5)
            }
            // Consume unbound Tab keys: no recipient picker or hidden-pane focus.
            // Configured global panel shortcuts have already been handled above.
            KeyCode::Tab | KeyCode::BackTab => {}
            KeyCode::Enter if key.modifiers.is_empty() => {
                let ui = &mut self.state.room_ui;
                ui.editor.normalize(&ui.composer);
                if ui.composer[..ui.editor.cursor].ends_with('\\') {
                    ui.editor.backspace(&mut ui.composer, false);
                    ui.editor.type_text(&mut ui.composer, "\n");
                } else {
                    self.post_room_composer();
                }
            }
            KeyCode::Char('v' | 'V')
                if key.modifiers == KeyModifiers::CONTROL
                    || key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
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
                let ui = &mut self.state.room_ui;
                ui.editor.type_text(&mut ui.composer, &ch.to_string());
            }
            _ => {}
        }
        true
    }

    fn copy_room_selection(&mut self) {
        if let Some(text) = self
            .state
            .room_ui
            .selection
            .as_ref()
            .and_then(|s| s.text(&self.state.room_ui.transcript_lines))
            .filter(|text| !text.is_empty())
        {
            self.state.request_clipboard_write = Some(text.into_bytes());
            self.dispatch_pending_clipboard_write();
        }
    }

    fn post_room_composer(&mut self) {
        let Some(index) = self.state.active else {
            return;
        };
        // UI posts are always group questions, even with an old presentation
        // target still present. Explicit targeting remains a neutral API feature.
        self.state.room_ui.recipient = None;
        let response = self.dispatch_api_request(
            "room.composer",
            crate::api::schema::Method::RoomPost(crate::api::schema::RoomPostParams {
                workspace_id: self.state.workspaces[index].id.clone(),
                text: self
                    .state
                    .room_ui
                    .editor
                    .expanded(&self.state.room_ui.composer)
                    .trim()
                    .to_owned(),
                recipient: None,
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
                let ui = &mut self.state.room_ui;
                ui.editor.sent(&mut ui.composer);
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
                    &ui.transcript_roles,
                    ui.transcript_begin,
                    ui.transcript_area,
                    position,
                ),
                &mut ui.selection,
            ) {
                selection.end = end;
            }
            if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                if let Some(selection) = &mut ui.selection {
                    selection.dragging = false;
                }
                // Match normal pane select-to-copy, using the same client-local
                // clipboard event route. Host terminals may swallow Ctrl+Shift+C.
                if self.state.copy_on_select {
                    self.copy_room_selection();
                }
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
                            &ui.transcript_roles,
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
                        let rows = ui.editor.rows(
                            &ui.composer,
                            ui.composer_area.width.saturating_sub(1) as usize,
                        );
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
                        ui.editor.clicked(&ui.composer);
                    }
                }
                _ => {}
            }
            return true;
        }
        false
    }
}
