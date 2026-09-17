//! Room-local editor. Behavior reference: installed Pi 0.85.1 editor.ts.
//! Byte offsets internally; vertical preferences use UTF-16, not display cells.
use ratatui::layout::{Position, Rect};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const LIMIT: usize = crate::room::MAX_MESSAGE_BYTES;
const RETAIN: usize = 100;

#[derive(Clone, Debug)]
struct Paste {
    range: Range<usize>,
    payload: String,
}
#[derive(Clone)]
struct Snapshot {
    text: String,
    cursor: usize,
    pastes: Vec<Paste>,
    counter: usize,
}
#[derive(Default, PartialEq, Eq)]
enum Action {
    #[default]
    None,
    Type,
    Kill,
    Yank,
}
#[derive(Default)]
pub struct Editor {
    pub cursor: usize,
    pub top: usize,
    preferred: Option<usize>,
    snapped: Option<usize>,
    history: Vec<String>,
    history_index: Option<usize>,
    draft: Option<Snapshot>,
    undo: Vec<Snapshot>,
    ring: Vec<String>,
    yank_range: Option<Range<usize>>,
    pastes: Vec<Paste>,
    counter: usize,
    action: Action,
    jump: Option<bool>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
}

fn cjk(g: &str) -> bool {
    g.chars().any(|c| matches!(c as u32, 0x2e80..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0x20000..=0x323af))
}

/// Pi word wrapping: keep whitespace; break after whitespace runs or beside
/// CJK, otherwise wrap long words at grapheme boundaries. No empty cursor row.
pub fn rows(text: &str, width: usize) -> Vec<Row> {
    wrap(text, width, &[])
}
fn wrap(text: &str, width: usize, pastes: &[Paste]) -> Vec<Row> {
    let width = width.max(1);
    let mut result = Vec::new();
    let mut base = 0;
    for line in text.split('\n') {
        let segments = atoms(line, base, pastes);
        let (mut start, mut columns, mut opportunity) = (0, 0, None);
        for (i, &(byte, g)) in segments.iter().enumerate() {
            let size = g.width();
            if columns + size > width {
                if let Some((at, used)) =
                    opportunity.filter(|(_, used)| columns - used + size <= width)
                {
                    result.push(Row {
                        start: base + start,
                        end: base + at,
                    });
                    start = at;
                    columns -= used;
                } else if byte > start {
                    result.push(Row {
                        start: base + start,
                        end: base + byte,
                    });
                    start = byte;
                    columns = 0;
                }
                opportunity = None;
            }
            // An oversized marker may span rows but remains one editing unit.
            if size > width && g.graphemes(true).count() > 1 {
                let sub = rows(g, width);
                for row in sub.iter().take(sub.len().saturating_sub(1)) {
                    result.push(Row {
                        start: base + byte + row.start,
                        end: base + byte + row.end,
                    });
                }
                if let Some(last) = sub.last() {
                    start = byte + last.start;
                    columns = g[last.start..].width();
                }
                continue;
            }
            columns += size;
            if let Some(&(next_byte, next)) = segments.get(i + 1) {
                let ws = g.chars().all(char::is_whitespace);
                let next_ws = next.chars().all(char::is_whitespace);
                if (ws && !next_ws) || (!ws && !next_ws && (cjk(g) || cjk(next))) {
                    opportunity = Some((next_byte, columns));
                }
            }
        }
        result.push(Row {
            start: base + start,
            end: base + line.len(),
        });
        base += line.len() + 1;
    }
    result
}

fn atoms<'a>(text: &'a str, base: usize, pastes: &[Paste]) -> Vec<(usize, &'a str)> {
    let mut result = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        let end = pastes
            .iter()
            .find(|p| p.range.start == base + pos)
            .map(|p| p.range.end.saturating_sub(base).min(text.len()))
            .unwrap_or_else(|| {
                pos + text[pos..]
                    .graphemes(true)
                    .next()
                    .map(str::len)
                    .unwrap_or(0)
            });
        result.push((pos, &text[pos..end]));
        pos = end;
    }
    result
}

