use super::*;

#[tokio::test]
async fn room_workspace_navigation_restores_independent_drafts_scroll_and_history() {
    let mut app = app();
    app.state.select_room();
    app.state.insert_room_text("A sent");
    app.post_room_composer();
    app.state.insert_room_text("A draft");
    app.state.room_ui.editor.cursor = 2;
    app.state.room_ui.scroll = 7;
    app.state.switch_workspace(1);
    assert!(!app.state.room_active());
    assert!(app.state.room_ui.composer.is_empty());
    app.state.switch_workspace(0);
    assert!(app.state.room_active());
    assert_eq!(app.state.room_ui.composer, "A draft");
    assert_eq!(app.state.room_ui.editor.cursor, 2);
    assert_eq!(app.state.room_ui.scroll, 7);

    app.state.switch_workspace(1);
    app.state.select_room();
    app.state.insert_room_text("B draft");
    app.state.room_ui.scroll = 3;
    app.state.switch_workspace(0);
    assert!(app.state.room_active());
    assert_eq!(app.state.room_ui.scroll, 7);
    for _ in 0..2 {
        route_room_event(
            &mut app,
            false,
            RawInputEvent::Key(TerminalKey::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .await;
    }
    assert_eq!(app.state.room_ui.composer, "A sent");
    route_room_event(
        &mut app,
        false,
        RawInputEvent::Key(TerminalKey::new(KeyCode::Down, KeyModifiers::NONE)),
    )
    .await;
    assert_eq!(app.state.room_ui.composer, "A draft");
    app.state.switch_workspace(1);
    assert!(app.state.room_active());
    assert_eq!(app.state.room_ui.composer, "B draft");
    assert_eq!(app.state.room_ui.scroll, 3);
    app.state.assert_invariants_for_test();
}

#[tokio::test]
async fn room_explicit_destination_focus_preserves_source_preference() {
    for focus in 0..3 {
        let mut app = app();
        app.state.workspaces.swap(0, 1);
        identify(&mut app, "destination-focus");
        app.state.workspaces.swap(0, 1);
        app.state.switch_workspace(1);
        app.state.select_room();
        app.state.insert_room_text("B draft");
        app.state.switch_workspace(0);
        app.state.select_room();
        app.state.insert_room_text("A draft");
        let pane = app.state.workspaces[1].focused_pane_id().unwrap();
        let pane_id = app.public_pane_id(1, pane).unwrap();
        let method = match focus {
            0 => Method::PaneFocus(PaneTarget { pane_id }),
            1 => Method::AgentFocus(AgentTarget { target: pane_id }),
            _ => Method::TabFocus(TabTarget {
                tab_id: app.public_tab_id(1, 0).unwrap(),
            }),
        };
        result(&mut app, method);
        assert_eq!(app.state.active, Some(1));
        assert!(!app.state.room_active());
        assert!(!app.state.room_ui.visible);
        assert_eq!(app.state.room_ui.composer, "B draft");
        app.state.switch_workspace(0);
        assert!(app.state.room_active());
        assert_eq!(app.state.room_ui.composer, "A draft");
        app.state.switch_workspace(1);
        assert!(!app.state.room_active());
        app.state.select_room();
        assert_eq!(app.state.room_ui.composer, "B draft");
        app.state.assert_invariants_for_test();
    }
}

#[tokio::test]
async fn room_escape_and_toggle_remember_terminal_on_return() {
    for local in [false, true] {
        for key in [
            TerminalKey::new(KeyCode::Esc, KeyModifiers::NONE),
            TerminalKey::new(
                KeyCode::Char('r'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
        ] {
            let mut app = app();
            app.state.select_room();
            app.state.insert_room_text("retained");
            route_room_event(&mut app, local, RawInputEvent::Key(key)).await;
            assert!(!app.state.room_active());
            app.state.switch_workspace(1);
            app.state.switch_workspace(0);
            assert!(!app.state.room_active());
            app.state.select_room();
            assert_eq!(app.state.room_ui.composer, "retained");
        }
    }
}

#[tokio::test]
async fn room_workspace_reorder_close_and_replacement_keep_stable_owners() {
    let mut app = app();
    let a = app.state.workspaces[0].id.clone();
    app.state.select_room();
    app.state.insert_room_text("A draft");
    app.state.switch_workspace(1);
    app.state.select_room();
    app.state.insert_room_text("B draft");
    assert!(app.state.move_workspace(0, 2));
    assert_eq!(app.state.active, Some(0));
    assert_eq!(app.state.room_ui.composer, "B draft");
    app.state.switch_workspace(1);
    assert_eq!(app.state.room_ui.workspace.as_ref(), Some(&a));
    assert_eq!(app.state.room_ui.composer, "A draft");
    app.state.close_selected_workspace();
    assert!(app.state.room_active());
    assert_eq!(app.state.room_ui.composer, "B draft");
    assert!(!app.state.room_presentations.contains_key(&a));
    app.state
        .workspaces
        .push(Workspace::test_new("replacement"));
    assert_ne!(app.state.workspaces[1].id, a);
    app.state.switch_workspace(1);
    assert!(!app.state.room_active());
    assert!(app.state.room_ui.composer.is_empty());
    // Closing a cached background room evicts it too.
    let b = app.state.workspaces[0].id.clone();
    app.state.selected = 0;
    app.state.close_selected_workspace();
    assert!(!app.state.room_presentations.contains_key(&b));
    assert!(!app.state.room_active());
    app.state.selected = 0;
    app.state.close_selected_workspace();
    assert!(app.state.room_ui.workspace.is_none());
    assert!(app.state.room_presentations.is_empty());
}
