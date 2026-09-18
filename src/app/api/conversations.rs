//! Action-triggered transcript I/O shared by JSON API and runtime clients.
use std::{collections::HashMap, path::Path};

use crate::agent_resume::AgentSessionRefKind;
use crate::api::schema::{
    AgentConversationParams, AgentForkParams, AgentSessionInfo, ErrorBody,
    PaneInfo, PaneSplitParams, ResponseResult, SplitDirection, SuccessResponse,
};
use crate::app::App;
use crate::pi_conversation::{MessageRef, PiConversation};

use super::responses::{encode_error, encode_error_body, encode_success};

fn unsupported(message: &str) -> ErrorBody {
    ErrorBody {
        code: "conversation_unsupported".into(),
        message: message.into(),
    }
}

fn bound_pi_path<'a>(
    agent: Option<&str>,
    session: Option<&'a AgentSessionInfo>,
) -> Result<&'a Path, ErrorBody> {
    let session = session.ok_or_else(|| unsupported("target has no bound Pi session path"))?;
    if session.agent != "pi" || agent.is_some_and(|agent| agent != "pi") {
        return Err(unsupported(
            "conversation operations support only Pi sessions",
        ));
    }
    if session.kind != AgentSessionRefKind::Path {
        return Err(unsupported(
            "Pi session ID alone is unsupported; report the current session path",
        ));
    }
    let path = Path::new(&session.value);
    if !path.is_absolute() {
        return Err(unsupported("bound Pi session path must be absolute"));
    }
    Ok(path)
}

fn validate_bound_reference(
    conversation: &PiConversation,
    reference: &MessageRef,
) -> Result<(), String> {
    // Compare to the canonical bound file, never load a client-provided path.
    if conversation.path.to_str() != Some(reference.session_path.as_str()) {
        return Err(
            "target is now bound to a different Pi session; search again before branching".into(),
        );
    }
    conversation.validate(reference)
}

fn conversation_page(
    conversation: &PiConversation,
    params: &AgentConversationParams,
) -> Result<ResponseResult, String> {
    let limit = params.limit.unwrap_or(100);
    if !(1..=500).contains(&limit) {
        return Err("conversation limit must be between 1 and 500".into());
    }
    let mut offset = params.offset.unwrap_or(0);
    let mut messages = conversation.messages()?;
    if let Some(entry_id) = &params.around_entry_id {
        if params.query.is_some() {
            return Err("around_entry_id requires query to be omitted".into());
        }
        let selected = messages
            .iter()
            .position(|message| message.reference.entry_id == *entry_id)
            .ok_or("selected Pi message is gone; search again")?;
        offset = selected
            .saturating_sub(limit / 2)
            .min(messages.len().saturating_sub(limit));
    }
    if let Some(query) = &params.query {
        // Same literal smart-case semantics as PiConversation::search.
        let sensitive = query.chars().any(char::is_uppercase);
        let needle = if sensitive {
            query.clone()
        } else {
            query.to_lowercase()
        };
        messages.retain(|message| {
            !needle.is_empty()
                && if sensitive {
                    message.text.contains(&needle)
                } else {
                    message.text.to_lowercase().contains(&needle)
                }
        });
        messages.reverse();
    }
    let total = messages.len();
    let mut messages: Vec<_> = messages.into_iter().skip(offset).take(limit).collect();
    // Leave room for JSON escaping and identity metadata within the 2 MiB
    // runtime transport frame. Full source content is retained by the fork writer.
    let per_message = (512 * 1024 / messages.len().max(1)).min(8000);
    for message in &mut messages {
        message
            .text
            .retain(|c| !c.is_control() || matches!(c, '\n' | '\t'));
        if message.text.len() > per_message {
            let mut end = per_message;
            while !message.text.is_char_boundary(end) {
                end -= 1;
            }
            message.text.truncate(end);
            message
                .text
                .push_str("\n[excerpt truncated — fork preserves full message]");
            message.text_truncated = true;
        }
    }
    let end = offset.saturating_add(messages.len());
    let has_more = end < total;
    Ok(ResponseResult::AgentConversation {
        session_id: conversation.header["id"]
            .as_str()
            .ok_or("Pi session missing id")?
            .into(),
        session_path: conversation
            .path
            .to_str()
            .ok_or("Pi session path is not UTF-8")?
            .into(),
        messages,
        total,
        partial_tail: conversation.partial_tail,
        offset,
        limit,
        has_more,
        next_offset: has_more.then_some(end),
    })
}

