//! Unified message state (E) — single on-disk transcript for all LAN/WAN paths.
//! Full read/write access to `chat_transcript_v1.json` (Flutter [ChatTranscriptStore] is read-only reload).

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use fs4::fs_std::FileExt;
use serde_json::Value;
use thiserror::Error;

use crate::app_paths::{chat_transcript_v1_path, storage_config_for_namespace};
use crate::contacts_v1::{find_by_peer_id, find_by_public_key, is_valid_public_key_hex};
use crate::flow_log;
use crate::public_key_util::{
    legacy_libp2p_peer_id_str_from_public_key_hex, legacy_public_key_from_peer_id_str,
};
use crate::storage::KeystoreStorageError;

static IO_CHAIN: OnceLock<Mutex<()>> = OnceLock::new();
static REVISIONS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();

fn io_chain() -> &'static Mutex<()> {
    IO_CHAIN.get_or_init(|| Mutex::new(()))
}

fn revision_map() -> &'static Mutex<HashMap<String, u64>> {
    REVISIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn revision_storage_key(app_namespace: &str, view_key: &str) -> String {
    format!(
        "{}|{}",
        app_namespace.trim(),
        view_key.trim().to_ascii_lowercase()
    )
}

/// Canonical UI view key for a thread — prefer contact identity wire when known.
pub fn transcript_view_key(app_namespace: &str, conversation_key: &str) -> String {
    let k = conversation_key.trim();
    if k.is_empty() {
        return String::new();
    }
    if is_valid_public_key_hex(k) {
        return k.to_ascii_lowercase();
    }
    if let Ok(Some(c)) = find_by_peer_id(app_namespace, k) {
        if c.has_public_key() {
            return c.public_key_hex.trim().to_ascii_lowercase();
        }
        return c.conversation_key();
    }
    if let Ok(Some(c)) = find_by_public_key(app_namespace, k) {
        if c.has_public_key() {
            return c.public_key_hex.trim().to_ascii_lowercase();
        }
        return c.conversation_key();
    }
    k.to_string()
}

fn bump_transcript_revision(app_namespace: &str, conversation_key: &str) {
    let view = transcript_view_key(app_namespace, conversation_key);
    if view.is_empty() {
        return;
    }
    let key = revision_storage_key(app_namespace, &view);
    if let Ok(mut m) = revision_map().lock() {
        let e = m.entry(key).or_insert(0);
        *e = e.saturating_add(1);
    }
}

/// Monotonic revision for one canonical view key (in-memory; bumped on every disk mutation).
pub fn thread_revision_for_view(app_namespace: &str, view_key: &str) -> u64 {
    let key = revision_storage_key(app_namespace, view_key);
    revision_map()
        .lock()
        .ok()
        .and_then(|m| m.get(&key).copied())
        .unwrap_or(0)
}

