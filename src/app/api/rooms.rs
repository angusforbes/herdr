//! Passive room API. Deliberately has no agent.prompt or terminal input calls.
use super::responses::encode_success;
use crate::{api::schema::*, app::App};

fn encode_error(id: String, code: &str, message: String) -> String {
    super::responses::encode_error(id, code, message)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl App {
    fn room_workspace_index(&self, id: &str) -> Option<usize> {
        // Room writes must not retarget after workspace reordering. Unlike
        // terminal navigation, this API accepts only exact stable identities.
        self.state.workspaces.iter().position(|ws| ws.id == id)
    }

    pub(super) fn handle_room_get(&mut self, id: String, params: WorkspaceTarget) -> String {
        let Some(index) = self.room_workspace_index(&params.workspace_id) else {
            return encode_error(id, "workspace_not_found", "unknown workspace".into());
        };
        let ws = &self.state.workspaces[index];
        encode_success(
            id,
            ResponseResult::RoomInfo {
                room_id: format!("room:{}", ws.id),
                workspace_id: ws.id.clone(),
                members: crate::room::members(&self.state, index),
                next_sequence: ws.room.next_sequence,
                outbound_delivery: "disabled_manual_pull_only".into(),
            },
        )
    }

    pub(super) fn handle_room_read(&mut self, id: String, params: RoomReadParams) -> String {
        let Some(index) = self.room_workspace_index(&params.workspace_id) else {
            return encode_error(id, "workspace_not_found", "unknown workspace".into());
        };
        let ws = &self.state.workspaces[index];
        encode_success(
            id,
            ResponseResult::RoomMessages {
                room_id: format!("room:{}", ws.id),
                next_sequence: ws.room.next_sequence,
                messages: ws
                    .room
                    .messages
                    .iter()
                    .filter(|m| m.sequence > params.after_sequence)
                    .take(params.limit.unwrap_or(100).min(100))
                    .cloned()
                    .collect(),
            },
        )
    }

    pub(super) fn handle_room_post(&mut self, id: String, params: RoomPostParams) -> String {
        let Some(index) = self.room_workspace_index(&params.workspace_id) else {
            return encode_error(id, "workspace_not_found", "unknown workspace".into());
        };
        let recipient = match params.recipient {
            Some(target) => match crate::room::members(&self.state, index)
                .into_iter()
                .find(|m| {
                    m.pane_id == target.pane_id
                        && m.terminal_id == target.terminal_id
                        && m.session.as_deref() == Some(target.session.as_str())
                }) {
                Some(member) => Some(member),
                None => {
                    return encode_error(
                        id,
                        "invalid_recipient",
                        "not a current room member".into(),
                    )
                }
            },
            None => None,
        };
        let mut candidate = self.state.workspaces[index].room.clone();
        match candidate.post(params.text, recipient, now()) {
            Ok(sequence) => self.finish_room_write(id, index, candidate, sequence),
            Err(error) => encode_error(id, "invalid_room_post", error),
        }
    }

    pub(super) fn handle_room_reply(&mut self, id: String, params: RoomReplyParams) -> String {
        let Some(index) = self.room_workspace_index(&params.workspace_id) else {
            return encode_error(id, "workspace_not_found", "unknown workspace".into());
        };
        let member = crate::room::members(&self.state, index)
            .into_iter()
            .find(|m| {
                m.pane_id == params.pane_id
                    && m.terminal_id == params.terminal_id
                    && m.session.as_deref() == Some(params.session.as_str())
            });
        let Some(member) = member else {
            return encode_error(
                id,
                "invalid_recipient",
                "session is not currently a member of this room".into(),
            );
        };
        let mut candidate = self.state.workspaces[index].room.clone();
        match candidate.reply(params.request_sequence, member, params.text, now()) {
            Ok(sequence) => self.finish_room_write(id, index, candidate, sequence),
            Err(error) => encode_error(id, "invalid_room_reply", error),
        }
    }

    fn finish_room_write(
        &mut self,
        id: String,
        index: usize,
        candidate: crate::room::Room,
        sequence: u64,
    ) -> String {
        match self.save_room_candidate(index, candidate) {
            Ok(persistence) => {
                self.render_dirty.request_generic();
                self.render_notify.notify_one();
                encode_success(
                    id,
                    ResponseResult::RoomWritten {
                        room_id: format!("room:{}", self.state.workspaces[index].id),
                        sequence,
                        persistence: persistence.into(),
                        outbound_delivery: "disabled_manual_pull_only".into(),
                    },
                )
            }
            Err(error) => encode_error(id, "room_save_failed", error),
        }
    }
}
