//! Right-hand search pane: keyword or AI search across every agent pane's
//! scrollback, grouped per agent with up to three recent snippets each.
//!
//! State lives here; rendering is in `ui/search_pane.rs`. Layout helpers are
//! pure functions of `(state, rect)` so render and mouse hit-testing agree.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use crate::{
    app::{
        state::{AppState, Mode, NavigatorTarget},
        App,
    },
    events::AppEvent,
    layout::PaneId,
    pane::TerminalTextMatch,
    terminal::TerminalRuntimeRegistry,
    ui::AgentPanelEntry,
};

pub(crate) const MAX_HITS_PER_PANE: usize = 3;
/// Scrollback lines per pane handed to the AI search.
const AI_LINES_PER_PANE: usize = 200;
const AI_MAX_PROMPT_CHARS: usize = 120_000;
const AI_TIMEOUT_SECS: u64 = 90;
const MIN_MAIN_WIDTH: u16 = 40;
const MIN_PANE_WIDTH: u16 = 30;
const MAX_PANE_WIDTH: u16 = 64;

pub(crate) const HEADER_ROWS: u16 = 3; // title/mode, input, status

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SearchPaneMode {
    #[default]
    Keyword,
    Ai,
}

impl SearchPaneMode {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Keyword => Self::Ai,
            Self::Ai => Self::Keyword,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchHit {
    pub text_match: TerminalTextMatch,
    /// Whitespace-collapsed logical line the match sits on.
    pub snippet: String,
    /// Char index into `snippet` where the matched text starts (for windowing).
    pub match_char: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchGroup {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: PaneId,
    /// e.g. "2. yt-radio" — sidebar index + topic/agent label.
    pub title: String,
    /// e.g. "Work · claude-fable-5-1".
    pub subtitle: String,
    /// The pane's metadata tokens (`name`, `name_fg`, ...) so the agent's display
    /// name renders exactly as in the left sidebar.
    pub tokens: std::collections::HashMap<String, String>,
    /// Newest first.
    pub hits: Vec<SearchHit>,
}

/// A pane offered to the AI, in prompt order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AiPaneRef {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: PaneId,
    pub title: String,
    pub subtitle: String,
    pub tokens: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SearchPaneState {
    pub visible: bool,
    pub query: String,
    pub mode: SearchPaneMode,
    pub groups: Vec<SearchGroup>,
    /// `(query, mode)` the current `groups` were produced for.
    pub searched: Option<(String, SearchPaneMode)>,
    /// Flat hit index (across groups) of the selected result.
    pub selected: Option<usize>,
    pub scroll: usize,
    pub status: Option<String>,
    pub ai_generation: u64,
    pub ai_inflight: bool,
    pub ai_panes: Vec<AiPaneRef>,
    /// The hit last jumped to, highlighted inside its pane while the search pane is open.
    pub jump_highlight: Option<JumpHighlight>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JumpHighlight {
    pub pane_id: PaneId,
    pub text_match: TerminalTextMatch,
}

impl SearchPaneState {
    pub(crate) fn hit_count(&self) -> usize {
        self.groups.iter().map(|g| g.hits.len()).sum()
    }

    pub(crate) fn results_fresh(&self) -> bool {
        self.searched
            .as_ref()
            .is_some_and(|(q, m)| *q == self.query.trim() && *m == self.mode)
    }

    pub(crate) fn hit(&self, flat: usize) -> Option<(&SearchGroup, &SearchHit)> {
        let mut index = flat;
        for group in &self.groups {
            if index < group.hits.len() {
                return Some((group, &group.hits[index]));
            }
            index -= group.hits.len();
        }
        None
    }

    fn invalidate(&mut self) {
        self.selected = None;
        self.jump_highlight = None;
        if self.searched.is_some() && !self.results_fresh() {
            self.status = Some("enter to search".into());
        }
    }
}

// ---------------------------------------------------------------------------
// Layout (pure)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyRow {
    Group(usize),
    Hit { group: usize, hit: usize, flat: usize },
    Gap,
}

pub(crate) fn body_rows(state: &SearchPaneState) -> Vec<BodyRow> {
    let mut rows = Vec::new();
    let mut flat = 0;
    for (g, group) in state.groups.iter().enumerate() {
        if g > 0 {
            rows.push(BodyRow::Gap);
        }
        rows.push(BodyRow::Group(g));
        for h in 0..group.hits.len() {
            rows.push(BodyRow::Hit {
                group: g,
                hit: h,
                flat,
            });
            flat += 1;
        }
    }
    rows
}

pub(crate) fn search_pane_width(app: &AppState, main_area: Rect) -> u16 {
    if !app.search_pane.visible || main_area.width < MIN_MAIN_WIDTH + MIN_PANE_WIDTH + 1 {
        return 0;
    }
    let want = (u32::from(main_area.width) * 35 / 100) as u16;
    want.clamp(MIN_PANE_WIDTH, MAX_PANE_WIDTH)
        .min(main_area.width.saturating_sub(MIN_MAIN_WIDTH))
}

/// Inner content rect (excludes the 1-col left border).
pub(crate) fn inner_rect(rect: Rect) -> Rect {
    Rect::new(
        rect.x.saturating_add(1),
        rect.y,
        rect.width.saturating_sub(1),
        rect.height,
    )
}

pub(crate) fn title_row(rect: Rect) -> Rect {
    let r = inner_rect(rect);
    Rect::new(r.x, r.y, r.width, 1.min(r.height))
}

pub(crate) fn input_row(rect: Rect) -> Rect {
    let r = inner_rect(rect);
    if r.height < 2 {
        return Rect::default();
    }
    Rect::new(r.x, r.y + 1, r.width, 1)
}

pub(crate) fn status_row(rect: Rect) -> Rect {
    let r = inner_rect(rect);
    if r.height < 3 {
        return Rect::default();
    }
    Rect::new(r.x, r.y + 2, r.width, 1)
}

pub(crate) fn body_rect(rect: Rect) -> Rect {
    let r = inner_rect(rect);
    Rect::new(
        r.x,
        r.y.saturating_add(HEADER_ROWS),
        r.width,
        r.height.saturating_sub(HEADER_ROWS),
    )
}

/// `(keyword_chip, ai_chip)` rects on the title row, right-aligned.
pub(crate) fn mode_chip_rects(rect: Rect) -> (Rect, Rect) {
    let row = title_row(rect);
    let ai_w = display_w(AI_CHIP);
    let kw_w = display_w(KEYWORD_CHIP);
    let total = ai_w + kw_w + 1;
    if row.width < total + 2 || row.height == 0 {
        return (Rect::default(), Rect::default());
    }
    let ai_x = row.x + row.width - ai_w - 1;
    let kw_x = ai_x - 1 - kw_w;
    (
        Rect::new(kw_x, row.y, kw_w, 1),
        Rect::new(ai_x, row.y, ai_w, 1),
    )
}

pub(crate) const KEYWORD_CHIP: &str = " Keyword ";
pub(crate) const AI_CHIP: &str = " AI ";

fn display_w(text: &str) -> u16 {
    crate::ui::text::display_width_u16(text)
}

fn contains(rect: Rect, col: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchPaneClick {
    ModeKeyword,
    ModeAi,
    Input,
    Hit(usize),
    Background,
}

pub(crate) fn click_target(
    state: &SearchPaneState,
    rect: Rect,
    col: u16,
    row: u16,
) -> Option<SearchPaneClick> {
    if !contains(rect, col, row) {
        return None;
    }
    let (kw, ai) = mode_chip_rects(rect);
    if contains(kw, col, row) {
        return Some(SearchPaneClick::ModeKeyword);
    }
    if contains(ai, col, row) {
        return Some(SearchPaneClick::ModeAi);
    }
    if contains(title_row(rect), col, row) || contains(input_row(rect), col, row) {
        return Some(SearchPaneClick::Input);
    }
    let body = body_rect(rect);
    if contains(body, col, row) {
        let index = usize::from(row - body.y) + state.scroll;
        if let Some(BodyRow::Hit { flat, .. }) = body_rows(state).get(index) {
            return Some(SearchPaneClick::Hit(*flat));
        }
    }
    Some(SearchPaneClick::Background)
}

fn row_index_of_flat(rows: &[BodyRow], flat: usize) -> Option<usize> {
    rows.iter().position(|r| matches!(r, BodyRow::Hit { flat: f, .. } if *f == flat))
}

// ---------------------------------------------------------------------------
// State transitions
// ---------------------------------------------------------------------------

impl AppState {
    pub(crate) fn toggle_search_pane(&mut self) {
        if self.search_pane.visible {
            self.search_pane.visible = false;
            self.search_pane.jump_highlight = None;
            if self.mode == Mode::SearchPane {
                self.leave_search_pane_focus();
            }
        } else {
            self.focus_search_pane();
        }
    }

    pub(crate) fn focus_search_pane(&mut self) {
        self.search_pane.visible = true;
        self.mode = Mode::SearchPane;
    }

    pub(crate) fn leave_search_pane_focus(&mut self) {
        if self.mode == Mode::SearchPane {
            self.mode = if self.active.is_some() {
                Mode::Terminal
            } else {
                Mode::Navigate
            };
        }
    }

    pub(crate) fn search_pane_insert_text(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() {
            return;
        }
        self.search_pane.query.push_str(&text);
        self.search_pane.invalidate();
    }

    pub(crate) fn search_pane_set_mode(&mut self, mode: SearchPaneMode) -> bool {
        if self.search_pane.mode == mode {
            return false;
        }
        self.search_pane.mode = mode;
        self.search_pane.invalidate();
        true
    }

    pub(crate) fn move_search_pane_selection(&mut self, delta: isize) {
        let count = self.search_pane.hit_count();
        if count == 0 {
            self.search_pane.selected = None;
            return;
        }
        let next = match self.search_pane.selected {
            None if delta >= 0 => 0,
            None => count - 1,
            Some(current) => (current as isize + delta).rem_euclid(count as isize) as usize,
        };
        self.search_pane.selected = Some(next);
        self.ensure_search_pane_selection_visible();
    }

    pub(crate) fn ensure_search_pane_selection_visible(&mut self) {
        let Some(flat) = self.search_pane.selected else {
            return;
        };
        let body = body_rect(self.view.search_pane_rect);
        let viewport = usize::from(body.height).max(1);
        let rows = body_rows(&self.search_pane);
        let Some(index) = row_index_of_flat(&rows, flat) else {
            return;
        };
        // keep the group header visible when jumping to a group's first hit
        let top = if index > 0 && matches!(rows[index - 1], BodyRow::Group(_)) {
            index - 1
        } else {
            index
        };
        if top < self.search_pane.scroll {
            self.search_pane.scroll = top;
        } else if index >= self.search_pane.scroll + viewport {
            self.search_pane.scroll = index + 1 - viewport;
        }
    }

    pub(crate) fn scroll_search_pane(&mut self, delta: isize) {
        let body = body_rect(self.view.search_pane_rect);
        let viewport = usize::from(body.height).max(1);
        let max = body_rows(&self.search_pane).len().saturating_sub(viewport);
        let next = (self.search_pane.scroll as isize + delta).clamp(0, max as isize) as usize;
        self.search_pane.scroll = next;
    }

    /// Keyword search over every agent pane's scrollback. Synchronous; a few ms.
    pub(crate) fn run_keyword_search(&mut self, terminal_runtimes: &TerminalRuntimeRegistry) {
        let query = self.search_pane.query.trim().to_string();
        self.search_pane.selected = None;
        self.search_pane.scroll = 0;
        if query.is_empty() {
            self.search_pane.groups.clear();
            self.search_pane.jump_highlight = None;
            self.search_pane.searched = None;
            self.search_pane.status = None;
            return;
        }
        let case_sensitive = query.chars().any(char::is_uppercase);
        let entries = crate::ui::agent_panel_entries_from(self, terminal_runtimes);
        let mut groups = Vec::new();
        for entry in &entries {
            let Some(runtime) =
                self.runtime_for_pane_in_workspace(terminal_runtimes, entry.ws_idx, entry.pane_id)
            else {
                continue;
            };
            let mut found = runtime.search_text_matches_with_lines(&query, case_sensitive);
            found.sort_by_key(|(m, _)| (m.start.row, m.start.col));
            let hits = recent_hits(found, &query);
            if hits.is_empty() {
                continue;
            }
            groups.push(SearchGroup {
                ws_idx: entry.ws_idx,
                tab_idx: entry.tab_idx,
                pane_id: entry.pane_id,
                title: group_title(entry),
                subtitle: group_subtitle(entry),
                tokens: entry.tokens.clone(),
                hits,
            });
        }
        let total: usize = groups.iter().map(|g| g.hits.len()).sum();
        self.search_pane.status = Some(if groups.is_empty() {
            "no matches".into()
        } else {
            format!("{total} in {} agent{}", groups.len(), plural(groups.len()))
        });
        self.search_pane.groups = groups;
        self.search_pane.searched = Some((query, SearchPaneMode::Keyword));
    }

    /// Focus the hit's pane and scroll its scrollback so the match is near the top.
    pub(crate) fn jump_to_search_hit(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        flat: usize,
    ) -> bool {
        let Some((group, hit)) = self.search_pane.hit(flat) else {
            return false;
        };
        let (ws_idx, tab_idx, pane_id, row) =
            (group.ws_idx, group.tab_idx, group.pane_id, hit.text_match.start.row);
        let text_match = hit.text_match;
        if !self.focus_navigator_target(NavigatorTarget::Pane {
            ws_idx,
            tab_idx,
            pane_id,
        }) {
            self.search_pane.status = Some("that pane is gone".into());
            return false;
        }
        if let Some(runtime) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
        {
            if let Some(metrics) = runtime.scroll_metrics() {
                let desired_top = (row as usize).saturating_sub(metrics.viewport_rows / 4);
                let offset = metrics.max_offset_from_bottom.saturating_sub(desired_top);
                runtime.set_scroll_offset_from_bottom(offset);
            }
        }
        self.search_pane.selected = Some(flat);
        self.search_pane.jump_highlight = Some(JumpHighlight {
            pane_id,
            text_match,
        });
        // focus_navigator_target leaves us in Terminal mode: keyboard goes to the pane,
        // the search pane stays open with its query and results.
        true
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Newest-first, one hit per row, at most `MAX_HITS_PER_PANE`.
fn recent_hits(sorted: Vec<(TerminalTextMatch, String)>, needle: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut last_row = None;
    for (text_match, line) in sorted.into_iter().rev() {
        if last_row == Some(text_match.start.row) {
            continue;
        }
        last_row = Some(text_match.start.row);
        hits.push(make_hit(text_match, &line, needle));
        if hits.len() >= MAX_HITS_PER_PANE {
            break;
        }
    }
    hits
}

fn make_hit(text_match: TerminalTextMatch, line: &str, needle: &str) -> SearchHit {
    let snippet = collapse_ws(line);
    let match_char = snippet
        .to_lowercase()
        .find(&needle.to_lowercase())
        .map(|byte| snippet[..byte].chars().count())
        .unwrap_or(0);
    SearchHit {
        text_match,
        snippet,
        match_char,
    }
}

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn group_title(entry: &AgentPanelEntry) -> String {
    let label = entry
        .tokens
        .get("topic")
        .filter(|t| !t.trim().is_empty())
        .cloned()
        .or_else(|| entry.agent_label.clone())
        .unwrap_or_else(|| entry.primary_label.clone());
    format!("{} · {}", entry.index, label)
}

fn group_subtitle(entry: &AgentPanelEntry) -> String {
    let model = entry
        .tokens
        .get("model")
        .filter(|t| !t.trim().is_empty())
        .cloned()
        .or_else(|| entry.agent_label.clone());
    match model {
        Some(model) if model != entry.primary_label => {
            format!("{} · {}", entry.primary_label, model)
        }
        _ => entry.primary_label.clone(),
    }
}

// ---------------------------------------------------------------------------
// Keys + AI (need App for the event channel)
// ---------------------------------------------------------------------------

impl App {
    /// Entry point from the mode dispatcher: the toggle keybind wins over text entry.
    pub(crate) fn handle_search_pane_terminal_key(&mut self, key: &crate::input::TerminalKey) {
        if crate::app::input::terminal_direct_non_indexed_navigation_action(&self.state, key)
            == Some(crate::app::input::NavigateAction::SearchPane)
        {
            if key.kind != crossterm::event::KeyEventKind::Release {
                self.state.toggle_search_pane();
            }
            return;
        }
        self.handle_search_pane_key(key.as_key_event());
    }

    pub(crate) fn handle_search_pane_key(&mut self, key: KeyEvent) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.state.leave_search_pane_focus(),
            KeyCode::Tab | KeyCode::BackTab => {
                let next = self.state.search_pane.mode.toggled();
                self.state.search_pane_set_mode(next);
                if !self.state.search_pane.query.trim().is_empty() {
                    self.run_search_pane_query();
                }
            }
            KeyCode::Enter => {
                if self.state.search_pane.results_fresh() {
                    if let Some(flat) = self.state.search_pane.selected {
                        self.state
                            .jump_to_search_hit(&self.terminal_runtimes, flat);
                        return;
                    }
                }
                self.run_search_pane_query();
            }
            KeyCode::Up => self.state.move_search_pane_selection(-1),
            KeyCode::Down => self.state.move_search_pane_selection(1),
            KeyCode::PageUp => self.state.scroll_search_pane(-5),
            KeyCode::PageDown => self.state.scroll_search_pane(5),
            KeyCode::Char('p') if ctrl => self.state.move_search_pane_selection(-1),
            KeyCode::Char('n') if ctrl => self.state.move_search_pane_selection(1),
            KeyCode::Char('u') if ctrl => {
                self.state.search_pane.query.clear();
                self.state.search_pane.invalidate();
            }
            KeyCode::Backspace => {
                self.state.search_pane.query.pop();
                self.state.search_pane.invalidate();
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.state.search_pane_insert_text(&c.to_string());
            }
            _ => {}
        }
    }

    pub(crate) fn run_search_pane_query(&mut self) {
        match self.state.search_pane.mode {
            SearchPaneMode::Keyword => self.state.run_keyword_search(&self.terminal_runtimes),
            SearchPaneMode::Ai => self.start_ai_search(),
        }
    }

    pub(crate) fn handle_search_pane_click(&mut self, click: SearchPaneClick) {
        match click {
            SearchPaneClick::ModeKeyword | SearchPaneClick::ModeAi => {
                let mode = if click == SearchPaneClick::ModeAi {
                    SearchPaneMode::Ai
                } else {
                    SearchPaneMode::Keyword
                };
                self.state.focus_search_pane();
                if self.state.search_pane_set_mode(mode)
                    && !self.state.search_pane.query.trim().is_empty()
                {
                    self.run_search_pane_query();
                }
            }
            SearchPaneClick::Input | SearchPaneClick::Background => {
                self.state.focus_search_pane();
            }
            SearchPaneClick::Hit(flat) => {
                self.state
                    .jump_to_search_hit(&self.terminal_runtimes, flat);
            }
        }
    }

    fn start_ai_search(&mut self) {
        let query = self.state.search_pane.query.trim().to_string();
        self.state.search_pane.selected = None;
        self.state.search_pane.scroll = 0;
        if query.is_empty() {
            self.state.search_pane.groups.clear();
            self.state.search_pane.jump_highlight = None;
            self.state.search_pane.searched = None;
            self.state.search_pane.status = None;
            return;
        }
        let entries = crate::ui::agent_panel_entries_from(&self.state, &self.terminal_runtimes);
        let mut panes = Vec::new();
        let mut sections = Vec::new();
        let mut budget = AI_MAX_PROMPT_CHARS;
        for entry in &entries {
            let Some(runtime) = self.state.runtime_for_pane_in_workspace(
                &self.terminal_runtimes,
                entry.ws_idx,
                entry.pane_id,
            ) else {
                continue;
            };
            let text = runtime
                .recent_unwrapped_text_snapshot(AI_LINES_PER_PANE)
                .text;
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let text: String = if text.chars().count() > budget {
                text.chars().rev().take(budget).collect::<Vec<_>>().into_iter().rev().collect()
            } else {
                text.to_string()
            };
            budget = budget.saturating_sub(text.chars().count());
            let n = panes.len() + 1;
            sections.push(format!(
                "### PANE {n} — {} ({})\n{text}\n",
                group_title(entry),
                group_subtitle(entry)
            ));
            panes.push(AiPaneRef {
                ws_idx: entry.ws_idx,
                tab_idx: entry.tab_idx,
                pane_id: entry.pane_id,
                title: group_title(entry),
                subtitle: group_subtitle(entry),
                tokens: entry.tokens.clone(),
            });
            if budget == 0 {
                break;
            }
        }
        if panes.is_empty() {
            self.state.search_pane.groups.clear();
            self.state.search_pane.jump_highlight = None;
            self.state.search_pane.status = Some("no agent panes to search".into());
            self.state.search_pane.searched = Some((query, SearchPaneMode::Ai));
            return;
        }

        self.state.search_pane.ai_generation += 1;
        let generation = self.state.search_pane.ai_generation;
        self.state.search_pane.ai_inflight = true;
        self.state.search_pane.ai_panes = panes;
        self.state.search_pane.groups.clear();
            self.state.search_pane.jump_highlight = None;
        self.state.search_pane.status = Some("thinking…".into());
        self.state.search_pane.searched = Some((query.clone(), SearchPaneMode::Ai));

        let prompt = build_ai_prompt(&query, &sections);
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let result = run_ai_command(prompt).await;
            let _ = tx
                .send(AppEvent::SearchPaneAiFinished { generation, result })
                .await;
        });
    }

    pub(crate) fn handle_search_pane_ai_finished(
        &mut self,
        generation: u64,
        result: Result<String, String>,
    ) {
        if generation != self.state.search_pane.ai_generation {
            return;
        }
        self.state.search_pane.ai_inflight = false;
        let output = match result {
            Ok(output) => output,
            Err(err) => {
                self.state.search_pane.status = Some(format!("AI search failed: {err}"));
                return;
            }
        };
        let picks = match parse_ai_output(&output) {
            Ok(picks) => picks,
            Err(err) => {
                self.state.search_pane.status = Some(format!("AI reply unreadable: {err}"));
                return;
            }
        };
        let panes = self.state.search_pane.ai_panes.clone();
        let mut groups = Vec::new();
        for pick in picks {
            let Some(pane) = pick.pane.checked_sub(1).and_then(|i| panes.get(i)) else {
                continue;
            };
            let Some(runtime) = self.state.runtime_for_pane_in_workspace(
                &self.terminal_runtimes,
                pane.ws_idx,
                pane.pane_id,
            ) else {
                continue;
            };
            let mut hits: Vec<SearchHit> = Vec::new();
            for quote in pick.quotes {
                let quote = collapse_ws(&quote);
                if quote.is_empty() {
                    continue;
                }
                // Exact quote first; if the model paraphrased, fall back to its first words.
                let candidates = [quote.clone(), first_words(&quote, 5), first_words(&quote, 3)];
                let found = candidates.iter().find_map(|needle| {
                    if needle.chars().count() < 3 {
                        return None;
                    }
                    let mut found = runtime.search_text_matches_with_lines(needle, false);
                    found.sort_by_key(|(m, _)| (m.start.row, m.start.col));
                    found.pop().map(|(m, line)| make_hit(m, &line, needle))
                });
                if let Some(hit) = found {
                    if !hits
                        .iter()
                        .any(|h| h.text_match.start.row == hit.text_match.start.row)
                    {
                        hits.push(hit);
                    }
                }
                if hits.len() >= MAX_HITS_PER_PANE {
                    break;
                }
            }
            if hits.is_empty() {
                continue;
            }
            hits.sort_by_key(|h| std::cmp::Reverse(h.text_match.start.row));
            groups.push(SearchGroup {
                ws_idx: pane.ws_idx,
                tab_idx: pane.tab_idx,
                pane_id: pane.pane_id,
                title: pane.title.clone(),
                subtitle: pane.subtitle.clone(),
                tokens: pane.tokens.clone(),
                hits,
            });
        }
        let total: usize = groups.iter().map(|g| g.hits.len()).sum();
        self.state.search_pane.status = Some(if groups.is_empty() {
            "AI found nothing relevant".into()
        } else {
            format!("AI: {total} in {} agent{}", groups.len(), plural(groups.len()))
        });
        self.state.search_pane.groups = groups;
        self.state.search_pane.scroll = 0;
    }
}

fn first_words(text: &str, n: usize) -> String {
    text.split_whitespace().take(n).collect::<Vec<_>>().join(" ")
}

#[derive(Debug, serde::Deserialize)]
struct AiPick {
    pane: usize,
    #[serde(default)]
    quotes: Vec<String>,
}

fn build_ai_prompt(query: &str, sections: &[String]) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are the search engine for a terminal multiplexer running several AI coding agents. \
         Below are the recent terminal transcripts of each agent pane, delimited by `### PANE n` headers.\n\n\
         The user is looking for: ",
    );
    prompt.push_str(query);
    prompt.push_str(
        "\n\nFind the passages most relevant to that request. For each pane that has something relevant, \
         return up to 3 short quotes (one line each, at most ~100 characters), copied VERBATIM from that pane's \
         transcript so they can be located by exact substring match. Prefer the most recent relevant passages. \
         Skip panes with nothing relevant.\n\n\
         Reply with JSON only, no prose, exactly this shape:\n\
         [{\"pane\": 1, \"quotes\": [\"exact text from pane 1\", \"...\"]}, {\"pane\": 3, \"quotes\": [\"...\"]}]\n\n",
    );
    for section in sections {
        prompt.push_str(section);
        prompt.push('\n');
    }
    prompt
}