pub fn byte_at_column(text: &str, column: usize) -> usize {
    let mut x = 0;
    for (byte, grapheme) in text.grapheme_indices(true) {
        x += grapheme.width();
        if x > column {
            return byte;
        }
    }
    text.len()
}

impl Editor {
    pub fn rows(&self, text: &str, width: usize) -> Vec<Row> {
        wrap(text, width, &self.pastes)
    }
    fn snapshot(&self, text: &str) -> Snapshot {
        Snapshot {
            text: text.into(),
            cursor: self.cursor,
            pastes: self.pastes.clone(),
            counter: self.counter,
        }
    }
    fn save(&mut self, text: &str) {
        if self.undo.len() == RETAIN {
            self.undo.remove(0);
        }
        self.undo.push(self.snapshot(text));
    }
    fn restore(&mut self, text: &mut String, state: Snapshot) {
        *text = state.text;
        self.cursor = state.cursor;
        self.pastes = state.pastes;
        self.counter = state.counter;
        self.top = 0;
        self.moved();
    }
    fn moved(&mut self) {
        self.action = Action::None;
        self.preferred = None;
        self.snapped = None;
        self.yank_range = None;
    }
    fn exit_history(&mut self) {
        self.history_index = None;
        self.draft = None;
    }
    pub fn clicked(&mut self, text: &str) {
        self.exit_history();
        self.moved();
        self.normalize(text);
    }
    pub fn normalize(&mut self, text: &str) {
        self.cursor = self.snap(text, self.cursor);
    }
    fn snap(&self, text: &str, cursor: usize) -> usize {
        let cursor = cursor.min(text.len());
        if let Some(p) = self
            .pastes
            .iter()
            .find(|p| p.range.start < cursor && cursor < p.range.end)
        {
            return p.range.start;
        }
        text.grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .take_while(|i| *i <= cursor)
            .last()
            .unwrap_or(0)
    }
    pub fn expanded(&self, text: &str) -> String {
        let mut result = String::new();
        let mut pos = 0;
        for paste in &self.pastes {
            if paste.range.start >= pos && paste.range.end <= text.len() {
                result.push_str(&text[pos..paste.range.start]);
                result.push_str(&paste.payload);
                pos = paste.range.end;
            }
        }
        result.push_str(&text[pos..]);
        result
    }
    fn replace(&mut self, text: &mut String, range: Range<usize>, input: &str) {
        self.pastes.retain_mut(|p| {
            if p.range.end <= range.start {
                return true;
            }
            if p.range.start < range.end {
                return false;
            }
            let start = p.range.start - range.len() + input.len();
            p.range = start..start + p.range.len();
            true
        });
        text.replace_range(range.clone(), input);
        self.cursor = range.start + input.len();
        // Combining input can merge with the next grapheme: snap forward.
        self.cursor = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .find(|i| *i >= self.cursor)
            .unwrap_or(text.len());
        self.preferred = None;
        self.snapped = None;
    }
    fn clean(input: &str) -> String {
        input
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ")
            .chars()
            .filter(|c| *c == '\n' || (*c as u32 >= 32 && !c.is_control()))
            .collect()
    }
    fn fitted(&self, text: &str, input: &str) -> String {
        let remaining = LIMIT.saturating_sub(self.expanded(text).len());
        let mut end = 0;
        for g in input.graphemes(true) {
            if end + g.len() > remaining {
                break;
            }
            end += g.len();
        }
        input[..end].into()
    }
    /// Text/IME input coalesces like Pi insertCharacter; newline is separate.
    pub fn type_text(&mut self, text: &mut String, input: &str) {
        self.normalize(text);
        self.exit_history();
        let safe = self.fitted(text, &Self::clean(input));
        if safe.is_empty() {
            return;
        }
        if self.action != Action::Type || safe.chars().any(char::is_whitespace) {
            self.save(text);
        }
        self.replace(text, self.cursor..self.cursor, &safe);
        self.action = if safe.contains('\n') {
            Action::None
        } else {
            Action::Type
        };
    }
    /// Bracketed/clipboard paste is one undo operation, capped before folding.
    pub fn insert(&mut self, text: &mut String, input: &str) {
        self.normalize(text);
        self.exit_history();
        self.moved();
        let mut safe = Self::clean(input);
        if safe.starts_with(['/', '~', '.'])
            && text[..self.cursor]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            safe.insert(0, ' ');
        }
        let safe = self.fitted(text, &safe);
        if safe.is_empty() {
            return;
        }
        self.save(text);
        let lines = safe.split('\n').count();
        let chars = safe.encode_utf16().count();
        if lines > 10 || chars > 1000 {
            self.counter += 1;
            let marker = if lines > 10 {
                format!("[paste #{} +{lines} lines]", self.counter)
            } else {
                format!("[paste #{} {chars} chars]", self.counter)
            };
            let start = self.cursor;
            self.replace(text, start..start, &marker);
            self.pastes.push(Paste {
                range: start..start + marker.len(),
                payload: safe,
            });
            self.pastes.sort_by_key(|p| p.range.start);
        } else {
            self.replace(text, self.cursor..self.cursor, &safe);
        }
    }
    pub fn left(&mut self, text: &str) {
        self.normalize(text);
        self.moved();
        self.cursor = atoms(&text[..self.cursor], 0, &self.pastes)
            .last()
            .map(|(i, _)| *i)
            .unwrap_or(0);
    }
    pub fn right(&mut self, text: &str) {
        self.normalize(text);
        self.moved();
        self.cursor += atoms(&text[self.cursor..], self.cursor, &self.pastes)
            .first()
            .map(|(_, g)| g.len())
            .unwrap_or(0);
    }
    pub fn home(&mut self, text: &str) {
        self.normalize(text);
        self.moved();
        self.cursor = text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    }
    pub fn end(&mut self, text: &str) {
        self.normalize(text);
        self.moved();
        self.cursor += text[self.cursor..]
            .find('\n')
            .unwrap_or(text.len() - self.cursor);
    }
    fn word_target(&self, text: &str, backwards: bool) -> usize {
        let cursor = self.cursor;
        if backwards && cursor > 0 && text[..cursor].ends_with('\n') {
            return cursor - 1;
        }
        if !backwards && text[cursor..].starts_with('\n') {
            return cursor + 1;
        }
        let start = text[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let end = cursor + text[cursor..].find('\n').unwrap_or(text.len() - cursor);
        let range = if backwards {
            start..cursor
        } else {
            cursor..end
        };
        let mut parts = word_parts(&text[range.clone()], range.start, &self.pastes);
        if backwards {
            parts.reverse();
        }
        let mut target = cursor;
        let mut category = None;
        for (i, g, kind) in parts {
            if let Some(prior) = category {
                // Word segments are indivisible; punctuation runs accumulate.
                if prior != 2 || kind != 2 {
                    break;
                }
            } else if kind != 0 {
                category = Some(kind);
            }
            target = if backwards { i } else { i + g.len() };
            if kind == 1 || kind == 3 {
                break;
            }
        }
        target
    }
    pub fn word_left(&mut self, text: &str) {
        self.normalize(text);
        self.moved();
        self.cursor = self.word_target(text, true);
    }
    pub fn word_right(&mut self, text: &str) {
        self.normalize(text);
        self.moved();
        self.cursor = self.word_target(text, false);
    }
    fn erase(&mut self, text: &mut String, target: usize, kill: bool) {
        self.exit_history();
        let range = target.min(self.cursor)..target.max(self.cursor);
        if range.is_empty() {
            return;
        }
        self.save(text);
        if kill {
            // Retain actual payload, never a dangling marker reference.
            let deleted = self.expanded_range(text, range.clone());
            if self.action == Action::Kill && !self.ring.is_empty() {
                if let Some(last) = self.ring.last_mut() {
                    if target < self.cursor {
                        last.insert_str(0, &deleted);
                    } else {
                        last.push_str(&deleted);
                    }
                    // A kill run cannot exceed the capped draft without an intervening edit.
                }
            } else {
                if self.ring.len() == RETAIN {
                    self.ring.remove(0);
                }
                self.ring.push(deleted);
            }
        }
        self.replace(text, range, "");
        self.normalize(text);
        self.action = if kill { Action::Kill } else { Action::None };
    }
    fn expanded_range(&self, text: &str, range: Range<usize>) -> String {
        let mut result = String::new();
        for (i, g) in atoms(&text[range.clone()], range.start, &self.pastes) {
            if let Some(p) = self
                .pastes
                .iter()
                .find(|p| p.range.start == range.start + i)
            {
                result.push_str(&p.payload);
            } else {
                result.push_str(g);
            }
        }
        result
    }
    pub fn backspace(&mut self, text: &mut String, word: bool) {
        self.normalize(text);
        let target = if word {
            self.word_target(text, true)
        } else {
            atoms(&text[..self.cursor], 0, &self.pastes)
                .last()
                .map(|(i, _)| *i)
                .unwrap_or(0)
        };
        if !word {
            self.moved();
        }
        self.erase(text, target, word);
    }
    pub fn delete(&mut self, text: &mut String) {
        self.normalize(text);
        self.moved();
        let target = self.cursor
            + atoms(&text[self.cursor..], self.cursor, &self.pastes)
                .first()
                .map(|(_, g)| g.len())
                .unwrap_or(0);
        self.erase(text, target, false);
    }
    pub fn kill(&mut self, text: &mut String, backwards: bool, line: bool) {
        self.normalize(text);
        let target = if !line {
            self.word_target(text, backwards)
        } else if backwards {
            let start = text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
            if start == self.cursor {
                start.saturating_sub(1)
            } else {
                start
            }
        } else {
            let end = self.cursor
                + text[self.cursor..]
                    .find('\n')
                    .unwrap_or(text.len() - self.cursor);
            if end == self.cursor {
                (end + 1).min(text.len())
            } else {
                end
            }
        };
        self.erase(text, target, true);
    }
    pub fn undo(&mut self, text: &mut String) {
        self.exit_history();
        self.moved();
        if let Some(snapshot) = self.undo.pop() {
            self.restore(text, snapshot);
        }
    }
    pub fn yank(&mut self, text: &mut String, pop: bool) {
        self.normalize(text);
        if self.ring.is_empty() || (pop && (self.action != Action::Yank || self.ring.len() < 2)) {
            return;
        }
        let range = if pop {
            self.yank_range.clone().unwrap_or(self.cursor..self.cursor)
        } else {
            self.cursor..self.cursor
        };
        let index = if pop {
            self.ring.len() - 2
        } else {
            self.ring.len() - 1
        };
        let payload = self.ring[index].clone();
        if self.expanded(text).len() - self.expanded_range(text, range.clone()).len()
            + payload.len()
            > LIMIT
        {
            return;
        }
        self.save(text);
        self.exit_history();
        if pop {
            self.ring.rotate_right(1);
        }
        let start = range.start;
        self.replace(text, range, &payload);
        self.yank_range = Some(start..self.cursor);
        self.action = Action::Yank;
    }
    pub fn sent(&mut self, text: &mut String) {
        let sent = self.expanded(text).trim().to_owned();
        if !sent.is_empty() && self.history.first() != Some(&sent) {
            self.history.insert(0, sent);
            self.history.truncate(RETAIN);
        }
        text.clear();
        self.cursor = 0;
        self.top = 0;
        self.pastes.clear();
        self.counter = 0;
        self.undo.clear();
        self.jump = None;
        self.exit_history();
        self.moved();
    }
    fn history(&mut self, text: &mut String, older: bool) {
        self.moved();
        let next = if older {
            let i = self.history_index.map(|i| i + 1).unwrap_or(0);
            if i >= self.history.len() {
                return;
            }
            Some(i)
        } else {
            let Some(i) = self.history_index else {
                return;
            };
            i.checked_sub(1)
        };
        if self.history_index.is_none() {
            self.save(text);
            self.draft = Some(self.snapshot(text));
        }
        self.history_index = next;
        if let Some(i) = next {
            *text = self.history[i].clone();
            self.pastes.clear();
            self.cursor = if older { 0 } else { text.len() };
            self.top = 0;
        } else if let Some(draft) = self.draft.take() {
            self.restore(text, draft);
        }
    }
    pub fn vertical(&mut self, text: &mut String, width: usize, delta: isize, history: bool) {
        self.normalize(text);
        self.action = Action::None;
        let rows = self.rows(text, width);
        let current = row_at(&rows, self.cursor);
        if history && delta < 0 && current == 0 {
            let line_start = text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
            if self.cursor == line_start || self.history_index.is_some() {
                self.history(text, true);
            } else {
                self.home(text);
            }
            return;
        }
        if history && delta > 0 && current + 1 == rows.len() {
            if self.history_index.is_some() {
                self.history(text, false);
            } else {
                self.end(text);
            }
            return;
        }
        let target = current.saturating_add_signed(delta).min(rows.len() - 1);
        self.move_visual(text, &rows, current, target);
    }
    fn move_visual(&mut self, text: &str, rows: &[Row], current: usize, mut target: usize) {
        let source_col = self.snapped.unwrap_or_else(|| {
            text[rows[current].start..self.cursor]
                .encode_utf16()
                .count()
        });
        let max_col = |i: usize| {
            let n = text[rows[i].start..rows[i].end].encode_utf16().count();
            if rows
                .get(i + 1)
                .is_some_and(|next| next.start == rows[i].end)
            {
                n.saturating_sub(1)
            } else {
                n
            }
        };
        let source_max = max_col(current);
        loop {
            let target_max = max_col(target);
            let col = if self.preferred.is_none() || source_col < source_max {
                if target_max < source_col {
                    self.preferred = Some(source_col);
                    target_max
                } else {
                    self.preferred = None;
                    source_col
                }
            } else if target_max < source_col || target_max < self.preferred.unwrap_or(0) {
                target_max
            } else {
                self.preferred.take().unwrap_or(source_col)
            };
            let mut utf16 = 0;
            let chunk = &text[rows[target].start..rows[target].end];
            let byte = chunk
                .char_indices()
                .find_map(|(i, c)| {
                    if utf16 + c.len_utf16() > col {
                        Some(i)
                    } else {
                        utf16 += c.len_utf16();
                        None
                    }
                })
                .unwrap_or(chunk.len());
            let requested = rows[target].start + byte;
            let snapped = self.snap(text, requested);
            if snapped < rows[target].start && target > current && target + 1 < rows.len() {
                target += 1;
                continue;
            }
            self.cursor = snapped;
            self.snapped = if snapped != requested || utf16 != col {
                Some(col)
            } else {
                None
            };
            break;
        }
    }
    pub fn jump_key(&mut self, text: &str, key: &crate::input::TerminalKey) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers as M};
        let start = key.code == KeyCode::Char(']')
            && (key.modifiers == M::CONTROL || key.modifiers == (M::CONTROL | M::ALT));
        if let Some(backwards) = self.jump.take() {
            if start {
                return true;
            }
            if let KeyCode::Char(ch) = key.code {
                if !key.modifiers.intersects(M::CONTROL | M::ALT | M::SUPER) {
                    self.normalize(text);
                    self.moved();
                    let found = if backwards {
                        text[..self.cursor].rfind(ch)
                    } else {
                        text[self.cursor..]
                            .char_indices()
                            .skip(1)
                            .find(|(_, c)| *c == ch)
                            .map(|(i, _)| self.cursor + i)
                    };
                    if let Some(i) = found {
                        self.cursor = self.snap(text, i);
                    }
                    return true;
                }
            }
        } else if start {
            self.jump = Some(key.modifiers.contains(M::ALT));
            return true;
        }
        false
    }
    pub fn project(&mut self, text: &str, rows: &[Row], area: Rect) -> Option<Position> {
        self.normalize(text);
        if area.is_empty() {
            return None;
        }
        let row = row_at(rows, self.cursor);
        if row < self.top {
            self.top = row;
        }
        if row >= self.top + area.height as usize {
            self.top = row + 1 - area.height as usize;
        }
        self.top = self
            .top
            .min(rows.len().saturating_sub(area.height as usize));
        let x = text[rows[row].start..self.cursor]
            .width()
            .min(area.width.saturating_sub(1) as usize);
        Some(Position::new(
            area.x + x as u16,
            area.y + (row - self.top) as u16,
        ))
    }
}

