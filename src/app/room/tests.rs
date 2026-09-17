#[path = "delivery_tests.rs"]
mod delivery_tests;
#[path = "parity_tests.rs"]
mod parity_tests;
#[path = "ux_tests.rs"]
mod ux_tests;
#[path = "workspace_tests.rs"]
mod workspace_tests;
use super::*;
use crate::{api::schema::*, raw_input::RawInputEvent, workspace::Workspace};
use ratatui::layout::Rect;

#[test]
fn room_summary_keeps_earlier_pending_questions_visible() {
    use crate::room_delivery::{DeliveryStatus, Status};
    let member = crate::room::Member {
        pane_id: "w1:p1".into(),
        terminal_id: "t1".into(),
        agent: "pi".into(),
        name: "Ada".into(),
        session: Some("Id:live".into()),
    };
    let make = |sequence, status| DeliveryStatus {
        delivery_id: format!("d{sequence}"),
        request_sequence: sequence,
        recipient: member.clone(),
        status,
        detail: None,
    };
    let deliveries = [
        make(1, Status::Queued),
        make(1, Status::Submitted),
        make(1, Status::Replied),
        make(2, Status::Queued),
    ];
    let summary = delivery_summary(&deliveries);
    assert!(summary.starts_with("Earlier requests: 2 pending"));
    assert!(summary.contains("#2: 1 queued"));
    assert!(!delivery_summary(&deliveries[3..]).contains("Earlier requests"));
}

#[test]
fn room_member_name_uses_sidebar_metadata_before_managed_name() {
    let mut app = app();
    identify(&mut app, "named");
    let initial = crate::room::members(&app.state, 0).remove(0);
    assert_eq!(initial.name, initial.pane_id);
    let id = app.state.workspaces[0].tabs[0]
        .panes
        .values()
        .next()
        .unwrap()
        .attached_terminal_id
        .clone();
    let terminal = app.state.terminals.get_mut(&id).unwrap();
    terminal.set_agent_name("managed-name".into());
    terminal.metadata_tokens.patch(
        std::collections::HashMap::from([("name".into(), Some("∴ Aporia".into()))]),
        None,
        std::time::Instant::now(),
    );
    assert_eq!(crate::room::members(&app.state, 0)[0].name, "Aporia");
}

fn app() -> App {
    let mut app = App::new(
        &crate::config::Config::default(),
        true,
        None,
        tokio::sync::mpsc::unbounded_channel().1,
        crate::api::EventHub::default(),
    );
    app.state = AppState::test_new();
    app.state.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
    app.state.active = Some(0);
    app.state.mode = Mode::Terminal;
    app.state.ensure_test_terminals();
    crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));
    app
}

fn identify(app: &mut App, session: &str) {
    let id = app.state.workspaces[0].tabs[0]
        .panes
        .values()
        .next()
        .unwrap()
        .attached_terminal_id
        .clone();
    let terminal = app.state.terminals.get_mut(&id).unwrap();
    terminal.set_detected_state(
        Some(crate::detect::Agent::Pi),
        crate::detect::AgentState::Idle,
    );
    let sequence = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    terminal.set_agent_session_ref_for_session_start(
        "herdr:pi".into(),
        "pi".into(),
        crate::agent_resume::AgentSessionRef::path(format!("/tmp/{session}.jsonl")),
        Some(sequence),
        Some("new".into()),
    );
    terminal.set_hook_authority_with_session_ref(
        "herdr:pi".into(),
        "pi".into(),
        crate::detect::AgentState::Idle,
        None,
        crate::agent_resume::AgentSessionRef::path(format!("/tmp/{session}.jsonl")),
        Some(sequence + 1),
    );
}

fn result(app: &mut App, method: Method) -> ResponseResult {
    let response = app.dispatch_api_request("room.test", method);
    serde_json::from_str::<SuccessResponse>(&response)
        .unwrap_or_else(|_| panic!("{response}"))
        .result
}

