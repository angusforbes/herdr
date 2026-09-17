use super::*;

fn member() -> Member {
    Member {
        pane_id: "w1:p1".into(),
        terminal_id: "t1".into(),
        agent: "pi".into(),
        name: "Ada".into(),
        session: Some("Path:/session".into()),
    }
}
fn current() -> HashMap<String, Vec<Member>> {
    HashMap::from([("w1".into(), vec![member()])])
}
fn registered() -> (Runtime, String, String) {
    let mut runtime = Runtime::default();
    let receiver = runtime
        .register("w1".into(), member(), "nonce".into(), 100)
        .unwrap();
    let epoch = runtime.epoch.clone();
    (runtime, receiver, epoch)
}
fn enqueue(runtime: &mut Runtime, sequence: u64) {
    runtime.enqueue_saved("w1", sequence, "question", vec![member()], 100);
}

#[test]
fn room_delivery_claim_is_at_most_once_and_submitted_retains_slot() {
    let (mut runtime, id, epoch) = registered();
    enqueue(&mut runtime, 1);
    enqueue(&mut runtime, 2);
    assert!(runtime.claim(&id, &epoch, false, 101).unwrap().is_none());
    let first = runtime.claim(&id, &epoch, true, 101).unwrap().unwrap();
    assert_eq!(first.request_sequence, 1);
    // Even a lost response cannot cause replay or substitution by the next job.
    assert!(runtime.claim(&id, &epoch, true, 102).unwrap().is_none());
    runtime
        .report(&id, &epoch, &first.delivery_id, Outcome::Submitted, None)
        .unwrap();
    assert!(runtime.claim(&id, &epoch, true, 103).unwrap().is_none());
    runtime
        .report(
            &id,
            &epoch,
            &first.delivery_id,
            Outcome::Unanswered,
            Some("x".repeat(1000)),
        )
        .unwrap();
    runtime
        .report(&id, &epoch, &first.delivery_id, Outcome::Submitted, None)
        .unwrap();
    assert_eq!(runtime.deliveries("w1")[0].status, Status::Unanswered);
    assert_eq!(
        runtime.deliveries("w1")[0].detail.as_ref().unwrap().len(),
        MAX_DETAIL
    );
    let second = runtime.claim(&id, &epoch, true, 104).unwrap().unwrap();
    assert_eq!(second.request_sequence, 2);
    runtime
        .report(&id, &epoch, &second.delivery_id, Outcome::Failed, None)
        .unwrap();
    assert!(runtime.claim(&id, &epoch, true, 105).unwrap().is_none());
}

#[test]
fn room_delivery_reply_clears_slot_and_late_reports_cannot_resurrect() {
    let (mut runtime, id, epoch) = registered();
    enqueue(&mut runtime, 1);
    enqueue(&mut runtime, 2);
    let first = runtime.claim(&id, &epoch, true, 101).unwrap().unwrap();
    runtime.replied("w1", 1, &member());
    for outcome in [Outcome::Submitted, Outcome::Unanswered, Outcome::Failed] {
        runtime
            .report(&id, &epoch, &first.delivery_id, outcome, None)
            .unwrap();
        assert_eq!(runtime.deliveries("w1")[0].status, Status::Replied);
    }
    assert_eq!(
        runtime
            .claim(&id, &epoch, true, 102)
            .unwrap()
            .unwrap()
            .request_sequence,
        2
    );
    assert!(runtime
        .report(&id, &epoch, "foreign", Outcome::Failed, None)
        .is_err());
}

#[test]
fn room_delivery_registration_replacement_offline_and_restart_never_replay() {
    let (mut runtime, id, epoch) = registered();
    assert_eq!(
        runtime
            .register("w1".into(), member(), "nonce".into(), 101)
            .unwrap(),
        id
    );
    enqueue(&mut runtime, 1);
    let replacement = runtime
        .register("w1".into(), member(), "new".into(), 102)
        .unwrap();
    assert_ne!(id, replacement);
    assert!(runtime.claim(&id, &epoch, true, 102).is_err());
    assert!(runtime
        .claim(&replacement, &epoch, true, 102)
        .unwrap()
        .is_none());
    assert_eq!(runtime.deliveries("w1")[0].status, Status::Unavailable);
    runtime.cleanup(&current(), 117);
    assert!(!runtime.receivers("w1", &[member()])[0].available);
    enqueue(&mut runtime, 2);
    let next = runtime
        .register("w1".into(), member(), "next".into(), 118)
        .unwrap();
    assert!(runtime.claim(&next, &epoch, true, 118).unwrap().is_none());
    let mut restart = Runtime::default();
    assert_ne!(restart.epoch, epoch);
    assert!(restart.deliveries("w1").is_empty());
    assert!(restart.claim(&next, &epoch, true, 118).is_err());
}

