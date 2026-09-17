//! Shared room data. No PTY, inference, or delivery side effects live here.
use serde::{Deserialize, Serialize};

pub const MAX_MESSAGE_BYTES: usize = 8192;
pub const MAX_MESSAGES: usize = 1000;
pub const REQUEST_TTL_SECONDS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Member {
    pub pane_id: String,
    pub terminal_id: String,
    pub agent: String,
    pub name: String,
    /// Only live hook session identity is reply-capable; restored metadata is not authority.
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Message {
    pub sequence: u64,
    pub text: String,
    pub created_unix: u64,
    /// None denotes the human owner, never a coordinator agent.
    pub author: Option<Member>,
    /// Legacy single-recipient representation, retained for saved transcripts.
    pub recipient: Option<Member>,
    /// Immutable audience captured when a human question is accepted.
    #[serde(default)]
    pub recipients: Vec<Member>,
    pub reply_to: Option<u64>,
    pub expires_unix: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Room {
    pub next_sequence: u64,
    pub messages: Vec<Message>,
}

impl Default for Room {
    fn default() -> Self {
        Self {
            next_sequence: 1,
            messages: Vec::new(),
        }
    }
}

impl Room {
    fn append(&mut self, mut message: Message) -> Result<u64, String> {
        if message.text.trim().is_empty() || message.text.len() > MAX_MESSAGE_BYTES {
            return Err(format!("text must contain 1..={MAX_MESSAGE_BYTES} bytes"));
        }
        if self.messages.len() >= MAX_MESSAGES {
            return Err("prototype room is full (1000 messages); nothing was discarded".into());
        }
        let next = self
            .next_sequence
            .checked_add(1)
            .ok_or("sequence exhausted")?;
        message.sequence = self.next_sequence;
        self.messages.push(message);
        self.next_sequence = next;
        Ok(next - 1)
    }

    pub fn post(
        &mut self,
        text: String,
        recipient: Option<Member>,
        now: u64,
    ) -> Result<u64, String> {
        if recipient
            .as_ref()
            .is_some_and(|member| member.session.is_none())
        {
            return Err("recipient has no live session identity".into());
        }
        let expires_unix = recipient
            .as_ref()
            .map(|_| now.saturating_add(REQUEST_TTL_SECONDS));
        self.append(Message {
            sequence: 0,
            text,
            created_unix: now,
            author: None,
            recipient,
            recipients: Vec::new(),
            reply_to: None,
            expires_unix,
        })
    }

    /// A group question is one transcript record, not one copy per recipient.
    pub fn post_to(
        &mut self,
        text: String,
        recipients: Vec<Member>,
        now: u64,
    ) -> Result<u64, String> {
        if recipients.iter().any(|member| member.session.is_none()) {
            return Err("recipient has no live session identity".into());
        }
        let mut audience: Vec<Member> = Vec::new();
        for member in recipients {
            if !audience.iter().any(|existing| {
                existing.terminal_id == member.terminal_id && existing.session == member.session
            }) {
                audience.push(member);
            }
        }
        let expires_unix = (!audience.is_empty()).then(|| now.saturating_add(REQUEST_TTL_SECONDS));
        self.append(Message {
            sequence: 0,
            text,
            created_unix: now,
            author: None,
            recipient: None,
            recipients: audience,
            reply_to: None,
            expires_unix,
        })
    }

    pub fn reply(
        &mut self,
        request: u64,
        member: Member,
        text: String,
        now: u64,
    ) -> Result<u64, String> {
        let original = self
            .messages
            .iter()
            .find(|m| m.sequence == request)
            .ok_or("unknown request")?;
        if original.recipient.is_none() && original.recipients.is_empty() {
            return Err("message is not an addressed request".into());
        }
        if original.author.is_some() || original.reply_to.is_some() {
            return Err("cannot reply to a reply".into());
        }
        if member.session.is_none()
            || !original
                .recipients
                .iter()
                .chain(original.recipient.iter())
                .any(|recipient| {
                    recipient.terminal_id == member.terminal_id
                        && recipient.session == member.session
                })
        {
            return Err("wrong recipient session".into());
        }
        let expires = original.expires_unix.ok_or("request missing expiry")?;
        if now >= expires {
            return Err("request expired".into());
        }
        if self.messages.iter().any(|m| {
            m.reply_to == Some(request)
                && m.author.as_ref().is_some_and(|author| {
                    author.terminal_id == member.terminal_id && author.session == member.session
                })
        }) {
            return Err("request already has a reply from this session".into());
        }
        // Capture the current author once. Future renames/moves never rewrite history.
        self.append(Message {
            sequence: 0,
            text,
            created_unix: now,
            author: Some(member),
            recipient: None,
            recipients: Vec::new(),
            reply_to: Some(request),
            expires_unix: None,
        })
    }
}

/// Derive current membership on demand outside rendering. Never use saved session
/// metadata to resurrect a binding, and never treat a plain shell as an agent.
pub fn members(state: &crate::app::AppState, ws_idx: usize) -> Vec<Member> {
    let Some(ws) = state.workspaces.get(ws_idx) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for tab in &ws.tabs {
        for (pane_id, pane) in &tab.panes {
            let Some(&public_number) = ws.public_pane_numbers.get(pane_id) else {
                // Malformed/unregistered panes have no valid public address.
                continue;
            };
            let Some(terminal) = state.terminals.get(&pane.attached_terminal_id) else {
                continue;
            };
            let Some(label) = terminal.effective_agent_label().map(str::to_string) else {
                continue;
            };
            let session = terminal
                .hook_authority
                .as_ref()
                .filter(|hook| hook.agent_label == label)
                .and_then(|hook| hook.session_ref.as_ref())
                .map(|session| {
                    let kind = match session.kind {
                        crate::agent_resume::AgentSessionRefKind::Path => "Path",
                        crate::agent_resume::AgentSessionRefKind::Id => "Id",
                    };
                    format!("{kind}:{}", session.value)
                });
            result.push(Member {
                pane_id: crate::workspace::public_pane_id_for_number(&ws.id, public_number),
                terminal_id: terminal.id.to_string(),
                name: terminal.agent_name.clone().unwrap_or_else(|| label.clone()),
                agent: label,
                session,
            });
        }
    }
    result.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn member() -> Member {
        Member {
            pane_id: "w1:p1".into(),
            terminal_id: "t1".into(),
            agent: "pi".into(),
            name: "Ada".into(),
            session: Some("session-a".into()),
        }
    }
    #[test]
    fn room_sequence_correlation_and_duplicate_reply() {
        let mut room = Room::default();
        assert_eq!(room.post("hello".into(), Some(member()), 100).unwrap(), 1);
        let mut wrong = member();
        wrong.session = Some("replacement".into());
        assert!(room.reply(1, wrong, "bad".into(), 101).is_err());
        assert!(room.reply(1, member(), "late".into(), 700).is_err());
        assert_eq!(room.reply(1, member(), "I decline".into(), 101).unwrap(), 2);
        assert!(room.reply(1, member(), "again".into(), 102).is_err());
        assert!(room.reply(2, member(), "fanout".into(), 102).is_err());
        assert_eq!(room.next_sequence, 3);
        let restored: Room = serde_json::from_str(&serde_json::to_string(&room).unwrap()).unwrap();
        assert_eq!(restored, room);
    }
    #[test]
    fn room_group_question_snapshots_audience_and_deduplicates_per_session() {
        let mut room = Room::default();
        let ada = member();
        let mut bob = member();
        bob.name = "Bob".into();
        bob.terminal_id = "t2".into();
        bob.session = Some("session-b".into());
        let request = room
            .post_to(
                "everyone?".into(),
                vec![ada.clone(), bob.clone(), ada.clone()],
                100,
            )
            .unwrap();
        assert_eq!(room.messages.len(), 1);
        assert_eq!(room.messages[0].recipients, vec![ada.clone(), bob.clone()]);
        assert!(room
            .reply(request, ada.clone(), "Ada here".into(), 101)
            .is_ok());
        assert!(room.reply(request, bob, "Bob here".into(), 102).is_ok());
        assert!(room.reply(request, ada, "duplicate".into(), 103).is_err());
        let mut newcomer = member();
        newcomer.terminal_id = "t3".into();
        assert!(room
            .reply(request, newcomer, "not addressed".into(), 104)
            .is_err());
        assert_eq!(room.messages.len(), 3);
        assert!(room.messages[1..]
            .iter()
            .all(|m| m.recipients.is_empty() && m.recipient.is_none()));
        let restored: Room = serde_json::from_str(&serde_json::to_string(&room).unwrap()).unwrap();
        assert_eq!(restored, room);
    }

    #[test]
    fn room_legacy_transcript_without_audience_still_accepts_its_recipient() {
        let mut room = Room::default();
        room.post("old request".into(), Some(member()), 100)
            .unwrap();
        let mut json = serde_json::to_value(room).unwrap();
        json["messages"][0]
            .as_object_mut()
            .unwrap()
            .remove("recipients");
        let mut restored: Room = serde_json::from_value(json).unwrap();
        assert!(restored
            .reply(1, member(), "legacy reply".into(), 101)
            .is_ok());
    }

    #[test]
    fn room_request_without_expiry_fails_closed() {
        let mut room = Room::default();
        room.post("question".into(), Some(member()), 0).unwrap();
        room.messages[0].expires_unix = None;
        assert_eq!(
            room.reply(1, member(), "reply".into(), 0).unwrap_err(),
            "request missing expiry"
        );
        assert_eq!(room.messages.len(), 1);
        assert_eq!(room.next_sequence, 2);
    }

    #[test]
    fn room_bounds_and_unaddressed_posts() {
        let mut room = Room::default();
        assert!(room.post(" ".into(), None, 0).is_err());
        assert!(room
            .post("x".repeat(MAX_MESSAGE_BYTES + 1), None, 0)
            .is_err());
        room.post("note".into(), None, 0).unwrap();
        assert!(room.reply(1, member(), "reply".into(), 1).is_err());
        assert_eq!(room.next_sequence, 2);
    }
}
