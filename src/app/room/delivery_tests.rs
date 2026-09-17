use super::*;
use crate::room_delivery::{Outcome, Status};

fn register(app: &mut App, member: &Member) -> (String, String) {
    let params = RoomDeliveryRegisterParams {
        workspace_id: app.state.workspaces[0].id.clone(),
        pane_id: member.pane_id.clone(),
        terminal_id: member.terminal_id.clone(),
        session: member.session.clone().unwrap(),
        receiver_nonce: member.terminal_id.clone(),
    };
    match result(app, Method::RoomDeliveryRegister(params)) {
        ResponseResult::RoomDeliveryRegistered {
            receiver_id,
            server_epoch,
        } => (receiver_id, server_epoch),
        other => panic!("{other:?}"),
    }
}
fn claim(
    app: &mut App,
    receiver: &(String, String),
    ready: bool,
) -> Option<crate::room_delivery::Delivery> {
    match result(
        app,
        Method::RoomDeliveryClaim(RoomDeliveryClaimParams {
            receiver_id: receiver.0.clone(),
            server_epoch: receiver.1.clone(),
            ready,
        }),
    ) {
        ResponseResult::RoomDeliveryClaimed { delivery } => delivery,
        other => panic!("{other:?}"),
    }
}
fn post(app: &mut App, recipient: Option<&Member>) -> ResponseResult {
    let workspace_id = app.state.workspaces[0].id.clone();
    result(
        app,
        Method::RoomPost(RoomPostParams {
            workspace_id,
            text: "question for audience".into(),
            recipient: recipient.map(|m| RoomRecipient {
                pane_id: m.pane_id.clone(),
                terminal_id: m.terminal_id.clone(),
                session: m.session.clone().unwrap(),
            }),
        }),
    )
}
fn add_member(app: &mut App, agent: crate::detect::Agent, session: Option<&str>) -> Member {
    let pane = app.state.workspaces[0].test_split(ratatui::layout::Direction::Horizontal);
    app.state.ensure_test_terminals();
    let id = app.state.workspaces[0].terminal_id(pane).unwrap().clone();
    let terminal = app.state.terminals.get_mut(&id).unwrap();
    terminal.set_detected_state(Some(agent), crate::detect::AgentState::Idle);
    if let Some(session) = session {
        let label = terminal.effective_agent_label().unwrap().to_owned();
        terminal.set_agent_session_ref_for_session_start(
            format!("herdr:{label}"),
            label.clone(),
            crate::agent_resume::AgentSessionRef::path(session),
            Some(1),
            Some("new".into()),
        );
        terminal.set_hook_authority_with_session_ref(
            format!("herdr:{label}"),
            label,
            crate::detect::AgentState::Idle,
            None,
            crate::agent_resume::AgentSessionRef::path(session),
            Some(2),
        );
    }
    crate::room::members(&app.state, 0)
        .into_iter()
        .find(|m| m.terminal_id == id.to_string())
        .unwrap()
}
fn room_info(
    app: &mut App,
) -> (
    Vec<crate::room_delivery::DeliveryStatus>,
    Vec<crate::room_delivery::ReceiverStatus>,
) {
    let workspace_id = app.state.workspaces[0].id.clone();
    match result(app, Method::RoomGet(WorkspaceTarget { workspace_id })) {
        ResponseResult::RoomInfo {
            deliveries,
            receivers,
            ..
        } => (deliveries, receivers),
        other => panic!("{other:?}"),
    }
}
fn reply(app: &mut App, job: &crate::room_delivery::Delivery) -> RoomReplyParams {
    let params = RoomReplyParams {
        workspace_id: job.workspace_id.clone(),
        request_sequence: job.request_sequence,
        pane_id: job.recipient.pane_id.clone(),
        terminal_id: job.recipient.terminal_id.clone(),
        session: job.recipient.session.clone().unwrap(),
        text: "answer".into(),
    };
    assert!(matches!(
        result(app, Method::RoomReply(params.clone())),
        ResponseResult::RoomWritten { queued: 0, .. }
    ));
    params
}

