use serde::{Deserialize, Serialize};

use crate::pi_conversation::MessageRef;
use crate::pi_fork::BranchPosition;

/// Read the currently bound Pi v3 conversation. No arbitrary file paths are accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentConversationParams {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Page size: defaults to 100; must be between 1 and 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// With no query, center the page on this visible entry, overriding offset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub around_entry_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentForkParams {
    pub target: String,
    pub reference: MessageRef,
    #[serde(default = "default_branch_position")]
    pub position: BranchPosition,
}

fn default_branch_position() -> BranchPosition {
    BranchPosition::Rewrite
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{Method, Request};

    #[test]
    fn conversation_and_fork_requests_roundtrip() {
        let read: Request = serde_json::from_value(serde_json::json!({
            "id": "read", "method": "agent.conversation", "params": {"target": "pane-1"}
        }))
        .unwrap();
        assert!(
            matches!(&read.method, Method::AgentConversation(p) if p.limit.is_none() && p.offset.is_none())
        );
        assert_eq!(
            serde_json::from_str::<Request>(&serde_json::to_string(&read).unwrap()).unwrap(),
            read
        );
        let fork: Request = serde_json::from_value(serde_json::json!({
            "id": "fork", "method": "agent.fork", "params": {
                "target": "pane-1", "reference": {
                    "session_path": "/fixture/pi.jsonl", "session_id": "session-1",
                    "entry_id": "u1", "ancestry_hash": "hash"
                }
            }
        }))
        .unwrap();
        assert!(
            matches!(&fork.method, Method::AgentFork(p) if p.position == BranchPosition::Rewrite)
        );
        let encoded = serde_json::to_value(&fork).unwrap();
        assert_eq!(encoded["params"]["position"], "rewrite");
        assert_eq!(serde_json::from_value::<Request>(encoded).unwrap(), fork);
        assert_eq!(
            serde_json::to_value(BranchPosition::Continue).unwrap(),
            "continue"
        );
    }
}
