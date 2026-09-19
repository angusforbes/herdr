use super::*;
use crate::input::TerminalKey;
use crossterm::event::{KeyCode as K, KeyModifiers as M};

fn typed(s: &str) -> (Editor, String) {
    let mut e = Editor::default();
    let mut text = String::new();
    for ch in s.chars() {
        e.type_text(&mut text, &ch.to_string());
    }
    (e, text)
}
fn press(e: &mut Editor, text: &mut String, code: K, modifiers: M) -> bool {
    let key = TerminalKey::new(code, modifiers);
    e.jump_key(text, &key) || handle_key(e, text, &key, 79, 7)
}

#[test]
fn room_editor_pi_word_navigation_fixtures() {
    // v0.85.1 packages/tui/test/word-navigation.test.ts.
    for (text, positions) in [
        ("hello world", vec![0, 5, 11]),
        ("foo.bar", vec![0, 3, 4, 7]),
        ("foo:bar", vec![0, 3, 4, 7]),
        ("path/to/file", vec![0, 4, 5, 7, 8, 12]),
        ("foo...bar", vec![0, 3, 6, 9]),
    ] {
        let mut e = Editor::default();
        for &p in &positions[1..] {
            e.word_right(text);
            assert_eq!(e.cursor, p, "{text}");
        }
        // Whitespace is skipped before a backwards word, unlike forward's end.
        if !text.contains(' ') {
            for &p in positions[..positions.len() - 1].iter().rev() {
                e.word_left(text);
                assert_eq!(e.cursor, p, "{text}");
            }
        }
    }
    let (mut e, t) = typed("  hello  \nworld");
    e.word_left(&t);
    assert_eq!(e.cursor, 10);
    e.word_left(&t);
    assert_eq!(e.cursor, 9); // cross only newline
    e.word_left(&t);
    assert_eq!(e.cursor, 2);
    e.word_left(&t);
    assert_eq!(e.cursor, 0);
    e.word_right(&t);
    assert_eq!(e.cursor, 7);
    e.word_right(&t);
    assert_eq!(e.cursor, 9);
    e.word_right(&t);
    assert_eq!(e.cursor, 10);
    // Explicit compatibility gap: UAX Han-per-character, not ICU dictionary words.
    let mut e = Editor::default();
    for pos in [3, 6, 9, 12, 17] {
        e.word_right("你好世界 test");
        assert_eq!(e.cursor, pos);
    }
}

#[test]
fn room_editor_history_restores_exact_draft_and_cursor_and_undo() {
    // Pi editor-history-keybindings.test.ts direct-history cursor fixture.
    let (mut e, mut t) = typed("older prompt");
    e.sent(&mut t);
    e.type_text(&mut t, "newer\nmultiline prompt");
    e.sent(&mut t);
    e.type_text(&mut t, "draft");
    e.left(&t);
    e.left(&t);
    e.history(&mut t, true);
    assert_eq!(t, "newer\nmultiline prompt");
    assert_eq!(e.cursor, 0);
    e.history(&mut t, true);
    assert_eq!(t, "older prompt");
    e.history(&mut t, false);
    assert_eq!(t, "newer\nmultiline prompt");
    assert_eq!(e.cursor, t.len());
    e.history(&mut t, false);
    assert_eq!(t, "draft");
    assert_eq!(e.cursor, 3);
    e.history(&mut t, true);
    e.undo(&mut t);
    assert_eq!(t, "draft");
    assert_eq!(e.cursor, 3);
    assert!(e.history_index.is_none());
}

#[test]
fn room_editor_up_down_history_boundaries_mutation_and_bounds() {
    let (mut e, mut t) = typed(" saved ");
    e.sent(&mut t);
    e.type_text(&mut t, "saved");
    e.sent(&mut t);
    assert_eq!(e.history, ["saved"]);
    e.type_text(&mut t, "draft");
    e.vertical(&mut t, 79, -1, true);
    assert_eq!(t, "draft");
    assert_eq!(e.cursor, 0);
    e.vertical(&mut t, 79, -1, true);
    assert_eq!(t, "saved");
    e.right(&t);
    assert!(e.history_index.is_some());
    e.vertical(&mut t, 79, 1, true);
    assert_eq!(t, "draft");
    assert_eq!(e.cursor, 0);
    e.vertical(&mut t, 79, -1, true);
    e.type_text(&mut t, "!");
    assert!(e.history_index.is_none());
    assert!(e.draft.is_none());
    e.home(&t);
    e.vertical(&mut t, 79, -1, true);
    e.clicked(&t);
    assert!(e.history_index.is_none());
    e.home(&t);
    e.vertical(&mut t, 79, -1, true);
    e.insert(&mut t, "paste");
    assert!(e.history_index.is_none());
    for i in 0..110 {
        t = format!("{i}");
        e.pastes.clear();
        e.sent(&mut t);
    }
    assert_eq!(e.history.len(), 100);
    assert_eq!(e.history[0], "109");
    assert!(e.undo.is_empty());
}