#[tokio::test]
async fn room_delivery_api_broadcast_snapshots_members_once_and_reports_availability() {
    let mut app = app();
    identify(&mut app, "ada");
    let ada = crate::room::members(&app.state, 0).pop().unwrap();
    let bob = add_member(&mut app, crate::detect::Agent::Pi, Some("/tmp/bob"));
    let offline = add_member(&mut app, crate::detect::Agent::Pi, Some("/tmp/offline"));
    let unsupported = add_member(&mut app, crate::detect::Agent::Claude, Some("/tmp/claude"));
    let unidentified = add_member(&mut app, crate::detect::Agent::Pi, None);
    let ada_receiver = register(&mut app, &ada);
    let bob_receiver = register(&mut app, &bob);
    assert_eq!(register(&mut app, &ada), ada_receiver);
    assert!(matches!(
        post(&mut app, None),
        ResponseResult::RoomWritten {
            queued: 2,
            unavailable: 3,
            ..
        }
    ));
    assert_eq!(app.state.workspaces[0].room.messages.len(), 1);
    let audience = app.state.workspaces[0].room.messages[0].recipients.clone();
    assert_eq!(audience.len(), 4);
    assert!(audience.contains(&offline) && audience.contains(&unsupported));
    assert!(!audience.contains(&unidentified));
    let (deliveries, receivers) = room_info(&mut app);
    assert_eq!(deliveries.len(), 5);
    assert_eq!(receivers.iter().filter(|r| r.available).count(), 2);
    assert!(deliveries
        .iter()
        .any(|d| d.recipient == unidentified
            && d.detail.as_deref() == Some("no live session identity")));
    let joiner = add_member(&mut app, crate::detect::Agent::Pi, Some("/tmp/joiner"));
    let joiner_receiver = register(&mut app, &joiner);
    assert!(claim(&mut app, &joiner_receiver, true).is_none());
    assert_eq!(
        app.state.workspaces[0].room.messages[0].recipients,
        audience
    );
    let first = claim(&mut app, &ada_receiver, true).unwrap();
    assert!(claim(&mut app, &ada_receiver, true).is_none());
    let second = claim(&mut app, &bob_receiver, true).unwrap();
    assert_eq!(first.request_sequence, second.request_sequence);
    assert_ne!(first.delivery_id, second.delivery_id);
    let params = reply(&mut app, &first);
    reply(&mut app, &second);
    assert!(app
        .dispatch_api_request("duplicate", Method::RoomReply(params))
        .contains("already has a reply"));
    assert_eq!(
        room_info(&mut app)
            .0
            .iter()
            .filter(|d| d.status == Status::Replied)
            .count(),
        2
    );
    assert_eq!(
        room_info(&mut app).0.len(),
        5,
        "replies never enqueue more work"
    );
    assert!(claim(&mut app, &ada_receiver, true).is_none());
    assert!(claim(&mut app, &bob_receiver, true).is_none());
}

