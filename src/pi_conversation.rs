//! Read-only Pi v3 transcript adapter. Terminal rows are never message identities.
//!
//! Reads happen on explicit search/preview/branch actions, never during rendering.
//! A partial final JSONL record is ignored; malformed complete records fail closed.
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub(crate) struct MessageRef {
    pub session_path: String,
    pub session_id: String,
    pub entry_id: String,
    /// Hash of the selected entry and its complete ancestry, not the growing file.
    pub ancestry_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub(crate) struct ConversationMessage {
    pub reference: MessageRef,
    pub parent_message_id: Option<String>,
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub text_truncated: bool,
    pub timestamp: String,
    pub depth: usize,
    pub active: bool,
    pub can_rewrite: bool,
    pub can_continue: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PiConversation {
    pub path: PathBuf,
    pub header: Value,
    pub entries: Vec<Value>,
    index: HashMap<String, usize>,
    hashes: HashMap<String, String>,
    pub partial_tail: bool,
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Pi entry missing {key}"))
}

impl PiConversation {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err("Pi session path must be absolute".into());
        }
        let path = path
            .canonicalize()
            .map_err(|e| format!("Pi session: {e}"))?;
        let file = File::open(&path).map_err(|e| format!("Pi session: {e}"))?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Pi session must be a regular file".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_SESSION_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_SESSION_BYTES {
            return Err("Pi session exceeds 64 MiB; not searched".into());
        }
        Self::parse(path, &bytes)
    }

    fn parse(path: PathBuf, bytes: &[u8]) -> Result<Self, String> {
        let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let partial_tail = complete < bytes.len();
        let text = std::str::from_utf8(&bytes[..complete]).map_err(|e| e.to_string())?;
        let mut lines = text.split('\n').filter(|s| !s.trim().is_empty());
        let header: Value = serde_json::from_str(lines.next().ok_or("empty Pi session")?)
            .map_err(|e| e.to_string())?;
        if header["type"] != "session" || header["version"] != 3 {
            return Err("message search requires Pi session format v3".into());
        }
        if field(&header, "id")?.len() > 128 {
            return Err("invalid Pi session ID".into());
        }
        let cwd = field(&header, "cwd")?;
        if !Path::new(cwd).is_absolute() {
            return Err("Pi session cwd must be absolute".into());
        }
        let mut entries = Vec::new();
        let mut index = HashMap::new();
        let mut hashes: HashMap<String, String> = HashMap::new();
        let header_hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&header).map_err(|e| e.to_string())?)
        );
        for line in lines {
            let entry: Value =
                serde_json::from_str(line).map_err(|e| format!("invalid Pi record: {e}"))?;
            if entry["type"] == "session" {
                return Err("duplicate Pi session header".into());
            }
            let id = field(&entry, "id")?.to_owned();
            if id.len() > 128 || id.chars().any(char::is_control) {
                return Err("invalid Pi entry ID".into());
            }
            if index.contains_key(&id) {
                return Err("duplicate Pi entry ID".into());
            }
            match entry.get("parentId") {
                Some(Value::Null) => {}
                Some(Value::String(parent)) if index.contains_key(parent) => {}
                _ => return Err("Pi entry has missing or forward parent".into()),
            }
            let parent_hash = entry["parentId"]
                .as_str()
                .and_then(|p| hashes.get(p))
                .unwrap_or(&header_hash);
            let mut hash = Sha256::new();
            hash.update(parent_hash.as_bytes());
            hash.update(b"\n");
            hash.update(serde_json::to_vec(&entry).map_err(|e| e.to_string())?);
            hashes.insert(id.clone(), format!("{:x}", hash.finalize()));
            index.insert(id, entries.len());
            entries.push(entry);
        }
        Ok(Self {
            path,
            header,
            entries,
            index,
            hashes,
            partial_tail,
        })
    }

    pub(crate) fn entry(&self, id: &str) -> Result<&Value, String> {
        self.index
            .get(id)
            .map(|i| &self.entries[*i])
            .ok_or_else(|| "that Pi message is gone".into())
    }

    pub(crate) fn ancestry(&self, id: &str) -> Result<Vec<&Value>, String> {
        let mut path = Vec::new();
        let mut entry = self.entry(id)?;
        loop {
            path.push(entry);
            match entry["parentId"].as_str() {
                Some(parent) => entry = self.entry(parent)?,
                None => break,
            }
        }
        path.reverse();
        Ok(path)
    }

    pub(crate) fn reference(&self, id: &str) -> Result<MessageRef, String> {
        let ancestry_hash = self
            .hashes
            .get(id)
            .ok_or("that Pi message is gone")?
            .clone();
        Ok(MessageRef {
            session_path: self.path.to_string_lossy().into_owned(),
            session_id: field(&self.header, "id")?.to_owned(),
            entry_id: id.into(),
            ancestry_hash,
        })
    }

    pub(crate) fn validate(&self, reference: &MessageRef) -> Result<(), String> {
        if self.reference(&reference.entry_id)? != *reference {
            return Err("Pi conversation changed; search again before branching".into());
        }
        Ok(())
    }

    pub(crate) fn messages(&self) -> Result<Vec<ConversationMessage>, String> {
        let active: HashSet<&str> = match self.entries.last().and_then(|e| e["id"].as_str()) {
            Some(id) => self
                .ancestry(id)?
                .into_iter()
                .filter_map(|e| e["id"].as_str())
                .collect(),
            None => HashSet::new(),
        };
        // Keep the nearest visible parent across model/tool/custom entries.
        let mut visible_ancestor: HashMap<&str, (&str, usize)> = HashMap::new();
        let mut messages = Vec::new();
        for entry in &self.entries {
            let id = field(entry, "id")?;
            let parent = entry["parentId"]
                .as_str()
                .and_then(|p| visible_ancestor.get(p))
                .copied();
            let role = entry["message"]["role"].as_str().unwrap_or("");
            if entry["type"] == "message" && matches!(role, "user" | "assistant") {
                let text = message_text(&entry["message"]);
                let depth = parent.map_or(0, |(_, d)| d + 1);
                visible_ancestor.insert(id, (id, depth));
                messages.push(ConversationMessage {
                    reference: self.reference(id)?,
                    parent_message_id: parent.map(|(p, _)| p.into()),
                    role: role.into(),
                    text,
                    text_truncated: false,
                    timestamp: entry["timestamp"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(100)
                        .collect(),
                    depth,
                    active: active.contains(id),
                    can_rewrite: role == "user" && plain_user_message(&entry["message"]),
                    can_continue: !has_tool_calls(&entry["message"])
                        && !matches!(
                            entry["message"]["stopReason"].as_str(),
                            Some("error" | "aborted")
                        ),
                });
            } else if let Some(parent) = parent {
                visible_ancestor.insert(id, parent);
            }
        }
        Ok(messages)
    }

    pub(crate) fn materialize_reference(
        &self,
        message: &mut ConversationMessage,
    ) -> Result<(), String> {
        message.reference = self.reference(&message.reference.entry_id)?;
        Ok(())
    }

    pub(crate) fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>, String> {
        let sensitive = query.chars().any(char::is_uppercase);
        let needle = if sensitive {
            query.into()
        } else {
            query.to_lowercase()
        };
        let mut hits = Vec::new();
        if needle.is_empty() {
            return Ok(hits);
        }
        for mut message in self.messages()?.into_iter().rev() {
            let text = if sensitive {
                message.text.clone()
            } else {
                message.text.to_lowercase()
            };
            if text.contains(&needle) {
                self.materialize_reference(&mut message)?;
                hits.push(message);
                if hits.len() >= limit {
                    break;
                }
            }
        }
        Ok(hits)
    }
}