#[test]
fn room_delivery_live_identity_revalidation_and_dead_workspace_cleanup() {
    let (mut runtime, id, epoch) = registered();
    enqueue(&mut runtime, 1);
    runtime.claim(&id, &epoch, true, 101).unwrap();
    let mut replacement = member();
    replacement.session = Some("Path:/replacement".into());
    runtime.cleanup(&HashMap::from([("w1".into(), vec![replacement])]), 102);
    assert!(runtime.claim(&id, &epoch, true, 102).is_err());
    assert!(runtime
        .report(
            &id,
            &epoch,
            &runtime.deliveries("w1")[0].delivery_id,
            Outcome::Submitted,
            None
        )
        .is_err());
    assert_eq!(runtime.deliveries("w1")[0].status, Status::Failed);
    runtime.cleanup(&HashMap::new(), 103);
    assert!(runtime.entries.is_empty());
    assert!(runtime.receivers.is_empty());
}

#[test]
fn room_delivery_expiry_heartbeat_bounds_and_unsupported_visibility() {
    let (mut runtime, id, epoch) = registered();
    enqueue(&mut runtime, 1);
    runtime.claim(&id, &epoch, true, 101).unwrap();
    for timestamp in (110..=700).step_by(10) {
        runtime.cleanup(&current(), timestamp);
        assert!(runtime
            .claim(&id, &epoch, false, timestamp)
            .unwrap()
            .is_none());
    }
    assert_eq!(runtime.deliveries("w1")[0].status, Status::Expired);
    runtime.enqueue_saved("w1", 2, "new", vec![member()], 700);
    assert_eq!(
        runtime
            .claim(&id, &epoch, true, 701)
            .unwrap()
            .unwrap()
            .request_sequence,
        2
    );
    runtime.cleanup(&current(), 1300);
    assert_eq!(
        runtime.entries.len(),
        1,
        "old bounded status history discarded"
    );
    assert!(runtime.check_capacity(MAX_DELIVERIES).is_err());
    let mut unsupported = member();
    unsupported.agent = "claude".into();
    let mut unidentified = member();
    unidentified.session = None;
    runtime.enqueue_saved(
        "w1",
        3,
        "all",
        vec![unsupported.clone(), unidentified.clone()],
        1300,
    );
    assert!(runtime
        .register("w1".into(), unsupported, "nonce".into(), 1300)
        .is_err());
    assert!(runtime
        .register("w1".into(), unidentified, "nonce".into(), 1300)
        .is_err());
    assert!(runtime.deliveries("w1")[1..]
        .iter()
        .all(|d| d.status == Status::Unavailable));
    assert!(runtime
        .register("w1".into(), member(), "".into(), 1300)
        .is_err());
    assert!(runtime
        .register("w1".into(), member(), "x".repeat(257), 1300)
        .is_err());
}

#[test]
fn room_delivery_persisted_reply_correlates_terminal_session_after_same_workspace_move() {
    let (mut runtime, id, epoch) = registered();
    enqueue(&mut runtime, 1);
    runtime.claim(&id, &epoch, true, 101).unwrap();
    let mut moved = member();
    moved.pane_id = "w1:p2".into();
    runtime.cleanup(&HashMap::from([("w1".into(), vec![moved.clone()])]), 102);
    assert_eq!(runtime.deliveries("w1")[0].status, Status::Failed);
    runtime.replied("w1", 1, &moved);
    assert_eq!(runtime.deliveries("w1")[0].status, Status::Replied);
}

#[test]
fn room_delivery_queued_work_cannot_be_reported_without_claim() {
    let (mut runtime, id, epoch) = registered();
    enqueue(&mut runtime, 1);
    let delivery = runtime.deliveries("w1")[0].delivery_id.clone();
    assert!(runtime
        .report(&id, &epoch, &delivery, Outcome::Submitted, None)
        .is_err());
    assert!(runtime.claim(&id, "old epoch", true, 101).is_err());
    assert_eq!(runtime.deliveries("w1")[0].status, Status::Queued);
}
