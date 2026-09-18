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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::room::TranscriptRole::*;

    #[test]
    fn room_padding_selection_maps_each_role_without_copying_insets() {
        let lines: Vec<String> = ["", "界hi", "", "", "Aporia", "agent"]
            .map(String::from)
            .into();
        let roles = [Human, Human, Human, Spacer, AgentHeader, Agent];
        let area = Rect::new(10, 10, 20, 6);
        let start = hit(&lines, &roles, 0, area, Position::new(11, 11)).unwrap();
        assert_eq!(start, Point { row: 1, byte: 0 });
        assert_eq!(
            hit(&lines, &roles, 0, area, Position::new(12, 11)).unwrap(),
            start
        );
        let end = hit(&lines, &roles, 0, area, Position::new(13, 15)).unwrap();
        for (anchor, end) in [(start, end), (end, start)] {
            let selection = Selection {
                anchor,
                end,
                dragging: false,
            };
            assert_eq!(selection.text(&lines).unwrap(), "界hi\n\n\nAporia\nage");
        }
        assert_eq!(
            hit(&lines, &roles, 4, area, Position::new(10, 10)).unwrap(),
            Point { row: 4, byte: 0 }
        );
        assert_eq!(
            hit(&lines, &roles, 4, area, Position::new(13, 11)).unwrap(),
            Point { row: 5, byte: 3 }
        );
    }
}

pub fn hit(
    lines: &[String],
    roles: &[super::TranscriptRole],
    begin: usize,
    area: Rect,
    position: Position,
) -> Option<Point> {
    if lines.is_empty() || area.is_empty() {
        return None;
    }
    let row = (begin + position.y.saturating_sub(area.y).min(area.height - 1) as usize)
        .min(lines.len() - 1);
    let inset = roles.get(row).map_or(0, |role| role.inset(area.width));
    let column = (position.x.saturating_sub(area.x).min(area.width) as usize).saturating_sub(inset);
    Some(Point {
        row,
        byte: super::editor::byte_at_column(&lines[row], column),
    })
}