pub(crate) fn message_text(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub(crate) fn plain_user_message(message: &Value) -> bool {
    message["role"] == "user"
        && match &message["content"] {
            Value::String(_) => true,
            Value::Array(blocks) => blocks.iter().all(|b| b["type"] == "text"),
            _ => false,
        }
}

fn has_tool_calls(message: &Value) -> bool {
    message["content"]
        .as_array()
        .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "toolCall"))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    pub(crate) struct TempDir(PathBuf);
    impl TempDir {
        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    pub(crate) fn tempdir() -> std::io::Result<TempDir> {
        let path = std::env::temp_dir().join(format!(
            "herdr-conversation-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        Ok(TempDir(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture(extra: &[Value]) -> Vec<u8> {
        let mut entries = vec![
            json!({"type":"session","version":3,"id":"session-1","cwd":"/tmp"}),
            json!({"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"repeated needle"}}),
            json!({"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"text","text":"answer"}]}}),
            json!({"type":"message","id":"u2","parentId":"a1","message":{"role":"user","content":"repeated needle"}}),
        ];
        entries.extend_from_slice(extra);
        entries
            .into_iter()
            .map(|e| format!("{e}\n"))
            .collect::<String>()
            .into_bytes()
    }
    fn parse(bytes: &[u8]) -> PiConversation {
        PiConversation::parse("/tmp/session.jsonl".into(), bytes).unwrap()
    }
    #[test]
    fn repeated_text_has_distinct_exact_identity_and_tree() {
        let c = parse(&fixture(&[]));
        let hits = c.search("needle", 3).unwrap();
        assert_eq!(hits.len(), 2);
        assert_ne!(hits[0].reference, hits[1].reference);
        assert_eq!(hits[0].parent_message_id.as_deref(), Some("a1"));
        assert_eq!(hits[0].depth, 2);
    }
    #[test]
    fn append_preserves_reference_but_ancestor_edit_invalidates() {
        let c = parse(&fixture(&[]));
        let r = c.reference("u2").unwrap();
        let newer = parse(&fixture(&[
            json!({"type":"message","id":"a2","parentId":"u2","message":{"role":"assistant","content":"later"}}),
        ]));
        assert!(newer.validate(&r).is_ok());
        let changed = String::from_utf8(fixture(&[]))
            .unwrap()
            .replace("answer", "different");
        assert!(parse(changed.as_bytes()).validate(&r).is_err());
    }
    #[test]
    fn partial_tail_ignored_malformed_complete_line_rejected() {
        let mut bytes = fixture(&[]);
        bytes.extend_from_slice(b"{\"type\":");
        let c = parse(&bytes);
        assert!(c.partial_tail);
        assert_eq!(c.entries.len(), 3);
        bytes.push(b'\n');
        assert!(PiConversation::parse("/tmp/s".into(), &bytes).is_err());
    }
    #[test]
    fn alternate_branch_is_searchable_but_not_active() {
        let c = parse(&fixture(&[
            json!({"type":"message","id":"u3","parentId":"a1","message":{"role":"user","content":"alternate"}}),
        ]));
        let msgs = c.messages().unwrap();
        assert!(
            !msgs
                .iter()
                .find(|m| m.reference.entry_id == "u2")
                .unwrap()
                .active
        );
        assert!(
            msgs.iter()
                .find(|m| m.reference.entry_id == "u3")
                .unwrap()
                .active
        );
    }
    #[test]
    fn unsafe_boundaries_and_images_are_capability_gated() {
        let c = parse(&fixture(&[
            json!({"type":"message","id":"tools","parentId":"u2","message":{"role":"assistant","content":[{"type":"toolCall","id":"call1"}]}}),
            json!({"type":"message","id":"image","parentId":"u1","message":{"role":"user","content":[{"type":"image","data":"not-real"}]}}),
        ]));
        let msgs = c.messages().unwrap();
        assert!(
            !msgs
                .iter()
                .find(|m| m.reference.entry_id == "tools")
                .unwrap()
                .can_continue
        );
        assert!(
            !msgs
                .iter()
                .find(|m| m.reference.entry_id == "image")
                .unwrap()
                .can_rewrite
        );
    }
}