impl App {
    fn bound_pi_conversation(&self, target: &str) -> Result<(PaneInfo, PiConversation), ErrorBody> {
        // Resolve terminal identity too, allowing persisted Pi refs after restore.
        // pane_info prefers live hook authority over persisted session identity.
        let resolved = self
            .resolve_terminal_target(target)
            .map_err(|err| self.agent_target_error_body(err))?;
        let pane = self
            .pane_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| unsupported("target pane is unavailable"))?;
        let path = bound_pi_path(pane.agent.as_deref(), pane.agent_session.as_ref())?;
        let conversation = PiConversation::load(path).map_err(|message| ErrorBody {
            code: "conversation_unavailable".into(),
            message,
        })?;
        Ok((pane, conversation))
    }

    pub(super) fn handle_agent_conversation(
        &mut self,
        id: String,
        params: AgentConversationParams,
    ) -> String {
        if !params.limit.is_none_or(|limit| (1..=500).contains(&limit)) {
            return encode_error(
                id,
                "invalid_params",
                "conversation limit must be between 1 and 500",
            );
        }
        if params.around_entry_id.is_some() && params.query.is_some() {
            return encode_error(
                id,
                "invalid_params",
                "around_entry_id requires query to be omitted",
            );
        }
        let (_, conversation) = match self.bound_pi_conversation(&params.target) {
            Ok(bound) => bound,
            Err(error) => return encode_error_body(id, error),
        };
        match conversation_page(&conversation, &params) {
            Ok(result) => encode_success(id, result),
            Err(message) => encode_error(id, "conversation_unavailable", message),
        }
    }

    pub(super) fn handle_agent_fork(&mut self, id: String, params: AgentForkParams) -> String {
        let (source, conversation) = match self.bound_pi_conversation(&params.target) {
            Ok(bound) => bound,
            Err(error) => return encode_error_body(id, error),
        };
        if let Err(message) = validate_bound_reference(&conversation, &params.reference) {
            return encode_error(id, "conversation_stale", message);
        }
        let Some(cwd) = conversation.header["cwd"]
            .as_str()
            .filter(|cwd| Path::new(cwd).is_dir())
        else {
            return encode_error(
                id,
                "conversation_cwd_unavailable",
                "Pi session cwd must be an existing directory; source was not changed",
            );
        };
        let prepared = match crate::pi_fork::prepare_fork(
            &params.reference,
            params.position,
            &crate::config::config_dir().join("conversation-forks"),
        ) {
            Ok(prepared) => prepared,
            Err(message) => return encode_error(id, "conversation_fork_failed", message),
        };
        let session_path = prepared.session_path.to_string_lossy().into_owned();
        let extension_path = prepared.extension_path.to_string_lossy().into_owned();
        let split = self.handle_pane_split(
            id.clone(),
            PaneSplitParams {
                workspace_id: None,
                target_pane_id: Some(source.pane_id.clone()),
                direction: SplitDirection::Right,
                ratio: None,
                cwd: Some(cwd.into()),
                focus: true,
                right_click: Default::default(),
                env: HashMap::new(),
            },
        );
        let pane = match serde_json::from_str::<SuccessResponse>(&split) {
            Ok(SuccessResponse { result: ResponseResult::PaneInfo { pane }, .. }) => pane,
            _ => return encode_error(id, "conversation_fork_split_failed", format!(
                "fork session_id={} session_path={} extension_path={} retained; source_pane={} unchanged; split response: {}",
                prepared.session_id, session_path, extension_path, source.pane_id, split,
            )),
        };
        let name = format!(
            "fork-{}",
            prepared
                .session_id
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(27)
                .collect::<String>()
                .to_ascii_lowercase()
        );
        let args = vec![
            "--session".into(),
            session_path.clone(),
            "-e".into(),
            extension_path.clone(),
        ];
        // A split has spawned a shell, not established that its startup is done.
        // Reserve its terminal identity and submit once on a later readiness event.
        let (agent, argv) = match self.queue_conversation_launch(&pane.pane_id, name, args) {
            Ok(queued) => queued,
            Err(message) => return encode_error(id, "conversation_fork_launch_failed", format!(
                "fork session_id={} session_path={} extension_path={} and new pane_id={} terminal_id={} retained; source_pane={} unchanged; launch not queued: {}",
                prepared.session_id, session_path, extension_path, pane.pane_id, pane.terminal_id, source.pane_id, message,
            )),
        };
        let pane = self
            .parse_pane_id(&pane.pane_id)
            .and_then(|(ws, pane_id)| self.pane_info(ws, pane_id))
            .unwrap_or(pane);
        encode_success(
            id,
            ResponseResult::AgentForked {
                session_id: prepared.session_id,
                session_path: session_path.clone(),
                session_ref: AgentSessionInfo {
                    source: "herdr:pi".into(),
                    agent: "pi".into(),
                    kind: AgentSessionRefKind::Path,
                    value: session_path,
                },
                extension_path,
                agent,
                pane,
                argv,
                launch_requested: true,
                ready: false,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pi_conversation::test_support as tempfile;

    fn fixture() -> (tempfile::TempDir, PiConversation) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut text = format!(
            "{}\n",
            serde_json::json!({"type":"session", "version":3, "id":"fixture", "cwd":dir.path()})
        );
        text.push_str("{\"type\":\"message\",\"id\":\"u1\",\"parentId\":null,\"message\":{\"role\":\"user\",\"content\":\"same needle\"}}\n");
        text.push_str("{\"type\":\"message\",\"id\":\"u2\",\"parentId\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"same needle\"}}\n{\"type\":");
        std::fs::write(&path, text).unwrap();
        let conversation = PiConversation::load(&path).unwrap();
        (dir, conversation)
    }

    #[test]
    fn unsupported_agent_and_id_only_fail_before_io() {
        let mut session = AgentSessionInfo {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: AgentSessionRefKind::Path,
            value: "/nonexistent/fixture.jsonl".into(),
        };
        assert!(bound_pi_path(Some("claude"), Some(&session)).is_err());
        assert!(bound_pi_path(Some("pi"), None).is_err());
        session.kind = AgentSessionRefKind::Id;
        assert!(bound_pi_path(Some("pi"), Some(&session))
            .unwrap_err()
            .message
            .contains("ID alone"));
        session.kind = AgentSessionRefKind::Path;
        assert!(bound_pi_path(Some("pi"), Some(&session)).is_ok());
    }

    #[test]
    fn pages_return_stable_messages_and_report_total_and_tail() {
        let (_dir, conversation) = fixture();
        let params = AgentConversationParams {
            target: "fixture".into(),
            query: Some("needle".into()),
            limit: Some(1),
            offset: None,
            around_entry_id: None,
        };
        let result =
            serde_json::to_value(conversation_page(&conversation, &params).unwrap()).unwrap();
        assert_eq!(result["total"], 2);
        assert_eq!(result["partial_tail"], true);
        assert_eq!(result["has_more"], true);
        assert_eq!(result["next_offset"], 1);
        assert_eq!(result["messages"][0]["reference"]["entry_id"], "u2");
        assert!(!result["messages"][0]["reference"]["ancestry_hash"]
            .as_str()
            .unwrap()
            .is_empty());
        let params = AgentConversationParams {
            query: None,
            offset: Some(1),
            ..params
        };
        let result = conversation_page(&conversation, &params).unwrap();
        let encoded = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serde_json::from_str::<ResponseResult>(&encoded).unwrap(),
            result
        );
        assert!(conversation_page(
            &conversation,
            &AgentConversationParams {
                limit: Some(501),
                ..params
            }
        )
        .is_err());
    }

    #[test]
    fn centered_page_finds_selection_beyond_first_500_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.jsonl");
        let mut text = format!(
            "{}\n",
            serde_json::json!({"type":"session", "version":3, "id":"large", "cwd":dir.path()})
        );
        for index in 0..650 {
            text.push_str(&format!("{}\n", serde_json::json!({"type":"message", "id":format!("u{index}"), "parentId":null, "message":{"role":"user", "content":"needle"}})));
        }
        std::fs::write(&path, text).unwrap();
        let conversation = PiConversation::load(&path).unwrap();
        let params = AgentConversationParams {
            target: "fixture".into(),
            query: None,
            limit: Some(100),
            offset: Some(0),
            around_entry_id: Some("u550".into()),
        };
        let result =
            serde_json::to_value(conversation_page(&conversation, &params).unwrap()).unwrap();
        assert_eq!(result["offset"], 500);
        assert_eq!(result["total"], 650);
        assert_eq!(result["messages"][50]["reference"]["entry_id"], "u550");
        assert_eq!(result["messages"].as_array().unwrap().len(), 100);
        assert!(conversation_page(
            &conversation,
            &AgentConversationParams {
                around_entry_id: Some("gone".into()),
                ..params
            }
        )
        .is_err());
    }

    #[test]
    fn stale_reference_or_rebound_target_is_rejected() {
        let (_dir, conversation) = fixture();
        let mut reference = conversation.reference("u1").unwrap();
        assert!(validate_bound_reference(&conversation, &reference).is_ok());
        reference.session_path.push_str("-different");
        assert!(validate_bound_reference(&conversation, &reference)
            .unwrap_err()
            .contains("different Pi session"));
        reference = conversation.reference("u1").unwrap();
        reference.ancestry_hash = "stale".into();
        assert!(validate_bound_reference(&conversation, &reference).is_err());
    }
}
