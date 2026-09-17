use crate::app::AppState;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Geometry computation, not rendering. At unchanged width, only new messages
/// are formatted/wrapped. Room history is bounded; resizing rebuilds this one
/// active-room cache, never a pane-scaled loop and never any filesystem I/O.
pub(super) fn compute_room_view(app: &mut AppState, area: Rect) {
    if !app.room_active() {
        return;
    }
    let Some(ws) = app.active.and_then(|index| app.workspaces.get(index)) else {
        return;
    };
    let ui = &mut app.room_ui;
    let transcript = room_areas(area)[2];
    let width = transcript.width.max(1);
    if ui.transcript_width != width || ui.transcript_messages > ws.room.messages.len() {
        ui.transcript_lines.clear();
        ui.transcript_messages = 0;
        ui.transcript_width = width;
    }
    for message in ws.room.messages.iter().skip(ui.transcript_messages) {
        let author = message
            .author
            .as_ref()
            .map(|member| format!("{} ({})", member.name, member.pane_id))
            .unwrap_or_else(|| "human".into());
        let audience: Vec<_> = message
            .recipients
            .iter()
            .chain(message.recipient.iter())
            .map(|m| m.name.as_str())
            .collect();
        let destination = if audience.is_empty() {
            String::new()
        } else {
            format!(
                " → {} [expires {}]",
                audience.join(", "),
                message.expires_unix.unwrap_or(0)
            )
        };
        let correlation = message
            .reply_to
            .map(|n| format!(" · reply to #{n}"))
            .unwrap_or_default();
        wrap_into(
            &format!("#{} {author}{destination}{correlation}", message.sequence),
            width as usize,
            &mut ui.transcript_lines,
        );
        wrap_into(&message.text, width as usize, &mut ui.transcript_lines);
        ui.transcript_lines.push(String::new());
    }
    ui.transcript_messages = ws.room.messages.len();
    ui.scroll = ui.scroll.min(
        ui.transcript_lines
            .len()
            .saturating_sub(transcript.height as usize),
    );
}

fn wrap_into(text: &str, width: usize, lines: &mut Vec<String>) {
    // Text remains verbatim in storage/API. Never render control escape sequences.
    let safe: String = text
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n')
        .collect();
    for logical in safe.split('\n') {
        let mut line = String::new();
        let mut columns = 0;
        let span = ratatui::text::Span::raw(logical);
        for grapheme in span.styled_graphemes(Style::default()) {
            let grapheme = grapheme.symbol;
            let size = grapheme.width();
            if columns + size > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                columns = 0;
            }
            line.push_str(grapheme);
            columns += size;
        }
        lines.push(line);
    }
}

fn room_areas(area: Rect) -> [Rect; 5] {
    Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(3),
    ])
    .areas(area)
}

