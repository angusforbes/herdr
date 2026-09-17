//! Rendering for the right-hand search pane. Pure: reads `AppState` only.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::text::{display_width, truncate_end};
use crate::app::{
    search_pane::{
        body_rect, body_rows, input_row, mode_chip_rects, status_row, title_row, BodyRow,
        SearchPaneMode, AI_CHIP, KEYWORD_CHIP,
    },
    state::{AppState, Mode},
};

pub(super) fn render_search_pane(app: &AppState, frame: &mut Frame, area: Rect) {
    if area.width < 4 || area.height == 0 {
        return;
    }
    let p = &app.palette;
    let focused = app.mode == Mode::SearchPane;
    let state = &app.search_pane;

    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(p.sidebar_bg));

    // left border
    let border_style = if focused {
        Style::default().fg(p.accent)
    } else {
        Style::default().fg(p.surface_dim)
    };
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(area.x, y)].set_symbol("│");
        buf[(area.x, y)].set_style(border_style);
    }

    // title + mode chips
    let title = title_row(area);
    if title.height > 0 {
        let label_style = Style::default()
            .fg(if focused { p.accent } else { p.subtext0 })
            .add_modifier(Modifier::BOLD);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(" Search", label_style))),
            title,
        );
        let (kw, ai) = mode_chip_rects(area);
        render_chip(
            frame,
            kw,
            KEYWORD_CHIP,
            state.mode == SearchPaneMode::Keyword,
            p,
        );
        render_chip(frame, ai, AI_CHIP, state.mode == SearchPaneMode::Ai, p);
    }

    // input
    let input = input_row(area);
    if input.height > 0 {
        let prompt = match state.mode {
            SearchPaneMode::Keyword => " / ",
            SearchPaneMode::Ai => " ? ",
        };
        let prompt_style = Style::default().fg(if focused { p.accent } else { p.overlay0 });
        let avail = usize::from(input.width).saturating_sub(display_width(prompt) + 1);
        let shown = tail_fit(&state.query, avail);
        let mut spans = vec![Span::styled(prompt, prompt_style)];
        if state.query.is_empty() && !focused {
            spans.push(Span::styled(
                truncate_end("type to search…", avail),
                Style::default().fg(p.overlay0).add_modifier(Modifier::DIM),
            ));
        } else {
            spans.push(Span::styled(shown.clone(), Style::default().fg(p.text)));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), input);
        if focused {
            let caret_x = input.x + display_width(prompt) as u16 + display_width(&shown) as u16;
            frame.set_cursor_position((caret_x.min(input.x + input.width - 1), input.y));
        }
    }

    // status
    let status = status_row(area);
    if status.height > 0 {
        let text = state.status.clone().unwrap_or_else(|| {
            match state.mode {
                SearchPaneMode::Keyword => "enter: search · tab: AI · esc: back to pane",
                SearchPaneMode::Ai => "enter: ask · tab: keyword · esc: back to pane",
            }
            .to_string()
        });
        let style = if state.ai_inflight {
            Style::default().fg(p.yellow)
        } else {
            Style::default().fg(p.overlay1).add_modifier(Modifier::DIM)
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    " {}",
                    truncate_end(&text, usize::from(status.width).saturating_sub(1))
                ),
                style,
            ))),
            status,
        );
    }

    // results
    let body = body_rect(area);
    if body.height == 0 || body.width < 4 {
        return;
    }
    let rows = body_rows(state);
    let width = usize::from(body.width);
    for (i, row) in rows
        .iter()
        .skip(state.scroll)
        .take(usize::from(body.height))
        .enumerate()
    {
        let y = body.y + i as u16;
        let rect = Rect::new(body.x, y, body.width, 1);
        match row {
            BodyRow::Gap => {}
            BodyRow::Group(g) => {
                let group = &state.groups[*g];
                let title = truncate_end(&group.title, width.saturating_sub(1));
                let mut used = display_width(&title) + 1;
                let mut spans = vec![Span::styled(
                    format!(" {title}"),
                    Style::default().fg(p.text).add_modifier(Modifier::BOLD),
                )];
                // The agent's display name, styled exactly as the left sidebar draws it.
                if let Some(name_spans) = super::custom_token_spans(
                    &group.tokens,
                    "name",
                    Style::default().fg(p.text),
                    width.saturating_sub(used + 3),
                ) {
                    spans.push(Span::styled("  ", Style::default()));
                    used += 2;
                    for span in name_spans {
                        used += display_width(&span.content);
                        spans.push(span);
                    }
                }
                let remaining = width.saturating_sub(used + 3);
                if remaining > 4 && !group.subtitle.is_empty() {
                    spans.push(Span::styled(
                        format!("  {}", truncate_end(&group.subtitle, remaining)),
                        Style::default().fg(p.overlay0).add_modifier(Modifier::DIM),
                    ));
                }
                frame.render_widget(Paragraph::new(Line::from(spans)), rect);
            }
            BodyRow::Hit { group, hit, flat } => {
                let hit = &state.groups[*group].hits[*hit];
                let selected = state.selected == Some(*flat);
                let base = if selected {
                    Style::default().fg(p.text).bg(p.active_row_bg)
                } else {
                    Style::default().fg(p.subtext0)
                };
                if selected {
                    frame.buffer_mut().set_style(rect, base);
                }
                let avail = width.saturating_sub(3);
                let (before, matched, after) = window_snippet(hit, avail);
                let marker = if selected { " ▸ " } else { "   " };
                let spans = vec![
                    Span::styled(marker, Style::default().fg(p.accent)),
                    Span::styled(before, base),
                    Span::styled(matched, base.fg(p.accent).add_modifier(Modifier::BOLD)),
                    Span::styled(after, base),
                ];
                frame.render_widget(Paragraph::new(Line::from(spans)), rect);
            }
        }
    }
}

