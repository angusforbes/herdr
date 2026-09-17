use super::*;
use crossterm::event::{KeyCode as K, KeyModifiers as M};

fn key(app: &mut App, code: K, modifiers: M) {
    app.route_client_events_from(
        42,
        vec![RawInputEvent::Key(TerminalKey::new(code, modifiers))],
        false,
    );
}

#[tokio::test]
async fn room_pi_submit_expands_trims_history_and_undo_reset() {
    let mut app = app();
    app.state.select_room();
    let payload = " x\n".repeat(12);
    app.route_client_events(vec![RawInputEvent::Paste(payload.clone())], false);
    assert!(app.state.room_ui.composer.starts_with("[paste #"));
    key(&mut app, K::Enter, M::CONTROL);
    key(&mut app, K::Enter, M::ALT);
    assert!(app.state.workspaces[0].room.messages.is_empty());
    key(&mut app, K::Enter, M::NONE);
    assert_eq!(
        app.state.workspaces[0].room.messages[0].text,
        payload.trim()
    );
    key(&mut app, K::Char('-'), M::CONTROL);
    assert!(
        app.state.room_ui.composer.is_empty(),
        "successful submit clears undo"
    );
    key(&mut app, K::Up, M::NONE);
    assert_eq!(app.state.room_ui.composer, payload.trim());
    assert_eq!(app.state.room_ui.editor.cursor, 0);
    // Down moves inside the multiline history entry, not to another prompt.
    crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));
    key(&mut app, K::Down, M::NONE);
    assert!(app.state.room_ui.editor.cursor > 0);
    // Editing exits history; Ctrl+J is a real newline, not a send.
    key(&mut app, K::Char('j'), M::CONTROL);
    assert_eq!(app.state.workspaces[0].room.messages.len(), 1);
}

#[tokio::test]
async fn room_pi_backslash_enter_and_failed_send_never_enter_history() {
    let mut app = app();
    app.state.select_room();
    for ch in "first\\".chars() {
        key(&mut app, K::Char(ch), M::NONE);
    }
    key(&mut app, K::Enter, M::NONE);
    assert_eq!(app.state.room_ui.composer, "first\n");
    assert!(app.state.workspaces[0].room.messages.is_empty());
    key(&mut app, K::Enter, M::NONE);
    assert_eq!(app.state.workspaces[0].room.messages[0].text, "first");
    app.state.select_room();
    app.state.insert_room_text("unsent draft");
    key(&mut app, K::Left, M::NONE);
    let cursor = app.state.room_ui.editor.cursor;
    // UI sends always broadcast; use a real persistence failure, not an old
    // recipient-selection guard, to exercise failed-send history semantics.
    app.session_save_thread = Some(std::thread::spawn(|| {
        panic!("injected prior writer failure")
    }));
    key(&mut app, K::Enter, M::NONE);
    assert_eq!(app.state.room_ui.editor.cursor, cursor);
    assert_eq!(app.state.room_ui.composer, "unsent draft");
    key(&mut app, K::Up, M::NONE); // first line -> start
    key(&mut app, K::Up, M::NONE); // newest successful entry, never unsent
    assert_eq!(app.state.room_ui.composer, "first");
    key(&mut app, K::Down, M::NONE);
    assert_eq!(app.state.room_ui.composer, "unsent draft");
    assert_eq!(app.state.room_ui.editor.cursor, 0);
}
