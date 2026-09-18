//! Message tree/context preview. Rendering is read-only.
use super::text::truncate_end;
use crate::app::{conversation::preview_rects, state::AppState};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

pub(super) fn render_conversation(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(preview) = app.conversation_preview.as_ref() else {
        return;
    };
    let p = &app.palette;
    let (controls, tree, status, text) = preview_rects(area);
    if controls.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " ← Results (Esc) · Pi tree",
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
            ))),
            Rect::new(controls.x, controls.y, controls.width, 1),
        );
    }
    if controls.height > 1 {
        let selected = preview.selected_message();
        let rewrite = selected.is_some_and(|m| m.can_rewrite);
        let after = selected.is_some_and(|m| m.can_continue);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " Rewrite ↵   ",
                    Style::default().fg(if rewrite { p.accent } else { p.overlay0 }),
                ),
                Span::styled(
                    " After C-↵",
                    Style::default().fg(if after { p.accent } else { p.overlay0 }),
                ),
            ])),
            Rect::new(controls.x, controls.y + 1, controls.width, 1),
        );
    }
    for (row, (index, message)) in preview
        .messages
        .iter()
        .enumerate()
        .skip(preview.tree_scroll)
        .take(usize::from(tree.height))
        .enumerate()
    {
        let selected = index == preview.selected;
        let style = if selected {
            Style::default().fg(p.text).bg(p.active_row_bg)
        } else {
            Style::default().fg(p.subtext0)
        };
        let depth = message.depth.min(5);
        let indent = " ".repeat(depth);
        let marker = if selected {
            "▸"
        } else if message.active {
            "│"
        } else {
            "├"
        };
        let role = if message.role == "user" { "you" } else { "ai" };
        let line = format!(
            "{marker}{indent}{role}: {}",
            message
                .text
                .chars()
                .take(160)
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
        let rect = Rect::new(tree.x, tree.y + row as u16, tree.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_end(&line, usize::from(tree.width)),
                style,
            ))),
            rect,
        );
    }
    if status.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_end(&preview.status, usize::from(status.width)),
                Style::default().fg(p.yellow),
            ))),
            status,
        );
    }
    if let Some(message) = preview.selected_message() {
        frame.render_widget(
            Paragraph::new(message.text.as_str())
                .style(Style::default().fg(p.text))
                .wrap(Wrap { trim: false })
                .scroll((preview.text_scroll.min(u16::MAX as usize) as u16, 0)),
            text,
        );
    }
}