#[tokio::test]
async fn room_membership_is_current_and_historical_attribution_is_immutable() {
    let mut app = app();
    assert!(crate::room::members(&app.state, 0).is_empty());
    identify(&mut app, "original");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    assert!(member.session.is_some());
    app.state.workspaces[0]
        .room
        .post("question".into(), Some(member.clone()), 0)
        .unwrap();
    identify(&mut app, "replacement");
    let replacement = crate::room::members(&app.state, 0).pop().unwrap();
    assert_ne!(replacement.session, member.session);
    assert!(app.state.workspaces[0]
        .room
        .reply(1, replacement, "wrong".into(), 1)
        .is_err());
    let terminal = app
        .state
        .terminals
        .get_mut(
            &app.state.workspaces[0].tabs[0]
                .panes
                .values()
                .next()
                .unwrap()
                .attached_terminal_id,
        )
        .unwrap();
    terminal.clear_agent_runtime_identity_after_respawn();
    assert!(crate::room::members(&app.state, 0).is_empty());
    assert_eq!(
        app.state.workspaces[0].room.messages[0].recipient.as_ref(),
        Some(&member)
    );
}

#[tokio::test]
async fn room_api_reads_posts_replies_never_feed_pty_and_reject_duplicate() {
    let mut app = app();
    identify(&mut app, "reader");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    let terminal_id = app.state.workspaces[0].tabs[0]
        .panes
        .values()
        .next()
        .unwrap()
        .attached_terminal_id
        .clone();
    let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
    app.terminal_runtimes.insert(terminal_id, runtime);
    let workspace_id = app.state.workspaces[0].id.clone();
    let request = RoomPostParams {
        workspace_id: workspace_id.clone(),
        text: "question".into(),
        recipient: Some(RoomRecipient {
            pane_id: member.pane_id.clone(),
            terminal_id: member.terminal_id.clone(),
            session: member.session.clone().unwrap(),
        }),
    };
    result(
        &mut app,
        Method::RoomGet(WorkspaceTarget {
            workspace_id: workspace_id.clone(),
        }),
    );
    assert!(matches!(
        result(&mut app, Method::RoomPost(request)),
        ResponseResult::RoomWritten { sequence: 1, .. }
    ));
    let reply = RoomReplyParams {
        workspace_id: workspace_id.clone(),
        request_sequence: 1,
        pane_id: member.pane_id.clone(),
        terminal_id: member.terminal_id.clone(),
        session: member.session.clone().unwrap(),
        text: "I decline".into(),
    };
    assert!(matches!(
        result(&mut app, Method::RoomReply(reply.clone())),
        ResponseResult::RoomWritten { sequence: 2, .. }
    ));
    let duplicate = app.dispatch_api_request("duplicate", Method::RoomReply(reply));
    assert!(duplicate.contains("already has a reply"));
    match result(
        &mut app,
        Method::RoomRead(RoomReadParams {
            workspace_id,
            after_sequence: 1,
            limit: None,
        }),
    ) {
        ResponseResult::RoomMessages {
            messages,
            next_sequence,
            ..
        } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].reply_to, Some(1));
            assert_eq!(next_sequence, 3);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(
        rx.try_recv().is_err(),
        "no read/join/post/reply may feed a PTY"
    );
}

#[tokio::test]
async fn room_input_isolation_local_headless_ime_mouse_and_panel_arrows() {
    let mut app = app();
    let terminal_id = app.state.workspaces[0].tabs[0]
        .panes
        .values()
        .next()
        .unwrap()
        .attached_terminal_id
        .clone();
    let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
    app.terminal_runtimes.insert(terminal_id, runtime);
    app.state.select_room();
    assert!(app.terminal_input_context().is_none());
    // Stale terminal geometry must not enable forwarding before the next render.
    let area = app.state.view.terminal_area;
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x + 2,
        row: area.y + 2,
        modifiers: KeyModifiers::NONE,
    };
    app.route_client_events(
        vec![
            RawInputEvent::Key(TerminalKey::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            RawInputEvent::Paste("b".into()),
            RawInputEvent::Text(crate::input::TextCommit::new("文")),
            RawInputEvent::Mouse(mouse),
        ],
        false,
    );
    app.handle_text_commit("界".into()).await;
    app.handle_paste("c".into()).await;
    app.handle_key(TerminalKey::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .await;
    app.state.mode = Mode::Navigate;
    let arrow = TerminalKey::new(KeyCode::Up, KeyModifiers::NONE);
    assert!(!app.panel_arrow_targets_terminal(&arrow));
    app.handle_terminal_key_headless(arrow);
    assert_eq!(app.state.room_ui.composer, "ab文界cd");
    assert!(rx.try_recv().is_err());
    app.state.mode = Mode::Terminal;
    app.route_client_events(
        vec![RawInputEvent::Key(TerminalKey::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ))],
        false,
    );
    assert!(!app.state.room_active());
    app.route_client_events(
        vec![RawInputEvent::Key(TerminalKey::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        ))],
        false,
    );
    assert_eq!(rx.try_recv().unwrap().as_ref(), b"x");
}

