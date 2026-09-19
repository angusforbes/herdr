//! Explicit, read-only-source Pi v3 branching for the Pi 0.85.1 binary.
//! Preparation runs only on a user action, never in a pane/render loop.
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::pi_conversation::{plain_user_message, MessageRef, PiConversation};

const PREFILL_EXTENSION: &str = include_str!("integration/assets/pi/herdr-fork-prefill.ts");
static ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BranchPosition {
    Rewrite,
    Continue,
}

#[derive(Debug)]
pub(crate) struct PreparedFork {
    pub session_path: PathBuf,
    pub extension_path: PathBuf,
    pub session_id: String,
}

/// Materialize a fresh fork before launching `pi --session ... -e ...`.
/// A private ticket authorizes one draft prefill, never a prompt submission.
/// A private per-fork directory also supplies inherited Windows ACL protection.
pub(crate) fn prepare_fork(
    reference: &MessageRef,
    position: BranchPosition,
    output_dir: &Path,
) -> Result<PreparedFork, String> {
    let conversation = PiConversation::load(Path::new(&reference.session_path))?;
    conversation.validate(reference)?;
    let selected = conversation.entry(&reference.entry_id)?;
    if selected["type"] != "message" {
        return Err("branch selection must be a Pi message".into());
    }
    let mut ancestry = conversation.ancestry(&reference.entry_id)?;
    let (draft_content, draft_text) = match position {
        BranchPosition::Rewrite => {
            if !plain_user_message(&selected["message"]) {
                return Err(
                    "rewrite requires a text-only user message; images must be reattached manually"
                        .into(),
                );
            }
            let content = selected["message"]["content"].clone();
            let text = draft_text(&content)?;
            ancestry.pop();
            (content, text)
        }
        BranchPosition::Continue => (Value::Null, String::new()),
    };
    validate_context(&ancestry)?;
    let endpoint = ancestry.last().and_then(|v| v["id"].as_str());
    let session_id = fresh_identity();
    let launch_token = fresh_identity();
    let timestamp = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|e| e.to_string())?;
    let mut used: HashSet<String> = ancestry
        .iter()
        .filter_map(|v| v["id"].as_str().map(str::to_owned))
        .collect();
    let marker_id = unused_entry_id(&mut used);
    let mut records = vec![json!({
        "type": "session", "version": 3, "id": session_id,
        "timestamp": timestamp, "cwd": conversation.header["cwd"],
        "parentSession": conversation.path,
    })];
    records.extend(ancestry.iter().map(|v| (*v).clone()));
    records.push(json!({
        "type": "custom", "customType": "herdr.fork", "id": marker_id,
        "parentId": endpoint, "timestamp": timestamp,
        "data": {
            "schemaVersion": 1, "sessionId": session_id, "launchToken": launch_token,
            "source": reference, "position": position,
            "draftContent": draft_content, "draftText": draft_text,
        }
    }));
    // Preserve historical entries exactly, but don't advertise the source's old name.
    if ancestry.iter().any(|entry| entry["type"] == "session_info") {
        records.push(json!({
            "type": "session_info", "id": unused_entry_id(&mut used),
            "parentId": marker_id, "timestamp": timestamp, "name": "",
        }));
    }
    let mut bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut bytes, &record).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
    }
    // Validation and serialization finish before creating any artifacts.
    fs::create_dir_all(output_dir).map_err(|e| format!("create fork output directory: {e}"))?;
    let output_dir = output_dir.canonicalize().map_err(|e| e.to_string())?;
    let directory = output_dir.join(format!("herdr-fork-{session_id}"));
    crate::platform::create_remote_private_dir(&directory)
        .map_err(|e| format!("create private fork directory: {e}"))?;
    let session_path = directory.join(format!("{session_id}.jsonl"));
    let extension_path = directory.join(format!("herdr-fork-prefill-{session_id}.ts"));
    let mut created = Vec::new();
    let result = (|| {
        write_exclusive(&extension_path, PREFILL_EXTENSION.as_bytes(), &mut created)?;
        write_exclusive(&session_path, &bytes, &mut created)?;
        let ticket =
            serde_json::to_vec(&json!({"sessionId": session_id, "launchToken": launch_token}))
                .map_err(|e| e.to_string())?;
        write_exclusive(
            &directory.join("herdr-prefill-ticket.json"),
            &ticket,
            &mut created,
        )?;
        sync_directory(&directory)?;
        sync_directory(&output_dir)?;
        if let Some(parent) = output_dir.parent() {
            sync_directory(parent)?;
        }
        Ok(PreparedFork {
            session_path,
            extension_path,
            session_id,
        })
    })();
    if result.is_err() {
        // Never remove a pre-existing path, nor recursively remove unexpected files.
        for path in created.iter().rev() {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(&directory);
    }
    result
}