fn render_chip(
    frame: &mut Frame,
    rect: Rect,
    label: &str,
    active: bool,
    p: &crate::app::state::Palette,
) {
    if rect.width == 0 {
        return;
    }
    let style = if active {
        Style::default()
            .fg(p.sidebar_bg)
            .bg(p.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.overlay1)
    };
    frame.render_widget(Paragraph::new(Line::from(Span::styled(label, style))), rect);
}

/// Last `avail` columns of the query so the caret stays visible.
fn tail_fit(text: &str, avail: usize) -> String {
    if display_width(text) <= avail {
        return text.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut w = 0;
    for ch in text.chars().rev() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > avail {
            break;
        }
        w += cw;
        out.push(ch);
    }
    out.into_iter().rev().collect()
}

/// Split the snippet into (before, match, after), windowed so the match is visible
/// in `avail` columns. The match length is taken from the searched query when it
/// still appears in the snippet; otherwise the whole line is shown plain.
fn window_snippet(
    hit: &crate::app::search_pane::SearchHit,
    avail: usize,
) -> (String, String, String) {
    let chars: Vec<char> = hit.snippet.chars().collect();
    let match_len = (hit.text_match.end.col as usize)
        .saturating_sub(hit.text_match.start.col as usize)
        .clamp(1, chars.len().saturating_sub(hit.match_char).max(1));
    let start = hit.match_char.min(chars.len());
    let end = (start + match_len).min(chars.len());

    // Window: try to show ~1/3 context before the match.
    let mut win_start = 0;
    if chars.len() > avail {
        let lead = avail / 3;
        win_start = start.saturating_sub(lead);
        if win_start + avail > chars.len() {
            win_start = chars.len().saturating_sub(avail);
        }
    }
    let win_end = (win_start + avail).min(chars.len());
    let slice = |a: usize, b: usize| chars[a.min(b)..b].iter().collect::<String>();
    let mut before = slice(win_start, start.max(win_start).min(win_end));
    let matched = slice(
        start.max(win_start).min(win_end),
        end.min(win_end).max(win_start),
    );
    let mut after = slice(end.max(win_start).min(win_end), win_end);
    if win_start > 0 && !before.is_empty() {
        before.replace_range(..before.chars().next().map_or(0, char::len_utf8), "…");
    }
    if win_end < chars.len() && !after.is_empty() {
        let last = after.chars().last().map_or(0, char::len_utf8);
        let cut = after.len() - last;
        after.replace_range(cut.., "…");
    }
    (before, matched, after)
}