#[tokio::test]
async fn room_navigation_preserves_terminal_tab_ids_and_invariants() {
    let mut app = app();
    app.state.workspaces[0].test_add_tab(Some("logs"));
    app.state.ensure_test_terminals();
    let numbers: Vec<_> = app.state.workspaces[0]
        .tabs
        .iter()
        .map(|tab| tab.number)
        .collect();
    crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));
    let room = app.state.view.room_hit_area;
    assert_eq!(app.state.view.tab_hit_areas.len(), 2);
    assert!(app.state.view.tab_hit_areas[0].x >= room.right());
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: room.x,
        row: room.y,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(click);
    assert!(app.state.room_active());
    crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));
    assert!(app.state.view.pane_infos.is_empty());
    assert!(app.state.switch_workspace_tab(0, 1));
    assert!(!app.state.room_active());
    app.route_client_events(
        vec![RawInputEvent::Key(TerminalKey::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ))],
        false,
    );
    assert!(app.state.room_active());
    assert_eq!(
        numbers,
        app.state.workspaces[0]
            .tabs
            .iter()
            .map(|tab| tab.number)
            .collect::<Vec<_>>()
    );
    app.state.assert_invariants_for_test();
}

#[tokio::test]
async fn room_api_rejects_ordinal_and_out_of_range_workspace_aliases() {
    let mut app = app();
    for workspace_id in ["1", "w_1", "999999", "w_999999", "missing"] {
        let methods = [
            Method::RoomGet(WorkspaceTarget {
                workspace_id: workspace_id.into(),
            }),
            Method::RoomRead(RoomReadParams {
                workspace_id: workspace_id.into(),
                after_sequence: 0,
                limit: None,
            }),
            Method::RoomPost(RoomPostParams {
                workspace_id: workspace_id.into(),
                text: "note".into(),
                recipient: None,
            }),
            Method::RoomReply(RoomReplyParams {
                workspace_id: workspace_id.into(),
                request_sequence: 1,
                pane_id: "missing".into(),
                terminal_id: "missing".into(),
                session: "missing".into(),
                text: "reply".into(),
            }),
        ];
        for method in methods {
            let response = app.dispatch_api_request("missing", method);
            let error: ErrorResponse = serde_json::from_str(&response).unwrap();
            assert_eq!(error.error.code, "workspace_not_found");
        }
    }
}

#[tokio::test]
async fn room_stable_workspace_target_survives_reordering() {
    let mut app = app();
    let workspace_id = app.state.workspaces[0].id.clone();
    app.state.workspaces.swap(0, 1);
    result(
        &mut app,
        Method::RoomPost(RoomPostParams {
            workspace_id,
            text: "stable destination".into(),
            recipient: None,
        }),
    );
    assert!(app.state.workspaces[0].room.messages.is_empty());
    assert_eq!(
        app.state.workspaces[1].room.messages[0].text,
        "stable destination"
    );
}

