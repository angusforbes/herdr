//! Selection uses cached room display rows and UTF-8 boundaries, never PTYs.
use ratatui::layout::{Position, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub row: usize,
    pub byte: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub anchor: Point,
    pub end: Point,
    pub dragging: bool,
}

impl Selection {
    pub fn bounds(&self) -> (Point, Point) {
        (self.anchor.min(self.end), self.anchor.max(self.end))
    }

    pub fn range(&self, row: usize, len: usize) -> std::ops::Range<usize> {
        let (start, end) = self.bounds();
        if row < start.row || row > end.row {
            return 0..0;
        }
        let from = if row == start.row {
            start.byte.min(len)
        } else {
            0
        };
        let to = if row == end.row {
            end.byte.min(len)
        } else {
            len
        };
        from..to
    }

    /// Soft-wrap newlines are intentionally preserved; chrome is never copied.
    pub fn text(&self, lines: &[String]) -> Option<String> {
        let (start, end) = self.bounds();
        if start == end || end.row >= lines.len() {
            return None;
        }
        Some(
            (start.row..=end.row)
                .map(|row| {
                    let line = &lines[row];
                    &line[self.range(row, line.len())]
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

pub fn hit(lines: &[String], begin: usize, area: Rect, position: Position) -> Option<Point> {
    if lines.is_empty() || area.is_empty() {
        return None;
    }
    let row = (begin + position.y.saturating_sub(area.y).min(area.height - 1) as usize)
        .min(lines.len() - 1);
    let column = position.x.saturating_sub(area.x).min(area.width) as usize;
    Some(Point {
        row,
        byte: super::editor::byte_at_column(&lines[row], column),
    })
}
