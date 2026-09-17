//! Room-local UTF-8 editor and display projection; no terminal/pane identity.
use ratatui::layout::{Position, Rect};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub struct Editor {
    /// Byte offset at an extended grapheme boundary.
    pub cursor: usize,
    pub top: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
}

/// Hard-wrap without trimming spaces. The final insertion position always has
/// its own cell, including at an exact-width line ending.
pub fn rows(text: &str, width: usize) -> Vec<Row> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let (mut start, mut columns) = (0, 0);
    for (byte, grapheme) in text.grapheme_indices(true) {
        if grapheme == "\n" {
            rows.push(Row { start, end: byte });
            if columns >= width {
                rows.push(Row {
                    start: byte,
                    end: byte,
                });
            }
            start = byte + 1;
            columns = 0;
            continue;
        }
        let size = grapheme.width();
        if columns + size > width && byte > start {
            rows.push(Row { start, end: byte });
            start = byte;
            columns = 0;
        }
        columns += size;
    }
    rows.push(Row {
        start,
        end: text.len(),
    });
    if columns >= width {
        rows.push(Row {
            start: text.len(),
            end: text.len(),
        });
    }
    rows
}

/// Hit a cell boundary, snapping both halves of a wide glyph to its start.
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
    pub fn normalize(&mut self, text: &str) {
        self.cursor = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .take_while(|i| *i <= self.cursor.min(text.len()))
            .last()
            .unwrap_or(0);
    }

    pub fn insert(&mut self, text: &mut String, input: &str) {
        self.normalize(text);
        let safe: String = input
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        let remaining = crate::room::MAX_MESSAGE_BYTES.saturating_sub(text.len());
        let mut end = 0;
        for g in safe.graphemes(true) {
            if end + g.len() > remaining {
                break;
            }
            end += g.len();
        }
        text.insert_str(self.cursor, &safe[..end]);
        self.cursor += end;
        // Insertion can merge a combining mark / ZWJ with the following glyph.
        self.cursor = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .find(|i| *i >= self.cursor)
            .unwrap_or(text.len());
    }

    pub fn left(&mut self, text: &str) {
        self.normalize(text);
        self.cursor = text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    pub fn right(&mut self, text: &str) {
        self.normalize(text);
        self.cursor += text[self.cursor..]
            .graphemes(true)
            .next()
            .map(str::len)
            .unwrap_or(0);
    }

    /// Words are non-whitespace runs (Pi-style Ctrl+W semantics).
    pub fn word_left(&mut self, text: &str) {
        self.normalize(text);
        let mut seen_word = false;
        for (i, g) in text[..self.cursor].grapheme_indices(true).rev() {
            let space = g.chars().all(char::is_whitespace);
            if seen_word && space {
                break;
            }
            seen_word |= !space;
            self.cursor = i;
        }
    }

    pub fn word_right(&mut self, text: &str) {
        self.normalize(text);
        let mut seen_space = false;
        let start = self.cursor;
        for (i, g) in text[start..].grapheme_indices(true) {
            let space = g.chars().all(char::is_whitespace);
            if seen_space && !space {
                break;
            }
            seen_space |= space;
            self.cursor = start + i + g.len();
        }
    }

    pub fn home(&mut self, text: &str, whole: bool) {
        self.normalize(text);
        self.cursor = if whole {
            0
        } else {
            text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0)
        };
    }

    pub fn end(&mut self, text: &str, whole: bool) {
        self.normalize(text);
        self.cursor = if whole {
            text.len()
        } else {
            self.cursor
                + text[self.cursor..]
                    .find('\n')
                    .unwrap_or(text.len() - self.cursor)
        };
    }

    pub fn backspace(&mut self, text: &mut String, word: bool) {
        self.normalize(text);
        let end = self.cursor;
        if word {
            self.word_left(text);
        } else {
            self.left(text);
        }
        text.replace_range(self.cursor..end, "");
        self.normalize(text);
    }

    pub fn delete(&mut self, text: &mut String) {
        self.normalize(text);
        let start = self.cursor;
        self.right(text);
        text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.normalize(text);
    }

    pub fn project(&mut self, text: &str, rows: &[Row], area: Rect) -> Option<Position> {
        self.normalize(text);
        if area.is_empty() {
            return None;
        }
        let row = rows
            .iter()
            .rposition(|r| r.start <= self.cursor)
            .unwrap_or(0);
        if row < self.top {
            self.top = row;
        }
        if row >= self.top + area.height as usize {
            self.top = row + 1 - area.height as usize;
        }
        let x = text[rows[row].start..self.cursor]
            .width()
            .min(area.width.saturating_sub(1) as usize);
        Some(Position::new(
            area.x + x as u16,
            area.y + (row - self.top) as u16,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_editor_graphemes_words_and_lines() {
        let mut editor = Editor::default();
        let mut text = String::new();
        editor.insert(&mut text, "a界e\u{301} 👩‍💻\nlast word");
        editor.word_left(&text);
        editor.backspace(&mut text, true);
        assert_eq!(text, "a界e\u{301} 👩‍💻\nword");
        editor.home(&text, false);
        editor.backspace(&mut text, false);
        editor.backspace(&mut text, false);
        assert_eq!(text, "a界e\u{301} word");
        editor.home(&text, true);
        editor.right(&text);
        editor.delete(&mut text);
        assert_eq!(text, "ae\u{301} word");
        editor.right(&text);
        editor.backspace(&mut text, false);
        assert_eq!(text, "a word");
        editor.insert(&mut text, "界");
        assert_eq!(text, "a界 word");
        editor.end(&text, true);
        editor.word_left(&text);
        assert_eq!(&text[editor.cursor..], "word");
    }

    #[test]
    fn room_editor_paste_normalizes_and_caps_complete_graphemes() {
        let mut editor = Editor::default();
        let mut text = String::new();
        editor.insert(&mut text, "a\r\nb\rc\n\t\x1b\0d");
        assert_eq!(text, "a\nb\nc\nd");
        text = "x".repeat(crate::room::MAX_MESSAGE_BYTES - 2);
        editor.end(&text, true);
        editor.insert(&mut text, "e\u{301}z");
        assert_eq!(text.len(), crate::room::MAX_MESSAGE_BYTES - 2);
        editor.insert(&mut text, "é");
        assert_eq!(text.len(), crate::room::MAX_MESSAGE_BYTES);
        editor.insert(&mut text, "z");
        assert_eq!(editor.cursor, text.len());
    }

    #[test]
    fn room_editor_wrap_projection_and_wide_hit() {
        let text = "a界e\u{301}\nabcdef";
        let rows = rows(text, 4);
        assert_eq!(&text[rows[0].start..rows[0].end], "a界e\u{301}");
        assert_eq!(byte_at_column("a界e\u{301}", 2), 1);
        assert_eq!(byte_at_column("a界e\u{301}", 3), 4);
        let mut editor = Editor {
            cursor: text.len(),
            top: 0,
        };
        assert_eq!(
            editor.project(text, &rows, Rect::new(2, 3, 4, 1)),
            Some(Position::new(4, 3))
        );
        editor.home(text, true);
        assert_eq!(
            editor.project(text, &rows, Rect::new(2, 3, 4, 1)),
            Some(Position::new(2, 3))
        );
        assert_eq!(editor.top, 0);
        let text = "abcd\nx";
        let rows = super::rows(text, 4);
        editor.cursor = 4; // End on an exactly full logical line
        assert_eq!(
            editor.project(text, &rows, Rect::new(0, 0, 4, 3)),
            Some(Position::new(0, 1))
        );
    }
}
