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
    /// Optional pane in this workspace. Records a manual-pull request only.
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
pub struct RoomReplyParams {
    pub workspace_id: String,
    pub request_sequence: u64,
    pub pane_id: String,
    pub terminal_id: String,
    pub session: String,
    pub text: String,
}