pub fn row_at(rows: &[Row], cursor: usize) -> usize {
    rows.iter().rposition(|r| r.start <= cursor).unwrap_or(0)
}

// UAX #29 word segmentation plus Pi's ASCII punctuation boundaries. Unlike
// ICU's dictionary segmentation, unicode-segmentation splits Han per character.
fn word_parts<'a>(text: &'a str, base: usize, pastes: &[Paste]) -> Vec<(usize, &'a str, u8)> {
    let mut result = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        if let Some(p) = pastes.iter().find(|p| p.range.start == base + pos) {
            let end = (p.range.end - base).min(text.len());
            result.push((base + pos, &text[pos..end], 3));
            pos = end;
            continue;
        }
        let end = pastes
            .iter()
            .filter(|p| p.range.start > base + pos)
            .map(|p| p.range.start - base)
            .min()
            .unwrap_or(text.len())
            .min(text.len());
        for (i, segment) in text[pos..end].split_word_bound_indices() {
            // Split punctuation even when UAX joins it inside a word (foo.bar).
            let mut start = 0;
            for (j, c) in segment.char_indices() {
                if c.is_ascii_punctuation() {
                    if j > start {
                        result.push((base + pos + i + start, &segment[start..j], 1));
                    }
                    result.push((base + pos + i + j, &segment[j..j + c.len_utf8()], 2));
                    start = j + c.len_utf8();
                }
            }
            if start < segment.len() {
                let rest = &segment[start..];
                let kind = if rest.chars().all(char::is_whitespace) {
                    0
                } else if rest.unicode_words().next().is_some() {
                    1
                } else {
                    2
                };
                result.push((base + pos + i + start, rest, kind));
            }
        }
        pos = end;
    }
    result
}