#[tokio::test]
async fn room_delivery_api_targeted_slot_report_and_current_session_revalidation() {
    let mut app = app();
    identify(&mut app, "target");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    let receiver = register(&mut app, &member);
    let other = add_member(&mut app, crate::detect::Agent::Pi, Some("/tmp/other"));
    let other_receiver = register(&mut app, &other);
    post(&mut app, Some(&member));
    post(&mut app, Some(&member));
    assert!(claim(&mut app, &other_receiver, true).is_none());
    assert!(claim(&mut app, &receiver, false).is_none());
    let first = claim(&mut app, &receiver, true).unwrap();
    let report = RoomDeliveryReportParams {
        receiver_id: receiver.0.clone(),
        server_epoch: receiver.1.clone(),
        delivery_id: first.delivery_id.clone(),
        outcome: Outcome::Submitted,
        detail: None,
    };
    assert!(matches!(
        result(&mut app, Method::RoomDeliveryReport(report.clone())),
        ResponseResult::RoomDeliveryReported { accepted: true }
    ));
    assert!(claim(&mut app, &receiver, true).is_none());
    reply(&mut app, &first);
    result(&mut app, Method::RoomDeliveryReport(report));
    assert_eq!(room_info(&mut app).0[0].status, Status::Replied);
    let second = claim(&mut app, &receiver, true).unwrap();
    // Replace the exact old terminal's live authority (not the newly split focus).
    let id = app
        .state
        .terminals
        .iter()
        .find(|(id, _)| id.to_string() == member.terminal_id)
        .unwrap()
        .0
        .clone();
    app.state
        .terminals
        .get_mut(&id)
        .unwrap()
        .hook_authority
        .as_mut()
        .unwrap()
        .session_ref = crate::agent_resume::AgentSessionRef::path("/tmp/new");
    let claim_error = app.dispatch_api_request(
        "stale",
        Method::RoomDeliveryClaim(RoomDeliveryClaimParams {
            receiver_id: receiver.0.clone(),
            server_epoch: receiver.1.clone(),
            ready: true,
        }),
    );
    assert!(claim_error.contains("invalid_room_receiver"));
    let reply_error = app.dispatch_api_request(
        "stale",
        Method::RoomReply(RoomReplyParams {
            workspace_id: second.workspace_id,
            request_sequence: second.request_sequence,
            pane_id: member.pane_id.clone(),
            terminal_id: member.terminal_id.clone(),
            session: member.session.clone().unwrap(),
            text: "stale".into(),
        }),
    );
    assert!(reply_error.contains("invalid_recipient"));
    let workspace_id = app.state.workspaces[0].id.clone();
    let post_error = app.dispatch_api_request(
        "stale",
        Method::RoomPost(RoomPostParams {
            workspace_id,
            text: "stale".into(),
            recipient: Some(RoomRecipient {
                pane_id: member.pane_id,
                terminal_id: member.terminal_id,
                session: member.session.unwrap(),
            }),
        }),
    );
    assert!(post_error.contains("invalid_recipient"));
    assert_eq!(room_info(&mut app).0[1].status, Status::Failed);
}

#[tokio::test]
async fn room_delivery_api_save_failure_no_dispatch_or_sequence_consumption() {
    let mut app = app();
    identify(&mut app, "save");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    let receiver = register(&mut app, &member);
    app.no_session = false;
    let mut candidate = app.state.workspaces[0].room.clone();
    let sequence = candidate
        .post_to(
            "must save first".into(),
            vec![member.clone()],
            crate::room_delivery::now(),
        )
        .unwrap();
    let response = app.finish_room_post_with(
        "save".into(),
        0,
        candidate.clone(),
        sequence,
        vec![member.clone()],
        |_| Err(std::io::Error::other("injected disk failure")),
    );
    assert!(response.contains("room_save_failed"));
    assert_eq!(app.state.workspaces[0].room.next_sequence, 1);
    assert!(app.state.workspaces[0].room.messages.is_empty());
    assert!(room_info(&mut app).0.is_empty());
    assert!(claim(&mut app, &receiver, true).is_none());
    let response =
        app.finish_room_post_with("save".into(), 0, candidate, sequence, vec![member], |_| {
            Ok(())
        });
    assert!(response.contains("\"queued\":1"));
    assert_eq!(
        claim(&mut app, &receiver, true).unwrap().request_sequence,
        1
    );
    app.no_session = true;
}