pub(super) fn render_room(app: &AppState, frame: &mut Frame, area: Rect) {
    let ui = &app.room_ui;
    let [header, members, transcript, status, composer] = room_areas(area);
    frame.render_widget(Paragraph::new("room · human owned · Pi delivery (at most once, 10 min)\nCtrl+Alt+R / Esc: terminal · Tab: all / one recipient · Enter: send · PgUp/PgDn: history")
        .style(Style::default().fg(app.palette.accent)), header);
    let selected = ui
        .recipient
        .as_ref()
        .and_then(|selected| ui.members.iter().position(|m| m == selected));
    let start = selected.unwrap_or(0).saturating_sub(1);
    let mut lines = vec![Line::from(format!(
        "Members: {} · {}",
        ui.members.len(),
        if ui.recipient.is_none() {
            "recipient: all current members"
        } else if selected.is_none() {
            "recipient changed: select again (Tab)"
        } else {
            "one recipient (10 min)"
        }
    ))];
    for member in ui.members.iter().skip(start).take(2) {
        lines.push(Line::from(format!(
            "{} {} · {} · {}",
            if ui.recipient.as_ref() == Some(member) {
                ">"
            } else {
                " "
            },
            member.name,
            member.pane_id,
            ui.receivers
                .iter()
                .find(|r| crate::room_delivery::same_identity(&r.member, member))
                .map(|r| if r.available {
                    "Pi receiver online"
                } else {
                    r.detail.as_deref().unwrap_or("unavailable")
                })
                .unwrap_or("checking receiver")
        )));
    }
    frame.render_widget(Paragraph::new(lines), members);
    let height = transcript.height as usize;
    let max_scroll = ui.transcript_lines.len().saturating_sub(height);
    let begin = max_scroll.saturating_sub(ui.scroll.min(max_scroll));
    let visible: Vec<_> = ui
        .transcript_lines
        .iter()
        .skip(begin)
        .take(height)
        .map(|line| Line::from(line.as_str()))
        .collect();
    frame.render_widget(Paragraph::new(visible), transcript);
    frame.render_widget(
        Paragraph::new(format!("{}\n{}", ui.status, ui.delivery_summary))
            .wrap(Wrap { trim: false }),
        status,
    );
    let input = Paragraph::new(ui.composer.as_str()).wrap(Wrap { trim: false });
    let input_lines = input.line_count(composer.width.saturating_sub(2).max(1));
    let input_scroll = input_lines
        .saturating_sub(composer.height.saturating_sub(2) as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        input.scroll((input_scroll, 0)).block(
            Block::default()
                .borders(Borders::ALL)
                .title("human question · Enter queues current recipients"),
        ),
        composer,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn room_wrapping_preserves_unicode_and_filters_control_sequences() {
        let mut lines = Vec::new();
        wrap_into("abc界d\ne\u{301}\x1b", 4, &mut lines);
        assert_eq!(lines, ["abc", "界d", "e\u{301}"]);
    }

    #[test]
    fn room_scroll_normalizes_to_wrapped_viewport_after_resize_and_append() {
        let mut app = AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("room")];
        app.active = Some(0);
        app.workspaces[0]
            .room
            .post("界".repeat(500), None, 0)
            .unwrap();
        app.select_room();
        for area in [
            Rect::new(0, 0, 20, 5),
            Rect::new(0, 0, 20, 30),
            Rect::new(0, 0, 80, 40),
        ] {
            app.room_ui.scroll = usize::MAX;
            compute_room_view(&mut app, area);
            let max = app
                .room_ui
                .transcript_lines
                .len()
                .saturating_sub(room_areas(area)[2].height as usize);
            assert_eq!(app.room_ui.scroll, max);
            app.workspaces[0]
                .room
                .post("new record".into(), None, 0)
                .unwrap();
            compute_room_view(&mut app, area);
            assert_eq!(
                app.room_ui.scroll, max,
                "append must not revive old overscroll"
            );
        }
        compute_room_view(&mut app, Rect::new(0, 0, 120, 200));
        assert_eq!(
            app.room_ui.scroll, 0,
            "short transcript fits the tall viewport"
        );
    }

    #[test]
    fn room_transcript_cache_is_incremental_and_rewraps_on_resize() {
        let mut app = AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("room")];
        app.active = Some(0);
        app.workspaces[0]
            .room
            .post("x".repeat(8192), None, 0)
            .unwrap();
        app.select_room();
        compute_room_view(&mut app, Rect::new(0, 0, 80, 30));
        let count = app.room_ui.transcript_lines.len();
        let pointer = app.room_ui.transcript_lines[0].as_ptr();
        compute_room_view(&mut app, Rect::new(0, 0, 80, 30));
        assert_eq!(app.room_ui.transcript_lines[0].as_ptr(), pointer);
        assert_eq!(app.room_ui.transcript_lines.len(), count);
        app.workspaces[0].room.post("last".into(), None, 0).unwrap();
        compute_room_view(&mut app, Rect::new(0, 0, 80, 30));
        assert_eq!(app.room_ui.transcript_lines[0].as_ptr(), pointer);
        assert!(app
            .room_ui
            .transcript_lines
            .iter()
            .any(|line| line == "last"));
        compute_room_view(&mut app, Rect::new(0, 0, 20, 30));
        assert!(app.room_ui.transcript_lines.len() > count);
    }
}