pub fn thread_revision_for_keys(app_namespace: &str, conversation_keys: &[String]) -> u64 {
    let expanded = expand_conversation_keys(app_namespace, conversation_keys);
    let mut views: HashSet<String> = HashSet::new();
    for k in expanded {
        let v = transcript_view_key(app_namespace, &k);
        if !v.is_empty() {
            views.insert(v);
        }
    }
    views
        .iter()
        .map(|v| thread_revision_for_view(app_namespace, v))
        .max()
        .unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct TranscriptThreadView {
    pub revision: u64,
    pub lines: Vec<StoredChatLine>,
    /// True when older lines exist before the returned window (paginated load only).
    pub has_more: bool,
}

/// Paginated thread view: when [limit] is `Some(n)`, returns at most the newest `n`
/// lines and sets `has_more` if older lines exist. `None` returns the full thread
/// (`has_more = false`). Lines stay in ascending (oldest-first) order either way.
///
/// The UI grows `limit` on scroll-up rather than fetching the entire transcript at
/// once; the full merged set is still read from disk, but only a bounded window is
/// serialized to the UI, which is the dominant rendering cost.
pub fn thread_view_limited(
    app_namespace: &str,
    conversation_keys: &[String],
    match_inbound_from_peer_id: Option<&str>,
    limit: Option<usize>,
) -> Result<TranscriptThreadView, TranscriptStoreError> {
    let all = load_merged(app_namespace, conversation_keys, match_inbound_from_peer_id)?;
    let revision = thread_revision_for_keys(app_namespace, conversation_keys);
    let (lines, has_more) = match limit {
        Some(n) if n < all.len() => {
            let start = all.len() - n;
            (all[start..].to_vec(), true)
        }
        _ => (all, false),
    };
    Ok(TranscriptThreadView {
        revision,
        lines,
        has_more,
    })
}

/// One in-process mutex + cross-process flock for the duration of a read/modify/write.
struct TranscriptIoGuard {
    _process: MutexGuard<'static, ()>,
    _lock_file: File,
}

impl TranscriptIoGuard {
    fn acquire(path: &Path) -> Result<Self, TranscriptStoreError> {
        let process = io_chain().lock().map_err(|_| {
            TranscriptStoreError::Io(std::io::Error::other("transcript io mutex poisoned"))
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock_path = path.with_extension("json.lock");
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        lock_file.lock_exclusive().map_err(|e| {
            TranscriptStoreError::Io(std::io::Error::other(format!(
                "transcript flock {lock_path:?}: {e}"
            )))
        })?;
        Ok(Self {
            _process: process,
            _lock_file: lock_file,
        })
    }

    fn for_namespace(app_namespace: &str) -> Result<(Self, PathBuf), TranscriptStoreError> {
        let path = resolve_transcript_path(app_namespace)?;
        Ok((Self::acquire(&path)?, path))
    }
}

/// Serialize transcript reads/writes that take an explicit on-disk path (`chat_server` upkeep).
pub(crate) fn with_transcript_path<T>(
    path: &Path,
    f: impl FnOnce(&Path) -> Result<T, TranscriptStoreError>,
) -> Result<T, TranscriptStoreError> {
    let _guard = TranscriptIoGuard::acquire(path)?;
    f(path)
}

#[derive(Clone, Debug)]
pub struct StoredChatLine {
    pub local_id: String,
    pub text: String,
    pub outgoing: bool,
    pub from: Option<String>,
    pub message_id: Option<String>,
    pub delivery: String,
    pub created_at_ms: Option<i64>,
    /// Inbound: when this device first accepted the text. Outbound: peer's `ack_received.received_at_ms`.
    pub received_at_ms: Option<i64>,
    pub read_ack_sent: bool,
    /// `"text"` (default), `"voice"`, `"attachment_offer"`, …
    pub msg_kind: String,
    pub duration_ms: Option<u32>,
    /// Local Opus/file path after decrypt (per-device; not on the wire).
    pub audio_path: Option<String>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub local_path: Option<String>,
}

impl StoredChatLine {
    pub fn to_json(&self) -> Value {
        let mut m = serde_json::json!({
            "local_id": self.local_id,
            "text": self.text,
            "outgoing": self.outgoing,
            "delivery": self.delivery,
        });
        if let Some(f) = &self.from {
            m["from"] = Value::String(f.clone());
        }
        if let Some(mid) = &self.message_id {
            m["message_id"] = Value::String(mid.clone());
        }
        if let Some(t) = self.created_at_ms {
            m["created_at_ms"] = Value::Number(t.into());
        }
        if let Some(t) = self.received_at_ms {
            m["received_at_ms"] = Value::Number(t.into());
        }
        if self.read_ack_sent {
            m["read_ack_sent"] = Value::Bool(true);
        }
        let kind = self.msg_kind.trim();
        if !kind.is_empty() && kind != "text" {
            m["msg_kind"] = Value::String(kind.to_string());
        }
        if let Some(d) = self.duration_ms {
            m["duration_ms"] = Value::Number(d.into());
        }
        if let Some(p) = &self.audio_path {
            m["audio_path"] = Value::String(p.clone());
        }
        if let Some(n) = &self.file_name {
            m["file_name"] = Value::String(n.clone());
        }
        if let Some(mt) = &self.mime_type {
            m["mime_type"] = Value::String(mt.clone());
        }
        if let Some(sz) = self.size_bytes {
            m["size_bytes"] = Value::Number(sz.into());
        }
        if let Some(p) = &self.local_path {
            m["local_path"] = Value::String(p.clone());
        }
        m
    }

    pub fn from_json(raw: &Value) -> Option<Self> {
        let obj = raw.as_object()?;
        let local_id = obj.get("local_id")?.as_str()?.trim();
        if local_id.is_empty() {
            return None;
        }
        let text = obj
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let file_name = obj
            .get("file_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let t = text.trim();
                t.strip_prefix('📎').map(|rest| rest.trim()).and_then(|name| {
                    if name.is_empty() {
                        None
                    } else {
                        Some(name.to_string())
                    }
                })
            });
        let raw_kind = obj
            .get("msg_kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let msg_kind = if !raw_kind.is_empty() && raw_kind != "text" {
            raw_kind.to_string()
        } else if file_name.is_some() || text.trim_start().starts_with('📎') {
            // Legacy / clobbered rows lost msg_kind — still render as attachments.
            "attachment_offer".to_string()
        } else {
            "text".to_string()
        };
        Some(Self {
            local_id: local_id.to_string(),
            text,
            outgoing: obj.get("outgoing").and_then(|v| v.as_bool()) == Some(true),
            from: obj.get("from").and_then(|v| v.as_str()).map(str::to_string),
            message_id: obj
                .get("message_id")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            delivery: obj
                .get("delivery")
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_string(),
            created_at_ms: obj.get("created_at_ms").and_then(|v| v.as_i64()),
            received_at_ms: obj.get("received_at_ms").and_then(|v| v.as_i64()),
            read_ack_sent: obj.get("read_ack_sent").and_then(|v| v.as_bool()) == Some(true),
            msg_kind,
            duration_ms: obj
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            audio_path: obj
                .get("audio_path")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            file_name,
            mime_type: obj
                .get("mime_type")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            size_bytes: obj.get("size_bytes").and_then(|v| v.as_u64()),
            local_path: obj
                .get("local_path")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    }
}

#[derive(Debug, Error)]
pub enum TranscriptStoreError {
    #[error("storage: {0}")]
    Storage(#[from] KeystoreStorageError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("transcript: {0}")]
    Transcript(#[from] crate::dm_transcript_v1::TranscriptError),
}

pub fn resolve_transcript_path(app_namespace: &str) -> Result<PathBuf, TranscriptStoreError> {
    Ok(chat_transcript_v1_path(&storage_config_for_namespace(
        app_namespace,
    ))?)
}

fn decode_root_lenient(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(serde_json::json!({}));
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }
    if !trimmed.starts_with('{') {
        return None;
    }
    let mut lo = 2usize;
    let mut hi = trimmed.len();
    let mut best: Option<Value> = None;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        match serde_json::from_str::<Value>(&trimmed[..mid]) {
            Ok(v) if v.is_object() => {
                best = Some(v);
                lo = mid + 1;
            }
            _ => hi = mid.saturating_sub(1),
        }
    }
    best
}

pub(crate) fn read_root_unlocked(path: &Path) -> Result<Value, TranscriptStoreError> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let raw = fs::read_to_string(path)?;
    Ok(decode_root_lenient(&raw).unwrap_or_else(|| serde_json::json!({})))
}

fn write_root_unlocked(path: &Path, root: &Value) -> Result<(), TranscriptStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(root)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn delivery_rank(delivery: &str) -> i32 {
    match delivery {
        "pending" => 0,
        "sent" => 1,
        "failed" => 1,
        "delivered" => 2,
        "read" => 3,
        "downloaded" => 3,
        _ => 0,
    }
}

fn pick_better_duplicate(a: &StoredChatLine, b: &StoredChatLine) -> StoredChatLine {
    let amid = a.message_id.as_deref().unwrap_or("").trim();
    let bmid = b.message_id.as_deref().unwrap_or("").trim();
    if amid.is_empty() && !bmid.is_empty() {
        return b.clone();
    }
    if bmid.is_empty() && !amid.is_empty() {
        return a.clone();
    }
    if a.outgoing && b.outgoing && delivery_rank(&b.delivery) > delivery_rank(&a.delivery) {
        return b.clone();
    }
    let at = a.created_at_ms.unwrap_or(0);
    let bt = b.created_at_ms.unwrap_or(0);
    if bt > at { b.clone() } else { a.clone() }
}

fn line_sort_key(line: &StoredChatLine) -> (i64, String, String) {
    (
        line.created_at_ms.unwrap_or(0),
        line.message_id.as_deref().unwrap_or("").trim().to_string(),
        line.local_id.clone(),
    )
}

/// Insert so thread order matches [`dedupe_lines`] / hub chat sort (`created_at_ms`, then `message_id`).
fn insert_line_in_thread_order(existing: &mut Vec<StoredChatLine>, line: StoredChatLine) {
    let key = line_sort_key(&line);
    let pos = existing
        .iter()
        .position(|e| line_sort_key(e) > key)
        .unwrap_or(existing.len());
    existing.insert(pos, line);
}

fn dedupe_lines(lines: Vec<StoredChatLine>) -> Vec<StoredChatLine> {
    if lines.len() < 2 {
        return lines;
    }
    let mut by_mid: HashMap<String, StoredChatLine> = HashMap::new();
    let mut no_mid: Vec<StoredChatLine> = Vec::new();
    for line in lines {
        let mid = line.message_id.as_deref().unwrap_or("").trim();
        if !mid.is_empty() {
            by_mid
                .entry(mid.to_string())
                .and_modify(|prev| *prev = pick_better_duplicate(prev, &line))
                .or_insert(line);
        } else {
            no_mid.push(line);
        }
    }
    let mut kept: Vec<StoredChatLine> = by_mid.into_values().chain(no_mid).collect();
    kept.sort_by(|a, b| line_sort_key(a).cmp(&line_sort_key(b)));
    kept
}

/// Include legacy/alternate thread keys (peer id vs public key hex) for the same contact.
fn expand_conversation_keys(app_namespace: &str, conversation_keys: &[String]) -> Vec<String> {
    let mut out: HashSet<String> = conversation_keys
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for key in conversation_keys {
        let k = key.trim();
        if k.is_empty() || k == "solo" {
            continue;
        }
        if let Ok(Some(c)) = find_by_peer_id(app_namespace, k) {
            out.insert(c.conversation_key());
            if is_valid_public_key_hex(&c.public_key_hex) {
                out.insert(c.public_key_hex.trim().to_string());
            }
        }
        if let Ok(Some(c)) = find_by_public_key(app_namespace, k) {
            out.insert(c.conversation_key());
            if is_valid_public_key_hex(&c.public_key_hex) {
                out.insert(c.public_key_hex.trim().to_string());
            }
        }
        if is_valid_public_key_hex(k) {
            if let Some(legacy_pid) = legacy_libp2p_peer_id_str_from_public_key_hex(k) {
                out.insert(legacy_pid);
            }
        } else if let Some(pk) = legacy_public_key_from_peer_id_str(k) {
            let pk = pk.clone();
            out.insert(pk.clone());
            if let Some(legacy_pid) = legacy_libp2p_peer_id_str_from_public_key_hex(&pk) {
                out.insert(legacy_pid);
            }
        }
    }
    out.into_iter().collect()
}

fn load_merged_unlocked(
    app_namespace: &str,
    conversation_keys: &[String],
    match_inbound_from_peer_id: Option<&str>,
    path: &Path,
) -> Result<Vec<StoredChatLine>, TranscriptStoreError> {
    let all = read_root_unlocked(path)?;
    let ns = all.get(app_namespace);
    let Some(ns_obj) = ns.and_then(|v| v.as_object()) else {
        return Ok(Vec::new());
    };
    let expanded = expand_conversation_keys(app_namespace, conversation_keys);
    let keys: HashSet<&str> = expanded
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    let from_peer = match_inbound_from_peer_id.unwrap_or("").trim();
    let mut by_mid: HashMap<String, StoredChatLine> = HashMap::new();
    let mut no_mid: HashMap<String, StoredChatLine> = HashMap::new();

    for (thread_key, thread) in ns_obj {
        let Some(arr) = thread.as_array() else {
            continue;
        };
        let mut include = keys.contains(thread_key.as_str());
        if !include && !from_peer.is_empty() {
            for item in arr {
                if let Some(line) = StoredChatLine::from_json(item) {
                    if line.from.as_deref().unwrap_or("").trim() == from_peer {
                        include = true;
                        break;
                    }
                }
            }
        }
        if !include {
            continue;
        }
        for item in arr {
            let Some(line) = StoredChatLine::from_json(item) else {
                continue;
            };
            let mid = line.message_id.as_deref().unwrap_or("").trim();
            if !mid.is_empty() {
                by_mid
                    .entry(mid.to_string())
                    .and_modify(|prev| *prev = pick_better_duplicate(prev, &line))
                    .or_insert(line);
            } else {
                no_mid.insert(line.local_id.clone(), line);
            }
        }
    }
    Ok(dedupe_lines(
        by_mid.into_values().chain(no_mid.into_values()).collect(),
    ))
}

pub fn load_merged(
    app_namespace: &str,
    conversation_keys: &[String],
    match_inbound_from_peer_id: Option<&str>,
) -> Result<Vec<StoredChatLine>, TranscriptStoreError> {
    let path = resolve_transcript_path(app_namespace)?;
    let _guard = TranscriptIoGuard::acquire(&path)?;
    load_merged_unlocked(
        app_namespace,
        conversation_keys,
        match_inbound_from_peer_id,
        &path,
    )
}

pub fn save_thread(
    app_namespace: &str,
    conversation_key: &str,
    lines: Vec<StoredChatLine>,
) -> Result<(), TranscriptStoreError> {
    let (_guard, path) = TranscriptIoGuard::for_namespace(app_namespace)?;
    let mut all = read_root_unlocked(&path)?;
    let root = all.as_object_mut().ok_or_else(|| {
        TranscriptStoreError::Io(std::io::Error::other("transcript root not object"))
    })?;
    let deduped = dedupe_lines(lines);
    // UI full-save must not downgrade delivery/read ticks already written by :p2p on poll.
    let expanded = expand_conversation_keys(app_namespace, &[conversation_key.to_string()]);
    let disk_rows = load_merged_unlocked(app_namespace, &expanded, None, &path).unwrap_or_default();
    let disk_by_mid: HashMap<String, StoredChatLine> = disk_rows
        .iter()
        .filter_map(|r| {
            let mid = r.message_id.as_deref().unwrap_or("").trim();
            if mid.is_empty() {
                None
            } else {
                Some((mid.to_string(), r.clone()))
            }
        })
        .collect();
    let mut merged: Vec<StoredChatLine> = deduped
        .into_iter()
        .map(|line| {
            let mid = line.message_id.as_deref().unwrap_or("").trim();
            if mid.is_empty() {
                line
            } else if let Some(disk) = disk_by_mid.get(mid) {
                pick_better_duplicate(&line, disk)
            } else {
                line
            }
        })
        .collect();
    let ui_mids: HashSet<String> = merged
        .iter()
        .filter_map(|l| {
            let mid = l.message_id.as_deref().unwrap_or("").trim();
            if mid.is_empty() {
                None
            } else {
                Some(mid.to_string())
            }
        })
        .collect();
    // Keep inbound rows :p2p wrote on poll that the UI shell has not merged yet.
    for disk_line in disk_rows {
        let mid = disk_line
            .message_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string();
        if !mid.is_empty() {
            if ui_mids.contains(&mid) {
                continue;
            }
            merged.push(disk_line);
            continue;
        }
        let dup = merged.iter().any(|u| {
            u.message_id.as_deref().unwrap_or("").trim().is_empty()
                && u.local_id == disk_line.local_id
        });
        if !dup {
            merged.push(disk_line);
        }
    }
    let deduped = dedupe_lines(merged);
    let arr: Vec<Value> = deduped.iter().map(|l| l.to_json()).collect();
    let ns_entry = root
        .entry(app_namespace.to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Some(obj) = ns_entry.as_object_mut() {
        obj.insert(conversation_key.to_string(), Value::Array(arr));
    }
    write_root_unlocked(&path, &all)?;
    bump_transcript_revision(app_namespace, conversation_key);
    Ok(())
}

pub fn append_if_new(
    app_namespace: &str,
    conversation_key: &str,
    line: StoredChatLine,
) -> Result<(), TranscriptStoreError> {
    if conversation_key.trim().is_empty() {
        return Ok(());
    }
    let (_guard, path) = TranscriptIoGuard::for_namespace(app_namespace)?;
    let mut all = read_root_unlocked(&path)?;
    let ns_obj = all
        .as_object_mut()
        .and_then(|r| r.get_mut(app_namespace))
        .and_then(|v| v.as_object_mut());
    let Some(ns_obj) = ns_obj else {
        let mut threads = serde_json::Map::new();
        threads.insert(
            conversation_key.to_string(),
            Value::Array(vec![line.to_json()]),
        );
        if let Some(root) = all.as_object_mut() {
            root.insert(app_namespace.to_string(), Value::Object(threads));
        }
        write_root_unlocked(&path, &all)?;
        bump_transcript_revision(app_namespace, conversation_key);
        return Ok(());
    };
    let mut existing: Vec<StoredChatLine> = ns_obj
        .get(conversation_key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|i| StoredChatLine::from_json(i))
                .collect()
        })
        .unwrap_or_default();
    let mid = line.message_id.as_deref().unwrap_or("").trim().to_string();
    for e in &existing {
        if !mid.is_empty() && e.message_id.as_deref().unwrap_or("").trim() == mid.as_str() {
            flow_log::info(
                "Transcript",
                format!("append skipped duplicate message_id={mid} conv={conversation_key}"),
            );
            return Ok(());
        }
    }
    let outgoing = line.outgoing;
    let text_len = line.text.len();
    insert_line_in_thread_order(&mut existing, line);
    ns_obj.insert(
        conversation_key.to_string(),
        Value::Array(existing.iter().map(|l| l.to_json()).collect()),
    );
    flow_log::info(
        "Transcript",
        format!("append line conv={conversation_key} mid={mid} outgoing={outgoing} len={text_len}",),
    );
    write_root_unlocked(&path, &all)?;
    bump_transcript_revision(app_namespace, conversation_key);
    Ok(())
}

pub fn patch_outgoing_delivery(
    app_namespace: &str,
    conversation_key: &str,
    message_id: &str,
    delivery: &str,
) -> Result<bool, TranscriptStoreError> {
    let mid = message_id.trim();
    if mid.is_empty() || conversation_key.trim().is_empty() {
        return Ok(false);
    }
    let target_rank = delivery_rank(delivery);
    if target_rank < 1 {
        return Ok(false);
    }
    let (_guard, path) = TranscriptIoGuard::for_namespace(app_namespace)?;
    let mut all = read_root_unlocked(&path)?;
    let Some(ns_obj) = all.get_mut(app_namespace).and_then(|v| v.as_object_mut()) else {
        return Ok(false);
    };
    let keys = expand_conversation_keys(app_namespace, &[conversation_key.to_string()]);
    let mut changed = false;
    for ck in keys {
        let Some(thread) = ns_obj.get_mut(ck.as_str()).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for item in thread.iter_mut() {
            let Some(parsed) = StoredChatLine::from_json(item) else {
                continue;
            };
            if parsed.outgoing && parsed.message_id.as_deref().unwrap_or("").trim() == mid {
                if delivery_rank(&parsed.delivery) < target_rank {
                    let mut next = parsed;
                    next.delivery = delivery.to_string();
                    *item = next.to_json();
                    changed = true;
                }
            }
        }
    }
    if changed {
        flow_log::info(
            "Transcript",
            format!("patched outgoing delivery={delivery} mid={mid} conv={conversation_key}"),
        );
        write_root_unlocked(&path, &all)?;
        bump_transcript_revision(app_namespace, conversation_key);
    } else {
        flow_log::debug(
            "Transcript",
            format!(
                "patch outgoing delivery unchanged mid={mid} conv={conversation_key} target={delivery}"
            ),
        );
    }
    Ok(changed)
}

/// Set outbound `received_at_ms` from peer `ack_received` (first value wins).
pub fn patch_outgoing_received_at(
    app_namespace: &str,
    conversation_key: &str,
    message_id: &str,
    received_at_ms: i64,
) -> Result<bool, TranscriptStoreError> {
    let mid = message_id.trim();
    if mid.is_empty() || conversation_key.trim().is_empty() || received_at_ms <= 0 {
        return Ok(false);
    }
    let (_guard, path) = TranscriptIoGuard::for_namespace(app_namespace)?;
    let mut all = read_root_unlocked(&path)?;
    let Some(ns_obj) = all.get_mut(app_namespace).and_then(|v| v.as_object_mut()) else {
        return Ok(false);
    };
    let keys = expand_conversation_keys(app_namespace, &[conversation_key.to_string()]);
    let mut changed = false;
    for ck in keys {
        let Some(thread) = ns_obj.get_mut(ck.as_str()).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for item in thread.iter_mut() {
            let Some(parsed) = StoredChatLine::from_json(item) else {
                continue;
            };
            if parsed.outgoing
                && parsed.message_id.as_deref().unwrap_or("").trim() == mid
                && parsed.received_at_ms.is_none()
            {
                let mut next = parsed;
                next.received_at_ms = Some(received_at_ms);
                *item = next.to_json();
                changed = true;
            }
        }
    }
    if changed {
        flow_log::info(
            "Transcript",
            format!(
                "patched outgoing received_at_ms={received_at_ms} mid={mid} conv={conversation_key}"
            ),
        );
        write_root_unlocked(&path, &all)?;
        bump_transcript_revision(app_namespace, conversation_key);
    }
    Ok(changed)
}

pub fn patch_attachment_local_path(
    app_namespace: &str,
    conversation_key: &str,
    message_id: &str,
    local_path: &str,
) -> Result<bool, TranscriptStoreError> {
    let mid = message_id.trim();
    let path_value = local_path.trim();
    if mid.is_empty() || conversation_key.trim().is_empty() || path_value.is_empty() {
        return Ok(false);
    }
    let (_guard, path) = TranscriptIoGuard::for_namespace(app_namespace)?;
    let mut all = read_root_unlocked(&path)?;
    let Some(ns_obj) = all.get_mut(app_namespace).and_then(|v| v.as_object_mut()) else {
        return Ok(false);
    };
    let keys = expand_conversation_keys(app_namespace, &[conversation_key.to_string()]);
    let mut changed = false;
    for ck in keys {
        let Some(thread) = ns_obj.get_mut(ck.as_str()).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for item in thread.iter_mut() {
            let Some(parsed) = StoredChatLine::from_json(item) else {
                continue;
            };
            if parsed.message_id.as_deref().unwrap_or("").trim() == mid
                && parsed.msg_kind.trim() == "attachment_offer"
                && parsed.local_path.as_deref().unwrap_or("") != path_value
            {
                let mut next = parsed;
                next.local_path = Some(path_value.to_string());
                *item = next.to_json();
                changed = true;
            }
        }
    }
    if changed {
        write_root_unlocked(&path, &all)?;
        bump_transcript_revision(app_namespace, conversation_key);
    }
    Ok(changed)
}

pub fn patch_inbound_read_ack_sent_for_thread(
    app_namespace: &str,
    conversation_key: &str,
    message_id: &str,
) -> Result<bool, TranscriptStoreError> {
    let mid = message_id.trim();
    if mid.is_empty() || conversation_key.trim().is_empty() {
        return Ok(false);
    }
    let (_guard, path) = TranscriptIoGuard::for_namespace(app_namespace)?;
    let mut all = read_root_unlocked(&path)?;
    let Some(ns_obj) = all.get_mut(app_namespace).and_then(|v| v.as_object_mut()) else {
        return Ok(false);
    };
    let keys = expand_conversation_keys(app_namespace, &[conversation_key.to_string()]);
    let mut changed = false;
    for ck in keys {
        let Some(thread) = ns_obj.get_mut(ck.as_str()).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for item in thread.iter_mut() {
            let Some(parsed) = StoredChatLine::from_json(item) else {
                continue;
            };
            if !parsed.outgoing
                && parsed.message_id.as_deref().unwrap_or("").trim() == mid
                && !parsed.read_ack_sent
            {
                let mut next = parsed;
                next.read_ack_sent = true;
                *item = next.to_json();
                changed = true;
            }
        }
    }
    if changed {
        write_root_unlocked(&path, &all)?;
        bump_transcript_revision(app_namespace, conversation_key);
    }
    Ok(changed)
}

fn patch_inbound_read_ack_sent_all_threads(
    all: &mut Value,
    app_namespace: &str,
    message_id: &str,
) -> (bool, Vec<String>) {
    let mid = message_id.trim();
    if mid.is_empty() {
        return (false, Vec::new());
    }
    let Some(ns_obj) = all.get_mut(app_namespace).and_then(|v| v.as_object_mut()) else {
        return (false, Vec::new());
    };
    let mut changed = false;
    let mut touched = Vec::new();
    for (conv_key, thread) in ns_obj.iter_mut() {
        let Some(lines) = thread.as_array_mut() else {
            continue;
        };
        let mut thread_changed = false;
        for item in lines.iter_mut() {
            let Some(parsed) = StoredChatLine::from_json(item) else {
                continue;
            };
            if parsed.outgoing
                || parsed.message_id.as_deref().unwrap_or("").trim() != mid
                || parsed.read_ack_sent
            {
                continue;
            }
            let mut next = parsed;
            next.read_ack_sent = true;
            *item = next.to_json();
            changed = true;
            thread_changed = true;
        }
        if thread_changed {
            touched.push(conv_key.clone());
        }
    }
    (changed, touched)
}

/// Search every thread under [app_namespace] (read-ack confirm from `chat_server`).
pub fn patch_inbound_read_ack_sent_at_path(
    path: &Path,
    app_namespace: &str,
    message_id: &str,
) -> Result<bool, TranscriptStoreError> {
    with_transcript_path(path, |path| {
        let mid = message_id.trim();
        if mid.is_empty() || !path.exists() {
            return Ok(false);
        }
        let mut all = read_root_unlocked(path)?;
        let (changed, touched) =
            patch_inbound_read_ack_sent_all_threads(&mut all, app_namespace, mid);
        if changed {
            write_root_unlocked(path, &all)?;
            for conv_key in touched {
                bump_transcript_revision(app_namespace, &conv_key);
            }
        }
        Ok(changed)
    })
}

pub fn patch_inbound_read_ack_sent_global(
    app_namespace: &str,
    message_id: &str,
) -> Result<(), TranscriptStoreError> {
    let path = resolve_transcript_path(app_namespace)?;
    let _ = patch_inbound_read_ack_sent_at_path(&path, app_namespace, message_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_rank_monotonic_read_wins() {
        assert!(delivery_rank("read") > delivery_rank("delivered"));
        assert!(delivery_rank("delivered") > delivery_rank("sent"));
        assert!(delivery_rank("read") > delivery_rank("pending"));
    }

    #[test]
    fn patch_outgoing_delivery_accepts_sent_rank() {
        use crate::c_ffi::configure_android_data_directory;
        use crate::storage::{StorageConfig, create_or_unlock_identity_v1};
        use tempfile::TempDir;

        let _guard = crate::c_ffi::test_storage_isolation_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let td = TempDir::new().unwrap();
        configure_android_data_directory(td.path().to_str().unwrap());
        let ns = "dev.transcript.sent";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let _id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let pk = "03a342b298a3500eaef16434e31a0376aea9c3a0403ab1592a62b2326b1dfa410b";
        let line = StoredChatLine {
            local_id: "loc1".into(),
            text: "hi".into(),
            outgoing: true,
            from: None,
            message_id: Some("mid-sent".into()),
            delivery: "pending".into(),
            created_at_ms: Some(1000),
            received_at_ms: None,
            read_ack_sent: false,
            msg_kind: "text".into(),
            duration_ms: None,
            audio_path: None,
            file_name: None,
            mime_type: None,
            size_bytes: None,
            local_path: None,
        };
        append_if_new(ns, pk, line).unwrap();
        assert!(patch_outgoing_delivery(ns, pk, "mid-sent", "sent").unwrap());
        let rows = load_merged(ns, &[pk.to_string()], None).unwrap();
        assert_eq!(rows[0].delivery, "sent");
    }

    #[test]
    fn pick_better_duplicate_outgoing_keeps_read_over_delivered() {
        let delivered = StoredChatLine {
            local_id: "a".into(),
            text: "hi".into(),
            outgoing: true,
            from: None,
            message_id: Some("mid1".into()),
            delivery: "delivered".into(),
            created_at_ms: Some(100),
            received_at_ms: None,
            read_ack_sent: false,
            msg_kind: "text".into(),
            duration_ms: None,
            audio_path: None,
            file_name: None,
            mime_type: None,
            size_bytes: None,
            local_path: None,
        };
        let read = StoredChatLine {
            delivery: "read".into(),
            created_at_ms: Some(101),
            ..delivered.clone()
        };
        let picked = pick_better_duplicate(&delivered, &read);
        assert_eq!(picked.delivery, "read");
        // WAN ack_received after LAN ack_read must not downgrade
        let picked2 = pick_better_duplicate(&read, &delivered);
        assert_eq!(picked2.delivery, "read");
    }

    #[test]
    fn thread_view_limited_returns_newest_window_with_has_more() {
        use crate::c_ffi::configure_android_data_directory;
        use crate::storage::{StorageConfig, create_or_unlock_identity_v1};
        use tempfile::TempDir;

        let _guard = crate::c_ffi::test_storage_isolation_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let td = TempDir::new().unwrap();
        configure_android_data_directory(td.path().to_str().unwrap());
        let ns = "dev.transcript.page";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let _id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let pk = "peerpk";
        for i in 0..5 {
            append_if_new(
                ns,
                pk,
                StoredChatLine {
                    local_id: format!("l{i}"),
                    text: format!("m{i}"),
                    outgoing: false,
                    from: Some("peer".into()),
                    message_id: Some(format!("mid{i}")),
                    delivery: "pending".into(),
                    created_at_ms: Some(1000 + i as i64),
                    received_at_ms: Some(1000 + i as i64),
                    read_ack_sent: false,
                    msg_kind: "text".into(),
                    duration_ms: None,
                    audio_path: None,
                    file_name: None,
                    mime_type: None,
                    size_bytes: None,
                    local_path: None,
                },
            )
            .unwrap();
        }
        // Window of 2 → newest two lines, has_more true.
        let page = thread_view_limited(ns, &[pk.to_string()], None, Some(2)).unwrap();
        assert_eq!(page.lines.len(), 2);
        assert_eq!(page.lines[0].text, "m3");
        assert_eq!(page.lines[1].text, "m4");
        assert!(page.has_more);
        // Window >= total → full thread, has_more false.
        let full = thread_view_limited(ns, &[pk.to_string()], None, Some(10)).unwrap();
        assert_eq!(full.lines.len(), 5);
        assert!(!full.has_more);
        // None → full thread, has_more false.
        let none = thread_view_limited(ns, &[pk.to_string()], None, None).unwrap();
        assert_eq!(none.lines.len(), 5);
        assert!(!none.has_more);
    }

    #[test]
    fn append_if_new_inserts_by_created_at_ms_not_append_order() {
        use crate::c_ffi::configure_android_data_directory;
        use crate::storage::{StorageConfig, create_or_unlock_identity_v1};
        use tempfile::TempDir;

        let _guard = crate::c_ffi::test_storage_isolation_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let td = TempDir::new().unwrap();
        configure_android_data_directory(td.path().to_str().unwrap());
        let ns = "dev.transcript.order";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let _id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let early = StoredChatLine {
            local_id: "late-send".into(),
            text: "newer".into(),
            outgoing: true,
            from: None,
            message_id: Some("mid-new".into()),
            delivery: "pending".into(),
            created_at_ms: Some(2000),
            received_at_ms: None,
            read_ack_sent: false,
            msg_kind: "text".into(),
            duration_ms: None,
            audio_path: None,
            file_name: None,
            mime_type: None,
            size_bytes: None,
            local_path: None,
        };
        let late = StoredChatLine {
            local_id: "early-recv".into(),
            text: "older".into(),
            outgoing: false,
            from: Some("peer".into()),
            message_id: Some("mid-old".into()),
            delivery: "pending".into(),
            created_at_ms: Some(1000),
            received_at_ms: Some(1500),
            read_ack_sent: false,
            msg_kind: "text".into(),
            duration_ms: None,
            audio_path: None,
            file_name: None,
            mime_type: None,
            size_bytes: None,
            local_path: None,
        };
        append_if_new(ns, "peerpk", early).unwrap();
        append_if_new(ns, "peerpk", late).unwrap();
        let rows = load_merged(ns, &["peerpk".to_string()], None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "older");
        assert_eq!(rows[1].text, "newer");
    }
}