#[tokio::test]
async fn room_members_skip_deliberately_unregistered_panes_and_preserve_session_spelling() {
    let mut app = app();
    identify(&mut app, "path-session");
    let pane = app.state.workspaces[0].focused_pane_id().unwrap();
    assert_eq!(
        crate::room::members(&app.state, 0)[0].session.as_deref(),
        Some("Path:/tmp/path-session.jsonl")
    );
    let id = app.state.workspaces[0].terminal_id(pane).unwrap().clone();
    app.state
        .terminals
        .get_mut(&id)
        .unwrap()
        .hook_authority
        .as_mut()
        .unwrap()
        .session_ref = crate::agent_resume::AgentSessionRef::id("session-id");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    assert_eq!(member.session.as_deref(), Some("Id:session-id"));
    let json = serde_json::to_value(&member).unwrap();
    assert_eq!(json["session"], "Id:session-id");
    // Deliberately malformed state, not a claim about the normal restore path.
    app.state.workspaces[0].public_pane_numbers.remove(&pane);
    assert!(crate::room::members(&app.state, 0).is_empty());
}

#[tokio::test]
async fn room_composer_reports_nonreplyable_member_without_hiding_it() {
    let mut app = app();
    identify(&mut app, "identity-only");
    let pane = app.state.workspaces[0].focused_pane_id().unwrap();
    let id = app.state.workspaces[0].terminal_id(pane).unwrap().clone();
    app.state
        .terminals
        .get_mut(&id)
        .unwrap()
        .hook_authority
        .as_mut()
        .unwrap()
        .session_ref = None;
    app.state.select_room();
    assert_eq!(app.state.room_ui.members.len(), 1);
    app.state.room_ui.recipient = app.state.room_ui.members.first().cloned();
    app.state.room_ui.composer = "question".into();
    app.post_room_composer();
    assert!(app.state.room_ui.status.contains("1 unavailable"));
    assert!(app.state.room_ui.composer.is_empty());
    assert!(app.state.room_ui.recipient.is_none());
    assert_eq!(app.state.workspaces[0].room.messages.len(), 1);
    assert_eq!(app.state.room_ui.members.len(), 1);
}

#[tokio::test]
async fn room_write_failure_does_not_publish_candidate_or_consume_sequence() {
    let mut app = app();
    app.no_session = false;
    let mut candidate = app.state.workspaces[0].room.clone();
    candidate.post("must save first".into(), None, 0).unwrap();
    let failed = app.save_room_candidate_with(0, candidate.clone(), |snapshot| {
        assert_eq!(snapshot.workspaces[0].room.messages.len(), 1);
        Err(std::io::Error::other("injected disk failure"))
    });
    assert!(failed.unwrap_err().contains("injected disk failure"));
    assert!(app.state.workspaces[0].room.messages.is_empty());
    assert_eq!(app.state.workspaces[0].room.next_sequence, 1);
    assert_eq!(
        app.save_room_candidate_with(0, candidate, |_| Ok(()))
            .unwrap(),
        "saved"
    );
    assert_eq!(app.state.workspaces[0].room.next_sequence, 2);
    app.no_session = true;
}

#[tokio::test]
async fn room_members_follow_runtime_panes_between_workspaces() {
    let mut app = app();
    identify(&mut app, "moving");
    let before = crate::room::members(&app.state, 0).pop().unwrap();
    // Keep a shell in the source so the baseline's empty-workspace removal
    // semantics do not close the source workspace during this move.
    app.state.workspaces[0].test_split(ratatui::layout::Direction::Horizontal);
    app.state.ensure_test_terminals();
    let workspace_id = app.state.workspaces[1].id.clone();
    result(
        &mut app,
        Method::PaneMove(PaneMoveParams {
            pane_id: before.pane_id.clone(),
            destination: PaneMoveDestination::NewTab {
                workspace_id: Some(workspace_id),
                label: None,
            },
            focus: false,
        }),
    );
    app.state.assert_invariants_for_test();
    assert!(crate::room::members(&app.state, 0).is_empty());
    let after = crate::room::members(&app.state, 1).pop().unwrap();
    assert_eq!(before.session, after.session);
    assert_eq!(before.terminal_id, after.terminal_id);
    assert_ne!(before.pane_id, after.pane_id);
}

