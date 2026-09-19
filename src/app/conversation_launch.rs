//! Event-driven, pre-submission shell readiness for a freshly split Pi fork.
//! Reservations are runtime-only and keyed by terminal identity, never pane position.
use std::time::{Duration, Instant};

use crate::{
    api::schema::{AgentInfo, AgentStartParams, Method, NotificationShowParams},
    events::AppEvent,
    layout::PaneId,
    terminal::{TerminalId, TerminalRuntime},
};

use super::{
    agents::{available_shell_name, AgentStartError},
    App,
};

const RETRY_INTERVAL: Duration = Duration::from_millis(100);
const LAUNCH_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub(super) struct PendingConversationLaunch {
    pub name: String,
    args: Vec<String>,
    deadline: Instant,
}

impl App {
    pub(super) fn queue_conversation_launch(
        &mut self,
        pane: &str,
        name: String,
        args: Vec<String>,
    ) -> Result<(AgentInfo, Vec<String>), String> {
        let (ws, pane_id) = self
            .parse_current_public_pane_id(pane)
            .ok_or("new fork pane disappeared")?;
        let terminal_id = self.state.workspaces[ws]
            .terminal_id(pane_id)
            .cloned()
            .ok_or("new fork terminal disappeared")?;
        if self
            .pending_conversation_launches
            .contains_key(&terminal_id)
        {
            return Err("fork terminal already has a queued launch".into());
        }
        if self.agent_info(ws, pane_id).is_some() {
            return Err("fork terminal already hosts an agent".into());
        }
        if self.terminal_runtimes.get(&terminal_id).is_none() {
            return Err("new fork terminal has no runtime".into());
        }
        let mut argv =
            vec![crate::detect::interactive_agent_executable(crate::detect::Agent::Pi).to_string()];
        argv.extend(args.iter().cloned());
        self.pending_conversation_launches.insert(
            terminal_id.clone(),
            PendingConversationLaunch {
                name,
                args,
                deadline: Instant::now() + LAUNCH_DEADLINE,
            },
        );
        let Some(agent) = self.agent_info(ws, pane_id) else {
            self.pending_conversation_launches.remove(&terminal_id);
            return Err("new fork terminal disappeared".into());
        };
        self.schedule_conversation_launch_retry(terminal_id);
        self.emit_pane_updated(ws, pane_id);
        Ok((agent, argv))
    }

