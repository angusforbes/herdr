use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomReadParams {
    pub workspace_id: String,
    #[serde(default)]
    pub after_sequence: u64,
    /// Bounded page; defaults to 100, maximum 100.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomPostParams {
    pub workspace_id: String,
    pub text: String,
    /// Exact current recipient; omitted means all current members, captured once.
    #[serde(default)]
    pub recipient: Option<RoomRecipient>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomRecipient {
    pub pane_id: String,
    pub terminal_id: String,
    pub session: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomAgentPostParams {
    pub workspace_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub session: String,
    pub text: String,
    /// Ignore text and record one deterministic arrival per agent/session/room.
    #[serde(default)]
    pub arrival: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomReplyParams {
    pub workspace_id: String,
    pub request_sequence: u64,
    pub pane_id: String,
    pub terminal_id: String,
    pub session: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomDeliveryRegisterParams {
    pub workspace_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub session: String,
    pub receiver_nonce: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomDeliveryClaimParams {
    pub receiver_id: String,
    pub server_epoch: String,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomDeliveryReportParams {
    pub receiver_id: String,
    pub server_epoch: String,
    pub delivery_id: String,
    pub outcome: crate::room_delivery::Outcome,
    #[serde(default)]
    pub detail: Option<String>,
}