#[tokio::test]
async fn room_ui_ignores_and_clears_stale_recipient_after_replacement() {
    let mut app = app();
    identify(&mut app, "old");
    app.state.select_room();
    app.state.room_ui.recipient = crate::room::members(&app.state, 0).pop();
    app.state.room_ui.composer = "question".into();
    identify(&mut app, "new");
    app.refresh_room_members();
    app.post_room_composer();
    assert!(app.state.room_ui.recipient.is_none());
    let message = &app.state.workspaces[0].room.messages[0];
    assert!(message.recipient.is_none());
    assert_eq!(message.recipients, crate::room::members(&app.state, 0));
}

#[tokio::test]
async fn room_tab_never_selects_recipients_or_changes_hidden_pane() {
    let mut app = app();
    identify(&mut app, "old");
    app.state.select_room();
    app.state.workspaces[0].test_split(ratatui::layout::Direction::Horizontal);
    app.state.ensure_test_terminals();
    let focused = app.state.workspaces[0].focused_pane_id();
    app.state.insert_room_text("draft");
    let cursor = app.state.room_ui.editor.cursor;
    for code in [KeyCode::Tab, KeyCode::BackTab, KeyCode::Tab] {
        app.route_client_events_from(
            42,
            vec![RawInputEvent::Key(TerminalKey::new(
                code,
                KeyModifiers::NONE,
            ))],
            false,
        );
        assert!(app.state.room_active());
        assert!(app.state.room_ui.recipient.is_none());
        assert_eq!(app.state.workspaces[0].focused_pane_id(), focused);
        assert_eq!(app.state.room_ui.composer, "draft");
        assert_eq!(app.state.room_ui.editor.cursor, cursor);
        assert!(app.state.workspaces[0].room.messages.is_empty());
    }
}

#[tokio::test]
async fn room_api_focus_and_move_leave_the_correct_surface_active() {
    let mut app = app();
    identify(&mut app, "focus");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    let original_ws = app.state.workspaces[0].id.clone();
    let destination_ws = app.state.workspaces[1].id.clone();
    let tab_id = app.public_tab_id(0, 0).unwrap();
    for method in [
        Method::PaneFocus(PaneTarget {
            pane_id: member.pane_id.clone(),
        }),
        Method::AgentFocus(AgentTarget {
            target: member.pane_id.clone(),
        }),
        Method::TabFocus(TabTarget { tab_id }),
    ] {
        app.state.select_room();
        result(&mut app, method);
        assert!(!app.state.room_active());
        assert!(!app.state.room_ui.visible);
    }
    app.state.select_room();
    result(
        &mut app,
        Method::WorkspaceFocus(WorkspaceTarget {
            workspace_id: destination_ws.clone(),
        }),
    );
    assert!(!app.state.room_active());
    result(
        &mut app,
        Method::WorkspaceFocus(WorkspaceTarget {
            workspace_id: original_ws.clone(),
        }),
    );
    assert!(
        app.state.room_active(),
        "workspace focus restores its previous room selection"
    );

    // A background move keeps the source room; a focused move selects terminals.
    app.state.workspaces[0].test_split(ratatui::layout::Direction::Horizontal);
    app.state.ensure_test_terminals();
    app.state.select_room();
    result(
        &mut app,
        Method::PaneMove(PaneMoveParams {
            pane_id: member.pane_id,
            destination: PaneMoveDestination::NewTab {
                workspace_id: Some(destination_ws),
                label: None,
            },
            focus: false,
        }),
    );
    assert!(app.state.room_active());
    let moved = crate::room::members(&app.state, 1).pop().unwrap();
    result(
        &mut app,
        Method::PaneMove(PaneMoveParams {
            pane_id: moved.pane_id,
            destination: PaneMoveDestination::NewTab {
                workspace_id: Some(original_ws),
                label: None,
            },
            focus: true,
        }),
    );
    assert!(!app.state.room_active());
    assert!(!app.state.room_ui.visible);
    app.state.assert_invariants_for_test();
}

