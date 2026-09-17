use super::*;
use ratatui::{backend::TestBackend, layout::Position, style::Modifier, Terminal};

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.route_client_events_from(
        42,
        vec![RawInputEvent::Key(TerminalKey::new(code, modifiers))],
        false,
    );
}

fn geometry(app: &mut App) {
    crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));
}

fn mouse(app: &mut App, kind: MouseEventKind, position: Position) {
    app.route_client_events_from(
        42,
        vec![RawInputEvent::Mouse(MouseEvent {
            kind,
            column: position.x,
            row: position.y,
            modifiers: KeyModifiers::NONE,
        })],
        false,
    );
}

#[tokio::test]
async fn room_text_cursor_keyboard_paste_and_send_through_app() {
    for local in [false, true] {
        let mut app = app();
        app.state.select_room();
        route_room_event(
            &mut app,
            local,
            RawInputEvent::Paste("a界e\u{301}\r\nlast\rend\t\0".into()),
        )
        .await;
        assert_eq!(app.state.room_ui.composer, "a界e\u{301}\nlast\nend    ");
        let press = |code, modifiers| RawInputEvent::Key(TerminalKey::new(code, modifiers));
        for event in [
            press(KeyCode::Home, KeyModifiers::CONTROL),
            press(KeyCode::Right, KeyModifiers::NONE),
            press(KeyCode::Delete, KeyModifiers::NONE),
            RawInputEvent::Text(crate::input::TextCommit::new("語")),
            press(KeyCode::Right, KeyModifiers::NONE),
            press(KeyCode::Backspace, KeyModifiers::NONE),
            press(KeyCode::Enter, KeyModifiers::SHIFT),
        ] {
            route_room_event(&mut app, local, event).await;
        }
        assert_eq!(app.state.room_ui.composer, "a界e\u{301}\nlast\ne語\n    ");
        assert_eq!(
            app.state.room_ui.editor.cursor,
            "a界e\u{301}\nlast\ne語\n".len()
        );
        route_room_event(&mut app, local, press(KeyCode::End, KeyModifiers::CONTROL)).await;
        route_room_event(
            &mut app,
            local,
            press(KeyCode::Char('w'), KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(app.state.room_ui.composer, "a界e\u{301}\nlast\ne語\n");
        route_room_event(&mut app, local, press(KeyCode::Home, KeyModifiers::NONE)).await;
        route_room_event(&mut app, local, press(KeyCode::Left, KeyModifiers::CONTROL)).await;
        assert_eq!(
            &app.state.room_ui.composer[app.state.room_ui.editor.cursor..],
            "\n"
        );
        route_room_event(
            &mut app,
            local,
            press(KeyCode::Right, KeyModifiers::CONTROL),
        )
        .await;
        assert_eq!(
            app.state.room_ui.editor.cursor,
            app.state.room_ui.composer.len()
        );
        route_room_event(&mut app, local, press(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert_eq!(app.state.workspaces[0].room.messages.len(), 1);
        assert!(app.state.room_ui.composer.is_empty());
        assert_eq!(app.state.room_ui.editor.cursor, 0);
        assert_eq!(app.state.room_ui.editor.top, 0);
        geometry(&mut app);
        assert_eq!(
            app.state.room_ui.composer_cursor,
            Some(app.state.room_ui.composer_area.as_position())
        );
    }
}

#[tokio::test]
async fn room_app_paste_cap_is_grapheme_safe_at_insertion_cursor() {
    let mut app = app();
    app.state.select_room();
    app.route_client_events(
        vec![RawInputEvent::Paste(
            "x".repeat(crate::room::MAX_MESSAGE_BYTES - 2),
        )],
        false,
    );
    key(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
    app.route_client_events(vec![RawInputEvent::Paste("e\u{301}界".into())], false);
    assert_eq!(
        app.state
            .room_ui
            .editor
            .expanded(&app.state.room_ui.composer)
            .len(),
        crate::room::MAX_MESSAGE_BYTES - 2
    );
    assert_eq!(app.state.room_ui.editor.cursor, 0);
    app.handle_paste("é".into()).await;
    assert_eq!(
        app.state
            .room_ui
            .editor
            .expanded(&app.state.room_ui.composer)
            .len(),
        crate::room::MAX_MESSAGE_BYTES
    );
    assert_eq!(app.state.room_ui.editor.cursor, 2);
    assert!(app
        .state
        .room_ui
        .editor
        .expanded(&app.state.room_ui.composer)
        .starts_with("éx"));
    app.handle_text_commit("👩‍💻".into()).await;
    assert_eq!(
        app.state
            .room_ui
            .editor
            .expanded(&app.state.room_ui.composer)
            .len(),
        crate::room::MAX_MESSAGE_BYTES
    );
}

#[tokio::test]
async fn room_copy_preserves_wrap_newlines_and_excludes_chrome_and_controls() {
    let mut app = app();
    app.state.select_room();
    geometry(&mut app);
    let width = app.state.room_ui.transcript_area.width as usize;
    app.state.workspaces[0]
        .room
        .post(format!("{}界e\u{301}\x1b\0", "x".repeat(width)), None, 0)
        .unwrap();
    geometry(&mut app);
    let area = app.state.room_ui.transcript_area;
    let row = app
        .state
        .room_ui
        .transcript_lines
        .iter()
        .position(|s| s == &"x".repeat(width))
        .unwrap();
    let start = Position::new(area.x, area.y + row as u16);
    let end = Position::new(area.right() + 10, area.bottom() + 10);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), start);
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), end);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), end);
    key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    match app.event_rx.try_recv().unwrap() {
        crate::events::AppEvent::ClipboardWrite { content } => assert_eq!(
            content,
            format!("{}\n界e\u{301}\n", "x".repeat(width)).as_bytes()
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn room_failed_send_preserves_cursor_and_multiline_draft() {
    let mut app = app();
    app.state.select_room();
    app.state.insert_room_text("hello\nworld");
    key(&mut app, KeyCode::Home, KeyModifiers::NONE);
    let cursor = app.state.room_ui.editor.cursor;
    // Exercise the actual composer -> API -> persistence failure path without
    // filesystem writes or any global session configuration.
    app.session_save_thread = Some(std::thread::spawn(|| {
        panic!("injected prior writer failure")
    }));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app
        .state
        .room_ui
        .status
        .contains("previous session writer panicked"));
    assert_eq!(app.state.room_ui.composer, "hello\nworld");
    assert_eq!(app.state.room_ui.editor.cursor, cursor);
    assert!(app.state.workspaces[0].room.messages.is_empty());
}

#[tokio::test]
async fn room_selection_reversed_unicode_highlight_clipboard_and_isolation() {
    for (reverse, auto_copy) in [(false, true), (true, true), (false, false), (true, false)] {
        let mut app = app();
        assert!(app.state.copy_on_select);
        app.state.copy_on_select = auto_copy;
        let id = app.state.workspaces[0]
            .terminal_id(app.state.workspaces[0].focused_pane_id().unwrap())
            .unwrap()
            .clone();
        let (runtime, mut pty) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.terminal_runtimes.insert(id, runtime);
        app.state.workspaces[0]
            .room
            .post("a界e\u{301}Z\nsecond 👩‍💻!".into(), None, 0)
            .unwrap();
        app.state.select_room();
        geometry(&mut app);
        let area = app.state.room_ui.transcript_area;
        let row = app
            .state
            .room_ui
            .transcript_lines
            .iter()
            .position(|s| s == "a界e\u{301}Z")
            .unwrap() as u16;
        let start = Position::new(area.x + 2, area.y + row); // second cell of 界 snaps to whole glyph
        let end = Position::new(area.x + 7, area.y + row + 1);
        let (a, b) = if reverse { (end, start) } else { (start, end) };
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), a);
        mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), b);
        assert!(app.event_rx.try_recv().is_err(), "drag alone must not copy");
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), b);
        if auto_copy {
            match app
                .event_rx
                .try_recv()
                .expect("release auto-copies by default")
            {
                crate::events::AppEvent::ClipboardWrite { content } => {
                    assert_eq!(content, "界e\u{301}Z\nsecond ".as_bytes())
                }
                other => panic!("{other:?}"),
            }
        } else {
            assert!(
                app.event_rx.try_recv().is_err(),
                "disabled auto-copy retains selection only"
            );
        }
        let selection = app.state.room_ui.selection.unwrap();
        assert!(!selection.dragging);
        assert_eq!(
            selection
                .text(&app.state.room_ui.transcript_lines)
                .as_deref(),
            Some("界e\u{301}Z\nsecond ")
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(&app.state, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert!(buffer[(area.x + 1, area.y + row)]
            .modifier
            .contains(Modifier::REVERSED));
        assert!(buffer[(area.x + 3, area.y + row)]
            .modifier
            .contains(Modifier::REVERSED));
        assert!(!buffer[(area.x, area.y + row)]
            .modifier
            .contains(Modifier::REVERSED));
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            key(&mut app, KeyCode::Char('c'), modifiers);
            match app.event_rx.try_recv().unwrap() {
                crate::events::AppEvent::ClipboardWrite { content } => {
                    assert_eq!(content, "界e\u{301}Z\nsecond ".as_bytes())
                }
                event => panic!("unexpected {event:?}"),
            }
            assert!(app.state.request_clipboard_write.is_none());
        }
        // A ZWJ emoji is one selectable grapheme, not several scalar values.
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), end);
        mouse(
            &mut app,
            MouseEventKind::Up(MouseButton::Left),
            Position::new(end.x + 2, end.y),
        );
        if !auto_copy {
            key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        }
        match app.event_rx.try_recv().unwrap() {
            crate::events::AppEvent::ClipboardWrite { content } => {
                assert_eq!(content, "👩‍💻".as_bytes())
            }
            other => panic!("{other:?}"),
        }
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), start);
        mouse(&mut app, MouseEventKind::Up(MouseButton::Left), start);
        key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(
            app.event_rx.try_recv().is_err(),
            "plain click resets selection"
        );
        assert!(
            pty.try_recv().is_err(),
            "copy must never interrupt hidden PTY"
        );
        assert!(app.state.selection.is_none());
        assert!(app.state.tab_presses.is_empty());
    }
}