#[test]
fn room_editor_visual_rows_sticky_utf16_and_grapheme_snap() {
    let (mut e, mut t) = typed("abcdefghij\n\nabcdefghij");
    e.vertical(&mut t, 79, -1, true);
    assert_eq!(e.cursor, 11);
    e.vertical(&mut t, 79, -1, true);
    assert_eq!(e.cursor, 10);
    // UTF-16 offset 2, NOT terminal display column 2 (界 occupies two cells).
    let (mut e, mut t) = typed("a界z\n012345");
    e.cursor = "a界".len();
    e.vertical(&mut t, 79, 1, true);
    assert_eq!(&t[e.cursor..], "2345");
    let (mut e, mut t) = typed("abc\n👩‍💻z\nabc");
    e.cursor = 2;
    e.vertical(&mut t, 79, 1, true);
    assert_eq!(e.cursor, 4);
    e.vertical(&mut t, 79, 1, true);
    assert_eq!(&t[e.cursor..], "c");
    let (mut e, mut t) = typed("abcdefghijklmnopqr\n123456789012345678");
    e.cursor = 18;
    e.vertical(&mut t, 9, 1, true);
    assert_eq!(e.cursor, 19 + 8);
    e.vertical(&mut t, 79, -1, true);
    assert_eq!(e.cursor, 8);
    e.vertical(&mut t, 79, 1, true);
    assert_eq!(e.cursor, 19 + 8);
}

#[test]
fn room_editor_wrapping_retains_whitespace_and_exact_width() {
    let t = "Word1 Word2 Word3 Word4";
    let r = rows(t, 15);
    assert_eq!(
        r.iter().map(|r| &t[r.start..r.end]).collect::<Vec<_>>(),
        ["Word1 Word2 ", "Word3 Word4"]
    );
    assert_eq!(
        rows("abcd\nx", 4),
        [Row { start: 0, end: 4 }, Row { start: 5, end: 6 }]
    );
    let mut e = Editor {
        cursor: 4,
        ..Default::default()
    };
    assert_eq!(
        e.project("abcd\nx", &rows("abcd\nx", 4), Rect::new(0, 0, 5, 2)),
        Some(Position::new(4, 0))
    );
    for width in 1..15 {
        let text = "abc 界e\u{301} 👩‍💻xyz     next";
        let r = rows(text, width);
        assert_eq!(
            r.iter().map(|r| &text[r.start..r.end]).collect::<String>(),
            text
        );
        for row in &r {
            assert!(
                text[row.start..row.end].width() <= width
                    || text[row.start..row.end].graphemes(true).count() == 1
            );
        }
    }
    assert_eq!(byte_at_column("a界e\u{301}", 2), 1);
}

#[test]
fn room_editor_pi_aliases_and_unbound_modified_enter() {
    let aliases = [
        (K::Left, M::NONE, K::Char('b'), M::CONTROL),
        (K::Right, M::NONE, K::Char('f'), M::CONTROL),
        (K::Home, M::CONTROL, K::Char('a'), M::CONTROL),
        (K::End, M::CONTROL, K::Char('e'), M::CONTROL),
        (K::Left, M::ALT, K::Char('b'), M::ALT),
        (K::Right, M::CONTROL, K::Char('f'), M::ALT),
        (K::Backspace, M::NONE, K::Backspace, M::SHIFT),
        (K::Delete, M::SHIFT, K::Char('d'), M::CONTROL),
        (K::Char('w'), M::CONTROL, K::Backspace, M::ALT),
        (K::Delete, M::ALT, K::Char('d'), M::ALT),
        (K::Enter, M::SHIFT, K::Char('j'), M::CONTROL),
    ];
    for (a, am, b, bm) in aliases {
        let (mut e, mut t) = typed("first\na界e\u{301} two words\nlast");
        let (mut f, mut u) = typed(&t);
        e.cursor = "first\na界e\u{301} two".len();
        f.cursor = e.cursor;
        assert!(press(&mut e, &mut t, a, am));
        assert!(press(&mut f, &mut u, b, bm));
        assert_eq!((t, e.cursor), (u, f.cursor));
    }
    let (mut e, mut t) = typed("first\nsecond");
    press(&mut e, &mut t, K::Home, M::CONTROL);
    assert_eq!(e.cursor, 6);
    press(&mut e, &mut t, K::End, M::CONTROL);
    assert_eq!(e.cursor, t.len());
    for (k, m) in [
        (K::Enter, M::CONTROL),
        (K::Enter, M::ALT),
        (K::Char('z'), M::CONTROL),
        (K::Backspace, M::CONTROL),
    ] {
        assert!(!press(&mut e, &mut t, k, m));
    }
    press(&mut e, &mut t, K::Char(']'), M::CONTROL | M::ALT);
    press(&mut e, &mut t, K::Char('s'), M::NONE);
    assert_eq!(e.cursor, 6);
    press(&mut e, &mut t, K::Char(']'), M::CONTROL);
    press(&mut e, &mut t, K::Char('d'), M::NONE);
    assert_eq!(e.cursor, 11);
}