/// Editor-only aliases. Room owns submit, selection copy, recipient Tab and
/// transcript paging; no Ctrl+Enter/Alt+Enter behavior is invented here.
pub fn handle_key(
    editor: &mut Editor,
    text: &mut String,
    key: &crate::input::TerminalKey,
    width: usize,
    page_rows: usize,
) -> bool {
    use crossterm::event::{KeyCode as K, KeyModifiers as M};
    let ctrl = key.modifiers == M::CONTROL;
    let alt = key.modifiers == M::ALT;
    let plain = key.modifiers.is_empty();
    let shift = key.modifiers == M::SHIFT;
    match key.code {
        K::Up | K::Down if plain => {
            editor.vertical(text, width, if key.code == K::Up { -1 } else { 1 }, true)
        }
        K::PageUp | K::PageDown if ctrl => {
            let page = page_rows.max(5) as isize;
            editor.vertical(
                text,
                width,
                if key.code == K::PageUp { -page } else { page },
                false,
            );
        }
        K::Left if ctrl || alt => editor.word_left(text),
        K::Right if ctrl || alt => editor.word_right(text),
        K::Char('b') if alt => editor.word_left(text),
        K::Char('f') if alt => editor.word_right(text),
        K::Left if plain => editor.left(text),
        K::Right if plain => editor.right(text),
        K::Char('b') if ctrl => editor.left(text),
        K::Char('f') if ctrl => editor.right(text),
        K::Home if plain || ctrl => editor.home(text),
        K::End if plain || ctrl => editor.end(text),
        K::Char('a') if ctrl => editor.home(text),
        K::Char('e') if ctrl => editor.end(text),
        K::Backspace if plain || shift => editor.backspace(text, false),
        K::Delete if plain || shift => editor.delete(text),
        K::Char('d') if ctrl => editor.delete(text),
        K::Char('w') if ctrl => editor.backspace(text, true),
        K::Backspace if alt => editor.backspace(text, true),
        K::Char('d') | K::Delete if alt => editor.kill(text, false, false),
        K::Char('u') if ctrl => editor.kill(text, true, true),
        K::Char('k') if ctrl => editor.kill(text, false, true),
        K::Char('y') if ctrl => editor.yank(text, false),
        K::Char('y') if alt => editor.yank(text, true),
        K::Char('-') if ctrl => editor.undo(text),
        K::Char('j') if ctrl => editor.type_text(text, "\n"),
        K::Enter if shift => editor.type_text(text, "\n"),
        _ => return false,
    }
    true
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