fn write_exclusive(path: &Path, bytes: &[u8], created: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut file = crate::platform::create_remote_ssh_config_file(path)
        .map_err(|e| format!("create fork artifact {}: {e}", path.display()))?;
    created.push(path.to_owned());
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("persist fork artifact {}: {e}", path.display()))
}

// Same portable directory-sync contract as detect/manifest_update.rs. Windows
// File::open cannot open a directory; file contents are still sync_all'd above.
fn sync_directory(path: &Path) -> Result<(), String> {
    match fs::File::open(path).and_then(|directory| directory.sync_all()) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::Unsupported
                    | std::io::ErrorKind::InvalidInput
                    | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "persist fork directory {}: {error}",
            path.display()
        )),
    }
}

fn draft_text(content: &Value) -> Result<String, String> {
    match content {
        Value::String(text) => Ok(text.clone()),
        Value::Array(blocks) => blocks
            .iter()
            .map(|block| {
                block["text"]
                    .as_str()
                    .ok_or_else(|| "user text block has no text".to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|texts| texts.join("")),
        _ => Err("unsupported user content".into()),
    }
}

/// Unlike native `position: at`, reject incomplete tool batches anywhere in the
/// copied history, not just at its leaf. Do not repair or fabricate tool results.
fn validate_context(entries: &[&Value]) -> Result<(), String> {
    let mut pending = HashSet::new();
    let mut seen_calls = HashSet::new();
    let mut ancestors = HashSet::new();
    let mut safe_starts = HashSet::new();
    for entry in entries {
        if pending.is_empty() {
            if let Some(id) = entry["id"].as_str() {
                safe_starts.insert(id);
            }
        }
        let kind = entry["type"].as_str().ok_or("Pi entry has no type")?;
        if kind == "compaction" {
            if !pending.is_empty() {
                return Err("compaction interrupts an unresolved tool batch".into());
            }
            let kept = entry["firstKeptEntryId"].as_str().ok_or(
                "Pi 0.85.1 requires firstKeptEntryId; retainedTail-only compactions are unsupported",
            )?;
            if !ancestors.contains(kept) {
                return Err("compaction retained entry is not in the fork ancestry".into());
            }
            if !safe_starts.contains(kept) {
                return Err("compaction retained boundary splits a tool batch".into());
            }
        }
        if (kind == "custom_message" || kind == "branch_summary") && !pending.is_empty() {
            return Err("context message interrupts an unresolved tool batch".into());
        }
        if kind == "message" {
            let message = &entry["message"];
            let role = message["role"].as_str().ok_or("Pi message has no role")?;
            if role == "toolResult" {
                let call = message["toolCallId"]
                    .as_str()
                    .ok_or("tool result has no call ID")?;
                if !pending.remove(call) {
                    return Err("orphan or duplicate tool result in fork ancestry".into());
                }
            } else {
                if !pending.is_empty() {
                    return Err(
                        "fork ancestry contains unresolved tool calls before another message"
                            .into(),
                    );
                }
                if role == "assistant" {
                    if matches!(
                        message["stopReason"].as_str(),
                        Some("error" | "aborted" | "pending")
                    ) {
                        return Err(
                            "fork ancestry contains an incomplete or failed assistant response"
                                .into(),
                        );
                    }
                    if let Some(blocks) = message["content"].as_array() {
                        for block in blocks {
                            if block["type"] == "toolCall" {
                                let id = block["id"]
                                    .as_str()
                                    .filter(|id| !id.is_empty())
                                    .ok_or("tool call has no ID")?;
                                if !seen_calls.insert(id) {
                                    return Err("duplicate tool call ID in fork ancestry".into());
                                }
                                pending.insert(id);
                            }
                        }
                    }
                    if message["stopReason"] == "toolUse" && pending.is_empty() {
                        return Err("toolUse response contains no tool calls".into());
                    }
                } else if !matches!(
                    role,
                    "user" | "bashExecution" | "custom" | "branchSummary" | "compactionSummary"
                ) {
                    return Err(format!("unsupported Pi message role: {role}"));
                }
            }
        }
        if let Some(id) = entry["id"].as_str() {
            ancestors.insert(id);
        }
    }
    if !pending.is_empty() {
        return Err(
            "selected cut leaves unresolved tool calls; select a completed response instead".into(),
        );
    }
    Ok(())
}

// UUID-shaped v4 identity, not a security credential. Sequence plus nanosecond
// time and process identity diversify inputs; exclusive creation is the final
// collision guard. No claim of cryptographic randomness is made for the token.
fn fresh_identity() -> String {
    let mut hash = Sha256::new();
    hash.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    hash.update(std::process::id().to_le_bytes());
    hash.update(ID_SEQUENCE.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let mut digest = hash.finalize();
    digest[6] = (digest[6] & 0x0f) | 0x40;
    digest[8] = (digest[8] & 0x3f) | 0x80;
    let hex = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn unused_entry_id(used: &mut HashSet<String>) -> String {
    loop {
        let id = fresh_identity()[..8].to_owned();
        if used.insert(id.clone()) {
            return id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        source: PathBuf,
        original: Vec<u8>,
    }
    impl Fixture {
        fn new(entries: Vec<Value>) -> Self {
            let root =
                std::env::temp_dir().join(format!("herdr-pi-fork-test-{}", fresh_identity()));
            fs::create_dir(&root).unwrap();
            let source = root.join("source.jsonl");
            let mut records =
                vec![json!({"type":"session","version":3,"id":"source-session","cwd":root})];
            records.extend(entries);
            let original = records
                .iter()
                .map(|v| format!("{v}\n"))
                .collect::<String>()
                .into_bytes();
            fs::write(&source, &original).unwrap();
            Self {
                root,
                source,
                original,
            }
        }
        fn reference(&self, id: &str) -> MessageRef {
            PiConversation::load(&self.source)
                .unwrap()
                .reference(id)
                .unwrap()
        }
        fn prepare(&self, id: &str, mode: BranchPosition) -> Result<PreparedFork, String> {
            prepare_fork(&self.reference(id), mode, &self.root.join("output"))
        }
        fn unchanged(&self) {
            assert_eq!(fs::read(&self.source).unwrap(), self.original);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn user(id: &str, parent: Option<&str>) -> Value {
        json!({"type":"message","id":id,"parentId":parent,"message":{"role":"user","content":"same text"},"unknown":{"preserve":true}})
    }
    fn assistant(id: &str, parent: &str) -> Value {
        json!({"type":"message","id":id,"parentId":parent,"message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"answer"}]}})
    }
    fn load(fork: &PreparedFork) -> PiConversation {
        PiConversation::load(&fork.session_path).unwrap()
    }
    fn marker(c: &PiConversation) -> &Value {
        c.entries
            .iter()
            .find(|v| v["customType"] == "herdr.fork")
            .unwrap()
    }

    #[test]
    fn root_rewrite_is_materialized_with_full_draft_and_fresh_identity() {
        let mut u = user("u", None);
        u["message"]["content"] =
            json!([{"type":"text","text":"first"},{"type":"text","text":"second"}]);
        let f = Fixture::new(vec![u.clone()]);
        let fork = f.prepare("u", BranchPosition::Rewrite).unwrap();
        let other = f.prepare("u", BranchPosition::Rewrite).unwrap();
        let c = load(&fork);
        assert_eq!(c.entries.len(), 1);
        assert_eq!(marker(&c)["parentId"], Value::Null);
        assert_eq!(marker(&c)["data"]["draftContent"], u["message"]["content"]);
        assert_eq!(marker(&c)["data"]["draftText"], "firstsecond");
        let ticket: Value = serde_json::from_slice(
            &fs::read(
                fork.session_path
                    .parent()
                    .unwrap()
                    .join("herdr-prefill-ticket.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(marker(&c)["data"]["launchToken"], ticket["launchToken"]);
        assert_eq!(ticket["sessionId"], fork.session_id);
        assert_eq!(c.header["id"], fork.session_id);
        assert_ne!(fork.session_id, other.session_id);
        assert_eq!(fork.session_id.len(), 36);
        assert_eq!(
            fs::read_to_string(&fork.extension_path).unwrap(),
            PREFILL_EXTENSION
        );
        f.unchanged();
    }

    #[test]
    fn exact_branch_not_chronological_prefix_and_stable_duplicate_text_ids() {
        let f = Fixture::new(vec![
            user("u", None),
            assistant("a", "u"),
            user("old", Some("a")),
            user("chosen", Some("a")),
            assistant("later", "chosen"),
        ]);
        let fork = f.prepare("chosen", BranchPosition::Continue).unwrap();
        let c = load(&fork);
        assert_eq!(
            c.entries
                .iter()
                .filter_map(|v| v["id"].as_str())
                .take(3)
                .collect::<Vec<_>>(),
            vec!["u", "a", "chosen"]
        );
        assert!(c.entry("old").is_err());
        assert!(c.entry("later").is_err());
        assert_eq!(marker(&c)["parentId"], "chosen");
        assert_eq!(
            c.entry("chosen").unwrap()["unknown"],
            json!({"preserve":true})
        );
        let rewrite = load(&f.prepare("chosen", BranchPosition::Rewrite).unwrap());
        assert!(rewrite.entry("chosen").is_err());
        assert_eq!(marker(&rewrite)["parentId"], "a");
        f.unchanged();
    }

    #[test]
    fn validation_and_image_rejection_create_no_artifacts() {
        let mut u = user("u", None);
        u["message"]["content"] = json!([{"type":"image","data":"fixture","mimeType":"image/png"}]);
        let f = Fixture::new(vec![u]);
        assert!(f.prepare("u", BranchPosition::Rewrite).is_err());
        let mut stale = f.reference("u");
        stale.ancestry_hash = "stale".into();
        assert!(prepare_fork(&stale, BranchPosition::Continue, &f.root.join("output")).is_err());
        assert!(!f.root.join("output").exists());
        f.unchanged();
    }

    #[test]
    fn compaction_checkpoint_gate_preserves_supported_raw_entry() {
        let old = json!({"type":"compaction","id":"c","parentId":"u","firstKeptEntryId":"u","summary":"s","details":{"unknown":1}});
        let f = Fixture::new(vec![user("u", None), old.clone(), user("next", Some("c"))]);
        assert_eq!(
            load(&f.prepare("next", BranchPosition::Continue).unwrap())
                .entry("c")
                .unwrap(),
            &old
        );
        let modern =
            json!({"type":"compaction","id":"c","parentId":"u","retainedTail":[],"summary":"s"});
        let f = Fixture::new(vec![user("u", None), modern, user("next", Some("c"))]);
        assert!(f.prepare("next", BranchPosition::Continue).is_err());
        assert!(!f.root.join("output").exists());
    }

    #[test]
    fn incomplete_batches_anywhere_in_ancestry_fail_but_completed_batch_passes() {
        let calls = json!({"type":"message","id":"calls","parentId":"u","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":"t1"},{"type":"toolCall","id":"t2"}]}});
        let result = |id: &str, parent: &str, call: &str| json!({"type":"message","id":id,"parentId":parent,"message":{"role":"toolResult","toolCallId":call,"content":[]}});
        let f = Fixture::new(vec![
            user("u", None),
            calls.clone(),
            result("r1", "calls", "t1"),
            user("bad", Some("r1")),
        ]);
        assert!(f.prepare("calls", BranchPosition::Continue).is_err());
        assert!(f.prepare("bad", BranchPosition::Continue).is_err());
        assert!(f.prepare("bad", BranchPosition::Rewrite).is_err());
        let f = Fixture::new(vec![
            user("u", None),
            calls,
            result("r1", "calls", "t1"),
            result("r2", "r1", "t2"),
            assistant("done", "r2"),
        ]);
        assert!(f.prepare("done", BranchPosition::Continue).is_ok());
        f.unchanged();
    }

    #[test]
    fn compaction_cannot_retain_only_a_tool_result() {
        let f = Fixture::new(vec![
            user("u", None),
            json!({"type":"message","id":"call","parentId":"u","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":"t"}]}}),
            json!({"type":"message","id":"result","parentId":"call","message":{"role":"toolResult","toolCallId":"t","content":[]}}),
            json!({"type":"compaction","id":"c","parentId":"result","firstKeptEntryId":"result","summary":"s"}),
            user("next", Some("c")),
        ]);
        assert!(f.prepare("next", BranchPosition::Continue).is_err());
        assert!(!f.root.join("output").exists());
    }

    #[cfg(unix)]
    #[test]
    fn artifacts_and_containing_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new(vec![user("u", None)]);
        let fork = f.prepare("u", BranchPosition::Rewrite).unwrap();
        let ticket = fork
            .session_path
            .parent()
            .unwrap()
            .join("herdr-prefill-ticket.json");
        for path in [&fork.session_path, &fork.extension_path, &ticket] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            fs::metadata(fork.session_path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn exclusive_writer_never_clobbers_or_claims_existing_file() {
        let f = Fixture::new(vec![user("u", None)]);
        let mut owned = Vec::new();
        assert!(write_exclusive(&f.source, b"overwrite", &mut owned).is_err());
        assert!(owned.is_empty());
        f.unchanged();
    }

    #[test]
    fn old_name_preserved_as_history_but_cleared_for_new_session() {
        let name = json!({"type":"session_info","id":"n","parentId":"u","name":"old project"});
        let f = Fixture::new(vec![user("u", None), name.clone(), user("next", Some("n"))]);
        let c = load(&f.prepare("next", BranchPosition::Continue).unwrap());
        assert_eq!(c.entry("n").unwrap(), &name);
        assert_eq!(c.entries.last().unwrap()["name"], "");
        assert_eq!(marker(&c)["parentId"], "next");
    }
}
