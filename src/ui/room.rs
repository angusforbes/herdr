use crate::app::{
    room::{editor, TranscriptRole},
    AppState, Mode,
};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Geometry computation, not rendering. At unchanged width, only new messages
/// are formatted/wrapped. Room history is bounded; resizing rebuilds this one
/// active-room cache, never a pane-scaled loop and never any filesystem I/O.
pub(super) fn compute_room_view(app: &mut AppState, area: Rect, terminal_height: u16) {
    if !app.room_active() {
        app.room_ui.selection = None;
        app.room_ui.composer_cursor = None;
        return;
    }
    let Some(ws) = app.active.and_then(|index| app.workspaces.get(index)) else {
        return;
    };
    let ui = &mut app.room_ui;
    // Composer work is bounded by 8192 bytes, independent of history/panes.
    ui.composer_max_rows = (terminal_height as usize * 3 / 10).max(5);
    ui.composer_rows = ui
        .editor
        .rows(&ui.composer, area.width.saturating_sub(1) as usize);
    let [transcript, _, composer] = room_areas(area, ui);
    ui.transcript_area = transcript;
    ui.composer_area = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .inner(composer);
    ui.composer_cursor = ui
        .editor
        .project(&ui.composer, &ui.composer_rows, ui.composer_area);
    let width = transcript.width.max(1);
    if ui.transcript_width != width || ui.transcript_messages > ws.room.messages.len() {
        ui.selection = None; // display-row coordinates cease to exist on rewrap
        ui.transcript_lines.clear();
        ui.transcript_roles.clear();
        ui.transcript_messages = 0;
        ui.transcript_width = width;
    }
    for message in ws.room.messages.iter().skip(ui.transcript_messages) {
        if let Some(author) = &message.author {
            wrap_into(
                crate::room::author_heading(author),
                width as usize,
                &mut ui.transcript_lines,
            );
        }
        wrap_into(&message.text, width as usize, &mut ui.transcript_lines);
        let role = if message.author.is_none() {
            TranscriptRole::Human
        } else {
            TranscriptRole::Agent
        };
        ui.transcript_roles.resize(ui.transcript_lines.len(), role);
        ui.transcript_lines.push(String::new());
        ui.transcript_roles.push(TranscriptRole::Spacer);
    }
    ui.transcript_messages = ws.room.messages.len();
    ui.scroll = ui.scroll.min(
        ui.transcript_lines
            .len()
            .saturating_sub(transcript.height as usize),
    );
    ui.transcript_begin = ui
        .transcript_lines
        .len()
        .saturating_sub(transcript.height as usize)
        .saturating_sub(ui.scroll);
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

fn room_areas(area: Rect, ui: &crate::app::room::RoomPresentation) -> [Rect; 3] {
    let status_rows = u16::from(!ui.status.is_empty()) + u16::from(!ui.delivery_summary.is_empty());
    Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(status_rows),
        Constraint::Length(
            (ui.composer_rows.len().clamp(1, ui.composer_max_rows.max(1)) as u16).saturating_add(2),
        ),
    ])
    .areas(area)
}

fn workspace_message_tint(surface: Color, accent: Color) -> Color {
    let (r, g, b) = match surface {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (245, 245, 245),
    };
    let Color::Rgb(ar, ag, ab) = accent else {
        return surface;
    };
    // A subtle workspace wash, relative to the theme surface so dark themes
    // retain readable text. Compute once per room render, never per pane/row.
    let mix = |base: u8, tint: u8| ((u16::from(base) * 4 + u16::from(tint)) / 5) as u8;
    Color::Rgb(mix(r, ar), mix(g, ag), mix(b, ab))
}