fn parse_ai_output(output: &str) -> Result<Vec<AiPick>, String> {
    let start = output.find('[').ok_or("no JSON array in reply")?;
    let end = output.rfind(']').ok_or("no JSON array in reply")?;
    if end <= start {
        return Err("malformed JSON array".into());
    }
    serde_json::from_str::<Vec<AiPick>>(&output[start..=end]).map_err(|e| e.to_string())
}

/// Run the AI command. `HERDR_SEARCH_AI_COMMAND` (a shell command reading the
/// prompt on stdin and printing JSON) overrides the default `pi -p` call;
/// `HERDR_SEARCH_AI_MODEL` picks the pi model (default claude-haiku-4-5).
async fn run_ai_command(prompt: String) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    let timeout = std::time::Duration::from_secs(AI_TIMEOUT_SECS);
    let mut command = if let Ok(custom) = std::env::var("HERDR_SEARCH_AI_COMMAND") {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(custom);
        c
    } else {
        let model = std::env::var("HERDR_SEARCH_AI_MODEL")
            .unwrap_or_else(|_| "claude-haiku-4-5".to_string());
        let mut c = tokio::process::Command::new("pi");
        c.args([
            "-p",
            "--no-session",
            "--no-tools",
            "--no-extensions",
            "--no-skills",
            "--no-context-files",
            "--no-prompt-templates",
            "--thinking",
            "off",
            "--system-prompt",
            "You output only JSON. No prose, no code fences.",
            "--model",
            &model,
        ]);
        c
    };
    // Prompt on stdin; a lone "-" is not universally supported, so pass it as the last arg
    // for pi and via stdin for custom commands.
    let custom = std::env::var("HERDR_SEARCH_AI_COMMAND").is_ok();
    if !custom {
        command.arg(&prompt);
    }
    command
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_SOCKET_PATH")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|e| format!("spawn: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let payload = if custom { prompt.clone() } else { String::new() };
        tokio::spawn(async move {
            let _ = stdin.write_all(payload.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| format!("timed out after {AI_TIMEOUT_SECS}s"))?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr.lines().last().unwrap_or("").trim();
        return Err(if line.is_empty() {
            format!("exit {}", output.status)
        } else {
            line.chars().take(120).collect()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ghostty::ActiveScreen, pane::TerminalTextPoint};

    fn m(row: u32) -> TerminalTextMatch {
        TerminalTextMatch {
            start: TerminalTextPoint { row, col: 0 },
            end: TerminalTextPoint { row, col: 3 },
            source_fingerprint: 0,
            scan_cols: 80,
            scan_screen: ActiveScreen::Primary,
        }
    }

    #[test]
    fn recent_hits_keeps_last_three_rows_newest_first() {
        let found = (0..6u32).map(|r| (m(r), format!("line {r} foo"))).collect();
        let hits = recent_hits(found, "foo");
        let rows: Vec<u32> = hits.iter().map(|h| h.text_match.start.row).collect();
        assert_eq!(rows, vec![5, 4, 3]);
        assert_eq!(hits[0].snippet, "line 5 foo");
        assert_eq!(hits[0].match_char, 7);
    }

    #[test]
    fn recent_hits_dedupes_same_row() {
        let found = vec![(m(2), "a foo foo".into()), (m(2), "a foo foo".into()), (m(1), "foo".into())];
        assert_eq!(recent_hits(found, "foo").len(), 2);
    }

    #[test]
    fn body_rows_flatten_groups_with_gaps() {
        let mut state = SearchPaneState::default();
        let group = |n: usize| SearchGroup {
            ws_idx: 0,
            tab_idx: 0,
            pane_id: PaneId::from_raw(1),
            title: String::new(),
            subtitle: String::new(),
            tokens: Default::default(),
            hits: (0..n)
                .map(|r| make_hit(m(r as u32), "x", "x"))
                .collect(),
        };
        state.groups = vec![group(2), group(1)];
        let rows = body_rows(&state);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[3], BodyRow::Gap);
        assert!(matches!(rows[5], BodyRow::Hit { flat: 2, .. }));
        assert_eq!(state.hit_count(), 3);
        assert!(state.hit(2).is_some());
        assert!(state.hit(3).is_none());
    }

    #[test]
    fn parse_ai_output_tolerates_prose_and_fences() {
        let out = "Sure!\n```json\n[{\"pane\": 2, \"quotes\": [\"hello\"]}]\n```";
        let picks = parse_ai_output(out).unwrap();
        assert_eq!(picks[0].pane, 2);
        assert_eq!(picks[0].quotes, vec!["hello"]);
        assert!(parse_ai_output("nothing here").is_err());
    }

    #[test]
    fn click_targets_follow_layout() {
        let mut state = SearchPaneState::default();
        state.groups = vec![SearchGroup {
            ws_idx: 0,
            tab_idx: 0,
            pane_id: PaneId::from_raw(1),
            title: "t".into(),
            subtitle: String::new(),
            tokens: Default::default(),
            hits: vec![make_hit(m(0), "x", "x")],
        }];
        let rect = Rect::new(100, 0, 40, 20);
        assert_eq!(click_target(&state, rect, 99, 5), None);
        assert_eq!(click_target(&state, rect, 105, 1), Some(SearchPaneClick::Input));
        let (kw, ai) = mode_chip_rects(rect);
        assert_eq!(click_target(&state, rect, kw.x, 0), Some(SearchPaneClick::ModeKeyword));
        assert_eq!(click_target(&state, rect, ai.x, 0), Some(SearchPaneClick::ModeAi));
        // body row 0 = Group header, row 1 = first hit
        assert_eq!(click_target(&state, rect, 105, HEADER_ROWS), Some(SearchPaneClick::Background));
        assert_eq!(click_target(&state, rect, 105, HEADER_ROWS + 1), Some(SearchPaneClick::Hit(0)));
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;
    use crate::{app::input::app_for_mouse_test, input::TerminalKey, workspace::Workspace};
    use crossterm::event::KeyModifiers;

    fn app_with_scrollback(bytes: &[u8]) -> (App, PaneId) {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let pane_infos = ws.tabs[0].layout.panes(Rect::new(0, 0, 40, 5));
        let info = pane_infos[0].clone();
        ws.tabs[0].runtimes.insert(
            pane_id,
            crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                info.inner_rect.width,
                info.inner_rect.height,
                16 * 1024,
                bytes,
            ),
        );
        // The agent panel (and therefore search) only lists panes running a known agent.
        let terminal_id = ws.tabs[0].panes[&pane_id].attached_terminal_id.clone();
        let mut terminal = crate::terminal::TerminalState::new(terminal_id.clone(), "/tmp".into());
        terminal.agent_name = Some("pi".into());
        app.state.terminals.insert(terminal_id, terminal);
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.pane_infos = pane_infos;
        app.state.view.search_pane_rect = Rect::new(40, 0, 30, 20);
        (app, pane_id)
    }

    fn type_text(app: &mut App, text: &str) {
        for ch in text.chars() {
            app.handle_search_pane_terminal_key(&TerminalKey::new(
                KeyCode::Char(ch),
                KeyModifiers::empty(),
            ));
        }
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_search_pane_terminal_key(&TerminalKey::new(code, KeyModifiers::empty()));
    }

    #[tokio::test]
    async fn toggle_opens_focuses_and_hides_keeping_query() {
        let (mut app, _) = app_with_scrollback(b"alpha\r\n");
        app.state.toggle_search_pane();
        assert!(app.state.search_pane.visible);
        assert_eq!(app.state.mode, Mode::SearchPane);
        type_text(&mut app, "alp");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.search_pane.visible, "esc keeps the pane open");
        app.state.toggle_search_pane();
        assert!(!app.state.search_pane.visible);
        assert_eq!(app.state.search_pane.query, "alp", "query survives hiding");
        assert_eq!(app.state.search_pane.jump_highlight, None);
    }

    #[tokio::test]
    async fn keyword_search_groups_recent_hits_and_enter_jumps() {
        let mut bytes = Vec::new();
        for i in 0..40 {
            bytes.extend_from_slice(format!("line {i} {}\r\n", if i % 10 == 0 { "needle" } else { "hay" }).as_bytes());
        }
        let (mut app, pane_id) = app_with_scrollback(&bytes);
        app.state.toggle_search_pane();
        type_text(&mut app, "needle");
        press(&mut app, KeyCode::Enter);

        let sp = &app.state.search_pane;
        assert_eq!(sp.groups.len(), 1);
        assert_eq!(sp.groups[0].pane_id, pane_id);
        assert_eq!(sp.groups[0].hits.len(), MAX_HITS_PER_PANE, "capped at 3 of the 4 matches");
        let rows: Vec<u32> = sp.groups[0].hits.iter().map(|h| h.text_match.start.row).collect();
        assert!(rows.windows(2).all(|w| w[0] > w[1]), "newest first: {rows:?}");
        assert!(sp.groups[0].hits[0].snippet.contains("line 30 needle"));
        assert!(sp.results_fresh());

        // Down selects the newest hit; Enter jumps: pane focused, scrolled so the row is visible.
        press(&mut app, KeyCode::Down);
        assert_eq!(app.state.search_pane.selected, Some(0));
        let target_row = rows[2] as usize; // oldest of the three: needs a real scroll
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.state.mode, Mode::Terminal);
        let jump = app.state.search_pane.jump_highlight.expect("jump highlight set");
        assert_eq!(jump.pane_id, pane_id);
        assert_eq!(jump.text_match.start.row as usize, target_row);
        let metrics = app
            .state
            .runtime_for_pane_in_workspace(&app.terminal_runtimes, 0, pane_id)
            .and_then(crate::terminal::TerminalRuntime::scroll_metrics)
            .expect("metrics");
        let top = metrics.max_offset_from_bottom - metrics.offset_from_bottom;
        assert!(
            top <= target_row && target_row < top + metrics.viewport_rows,
            "row {target_row} not in viewport starting {top} (rows {})",
            metrics.viewport_rows
        );
    }

    #[tokio::test]
    async fn editing_query_invalidates_results_and_enter_searches_again() {
        let (mut app, _) = app_with_scrollback(b"foo\r\nbar\r\n");
        app.state.toggle_search_pane();
        type_text(&mut app, "foo");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.state.search_pane.hit_count(), 1);
        press(&mut app, KeyCode::Backspace);
        assert!(!app.state.search_pane.results_fresh());
        assert_eq!(app.state.search_pane.selected, None);
        type_text(&mut app, "x");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.state.search_pane.hit_count(), 0);
        assert_eq!(app.state.search_pane.status.as_deref(), Some("no matches"));
    }
}
