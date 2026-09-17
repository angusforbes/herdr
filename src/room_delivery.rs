//! Volatile, at-most-once room inbox. Never reconstructed from the transcript.
//! Only a committed human post enqueues work; claims mark before returning.
#[cfg(test)]
mod tests;
use crate::room::Member;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const RECEIVER_TTL: u64 = 15;
const MAX_RECEIVERS: usize = 1024;
const MAX_DELIVERIES: usize = 8192;
const RETENTION: u64 = 600;
const MAX_DETAIL: usize = 512;

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// UUIDv8 using existing SHA-256, process/time/counter entropy and std's random
// hash seed. These are correlation tokens on a trusted socket, not credentials.
fn uuid() -> String {
    use sha2::{Digest, Sha256};
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let seed = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let digest = Sha256::digest(format!(
        "{seed}:{}:{:?}:{}",
        std::process::id(),
        std::time::SystemTime::now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Claimed,
    Submitted,
    Replied,
    Unanswered,
    Failed,
    Unavailable,
    Expired,
}
impl Status {
    fn active(self) -> bool {
        matches!(self, Self::Claimed | Self::Submitted)
    }
    fn pending(self) -> bool {
        self == Self::Queued || self.active()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Delivery {
    pub delivery_id: String,
    pub workspace_id: String,
    pub request_sequence: u64,
    pub recipient: Member,
    pub text: String,
    pub expires_unix: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeliveryStatus {
    pub delivery_id: String,
    pub request_sequence: u64,
    pub recipient: Member,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReceiverStatus {
    pub member: Member,
    pub available: bool,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Submitted,
    Unanswered,
    Failed,
}

struct Receiver {
    id: String,
    workspace: String,
    member: Member,
    nonce: String,
    deadline: u64,
}
struct Entry {
    delivery: Delivery,
    receiver: Option<String>,
    status: Status,
    detail: Option<String>,
}

pub struct Runtime {
    pub epoch: String,
    receivers: Vec<Receiver>,
    entries: Vec<Entry>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            epoch: uuid(),
            receivers: Vec::new(),
            entries: Vec::new(),
        }
    }
}

pub fn same_identity(a: &Member, b: &Member) -> bool {
    a.pane_id == b.pane_id
        && a.terminal_id == b.terminal_id
        && a.session == b.session
        && a.agent == b.agent
}

impl Runtime {
    /// Sweep only on API/presentation refresh, never in per-pane render paths.
    pub fn cleanup(&mut self, current: &HashMap<String, Vec<Member>>, now: u64) {
        self.receivers.retain(|r| {
            r.deadline > now
                && current
                    .get(&r.workspace)
                    .is_some_and(|members| members.iter().any(|m| same_identity(m, &r.member)))
        });
        self.entries.retain(|entry| {
            current.contains_key(&entry.delivery.workspace_id)
                && now < entry.delivery.expires_unix.saturating_add(RETENTION)
        });
        for entry in &mut self.entries {
            if !entry.status.pending() {
                continue;
            }
            if now >= entry.delivery.expires_unix {
                entry.status = Status::Expired;
                entry.detail = Some("request expired; never retried".into());
            } else if !self
                .receivers
                .iter()
                .any(|r| Some(&r.id) == entry.receiver.as_ref())
            {
                entry.status = if entry.status == Status::Queued {
                    Status::Unavailable
                } else {
                    Status::Failed
                };
                entry.detail = Some("receiver offline or identity changed; never retried".into());
            }
        }
    }

    pub fn register(
        &mut self,
        workspace: String,
        member: Member,
        nonce: String,
        now: u64,
    ) -> Result<String, String> {
        if member.session.is_none() || member.agent != "pi" {
            return Err("only live Pi sessions support room delivery".into());
        }
        if nonce.is_empty() || nonce.len() > 256 {
            return Err("receiver_nonce must contain 1..=256 bytes".into());
        }
        if let Some(receiver) = self.receivers.iter_mut().find(|r| {
            r.workspace == workspace && same_identity(&r.member, &member) && r.nonce == nonce
        }) {
            receiver.deadline = now.saturating_add(RECEIVER_TTL);
            return Ok(receiver.id.clone());
        }
        let replaced: Vec<_> = self
            .receivers
            .iter()
            .filter(|r| r.workspace == workspace && same_identity(&r.member, &member))
            .map(|r| r.id.clone())
            .collect();
        self.receivers.retain(|r| !replaced.contains(&r.id));
        for entry in &mut self.entries {
            if entry.status.pending()
                && entry
                    .receiver
                    .as_ref()
                    .is_some_and(|id| replaced.contains(id))
            {
                entry.status = if entry.status == Status::Queued {
                    Status::Unavailable
                } else {
                    Status::Failed
                };
                entry.detail = Some("receiver replaced; never retried".into());
            }
        }
        if self.receivers.len() >= MAX_RECEIVERS {
            return Err("room receiver limit reached".into());
        }
        let id = uuid();
        self.receivers.push(Receiver {
            id: id.clone(),
            workspace,
            member,
            nonce,
            deadline: now.saturating_add(RECEIVER_TTL),
        });
        Ok(id)
    }

    pub fn check_capacity(&self, targets: usize) -> Result<(), String> {
        if self.entries.len().saturating_add(targets) > MAX_DELIVERIES {
            Err("room delivery history limit reached; entries are retained up to 20 minutes after posting, including completed deliveries".into())
        } else {
            Ok(())
        }
    }

    fn unavailable_reason(&self, workspace: &str, member: &Member) -> Option<String> {
        if member.session.is_none() {
            Some("no live session identity".into())
        } else if member.agent != "pi" {
            Some("unsupported agent (Pi receiver only)".into())
        } else if !self
            .receivers
            .iter()
            .any(|r| r.workspace == workspace && same_identity(&r.member, member))
        {
            Some("Pi receiver offline".into())
        } else {
            None
        }
    }

    /// Caller has saved first and reserved capacity in the same synchronous turn.
    pub fn enqueue_saved(
        &mut self,
        workspace: &str,
        sequence: u64,
        text: &str,
        targets: Vec<Member>,
        now: u64,
    ) {
        for recipient in targets {
            let detail = self.unavailable_reason(workspace, &recipient);
            let receiver = self
                .receivers
                .iter()
                .find(|r| r.workspace == workspace && same_identity(&r.member, &recipient))
                .map(|r| r.id.clone());
            self.entries.push(Entry {
                delivery: Delivery {
                    delivery_id: uuid(),
                    workspace_id: workspace.into(),
                    request_sequence: sequence,
                    recipient,
                    text: text.into(),
                    expires_unix: now.saturating_add(crate::room::REQUEST_TTL_SECONDS),
                },
                receiver,
                status: if detail.is_none() {
                    Status::Queued
                } else {
                    Status::Unavailable
                },
                detail,
            });
        }
    }

    fn validate(&self, id: &str, epoch: &str) -> Result<(), String> {
        if epoch != self.epoch || !self.receivers.iter().any(|r| r.id == id) {
            Err("invalid or expired room receiver; register current session".into())
        } else {
            Ok(())
        }
    }

    // App calls cleanup with live identity immediately before every operation.
    pub fn claim(
        &mut self,
        id: &str,
        epoch: &str,
        ready: bool,
        now: u64,
    ) -> Result<Option<Delivery>, String> {
        self.validate(id, epoch)?;
        if let Some(receiver) = self.receivers.iter_mut().find(|r| r.id == id) {
            receiver.deadline = now.saturating_add(RECEIVER_TTL);
        }
        if !ready
            || self
                .entries
                .iter()
                .any(|e| e.receiver.as_deref() == Some(id) && e.status.active())
        {
            return Ok(None);
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.receiver.as_deref() == Some(id) && e.status == Status::Queued)
        {
            entry.status = Status::Claimed;
            return Ok(Some(entry.delivery.clone()));
        }
        Ok(None)
    }

    pub fn report(
        &mut self,
        id: &str,
        epoch: &str,
        delivery: &str,
        outcome: Outcome,
        detail: Option<String>,
    ) -> Result<(), String> {
        self.validate(id, epoch)?;
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.delivery.delivery_id == delivery && e.receiver.as_deref() == Some(id))
            .ok_or("unknown delivery for this receiver")?;
        if entry.status == Status::Queued {
            return Err("delivery has not been claimed".into());
        }
        if entry.status.active() {
            entry.status = match outcome {
                Outcome::Submitted => Status::Submitted,
                Outcome::Unanswered => Status::Unanswered,
                Outcome::Failed => Status::Failed,
            };
            entry.detail = detail.map(|s| {
                s.chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_DETAIL)
                    .collect()
            });
        }
        Ok(()) // Terminal outcomes are idempotent and cannot resurrect an entry.
    }

    pub fn replied(&mut self, workspace: &str, sequence: u64, member: &Member) {
        for entry in &mut self.entries {
            if entry.delivery.workspace_id == workspace
                && entry.delivery.request_sequence == sequence
                // The domain correlates replies by terminal/session; a pane
                // move inside this workspace must not hide a persisted reply.
                && entry.delivery.recipient.terminal_id == member.terminal_id
                && entry.delivery.recipient.session == member.session
            {
                entry.status = Status::Replied;
                entry.detail = None;
            }
        }
    }

    pub fn deliveries(&self, workspace: &str) -> Vec<DeliveryStatus> {
        self.entries
            .iter()
            .filter(|e| e.delivery.workspace_id == workspace)
            .map(|e| DeliveryStatus {
                delivery_id: e.delivery.delivery_id.clone(),
                request_sequence: e.delivery.request_sequence,
                recipient: e.delivery.recipient.clone(),
                status: e.status,
                detail: e.detail.clone(),
            })
            .collect()
    }
    pub fn receivers(&self, workspace: &str, members: &[Member]) -> Vec<ReceiverStatus> {
        members
            .iter()
            .map(|member| {
                let detail = self.unavailable_reason(workspace, member);
                ReceiverStatus {
                    member: member.clone(),
                    available: detail.is_none(),
                    detail,
                }
            })
            .collect()
    }
}