pub(super) fn render_room(app: &AppState, frame: &mut Frame, area: Rect) {
    let ui = &app.room_ui;
    let [transcript, status, composer] = room_areas(area, ui);
    let agent_bg = workspace_message_tint(
        app.palette.surface0,
        app.workspace_color(app.active.unwrap_or(0)),
    );
    let message_style = |role: TranscriptRole| match role {
        TranscriptRole::Human => Style::default()
            .fg(app.palette.text)
            .bg(app.palette.surface0),
        TranscriptRole::Agent => Style::default().fg(app.palette.text).bg(agent_bg),
        TranscriptRole::Spacer => Style::default(),
    };
    // Paint message rows including their padding; leave inter-message gaps clear.
    for (offset, role) in ui
        .transcript_roles
        .iter()
        .skip(ui.transcript_begin)
        .take(transcript.height as usize)
        .enumerate()
    {
        if *role != TranscriptRole::Spacer {
            frame.buffer_mut().set_style(
                Rect::new(
                    transcript.x,
                    transcript.y + offset as u16,
                    transcript.width,
                    1,
                ),
                message_style(*role),
            );
        }
    }
    let visible: Vec<_> = ui
        .transcript_lines
        .iter()
        .enumerate()
        .skip(ui.transcript_begin)
        .take(transcript.height as usize)
        .map(|(row, line)| {
            let range = ui
                .selection
                .as_ref()
                .map(|s| s.range(row, line.len()))
                .unwrap_or(0..0);
            let style = message_style(
                ui.transcript_roles
                    .get(row)
                    .copied()
                    .unwrap_or(TranscriptRole::Spacer),
            );
            Line::from(vec![
                Span::raw(&line[..range.start]),
                Span::styled(
                    &line[range.clone()],
                    Style::default().add_modifier(Modifier::REVERSED),
                ),
                Span::raw(&line[range.end..]),
            ])
            .style(style)
        })
        .collect();
    frame.render_widget(Paragraph::new(visible), transcript);
    frame.render_widget(
        Paragraph::new(
            [&ui.status, &ui.delivery_summary]
                .into_iter()
                .filter(|line| !line.is_empty())
                .map(|line| Line::from(line.as_str()))
                .collect::<Vec<_>>(),
        )
        .style(Style::default().fg(app.palette.overlay0))
        .wrap(Wrap { trim: false }),
        status,
    );
    if composer.height > 0 {
        frame.render_widget(
            Paragraph::new(scroll_border("↑", ui.editor.top, composer.width)),
            Rect::new(composer.x, composer.y, composer.width, 1),
        );
    }
    if composer.height > 1 {
        let below = ui
            .composer_rows
            .len()
            .saturating_sub(ui.editor.top + ui.composer_area.height as usize);
        frame.render_widget(
            Paragraph::new(scroll_border("↓", below, composer.width)),
            Rect::new(composer.x, composer.bottom() - 1, composer.width, 1),
        );
    }
    let input: Vec<_> = ui
        .composer_rows
        .iter()
        .skip(ui.editor.top)
        .take(ui.composer_area.height as usize)
        .map(|row| {
            let cursor_row = editor::row_at(&ui.composer_rows, ui.editor.cursor);
            if app.mode != Mode::Terminal || ui.composer_rows.get(cursor_row) != Some(row) {
                return Line::from(&ui.composer[row.start..row.end]);
            }
            use unicode_segmentation::UnicodeSegmentation;
            let cursor = ui.editor.cursor;
            let end = cursor
                + ui.composer[cursor..row.end]
                    .graphemes(true)
                    .next()
                    .map(str::len)
                    .unwrap_or(0);
            Line::from(vec![
                Span::raw(&ui.composer[row.start..cursor]),
                Span::styled(
                    if end == cursor {
                        " "
                    } else {
                        &ui.composer[cursor..end]
                    },
                    Style::default().add_modifier(Modifier::REVERSED),
                ),
                Span::raw(&ui.composer[end..row.end]),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(input), ui.composer_area);
    if app.mode == Mode::Terminal {
        if let Some(cursor) = ui.composer_cursor {
            frame.set_cursor_position(cursor);
        }
    }
}

fn scroll_border(direction: &str, hidden: usize, width: u16) -> String {
    let width = width as usize;
    if hidden == 0 {
        return "─".repeat(width);
    }
    let label = format!(" {direction} {hidden} more ");
    if label.width() + 2 <= width {
        let left = (width - label.width()) / 2;
        format!(
            "{}{}{}",
            "─".repeat(left),
            label,
            "─".repeat(width - label.width() - left)
        )
    } else {
        let label = format!("───{label}");
        let dots = ".".repeat(width.min(3));
        let end = editor::byte_at_column(&label, width.saturating_sub(dots.len()));
        format!("{}{dots}", &label[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn room_render_is_only_conversation_optional_status_and_composer() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("room")];
        app.active = Some(0);
        app.select_room();
        app.room_ui.members.push(crate::room::Member {
            pane_id: "w1:p1".into(),
            terminal_id: "t1".into(),
            agent: "pi".into(),
            name: "hidden member listing".into(),
            session: None,
        });
        app.room_ui.recipient = app.room_ui.members.first().cloned();
        app.workspaces[0]
            .room
            .post("shared conversation".into(), None, 0)
            .unwrap();
        app.insert_room_text("draft");
        let area = Rect::new(0, 0, 100, 20);
        for status in ["", "write failed"] {
            app.room_ui.status = status.into();
            compute_room_view(&mut app, area, area.height);
            assert_eq!(app.room_ui.transcript_area.y, area.y);
            assert_eq!(app.room_ui.composer_area.bottom() + 1, area.bottom());
            assert_eq!(
                app.room_ui.transcript_area.height,
                17 - u16::from(!status.is_empty())
            );
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal
                .draw(|frame| render_room(&app, frame, area))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains("shared conversation"));
            assert!(text.contains("draft"));
            assert!(text.contains(status));
            for removed in [
                "Members:",
                "recipient",
                "human owned",
                "Tab:",
                "hidden member listing",
            ] {
                assert!(!text.contains(removed), "unexpected room chrome: {removed}");
            }
        }
    }

    #[test]
    fn room_agent_tint_tracks_workspace_and_surface() {
        let orange = Color::Rgb(255, 158, 100);
        assert_eq!(
            workspace_message_tint(Color::Rgb(245, 245, 247), orange),
            Color::Rgb(247, 227, 217)
        );
        assert_eq!(
            workspace_message_tint(Color::Rgb(30, 30, 30), orange),
            Color::Rgb(75, 55, 44)
        );
        assert_ne!(
            workspace_message_tint(Color::Rgb(245, 245, 247), orange),
            workspace_message_tint(Color::Rgb(245, 245, 247), Color::Rgb(122, 162, 247))
        );
    }

    #[test]
    fn room_headers_and_human_background_are_conversation_only() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("room")];
        app.active = Some(0);
        let author = crate::room::Member {
            name: "Aporia".into(),
            pane_id: "wB:p2".into(),
            terminal_id: "t1".into(),
            agent: "pi".into(),
            session: Some("Id:original".into()),
        };
        app.workspaces[0]
            .room
            .post("question\n界".into(), Some(author.clone()), 0)
            .unwrap();
        app.workspaces[0]
            .room
            .reply(1, author, "answer".into(), 1)
            .unwrap();
        let stored = app.workspaces[0].room.clone();
        app.select_room();
        let area = Rect::new(0, 0, 80, 20);
        compute_room_view(&mut app, area, area.height);
        assert_eq!(
            app.room_ui.transcript_lines,
            ["question", "界", "", "Aporia", "answer", ""]
        );
        assert_eq!(
            app.room_ui.transcript_roles,
            [
                TranscriptRole::Human,
                TranscriptRole::Human,
                TranscriptRole::Spacer,
                TranscriptRole::Agent,
                TranscriptRole::Agent,
                TranscriptRole::Spacer
            ]
        );
        use crate::app::room::selection::{Point, Selection};
        app.room_ui.selection = Some(Selection {
            anchor: Point { row: 0, byte: 0 },
            end: Point { row: 4, byte: 6 },
            dragging: false,
        });
        assert_eq!(
            app.room_ui
                .selection
                .unwrap()
                .text(&app.room_ui.transcript_lines)
                .unwrap(),
            "question\n界\n\nAporia\nanswer"
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal
            .draw(|frame| render_room(&app, frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        for row in [0, 1] {
            assert_eq!(
                buffer[(79, row)].bg,
                app.palette.surface0,
                "human row padding highlighted"
            );
            assert_eq!(buffer[(0, row)].bg, app.palette.surface0);
            assert!(
                buffer[(0, row)].modifier.contains(Modifier::REVERSED),
                "selection distinct from background"
            );
        }
        let tint = workspace_message_tint(app.palette.surface0, app.workspace_color(0));
        assert_ne!(tint, app.palette.surface0);
        for row in [3, 4] {
            assert_eq!(
                buffer[(79, row)].bg,
                tint,
                "agent header and body padding tinted"
            );
            assert_eq!(buffer[(0, row)].bg, tint);
        }
        assert_ne!(buffer[(79, 2)].bg, tint, "message gap stays clear");
        assert_eq!(
            app.workspaces[0].room, stored,
            "presentation must not mutate history"
        );
    }

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
            compute_room_view(&mut app, area, area.height);
            let max = app
                .room_ui
                .transcript_lines
                .len()
                .saturating_sub(room_areas(area, &app.room_ui)[0].height as usize);
            assert_eq!(app.room_ui.scroll, max);
            app.workspaces[0]
                .room
                .post("new record".into(), None, 0)
                .unwrap();
            compute_room_view(&mut app, area, area.height);
            assert_eq!(
                app.room_ui.scroll, max,
                "append must not revive old overscroll"
            );
        }
        compute_room_view(&mut app, Rect::new(0, 0, 120, 200), 200);
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
        compute_room_view(&mut app, Rect::new(0, 0, 80, 30), 30);
        let count = app.room_ui.transcript_lines.len();
        let pointer = app.room_ui.transcript_lines[0].as_ptr();
        compute_room_view(&mut app, Rect::new(0, 0, 80, 30), 30);
        assert_eq!(app.room_ui.transcript_lines[0].as_ptr(), pointer);
        assert_eq!(app.room_ui.transcript_lines.len(), count);
        app.workspaces[0].room.post("last".into(), None, 0).unwrap();
        compute_room_view(&mut app, Rect::new(0, 0, 80, 30), 30);
        assert_eq!(app.room_ui.transcript_lines[0].as_ptr(), pointer);
        assert!(app
            .room_ui
            .transcript_lines
            .iter()
            .any(|line| line == "last"));
        compute_room_view(&mut app, Rect::new(0, 0, 20, 30), 30);
        assert!(app.room_ui.transcript_lines.len() > count);
    }
}