#[tokio::test]
async fn room_selection_scroll_clamp_resize_and_chrome_ownership() {
    let mut app = app();
    app.state.workspaces[0]
        .room
        .post((0..100).map(|i| format!("line {i}\n")).collect(), None, 0)
        .unwrap();
    app.state.select_room();
    app.state.insert_room_text("draft");
    geometry(&mut app);
    let area = app.state.room_ui.transcript_area;
    let start = area.as_position();
    let end = Position::new(area.x + 6, area.y + 1);
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), start);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), end);
    let copied = app
        .state
        .room_ui
        .selection
        .unwrap()
        .text(&app.state.room_ui.transcript_lines);
    let cursor = app.state.room_ui.editor.cursor;
    key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    geometry(&mut app);
    assert_eq!(app.state.room_ui.scroll, 0);
    assert_eq!(app.state.room_ui.editor.cursor, 0);
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    geometry(&mut app);
    assert_eq!(app.state.room_ui.scroll, 0);
    assert_eq!(app.state.room_ui.editor.cursor, cursor);
    mouse(&mut app, MouseEventKind::ScrollUp, start);
    geometry(&mut app);
    assert_eq!(app.state.room_ui.scroll, 3);
    assert_eq!(
        app.state
            .room_ui
            .selection
            .unwrap()
            .text(&app.state.room_ui.transcript_lines),
        copied
    );
    // Drag and release on chrome are owned by the room and clamped, never tabs.
    mouse(&mut app, MouseEventKind::Down(MouseButton::Left), start);
    let chrome = app.state.view.tab_hit_areas[0].as_position();
    mouse(&mut app, MouseEventKind::Drag(MouseButton::Left), chrome);
    mouse(&mut app, MouseEventKind::Up(MouseButton::Left), chrome);
    assert!(app.state.room_active());
    assert!(!app.state.room_ui.selection.unwrap().dragging);
    assert!(app.state.tab_presses.is_empty());
    crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 100, 30));
    assert!(app.state.room_ui.selection.is_none());
    // Closing safely hides caret; reopening never revives a selection gesture.
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    geometry(&mut app);
    assert!(app.state.room_ui.composer_cursor.is_none());
    app.state.select_room();
    assert!(app.state.room_ui.selection.is_none());
}

