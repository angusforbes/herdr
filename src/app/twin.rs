//! Background dispatch for pi-twin CLI actions triggered from the context menu.
//!
//! The CLI binary (`pi-twin`) talks to the Pi extension over its private IPC socket.
//! We invoke it without a shell and report results back through the App event channel so
//! errors appear in the config_diagnostic overlay.
//!
//! Bounded execution:
//! - stderr is capped at 16 KiB; any further output is discarded.
//! - The whole child process is given 45 s; on timeout the process is killed (kill_on_drop)
//!   and the outcome is reported as uncertain (no retry).

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt as _;

use crate::app::state::TwinAction;
use crate::app::App;
use crate::events::AppEvent;

/// Name of the external CLI binary.
const CLI: &str = "pi-twin";
/// Maximum bytes read from the child's stderr for error messages.
const STDERR_LIMIT: u64 = 16 * 1024;
/// Wall-clock budget for the whole child invocation (spawn → exit).
const TIMEOUT: Duration = Duration::from_secs(45);

impl App {
    /// Validate eligibility and launch the pi-twin CLI as an async tokio task.
    ///
    /// ## Revalidation
    ///
    /// The pane ID and session token were captured when the context menu opened.
    /// Before spawning, we re-check:
    /// - `twin=1` is still present on the same pane,
    /// - the `twin_session` token still matches the captured `session_id`
    ///   (detects session replacement on pane reuse — a bool-only check cannot catch this),
    /// - for Merge: `twin_parent=1` is still set (the pane is still a clone session).
    ///
    /// On failure a visible error appears in the config_diagnostic overlay.
    ///
    /// ## Focus (Merge only)
    ///
    /// For Merge the pane is focused before queuing the CLI so the merge-preview dialog
    /// is visible even when the user right-clicked a background tab.
    ///
    /// ## CLI arguments
    ///
    /// `pi-twin clone|merge --pane <id> --expected-session <uuid> --socket <path>`
    ///
    /// `--expected-session` lets the CLI refuse a stale connection if the pane was
    /// recycled between our validation and the CLI's own IPC handshake.
    /// `--socket` pins the CLI to this Herdr server's API socket, preventing
    /// cross-server confusion in environments with multiple running instances.
    pub(crate) fn spawn_twin_command(&mut self, action: TwinAction) {
        let (subcommand, public_pane_id, session_id, action_label, is_merge) = match action {
            TwinAction::Clone {
                public_pane_id,
                session_id,
            } => ("clone", public_pane_id, session_id, "Agent Split", false),
            TwinAction::Merge {
                public_pane_id,
                session_id,
            } => ("merge", public_pane_id, session_id, "Agent Merge", true),
        };

        // ── Revalidate ────────────────────────────────────────────────────────────────
        let resolved =
            self.parse_current_public_pane_id(&public_pane_id)
                .and_then(|(ws_idx, pane_id)| {
                    let ws = self.state.workspaces.get(ws_idx)?;
                    let tab_idx = ws.find_tab_index_for_pane(pane_id)?;
                    let pane_state = ws.tabs.get(tab_idx)?.panes.get(&pane_id)?;
                    let terminal = self.state.terminals.get(&pane_state.attached_terminal_id)?;
                    validate_twin_tokens(&terminal.metadata_tokens, &session_id, is_merge).ok()?;
                    Some((ws_idx, pane_id))
                });

        let (ws_idx, pane_id) = match resolved {
            Some(pair) => pair,
            None => {
                self.state.config_diagnostic = Some(format!(
                    "{action_label}: pane {public_pane_id} session has changed \
                     or is no longer a valid live-clone session"
                ));
                self.config_diagnostic_deadline =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
                return;
            }
        };

        // Focus only after revalidation, never using a stale menu's workspace index.
        if is_merge {
            self.focus_pane_internal_via_api(ws_idx, pane_id);
        }

        // ── Build argv and spawn ──────────────────────────────────────────────────────
        let socket = crate::api::socket_path().to_string_lossy().into_owned();
        let event_tx = self.event_tx.clone();
        let action_owned = action_label.to_string();

        tokio::spawn(async move {
            let result = tokio::time::timeout(
                TIMEOUT,
                run_cli(subcommand, &public_pane_id, &session_id, &socket),
            )
            .await;

            let outcome = match result {
                Err(_elapsed) => Err(format!(
                    "timed out after {}s — outcome uncertain, no retry",
                    TIMEOUT.as_secs()
                )),
                Ok(inner) => inner,
            };

            let _ = event_tx
                .send(AppEvent::TwinCommandFinished {
                    action: action_owned,
                    result: outcome,
                })
                .await;
        });
    }
}

/// Spawn the CLI child and wait for it, returning `Ok(())` on success.
async fn run_cli(
    subcommand: &str,
    pane_id: &str,
    session_id: &str,
    socket: &str,
) -> Result<(), String> {
    let argv = twin_argv(subcommand, pane_id, session_id, socket);
    let mut child = tokio::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("`{CLI}` not found — install the pi-twin package")
            } else {
                format!("could not launch `{CLI}`: {e}")
            }
        })?;

    let stderr_buf = match child.stderr.take() {
        Some(stderr) => read_bounded_stderr(stderr).await,
        None => Vec::new(),
    };

    let status = child.wait().await.map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&stderr_buf).trim().to_string();
        Err(if detail.is_empty() {
            format!("exited with {status}")
        } else {
            format!("{status}: {detail}")
        })
    }
}