#[tokio::test]
async fn room_read_zero_limit_returns_only_current_metadata() {
    let mut app = app();
    app.state.workspaces[0]
        .room
        .post("record".into(), None, 0)
        .unwrap();
    let workspace_id = app.state.workspaces[0].id.clone();
    match result(
        &mut app,
        Method::RoomRead(RoomReadParams {
            workspace_id: workspace_id.clone(),
            after_sequence: 0,
            limit: Some(0),
        }),
    ) {
        ResponseResult::RoomMessages {
            room_id,
            messages,
            next_sequence,
        } => {
            assert_eq!(room_id, format!("room:{workspace_id}"));
            assert!(messages.is_empty());
            assert_eq!(next_sequence, 2);
        }
        other => panic!("unexpected {other:?}"),
    }
}

async fn route_room_event(app: &mut App, local: bool, event: RawInputEvent) {
    if local {
        app.handle_raw_input_event(event).await;
    } else {
        app.route_client_events_from(42, vec![event], false);
    }
}

#[tokio::test]
async fn room_entry_preserves_pinned_release_without_forwarding_repeats_or_text() {
    for local in [false, true] {
        for mouse_entry in [false, true] {
            let mut app = app();
            let original = app.state.workspaces[0].focused_pane_id().unwrap();
            let terminal_id = app.state.workspaces[0]
                .terminal_id(original)
                .unwrap()
                .clone();
            let (runtime, mut rx) =
                crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                    80,
                    24,
                    0,
                    b"\x1b[>15u",
                    32,
                );
            app.terminal_runtimes.insert(terminal_id, runtime);
            let key = TerminalKey::new(KeyCode::Char('a'), KeyModifiers::NONE);
            route_room_event(&mut app, local, RawInputEvent::Key(key.clone())).await;
            assert_eq!(rx.try_recv().unwrap().as_ref(), b"\x1b[97;1:1u");
            let source = if local {
                crate::app::LOCAL_INPUT_SOURCE
            } else {
                42
            };
            let lease = crate::app::input::InputLeaseKey::new(source, &key);
            assert!(app.input_leases.contains(&lease));
            let toggle = TerminalKey::new(
                KeyCode::Char('r'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            );
            let event = if mouse_entry {
                let area = app.state.view.room_hit_area;
                RawInputEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                })
            } else {
                RawInputEvent::Key(toggle.clone())
            };
            // Room chrome works even without terminal mouse capture.
            app.state.mouse_capture = false;
            route_room_event(&mut app, local, event).await;
            assert!(app.state.room_active());
            route_room_event(
                &mut app,
                local,
                RawInputEvent::Key(toggle.with_kind(KeyEventKind::Repeat)),
            )
            .await;
            assert!(app.state.room_active(), "toggle repeat must not leave room");
            route_room_event(
                &mut app,
                local,
                RawInputEvent::Key(key.clone().with_kind(KeyEventKind::Repeat)),
            )
            .await;
            route_room_event(
                &mut app,
                local,
                RawInputEvent::Key(TerminalKey::new(KeyCode::Char('b'), KeyModifiers::NONE)),
            )
            .await;
            route_room_event(
                &mut app,
                local,
                RawInputEvent::Text(crate::input::TextCommit::new("text")),
            )
            .await;
            route_room_event(&mut app, local, RawInputEvent::Paste("paste".into())).await;
            assert!(rx.try_recv().is_err());
            assert!(app.input_leases.contains(&lease));
            // Changing hidden terminal focus cannot rebind the held key.
            let tab = app.state.workspaces[0].test_add_tab(Some("other"));
            app.state.workspaces[0].switch_tab(tab);
            app.state.ensure_test_terminals();
            let other = app.state.workspaces[0].focused_pane_id().unwrap();
            assert_ne!(other, original);
            let other_id = app.state.workspaces[0].terminal_id(other).unwrap().clone();
            let (runtime, mut other_rx) =
                crate::terminal::TerminalRuntime::test_with_channel(80, 24);
            app.terminal_runtimes.insert(other_id, runtime);
            // Cover releases with both composer and panel focus. Panel-focused
            // repeats reach the lease plan rather than the room key handler.
            if mouse_entry {
                app.state.mode = Mode::Navigate;
            }
            route_room_event(
                &mut app,
                local,
                RawInputEvent::Key(key.clone().with_kind(KeyEventKind::Repeat)),
            )
            .await;
            assert!(app.input_leases.contains(&lease));
            route_room_event(
                &mut app,
                local,
                RawInputEvent::Key(key.clone().with_kind(KeyEventKind::Release)),
            )
            .await;
            assert_eq!(rx.try_recv().unwrap().as_ref(), b"\x1b[97;1:3u");
            assert!(!app.input_leases.contains(&lease));
            route_room_event(
                &mut app,
                local,
                RawInputEvent::Key(key.with_kind(KeyEventKind::Release)),
            )
            .await;
            assert!(rx.try_recv().is_err());
            assert!(other_rx.try_recv().is_err());
            assert!(app.input_leases.is_empty());
        }
    }
}