    fn schedule_conversation_launch_retry(&self, terminal_id: TerminalId) {
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(RETRY_INTERVAL).await;
            let _ = tx
                .send(AppEvent::ConversationLaunchRetry { terminal_id })
                .await;
        });
    }

    fn conversation_launch_location(&self, terminal_id: &TerminalId) -> Option<(usize, PaneId)> {
        self.state
            .workspaces
            .iter()
            .enumerate()
            .find_map(|(ws_idx, ws)| {
                ws.tabs.iter().find_map(|tab| {
                    tab.layout
                        .pane_ids()
                        .into_iter()
                        .find(|pane_id| tab.terminal_id(*pane_id) == Some(terminal_id))
                        .map(|pane_id| (ws_idx, pane_id))
                })
            })
    }

    pub(super) fn handle_conversation_launch_retry(&mut self, terminal_id: TerminalId) {
        self.retry_conversation_launch_with(
            terminal_id,
            Instant::now(),
            |runtime| available_shell_name(runtime).is_some(),
            |app, params| app.start_agent(params),
        );
    }

    // Injectable clock/readiness/submission keep the no-retry boundary testable
    // without real sessions, sleeps, or PTYs.
    fn retry_conversation_launch_with(
        &mut self,
        terminal_id: TerminalId,
        now: Instant,
        ready: impl FnOnce(&TerminalRuntime) -> bool,
        submit: impl FnOnce(
            &mut App,
            AgentStartParams,
        ) -> Result<(AgentInfo, Vec<String>), AgentStartError>,
    ) {
        let Some(pending) = self.pending_conversation_launches.get(&terminal_id) else {
            return; // Closed/cancelled/completed launches and late events are inert.
        };
        if now >= pending.deadline {
            self.cancel_conversation_launch(&terminal_id, "timed out waiting for the new shell");
            return;
        }
        let Some((ws, pane_id)) = self.conversation_launch_location(&terminal_id) else {
            self.cancel_conversation_launch(&terminal_id, "target pane was closed or detached");
            return;
        };
        let Some(terminal) = self.state.terminals.get(&terminal_id) else {
            self.cancel_conversation_launch(&terminal_id, "target terminal disappeared");
            return;
        };
        if terminal.is_agent_terminal() || terminal.managed_agent_kind().is_some() {
            self.cancel_conversation_launch(
                &terminal_id,
                "another agent took over the target terminal",
            );
            return;
        }
        let Some(runtime) = self.terminal_runtimes.get(&terminal_id) else {
            self.cancel_conversation_launch(&terminal_id, "target terminal runtime disappeared");
            return;
        };
        if !ready(runtime) {
            // A shell rc may itself run foreground programs. Never type through
            // them; wait for the shell alone, bounded by the deadline above.
            self.schedule_conversation_launch_retry(terminal_id);
            return;
        }
        let Some(pane) = self.public_pane_id(ws, pane_id) else {
            self.cancel_conversation_launch(&terminal_id, "target pane disappeared");
            return;
        };
        // Remove BEFORE start_agent: this releases its reservation check and
        // ensures every result (including ambiguous InputFailed) is terminal.
        let Some(pending) = self.pending_conversation_launches.remove(&terminal_id) else {
            return;
        };
        let result = submit(
            self,
            AgentStartParams {
                name: pending.name,
                kind: "pi".into(),
                pane_id: pane,
                args: pending.args,
                timeout_ms: None,
            },
        );
        if let Err(error) = result {
            let error = self.agent_start_error_body(error);
            self.report_conversation_launch_failure(
                &terminal_id,
                &format!("{}: {}", error.code, error.message),
            );
        }
        self.emit_pane_updated(ws, pane_id);
    }

    fn cancel_conversation_launch(&mut self, terminal_id: &TerminalId, reason: &str) {
        self.pending_conversation_launches.remove(terminal_id);
        self.report_conversation_launch_failure(terminal_id, reason);
        if let Some((ws, pane)) = self.conversation_launch_location(terminal_id) {
            self.emit_pane_updated(ws, pane);
        }
    }

    fn report_conversation_launch_failure(&mut self, terminal_id: &TerminalId, reason: &str) {
        tracing::warn!(%terminal_id, reason, "Pi fork launch stopped; artifacts retained; source unchanged");
        // Neutral, silent report; retain every artifact and never touch the source.
        let _ = self.handle_api_request_after_internal_events_drained(crate::api::schema::Request {
            id: "conversation-launch-report".into(),
            method: Method::NotificationShow(NotificationShowParams {
                title: "Pi fork launch stopped".into(),
                body: Some(format!("{terminal_id}: {reason}. Fork files retained; original unchanged. Inspect the new pane before retrying.")),
                position: None,
                sound: Default::default(),
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::input::app_for_mouse_test, terminal::TerminalState, workspace::Workspace};

    fn fixture() -> (App, TerminalId, String) {
        let mut app = app_for_mouse_test();
        app.state.toast_config.delivery = crate::config::ToastDelivery::Off;
        let ws = Workspace::test_new("fork-test");
        let pane_id = ws.tabs[0].root_pane;
        let terminal_id = ws.terminal_id(pane_id).unwrap().clone();
        app.state.terminals.insert(
            terminal_id.clone(),
            TerminalState::new(terminal_id.clone(), "/tmp".into()),
        );
        app.terminal_runtimes.insert(
            terminal_id.clone(),
            TerminalRuntime::test_with_scrollback_bytes(40, 5, 1024, b""),
        );
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        let pane = app.public_pane_id(0, pane_id).unwrap();
        app.queue_conversation_launch(
            &pane,
            "fork-test".into(),
            vec!["--session".into(), "/fixture/fork.jsonl".into()],
        )
        .unwrap();
        (app, terminal_id, pane)
    }

    #[tokio::test]
    async fn conversation_launch_waits_for_slow_shell_and_exposes_reservation() {
        let (mut app, terminal_id, pane) = fixture();
        let info = app.agent_info_for_target(&pane).unwrap();
        assert!(info.launch_pending);
        assert!(!info.interactive_ready);
        assert_eq!(
            app.agent_info_for_target("fork-test").unwrap().terminal_id,
            terminal_id.to_string()
        );
        app.retry_conversation_launch_with(
            terminal_id.clone(),
            Instant::now(),
            |_| false,
            |_, _| panic!("startup child is still running"),
        );
        assert!(app.pending_conversation_launches.contains_key(&terminal_id));
        let err = app
            .start_agent(AgentStartParams {
                name: "other".into(),
                kind: "pi".into(),
                pane_id: pane,
                args: vec![],
                timeout_ms: None,
            })
            .unwrap_err();
        assert!(matches!(err, AgentStartError::TargetBusy(_)));
    }

    #[tokio::test]
    async fn conversation_launch_expiry_and_closed_target_never_submit() {
        for closed in [false, true] {
            let (mut app, terminal_id, _) = fixture();
            let now = if closed {
                app.state.workspaces.clear();
                Instant::now()
            } else {
                app.pending_conversation_launches[&terminal_id].deadline
            };
            app.retry_conversation_launch_with(
                terminal_id.clone(),
                now,
                |_| panic!("cancel before probing"),
                |_, _| panic!("cancel before submitting"),
            );
            assert!(!app.pending_conversation_launches.contains_key(&terminal_id));
        }
    }

    #[tokio::test]
    async fn conversation_launch_runtime_loss_or_agent_takeover_cancels() {
        for takeover in [false, true] {
            let (mut app, terminal_id, _) = fixture();
            if takeover {
                app.state
                    .terminals
                    .get_mut(&terminal_id)
                    .unwrap()
                    .detected_agent = Some(crate::detect::Agent::Pi);
            } else {
                app.terminal_runtimes.remove(&terminal_id);
            }
            app.retry_conversation_launch_with(
                terminal_id.clone(),
                Instant::now(),
                |_| panic!("cancel before readiness"),
                |_, _| panic!("cancel before submission"),
            );
            assert!(!app.pending_conversation_launches.contains_key(&terminal_id));
        }
    }

    #[tokio::test]
    async fn conversation_launch_rebinds_by_terminal_and_never_retries_submission_error() {
        let (mut app, terminal_id, _) = fixture();
        app.state.workspaces.insert(0, Workspace::test_new("other"));
        let expected = app
            .public_pane_id(1, app.state.workspaces[1].tabs[0].root_pane)
            .unwrap();
        let attempts = std::cell::Cell::new(0);
        app.retry_conversation_launch_with(
            terminal_id.clone(),
            Instant::now(),
            |_| true,
            |app, params| {
                attempts.set(attempts.get() + 1);
                assert_eq!(params.pane_id, expected);
                assert!(!app.pending_conversation_launches.contains_key(&terminal_id));
                Err(AgentStartError::InputFailed("delivery unknown".into()))
            },
        );
        app.retry_conversation_launch_with(
            terminal_id.clone(),
            Instant::now(),
            |_| panic!("late event"),
            |_, _| panic!("must not retry ambiguous submission"),
        );
        assert_eq!(attempts.get(), 1);
    }
}