/// Retain a bounded prefix while draining the rest without closing the pipe early.
async fn read_bounded_stderr(mut reader: impl tokio::io::AsyncRead + Unpin) -> Vec<u8> {
    let mut prefix = Vec::new();
    let _ = (&mut reader)
        .take(STDERR_LIMIT)
        .read_to_end(&mut prefix)
        .await;
    let _ = tokio::io::copy(&mut reader, &mut tokio::io::sink()).await;
    prefix
}

/// Build the exact argv used by production and exercised by unit tests.
pub(super) fn twin_argv(
    subcommand: &str,
    pane_id: &str,
    session_id: &str,
    socket: &str,
) -> [String; 8] {
    [
        CLI.to_string(),
        subcommand.to_string(),
        "--pane".to_string(),
        pane_id.to_string(),
        "--expected-session".to_string(),
        session_id.to_string(),
        "--socket".to_string(),
        socket.to_string(),
    ]
}

/// Production validation shared with tests: tokens must match the captured session.
pub(super) fn validate_twin_tokens(
    tokens: &crate::metadata_tokens::MetadataTokens,
    captured_session_id: &str,
    is_merge: bool,
) -> Result<(), &'static str> {
    if tokens.value("twin") != Some("1") {
        return Err("twin token no longer present");
    }
    let current = tokens.value("twin_session").unwrap_or("");
    if current.is_empty() || current != captured_session_id {
        return Err("twin_session token changed since menu was opened");
    }
    if is_merge && tokens.value("twin_parent") != Some("1") {
        return Err("twin_parent token no longer present (pane is not a clone session)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Instant;

    use super::*;
    use crate::metadata_tokens::MetadataTokens;

    fn tokens_from(pairs: &[(&str, &str)]) -> MetadataTokens {
        let mut t = MetadataTokens::default();
        let patch: HashMap<String, Option<String>> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Some(v.to_string())))
            .collect();
        t.patch(patch, None, Instant::now());
        t
    }

    #[tokio::test]
    async fn stderr_is_capped_but_drained_to_eof() {
        use tokio::io::AsyncWriteExt as _;
        let (reader, mut writer) = tokio::io::duplex(1024);
        let producer = tokio::spawn(async move {
            writer.write_all(&vec![b'x'; 128 * 1024]).await.unwrap();
        });
        let prefix = tokio::time::timeout(Duration::from_secs(2), read_bounded_stderr(reader))
            .await
            .unwrap();
        producer.await.unwrap();
        assert_eq!(prefix, vec![b'x'; STDERR_LIMIT as usize]);
    }

    // ── argv shape ────────────────────────────────────────────────────────────────────

    #[test]
    fn argv_clone_has_expected_shape() {
        let argv = twin_argv("clone", "w1:p3", "uuid-abc", "/run/herdr.sock");
        assert_eq!(argv[0], "pi-twin");
        assert_eq!(argv[1], "clone");
        assert_eq!(argv[2], "--pane");
        assert_eq!(argv[3], "w1:p3");
        assert_eq!(argv[4], "--expected-session");
        assert_eq!(argv[5], "uuid-abc");
        assert_eq!(argv[6], "--socket");
        assert_eq!(argv[7], "/run/herdr.sock");
    }

    #[test]
    fn argv_merge_uses_merge_subcommand() {
        let argv = twin_argv("merge", "w2:p5", "uuid-xyz", "/run/herdr.sock");
        assert_eq!(argv[1], "merge");
        assert_eq!(argv[3], "w2:p5");
        assert_eq!(argv[5], "uuid-xyz");
    }

    // ── token validation ──────────────────────────────────────────────────────────────

    #[test]
    fn validation_accepts_valid_original_session() {
        let t = tokens_from(&[("twin", "1"), ("twin_session", "my-session-id")]);
        assert!(validate_twin_tokens(&t, "my-session-id", false).is_ok());
    }

    #[test]
    fn validation_accepts_valid_clone_session_for_merge() {
        let t = tokens_from(&[
            ("twin", "1"),
            ("twin_session", "my-session-id"),
            ("twin_parent", "1"),
        ]);
        assert!(validate_twin_tokens(&t, "my-session-id", true).is_ok());
    }

    #[test]
    fn validation_rejects_when_twin_token_gone() {
        // Session was replaced — twin token removed.
        let t = tokens_from(&[("twin_session", "my-session-id")]);
        let err = validate_twin_tokens(&t, "my-session-id", false).unwrap_err();
        assert!(err.contains("twin token"), "err: {err}");
    }

    #[test]
    fn validation_rejects_wrong_session_id() {
        let t = tokens_from(&[("twin", "1"), ("twin_session", "new-session")]);
        let err = validate_twin_tokens(&t, "old-session", false).unwrap_err();
        assert!(err.contains("session"), "err: {err}");
    }

    #[test]
    fn validation_rejects_empty_session_id_in_tokens() {
        // Extension published token before session was set.
        let t = tokens_from(&[("twin", "1"), ("twin_session", "")]);
        let err = validate_twin_tokens(&t, "some-id", false).unwrap_err();
        assert!(err.contains("session"), "err: {err}");
    }

    #[test]
    fn validation_rejects_merge_when_parent_token_gone() {
        // Clone session was superseded; twin_parent no longer published.
        let t = tokens_from(&[("twin", "1"), ("twin_session", "my-session-id")]);
        let err = validate_twin_tokens(&t, "my-session-id", true).unwrap_err();
        assert!(err.contains("twin_parent"), "err: {err}");
    }
}