#[tokio::test]
async fn room_lease_cleanup_releases_on_focus_loss_and_never_rebinds_missing_targets() {
    for local in [false, true] {
        for removed in [false, true] {
            let mut app = app();
            let pane = app.state.workspaces[0].focused_pane_id().unwrap();
            let id = app.state.workspaces[0].terminal_id(pane).unwrap().clone();
            let (runtime, mut rx) =
                crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                    80,
                    24,
                    0,
                    b"\x1b[>15u",
                    8,
                );
            app.terminal_runtimes.insert(id.clone(), runtime);
            let key = TerminalKey::new(KeyCode::Char('a'), KeyModifiers::NONE);
            route_room_event(&mut app, local, RawInputEvent::Key(key.clone())).await;
            assert_eq!(rx.try_recv().unwrap().as_ref(), b"\x1b[97;1:1u");
            app.state.select_room();
            let (replacement, mut replacement_rx) =
                crate::terminal::TerminalRuntime::test_with_channel(80, 24);
            if removed {
                app.terminal_runtimes.remove(&id);
                let replacement_id = crate::terminal::TerminalId::alloc();
                app.state.workspaces[0].tabs[0]
                    .panes
                    .get_mut(&pane)
                    .unwrap()
                    .attached_terminal_id = replacement_id.clone();
                app.terminal_runtimes.insert(replacement_id, replacement);
                route_room_event(
                    &mut app,
                    local,
                    RawInputEvent::Key(key.with_kind(KeyEventKind::Release)),
                )
                .await;
                assert!(rx.try_recv().is_err());
            } else {
                route_room_event(&mut app, local, RawInputEvent::OuterFocusLost).await;
                assert_eq!(rx.try_recv().unwrap().as_ref(), b"\x1b[97;1:3u");
            }
            assert!(replacement_rx.try_recv().is_err());
            assert!(app.input_leases.is_empty());
        }
    }
}

#[tokio::test]
async fn room_keyboard_and_wheel_overscroll_is_normalized_before_scrolling_back() {
    let mut app = app();
    app.state.workspaces[0]
        .room
        .post("long wrapped text ".repeat(400), None, 0)
        .unwrap();
    app.state.select_room();
    for frame in [Rect::new(0, 0, 120, 15), Rect::new(0, 0, 120, 40)] {
        app.state.room_ui.scroll = usize::MAX;
        crate::ui::compute_view(&mut app.state, frame);
        let max = app.state.room_ui.scroll;
        assert!(max > 5 && max < usize::MAX);
        app.handle_room_key(&TerminalKey::new(KeyCode::PageUp, KeyModifiers::NONE));
        crate::ui::compute_view(&mut app.state, frame);
        assert_eq!(app.state.room_ui.scroll, max);
        app.handle_room_key(&TerminalKey::new(KeyCode::PageDown, KeyModifiers::NONE));
        crate::ui::compute_view(&mut app.state, frame);
        assert_eq!(app.state.room_ui.scroll, max - 5);
        let area = app.state.view.terminal_area;
        let mut mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        };
        for _ in 0..100 {
            app.handle_room_mouse(mouse);
        }
        crate::ui::compute_view(&mut app.state, frame);
        assert_eq!(app.state.room_ui.scroll, max);
        mouse.kind = MouseEventKind::ScrollDown;
        app.handle_room_mouse(mouse);
        crate::ui::compute_view(&mut app.state, frame);
        assert_eq!(app.state.room_ui.scroll, max - 3);
    }
}
