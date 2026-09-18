//! Room API: saved human questions feed only the volatile polling inbox.
use super::responses::encode_success;
use crate::{
    api::schema::*,
    app::App,
    room_delivery::{now, Status},
};

fn encode_error(id: String, code: &str, message: String) -> String {
    super::responses::encode_error(id, code, message)
}

impl App {
    fn room_workspace_index(&self, id: &str) -> Option<usize> {
        // Exact stable identities only; ordinal navigation aliases can retarget.
        self.state.workspaces.iter().position(|ws| ws.id == id)
    }

    pub(crate) fn cleanup_room_delivery(&mut self, now: u64) {
        let current = self
            .state
            .workspaces
            .iter()
            .enumerate()
            .map(|(index, ws)| (ws.id.clone(), crate::room::members(&self.state, index)))
            .collect();
        self.room_delivery.cleanup(&current, now);
    }

    pub(super) fn handle_room_get(&mut self, id: String, params: WorkspaceTarget) -> String {
        self.cleanup_room_delivery(now());
        let Some(index) = self.room_workspace_index(&params.workspace_id) else {
            return encode_error(id, "workspace_not_found", "unknown workspace".into());
        };
        let ws = &self.state.workspaces[index];
        let members = crate::room::members(&self.state, index);
        encode_success(
            id,
            ResponseResult::RoomInfo {
                room_id: format!("room:{}", ws.id),
                workspace_id: ws.id.clone(),
                receivers: self.room_delivery.receivers(&ws.id, &members),
                deliveries: self.room_delivery.deliveries(&ws.id),
                members,
                next_sequence: ws.room.next_sequence,
                outbound_delivery: "pi_polling_at_most_once".into(),
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
        let timestamp = now();
        self.cleanup_room_delivery(timestamp);
        let Some(index) = self.room_workspace_index(&params.workspace_id) else {
            return encode_error(id, "workspace_not_found", "unknown workspace".into());
        };
        let members = crate::room::members(&self.state, index);
        let find = |members: &[crate::room::Member], target: &RoomRecipient| {
            members
                .iter()
                .find(|m| {
                    m.pane_id == target.pane_id
                        && m.terminal_id == target.terminal_id
                        && m.session.as_deref() == Some(target.session.as_str())
                })
                .cloned()
        };
        if let Some(list) = params.recipients.filter(|list| !list.is_empty()) {
            // Addressed group question: every named recipient must be a current member.
            let mut targets = Vec::with_capacity(list.len());
            for target in &list {
                match find(&members, target) {
                    Some(member) => targets.push(member),
                    None => {
                        return encode_error(
                            id,
                            "invalid_recipient",
                            format!("{} is not a current room member", target.pane_id),
                        )
                    }
                }
            }
            let mut seen = std::collections::HashSet::new();
            targets.retain(|m| seen.insert((m.terminal_id.clone(), m.session.clone())));
            if let Err(error) = self.room_delivery.check_capacity(targets.len()) {
                return encode_error(id, "room_delivery_full", error);
            }
            let mut candidate = self.state.workspaces[index].room.clone();
            return match candidate.post_to(params.text, targets.clone(), timestamp) {
                Ok(sequence) => self.finish_room_post_with(
                    id,
                    index,
                    candidate,
                    sequence,
                    targets,
                    crate::persist::save_checked,
                ),
                Err(error) => encode_error(id, "invalid_room_post", error),
            };
        }
        let targeted = params.recipient.is_some();
        let mut targets = match params.recipient {
            Some(target) => match members.into_iter().find(|m| {
                m.pane_id == target.pane_id
                    && m.terminal_id == target.terminal_id
                    && m.session.as_deref() == Some(target.session.as_str())
            }) {
                Some(member) => vec![member],
                None => {
                    return encode_error(
                        id,
                        "invalid_recipient",
                        "not a current room member".into(),
                    )
                }
            },
            None => members,
        };
        // Match the domain's per-terminal/session audience deduplication, even
        // if malformed/aliased pane state exposes the same session twice.
        let mut seen = std::collections::HashSet::new();
        targets.retain(|m| seen.insert((m.terminal_id.clone(), m.session.clone())));
        if let Err(error) = self.room_delivery.check_capacity(targets.len()) {
            return encode_error(id, "room_delivery_full", error);
        }
        let mut candidate = self.state.workspaces[index].room.clone();
        let audience = targets
            .iter()
            .filter(|m| m.session.is_some())
            .cloned()
            .collect();
        let posted = if targeted {
            candidate.post(params.text, targets.first().cloned(), timestamp)
        } else {
            candidate.post_to(params.text, audience, timestamp)
        };
        match posted {
            Ok(sequence) => self.finish_room_post_with(
                id,
                index,
                candidate,
                sequence,
                targets,
                crate::persist::save_checked,
            ),
            Err(error) => encode_error(id, "invalid_room_post", error),
        }
    }

    // Shared production/test transaction seam. No delivery exists until save succeeds.
    pub(crate) fn finish_room_post_with(
        &mut self,
        id: String,
        index: usize,
        candidate: crate::room::Room,
        sequence: u64,
        targets: Vec<crate::room::Member>,
        save: impl FnOnce(&crate::persist::SessionSnapshot) -> std::io::Result<()>,
    ) -> String {
        match self.save_room_candidate_with(index, candidate, save) {
            Ok(persistence) => {
                let ws = &self.state.workspaces[index];
                if let Some(message) = ws.room.messages.iter().find(|m| m.sequence == sequence) {
                    self.room_delivery.enqueue_saved(
                        &ws.id,
                        sequence,
                        &message.text,
                        targets,
                        message.created_unix,
                    );
                }
                self.room_written(id, index, sequence, persistence)
            }
            Err(error) => encode_error(id, "room_save_failed", error),
        }
    }

    pub(super) fn handle_room_agent_post(
        &mut self,
        id: String,
        params: RoomAgentPostParams,
    ) -> String {
        self.room_agent_post_with(id, params, crate::persist::save_checked)
    }

    pub(crate) fn room_agent_post_with(
        &mut self,
        id: String,
        params: RoomAgentPostParams,
        save: impl FnOnce(&crate::persist::SessionSnapshot) -> std::io::Result<()>,
    ) -> String {
        let Some(index) = self.room_workspace_index(&params.workspace_id) else {
            return encode_error(id, "workspace_not_found", "unknown workspace".into());
        };
        // Resolve authority before even an idempotent arrival lookup. Names and
        // agent labels always come from live membership, never caller claims.
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
        if params.arrival {
            if let Some(sequence) = self.state.workspaces[index].room.arrival_sequence(&member) {
                let persistence = if self.no_session {
                    "memory_only"
                } else {
                    "saved"
                };
                return self.room_written(id, index, sequence, persistence);
            }
        }
        let mut candidate = self.state.workspaces[index].room.clone();
        match candidate.post_agent(member, params.text, params.arrival, now()) {
            Ok(sequence) => match self.save_room_candidate_with(index, candidate, save) {
                Ok(persistence) => self.room_written(id, index, sequence, persistence),
                Err(error) => encode_error(id, "room_save_failed", error),
            },
            Err(error) => encode_error(id, "invalid_room_post", error),
        }
        // Deliberately no enqueue or delivery-status mutation for agent posts.
    }

    pub(super) fn handle_room_reply(&mut self, id: String, params: RoomReplyParams) -> String {
        self.cleanup_room_delivery(now());
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
        match candidate.reply(params.request_sequence, member.clone(), params.text, now()) {
            Ok(sequence) => match self.save_room_candidate(index, candidate) {
                Ok(persistence) => {
                    self.room_delivery.replied(
                        &params.workspace_id,
                        params.request_sequence,
                        &member,
                    );
                    self.room_written(id, index, sequence, persistence)
                }
                Err(error) => encode_error(id, "room_save_failed", error),
            },
            Err(error) => encode_error(id, "invalid_room_reply", error),
        }
    }

    fn room_written(
        &mut self,
        id: String,
        index: usize,
        sequence: u64,
        persistence: &str,
    ) -> String {
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        let workspace = &self.state.workspaces[index].id;
        let deliveries = self.room_delivery.deliveries(workspace);
        encode_success(
            id,
            ResponseResult::RoomWritten {
                room_id: format!("room:{workspace}"),
                sequence,
                persistence: persistence.into(),
                outbound_delivery: "pi_polling_at_most_once".into(),
                queued: deliveries
                    .iter()
                    .filter(|d| d.request_sequence == sequence && d.status == Status::Queued)
                    .count(),
                unavailable: deliveries
                    .iter()
                    .filter(|d| d.request_sequence == sequence && d.status == Status::Unavailable)
                    .count(),
            },
        )
    }

    pub(super) fn handle_room_delivery_register(
        &mut self,
        id: String,
        params: RoomDeliveryRegisterParams,
    ) -> String {
        let timestamp = now();
        self.cleanup_room_delivery(timestamp);
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
                "not a current room member/session".into(),
            );
        };
        match self.room_delivery.register(
            params.workspace_id,
            member,
            params.receiver_nonce,
            timestamp,
        ) {
            Ok(receiver_id) => encode_success(
                id,
                ResponseResult::RoomDeliveryRegistered {
                    receiver_id,
                    server_epoch: self.room_delivery.epoch.clone(),
                },
            ),
            Err(error) => encode_error(id, "invalid_room_receiver", error),
        }
    }

    pub(super) fn handle_room_delivery_claim(
        &mut self,
        id: String,
        params: RoomDeliveryClaimParams,
    ) -> String {
        let timestamp = now();
        self.cleanup_room_delivery(timestamp);
        match self.room_delivery.claim(
            &params.receiver_id,
            &params.server_epoch,
            params.ready,
            timestamp,
        ) {
            Ok(delivery) => encode_success(id, ResponseResult::RoomDeliveryClaimed { delivery }),
            Err(error) => encode_error(id, "invalid_room_receiver", error),
        }
    }

    pub(super) fn handle_room_delivery_report(
        &mut self,
        id: String,
        params: RoomDeliveryReportParams,
    ) -> String {
        self.cleanup_room_delivery(now());
        match self.room_delivery.report(
            &params.receiver_id,
            &params.server_epoch,
            &params.delivery_id,
            params.outcome,
            params.detail,
        ) {
            Ok(()) => encode_success(id, ResponseResult::RoomDeliveryReported { accepted: true }),
            Err(error) => encode_error(id, "invalid_room_delivery", error),
        }
    }
}