#[test]
fn room_editor_kill_ring_direction_newlines_yank_pop_and_submit() {
    let (mut e, mut t) = typed("one two\nthree");
    e.kill(&mut t, true, false);
    e.kill(&mut t, true, true);
    e.kill(&mut t, true, false);
    assert_eq!(t, "one ");
    assert_eq!(e.ring, ["two\nthree"]);
    e.home(&t);
    e.kill(&mut t, false, true);
    assert_eq!(e.ring, ["two\nthree", "one "]);
    e.yank(&mut t, false);
    assert_eq!(t, "one ");
    e.yank(&mut t, true);
    assert_eq!(t, "two\nthree");
    e.yank(&mut t, true);
    assert_eq!(t, "one ");
    e.left(&t);
    e.yank(&mut t, true);
    assert_eq!(t, "one ");
    e.sent(&mut t);
    assert!(e.undo.is_empty());
    assert_eq!(e.ring.len(), 2);
    e.yank(&mut t, false);
    assert_eq!(t, "one ");
    e.backspace(&mut t, false);
    assert_eq!(e.ring.len(), 2);
    let (mut e, mut t) = typed("a\nb");
    e.cursor = 1;
    e.kill(&mut t, false, true);
    e.kill(&mut t, false, true);
    assert_eq!(t, "a");
    assert_eq!(e.ring, ["\nb"]);
}

#[test]
fn room_editor_typing_undo_coalesces_whitespace_movement_and_paste() {
    let (mut e, mut t) = typed("hello world");
    e.undo(&mut t);
    assert_eq!(t, "hello");
    e.undo(&mut t);
    assert_eq!(t, "");
    e.type_text(&mut t, "abc");
    e.left(&t);
    e.type_text(&mut t, "!");
    e.undo(&mut t);
    assert_eq!(t, "abc");
    assert_eq!(e.cursor, 2);
    e.insert(&mut t, "X\r\nY\t");
    e.undo(&mut t);
    assert_eq!(t, "abc");
    assert_eq!(e.cursor, 2);
}

#[test]
fn room_editor_paste_registry_atomicity_undo_and_expanded_limit() {
    let (mut e, mut t) = typed("A");
    let payload = "line\n".repeat(20);
    e.insert(&mut t, &payload);
    e.type_text(&mut t, "B");
    assert!(t.contains("[paste #1 +21 lines]"));
    e.home(&t);
    e.right(&t);
    assert_eq!(e.cursor, 1);
    e.right(&t);
    assert_eq!(e.cursor, t.len() - 1);
    e.backspace(&mut t, false);
    assert_eq!(t, "AB");
    assert!(e.pastes.is_empty());
    e.undo(&mut t);
    assert_eq!(e.expanded(&t), format!("A{payload}B"));
    e.word_left(&t);
    assert_eq!(e.cursor, 1);
    e.delete(&mut t);
    assert_eq!(t, "AB");
    e.undo(&mut t);
    e.kill(&mut t, true, true);
    e.yank(&mut t, false);
    assert!(e.expanded(&t).contains(&payload));
    let (mut e, mut t) = typed("");
    e.insert(&mut t, &"x".repeat(LIMIT - 2));
    assert!(t.starts_with("[paste #"));
    e.home(&t);
    e.insert(&mut t, "e\u{301}z");
    assert_eq!(e.cursor, 0);
    e.insert(&mut t, "é");
    assert_eq!(e.expanded(&t).len(), LIMIT);
    e.type_text(&mut t, "z");
    assert_eq!(e.expanded(&t).len(), LIMIT);
    e.sent(&mut t);
    assert_eq!(e.history[0].len(), LIMIT);
    // Typed lookalike remains literal even when it shares a live marker ID.
    let fake = "[paste #1 1001 chars]";
    e.type_text(&mut t, fake);
    e.insert(&mut t, &"x".repeat(1001));
    assert_eq!(e.expanded(&t), format!("{fake}{}", "x".repeat(1001)));
    e.home(&t);
    e.right(&t);
    assert_eq!(e.cursor, 1);
}

#[test]
fn room_editor_paste_normalizes_paths_utf16_threshold_and_narrow_markers() {
    let (mut e, mut t) = typed("word");
    e.insert(&mut t, "/tmp/a\r\nb\rc\t\x1b\0");
    assert_eq!(t, "word /tmp/a\nb\nc    ");
    let (mut e, mut t) = typed("");
    e.insert(&mut t, &"😀".repeat(501));
    assert_eq!(t, "[paste #1 1002 chars]");
    for width in 1..25 {
        let r = e.rows(&t, width);
        assert_eq!(r.iter().map(|r| &t[r.start..r.end]).collect::<String>(), t);
        for row in &r {
            assert!(t[row.start..row.end].width() <= width);
        }
        e.cursor = 0;
        e.vertical(&mut t, width, 1, false);
        assert!(e.cursor == 0 || e.cursor == t.len());
        e.project(&t, &r, Rect::new(0, 0, (width + 1) as u16, 2));
    }
}