#[tokio::test]
async fn room_delivery_reads_join_and_runtime_restart_do_not_create_work() {
    let mut app = app();
    identify(&mut app, "restart");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    let receiver = register(&mut app, &member);
    post(&mut app, None);
    let persisted = serde_json::to_string(&app.state.workspaces[0].room).unwrap();
    app.room_delivery = crate::room_delivery::Runtime::default();
    app.state.workspaces[0].room = serde_json::from_str(&persisted).unwrap();
    let next = register(&mut app, &member);
    assert_ne!(receiver.1, next.1);
    let workspace_id = app.state.workspaces[0].id.clone();
    result(
        &mut app,
        Method::RoomRead(RoomReadParams {
            workspace_id,
            after_sequence: 0,
            limit: None,
        }),
    );
    app.state.select_room();
    app.refresh_room_members();
    assert!(room_info(&mut app).0.is_empty());
    assert!(claim(&mut app, &next, true).is_none());
    assert_eq!(app.state.workspaces[0].room.messages.len(), 1);
    assert!(app.state.room_ui.receivers[0].available);
}

#[tokio::test]
async fn room_delivery_failed_reply_persistence_keeps_active_slot() {
    let mut app = app();
    identify(&mut app, "reply-save");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    let receiver = register(&mut app, &member);
    post(&mut app, None);
    post(&mut app, None);
    let job = claim(&mut app, &receiver, true).unwrap();
    // The real persistence gate fails if its prior writer cannot be joined.
    // No disk/global state is touched by this test.
    app.session_save_thread = Some(std::thread::spawn(|| {
        panic!("injected prior writer failure")
    }));
    let params = RoomReplyParams {
        workspace_id: job.workspace_id.clone(),
        request_sequence: job.request_sequence,
        pane_id: member.pane_id,
        terminal_id: member.terminal_id,
        session: member.session.unwrap(),
        text: "answer".into(),
    };
    let response = app.dispatch_api_request("failure", Method::RoomReply(params.clone()));
    assert!(response.contains("room_save_failed"));
    assert_eq!(app.state.workspaces[0].room.messages.len(), 2);
    assert_eq!(room_info(&mut app).0[0].status, Status::Claimed);
    assert!(claim(&mut app, &receiver, true).is_none());
    result(&mut app, Method::RoomReply(params));
    assert_eq!(room_info(&mut app).0[0].status, Status::Replied);
    assert_eq!(
        claim(&mut app, &receiver, true).unwrap().request_sequence,
        2
    );
}

#[test]
fn room_delivery_json_contract_names_and_shapes_are_additive() {
    for json in [
        serde_json::json!({"id":"r", "method":"room.delivery.register", "params":{"workspace_id":"w1", "pane_id":"w1:p1", "terminal_id":"t1", "session":"Path:/session", "receiver_nonce":"nonce"}}),
        serde_json::json!({"id":"c", "method":"room.delivery.claim", "params":{"receiver_id":"receiver", "server_epoch":"epoch", "ready":false}}),
        serde_json::json!({"id":"s", "method":"room.delivery.report", "params":{"receiver_id":"receiver", "server_epoch":"epoch", "delivery_id":"delivery", "outcome":"submitted", "detail":null}}),
    ] {
        let request: Request = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), json);
    }
    let claimed = ResponseResult::RoomDeliveryClaimed { delivery: None };
    assert_eq!(
        serde_json::to_value(claimed).unwrap(),
        serde_json::json!({"type":"room_delivery_claimed", "delivery":null})
    );
}

#[tokio::test]
async fn room_delivery_composer_none_broadcasts_and_shows_real_status() {
    let mut app = app();
    identify(&mut app, "composer");
    let member = crate::room::members(&app.state, 0).pop().unwrap();
    register(&mut app, &member);
    add_member(&mut app, crate::detect::Agent::Pi, None);
    app.state.select_room();
    app.state.room_ui.composer = "everyone?".into();
    app.post_room_composer();
    assert!(app.state.room_ui.composer.is_empty());
    assert!(app
        .state
        .room_ui
        .status
        .contains("1 queued · 1 unavailable"));
    assert!(app
        .state
        .room_ui
        .delivery_summary
        .contains("no live session identity"));
    assert_eq!(room_info(&mut app).0.len(), 2);
    assert_eq!(
        app.state.workspaces[0].room.messages[0].recipients,
        vec![member]
    );
}