#[tokio::test]
async fn room_composer_mouse_caret_render_and_bounded_multiline_view() {
    let mut app = app();
    app.state.select_room();
    app.state.insert_room_text("a界e\u{301}Z");
    geometry(&mut app);
    let area = app.state.room_ui.composer_area;
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        Position::new(area.x + 2, area.y),
    );
    assert_eq!(app.state.room_ui.editor.cursor, 1);
    key(&mut app, KeyCode::Char('!'), KeyModifiers::NONE);
    assert_eq!(app.state.room_ui.composer, "a!界e\u{301}Z");
    geometry(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| crate::ui::render(&app.state, frame))
        .unwrap();
    terminal
        .backend_mut()
        .assert_cursor_position((area.x + 2, area.y));
    let cursor = crate::ui::tab_surface_cursor(
        &app.state,
        &app.terminal_runtimes,
        app.state.view.tab_surface(),
    )
    .unwrap();
    assert_eq!((cursor.x, cursor.y, cursor.shape), (area.x + 2, area.y, 6));
    assert!(
        !cursor.visible,
        "hardware cursor hidden; position retained for IME"
    );
    assert!(terminal.backend().buffer()[(area.x + 2, area.y)]
        .modifier
        .contains(Modifier::REVERSED));
    key(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    app.state.room_ui.editor.type_text(
        &mut app.state.room_ui.composer,
        "\n1\n2\n3\n4\n5\n6\n7\n8\n9",
    );
    geometry(&mut app);
    assert_eq!(app.state.room_ui.composer_area.height, 9);
    assert!(app.state.room_ui.editor.top > 0);
    let area = app.state.room_ui.composer_area;
    assert_eq!(
        app.state.room_ui.composer_cursor,
        Some(Position::new(area.x + 1, area.bottom() - 1))
    );
    key(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
    for _ in 0..9 {
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    }
    geometry(&mut app);
    assert_eq!(app.state.room_ui.editor.top, 0);
    assert_eq!(app.state.room_ui.composer_cursor, Some(area.as_position()));
    // Multiple batched edits then click cannot slice stale geometry.
    for _ in 0..30 {
        key(&mut app, KeyCode::Delete, KeyModifiers::NONE);
    }
    mouse(
        &mut app,
        MouseEventKind::Down(MouseButton::Left),
        Position::new(area.x + 2, area.y + 4),
    );
    geometry(&mut app);
    assert!(app.state.room_ui.editor.cursor <= app.state.room_ui.composer.len());
    app.state.insert_room_text("界e\u{301}👩‍💻\nend");
    for (width, height) in [(1, 1), (4, 5), (20, 10), (80, 20)] {
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, width, height));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(&app.state, frame))
            .unwrap();
        if let Some(cursor) = app.state.room_ui.composer_cursor {
            assert!(app.state.room_ui.composer_area.contains(cursor));
        }
    }
}
