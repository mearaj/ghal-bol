//! Persisted 1:1 contacts (`contacts_v1.json`) — same format as Flutter [ContactStore].

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde_json::Value;
use thiserror::Error;

use crate::app_paths::{contacts_v1_path, storage_config_for_namespace};
use crate::dm_transport::normalize_contact_pk;
use crate::flow_log::{self, short_hex};
use crate::identity::Identity;
use crate::public_key_util::{
    legacy_public_key_from_peer_id_str, normalize_contact_identity_wire,
};
use crate::storage::KeystoreStorageError;

static CHANGE_VERSION: AtomicU64 = AtomicU64::new(0);
static IO_CHAIN: OnceLock<Mutex<()>> = OnceLock::new();

fn io_chain() -> &'static Mutex<()> {
    IO_CHAIN.get_or_init(|| Mutex::new(()))
}

fn bump_change() {
    CHANGE_VERSION.fetch_add(1, Ordering::SeqCst);
}

pub fn contacts_change_version() -> u64 {
    CHANGE_VERSION.load(Ordering::SeqCst)
}

#[derive(Clone, Debug)]
pub struct SavedContact {
    pub public_key_hex: String,
    pub display_alias: Option<String>,
    pub last_message_preview: Option<String>,
    pub last_message_at_ms: Option<i64>,
    pub unread_count: u32,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    /// User accepted this peer on this device (scan, Add, or outbound send).
    pub is_known: bool,
    /// User blocked this peer on this device.
    pub is_blocked: bool,
    /// Last moment user was actively in the chat room with this peer (DESIGN.md). Updated in lockstep
    /// with the live session clock while the room is open; frozen on leave/switch/inactive.
    pub chat_room_exit_at_ms: Option<i64>,
}

impl SavedContact {
    pub fn has_public_key(&self) -> bool {
        is_valid_public_key_hex(&self.public_key_hex)
    }

    pub fn conversation_key(&self) -> String {
        self.public_key_hex.trim().to_string()
    }

    pub fn to_json(&self) -> Value {
        let mut m = serde_json::json!({
            "unread_count": self.unread_count,
            "is_known": self.is_known,
            "is_blocked": self.is_blocked,
        });
        if !self.public_key_hex.is_empty() {
            m["public_key_hex"] = Value::String(self.public_key_hex.clone());
        }
        if let Some(a) = &self.display_alias {
            if !a.is_empty() {
                m["display_alias"] = Value::String(a.clone());
            }
        }
        if let Some(p) = &self.last_message_preview {
            m["last_message_preview"] = Value::String(p.clone());
        }
        if let Some(t) = self.last_message_at_ms {
            m["last_message_at_ms"] = Value::Number(t.into());
        }
        if let Some(t) = self.created_at_ms {
            m["created_at_ms"] = Value::Number(t.into());
        }
        if let Some(t) = self.updated_at_ms {
            m["updated_at_ms"] = Value::Number(t.into());
        }
        if let Some(t) = self.chat_room_exit_at_ms {
            m["chat_room_exit_at_ms"] = Value::Number(t.into());
        }
        m
    }

    pub fn from_json(raw: &Value) -> Option<Self> {
        let obj = raw.as_object()?;
        let mut pk = obj
            .get("public_key_hex")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if !is_valid_public_key_hex(&pk) {
            let legacy_pid = obj
                .get("libp2p_peer_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            pk = legacy_public_key_from_peer_id_str(legacy_pid).unwrap_or_default();
        }
        if !is_valid_public_key_hex(&pk) {
            return None;
        }
        let _ = normalize_contact_pk(&pk);
        let is_known = obj
            .get("is_known")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let is_blocked = obj
            .get("is_blocked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Some(Self {
            public_key_hex: pk,
            display_alias: obj
                .get("display_alias")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            last_message_preview: obj
                .get("last_message_preview")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            last_message_at_ms: obj.get("last_message_at_ms").and_then(|v| v.as_i64()),
            unread_count: obj
                .get("unread_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(9999) as u32,
            created_at_ms: obj.get("created_at_ms").and_then(|v| v.as_i64()),
            updated_at_ms: obj.get("updated_at_ms").and_then(|v| v.as_i64()),
            is_known,
            is_blocked,
            chat_room_exit_at_ms: obj.get("chat_room_exit_at_ms").and_then(|v| v.as_i64()),
        })
    }
}

pub fn is_valid_public_key_hex(hex_s: &str) -> bool {
    Identity::parse(hex_s).is_ok()
}

#[derive(Debug, Error)]
pub enum ContactsError {
    #[error("storage: {0}")]
    Storage(#[from] KeystoreStorageError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
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

fn read_all(path: &Path) -> Result<HashMap<String, Vec<SavedContact>>, ContactsError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = fs::read_to_string(path)?;
    let root = match decode_root_lenient(&raw) {
        Some(v) => v,
        None => return Ok(HashMap::new()),
    };
    let Some(obj) = root.as_object() else {
        return Ok(HashMap::new());
    };
    let mut out = HashMap::new();
    for (ns, val) in obj {
        let Some(arr) = val.as_array() else {
            continue;
        };
        let mut list = Vec::new();
        for item in arr {
            if let Some(c) = SavedContact::from_json(item) {
                list.push(c);
            }
        }
        out.insert(ns.clone(), list);
    }
    Ok(out)
}

fn write_all(path: &Path, all: &HashMap<String, Vec<SavedContact>>) -> Result<(), ContactsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let encodable: HashMap<String, Vec<Value>> = all
        .iter()
        .map(|(k, v)| (k.clone(), v.iter().map(|c| c.to_json()).collect()))
        .collect();
    let json = serde_json::to_string_pretty(&encodable)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn with_store<T>(
    app_namespace: &str,
    f: impl FnOnce(
        &Path,
        HashMap<String, Vec<SavedContact>>,
    ) -> Result<(T, HashMap<String, Vec<SavedContact>>), ContactsError>,
) -> Result<T, ContactsError> {
    let _guard = io_chain()
        .lock()
        .map_err(|_| ContactsError::Io(std::io::Error::other("contacts io mutex poisoned")))?;
    let cfg = storage_config_for_namespace(app_namespace);
    let path = contacts_v1_path(&cfg)?;
    let mut all = read_all(&path)?;
    let (out, next) = f(&path, all)?;
    all = next;
    write_all(&path, &all)?;
    bump_change();
    Ok(out)
}

fn index_of(list: &[SavedContact], contact: &SavedContact) -> Option<usize> {
    if !contact.has_public_key() {
        return None;
    }
    let target = Identity::parse(contact.public_key_hex.trim()).ok()?;
    list.iter().position(|c| {
        Identity::parse(c.public_key_hex.trim())
            .ok()
            .is_some_and(|id| id == target)
    })
}

pub fn list_contacts(app_namespace: &str) -> Result<Vec<SavedContact>, ContactsError> {
    let cfg = storage_config_for_namespace(app_namespace);
    let path = contacts_v1_path(&cfg)?;
    let mut all = read_all(&path)?;
    let mut dirty = false;
    {
        let list = all.entry(app_namespace.to_string()).or_default();
        for c in list.iter_mut() {
            if !c.has_public_key() {
                continue;
            }
            if let Ok(norm) = normalize_contact_identity_wire(c.public_key_hex.trim()) {
                if norm != c.public_key_hex {
                    c.public_key_hex = norm;
                    dirty = true;
                }
            }
        }
    }
    if dirty {
        write_all(&path, &all)?;
        bump_change();
    }
    let mut list = all.get(app_namespace).cloned().unwrap_or_default();
    list.sort_by(|a, b| {
        let ta = a
            .last_message_at_ms
            .or(a.updated_at_ms)
            .or(a.created_at_ms)
            .unwrap_or(0);
        let tb = b
            .last_message_at_ms
            .or(b.updated_at_ms)
            .or(b.created_at_ms)
            .unwrap_or(0);
        tb.cmp(&ta)
    });
    Ok(list)
}

pub fn find_by_public_key(
    app_namespace: &str,
    public_key_hex: &str,
) -> Result<Option<SavedContact>, ContactsError> {
    let target = match Identity::parse(public_key_hex.trim()) {
        Ok(id) => id,
        Err(_) => return Ok(None),
    };
    Ok(list_contacts(app_namespace)?
        .into_iter()
        .find(|c| {
            Identity::parse(c.public_key_hex.trim())
                .ok()
                .is_some_and(|id| id == target)
        }))
}

/// Lookup by public key hex, or legacy libp2p PeerId string on disk (migrated to pk).
pub fn find_by_peer_id(
    app_namespace: &str,
    conversation_key: &str,
) -> Result<Option<SavedContact>, ContactsError> {
    let k = conversation_key.trim();
    if k.is_empty() {
        return Ok(None);
    }
    if is_valid_public_key_hex(k) {
        return find_by_public_key(app_namespace, k);
    }
    if let Some(pk) = legacy_public_key_from_peer_id_str(k) {
        return find_by_public_key(app_namespace, &pk);
    }
    Ok(None)
}

pub fn upsert_contact(
    app_namespace: &str,
    contact: SavedContact,
) -> Result<SavedContact, ContactsError> {
    let now = now_ms();
    with_store(app_namespace, |_path, mut all| {
        let list = all.entry(app_namespace.to_string()).or_default();
        let idx = index_of(list, &contact);
        let base = if let Some(i) = idx {
            list[i].clone()
        } else {
            contact.clone()
        };
        let pk_norm = if contact.has_public_key() {
            normalize_contact_identity_wire(contact.public_key_hex.trim())
                .map_err(|e| ContactsError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?
        } else {
            base.public_key_hex.clone()
        };
        let next = SavedContact {
            public_key_hex: pk_norm,
            // `Some(_)` in the upsert payload updates alias (empty → clear); `None` keeps existing.
            display_alias: match &contact.display_alias {
                Some(s) => crate::preferences_v1::sanitize_peer_display_alias(s.as_str()),
                None => base.display_alias.clone(),
            },
            last_message_preview: contact
                .last_message_preview
                .clone()
                .or(base.last_message_preview.clone()),
            last_message_at_ms: contact.last_message_at_ms.or(base.last_message_at_ms),
            unread_count: if contact.last_message_preview.is_some()
                || contact.last_message_at_ms.is_some()
            {
                contact.unread_count
            } else {
                base.unread_count
            },
            created_at_ms: base.created_at_ms.or(Some(now)),
            updated_at_ms: Some(now),
            is_known: contact.is_known || base.is_known,
            is_blocked: contact.is_blocked || base.is_blocked,
            chat_room_exit_at_ms: contact
                .chat_room_exit_at_ms
                .or(base.chat_room_exit_at_ms),
        };
        if let Some(i) = idx {
            list[i] = next.clone();
        } else {
            list.push(next.clone());
        }
        flow_log::info(
            "Contacts",
            format!(
                "upsert pk={} roster_size={}",
                short_hex(&next.public_key_hex),
                list.len()
            ),
        );
        Ok((next, all))
    })
}

pub fn remove_contact(app_namespace: &str, contact: &SavedContact) -> Result<(), ContactsError> {
    with_store(app_namespace, |_path, mut all| {
        let list = all.entry(app_namespace.to_string()).or_default();
        if let Some(i) = index_of(list, contact) {
            list.remove(i);
        }
        Ok(((), all))
    })
}

pub fn merge_discovered_peer_id(
    app_namespace: &str,
    public_key_hex: &str,
    _legacy_conversation_key: &str,
) -> Result<(), ContactsError> {
    let pk = public_key_hex.trim().to_lowercase();
    if !is_valid_public_key_hex(&pk) {
        return Ok(());
    }
    if find_by_public_key(app_namespace, &pk)?.is_some() {
        return Ok(());
    }
    let _ = upsert_contact(
        app_namespace,
        SavedContact {
            public_key_hex: pk,
            display_alias: None,
            last_message_preview: None,
            last_message_at_ms: None,
            unread_count: 0,
            created_at_ms: Some(now_ms()),
            updated_at_ms: Some(now_ms()),
            is_known: false,
            is_blocked: false,
            chat_room_exit_at_ms: None,
        },
    )?;
    Ok(())
}

/// Update trust flags for an existing contact (Add / Block / unblock).
pub fn set_contact_trust(
    app_namespace: &str,
    public_key_hex: &str,
    is_known: Option<bool>,
    is_blocked: Option<bool>,
) -> Result<SavedContact, ContactsError> {
    let pk = public_key_hex.trim().to_lowercase();
    if !is_valid_public_key_hex(&pk) {
        return Err(ContactsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid public_key_hex",
        )));
    }
    let now = now_ms();
    with_store(app_namespace, |_path, mut all| {
        let list = all.entry(app_namespace.to_string()).or_default();
        let Some(i) = list.iter().position(|c| c.public_key_hex == pk) else {
            return Err(ContactsError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "contact not found",
            )));
        };
        let mut c = list[i].clone();
        if let Some(k) = is_known {
            c.is_known = k;
        }
        if let Some(b) = is_blocked {
            c.is_blocked = b;
        }
        c.updated_at_ms = Some(now);
        list[i] = c.clone();
        flow_log::info(
            "Contacts",
            format!(
                "trust pk={} is_known={} is_blocked={}",
                short_hex(&pk),
                c.is_known,
                c.is_blocked
            ),
        );
        Ok((c, all))
    })
}

/// Mirror the live chat-room session clock onto the foreground contact (DESIGN.md).
pub fn sync_chat_room_exit_at_ms(
    app_namespace: &str,
    public_key_hex: &str,
    at_ms: i64,
) -> Result<(), ContactsError> {
    if at_ms <= 0 {
        return Ok(());
    }
    let pk = public_key_hex.trim().to_lowercase();
    if !is_valid_public_key_hex(&pk) {
        return Ok(());
    }
    with_store(app_namespace, |_path, mut all| {
        let list = all.entry(app_namespace.to_string()).or_default();
        let Some(i) = list.iter().position(|c| c.public_key_hex == pk) else {
            return Ok(((), all));
        };
        let mut c = list[i].clone();
        c.chat_room_exit_at_ms = Some(at_ms);
        list[i] = c;
        Ok(((), all))
    })
}

pub fn chat_room_exit_at_ms(
    app_namespace: &str,
    public_key_hex: &str,
) -> Result<Option<i64>, ContactsError> {
    Ok(find_by_public_key(app_namespace, public_key_hex)?
        .and_then(|c| c.chat_room_exit_at_ms))
}

/// Hub preview for the latest line in a DM thread (`contact_public_key_hex` is the **other** party).
pub fn record_thread_message_preview(
    app_namespace: &str,
    contact_public_key_hex: &str,
    preview: &str,
    mark_unread: bool,
    message_at_ms: Option<i64>,
) -> Result<(), ContactsError> {
    record_inbound_preview(
        app_namespace,
        contact_public_key_hex,
        preview,
        mark_unread,
        message_at_ms,
    )
}

/// Update contact list preview for the latest message in a thread (`contact_public_key_hex`
/// is the **other** party — inbound sender or outbound recipient).
pub fn record_inbound_preview(
    app_namespace: &str,
    contact_public_key_hex: &str,
    preview: &str,
    mark_unread: bool,
    message_at_ms: Option<i64>,
) -> Result<(), ContactsError> {
    let pk = contact_public_key_hex.trim().to_lowercase();
    if !is_valid_public_key_hex(&pk) {
        return Ok(());
    }
    let at = message_at_ms.unwrap_or_else(now_ms);
    with_store(app_namespace, |_path, mut all| {
        let list = all.entry(app_namespace.to_string()).or_default();
        let Some(i) = list.iter().position(|c| c.public_key_hex == pk) else {
            return Ok(((), all));
        };
        let c = &list[i];
        // Out-of-order apply (poll replay, wire batch) must still bump unread per message.
        if c.last_message_at_ms.is_some_and(|t| at < t) {
            if mark_unread {
                list[i] = SavedContact {
                    unread_count: c.unread_count.saturating_add(1),
                    updated_at_ms: Some(now_ms()),
                    chat_room_exit_at_ms: c.chat_room_exit_at_ms,
                    ..c.clone()
                };
            }
            return Ok(((), all));
        }
        let p = truncate_preview(preview);
        list[i] = SavedContact {
            public_key_hex: c.public_key_hex.clone(),
            display_alias: c.display_alias.clone(),
            last_message_preview: Some(p),
            last_message_at_ms: Some(at),
            // Foreground room clears via `clear_unread`; in-room inbound uses mark_unread false without wiping backlog.
            unread_count: if mark_unread {
                c.unread_count.saturating_add(1)
            } else {
                c.unread_count
            },
            created_at_ms: c.created_at_ms,
            updated_at_ms: Some(now_ms()),
            is_known: c.is_known,
            is_blocked: c.is_blocked,
            chat_room_exit_at_ms: c.chat_room_exit_at_ms,
        };
        Ok(((), all))
    })
}

/// Hub roster preview from merged transcript (chronological latest line). Repairs stale preview
/// when inbound batches arrive out of order after local outbound sends (DESIGN.md — Rust owns contacts).
pub fn refresh_thread_preview_from_transcript(
    app_namespace: &str,
    contact_public_key_hex: &str,
    wire_peer_id: Option<&str>,
) -> Result<(), ContactsError> {
    use crate::dm_transcript_store::load_merged;

    let pk = contact_public_key_hex.trim().to_lowercase();
    if !is_valid_public_key_hex(&pk) {
        return Ok(());
    }
    let mut keys = vec![pk.clone()];
    if let Ok(Some(c)) = find_by_public_key(app_namespace, &pk) {
        let ck = c.conversation_key();
        if !ck.is_empty() {
            keys.push(ck);
        }
    }
    if let Some(w) = wire_peer_id.map(str::trim).filter(|s| !s.is_empty()) {
        keys.push(w.to_string());
    }
    let rows = load_merged(app_namespace, &keys, wire_peer_id).unwrap_or_default();
    let Some(latest) = rows.iter().max_by(|a, b| {
        let ta = a.created_at_ms.unwrap_or(0);
        let tb = b.created_at_ms.unwrap_or(0);
        ta.cmp(&tb).then_with(|| {
            a.message_id
                .as_deref()
                .unwrap_or("")
                .cmp(b.message_id.as_deref().unwrap_or(""))
        })
    }) else {
        return Ok(());
    };
    record_thread_message_preview(
        app_namespace,
        &pk,
        &latest.text,
        false,
        latest.created_at_ms.filter(|t| *t > 0),
    )
}

pub fn clear_unread(app_namespace: &str, public_key_hex: &str) -> Result<(), ContactsError> {
    let pk = public_key_hex.trim().to_lowercase();
    if !is_valid_public_key_hex(&pk) {
        return Ok(());
    }
    with_store(app_namespace, |_path, mut all| {
        let list = all.entry(app_namespace.to_string()).or_default();
        let Some(i) = list.iter().position(|c| c.public_key_hex == pk) else {
            return Ok(((), all));
        };
        let c = list[i].clone();
        list[i] = SavedContact {
            unread_count: 0,
            updated_at_ms: Some(now_ms()),
            ..c
        };
        Ok(((), all))
    })
}

fn truncate_preview(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 120 {
        return t.to_string();
    }
    let mut out: String = t.chars().take(117).collect();
    out.push('…');
    out
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1_with_algorithm;
    use crate::identity::IdentityAlgorithm;
    use crate::storage::{StorageConfig, create_or_unlock_identity_v1};
    use tempfile::TempDir;

    /// Guest-scanned host identity wire — never the local identity under test.
    const REMOTE_PK: &str = "02f229f167ac2337144dbeba4392a6300c8fe97fb061efdb4f81ec9f29dec76936";

    #[test]
    fn upsert_display_alias_set_and_clear() {
        let td = TempDir::new().unwrap();
        let ns = "dev.contacts.alias";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let _id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let pk = "0305b1b0d27745e0a38a7254ea100abc38857b51ded2ac7ea88d3063fb8da21784";

        upsert_contact(
            ns,
            SavedContact {
                public_key_hex: pk.to_string(),
                display_alias: Some("Alice".to_string()),
                last_message_preview: None,
                last_message_at_ms: None,
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: true,
                is_blocked: false,
                chat_room_exit_at_ms: None,
            },
        )
        .unwrap();

        upsert_contact(
            ns,
            SavedContact {
                public_key_hex: pk.to_string(),
                display_alias: Some("Bob".to_string()),
                last_message_preview: None,
                last_message_at_ms: None,
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: true,
                is_blocked: false,
                chat_room_exit_at_ms: None,
            },
        )
        .unwrap();
        let c = find_by_public_key(ns, pk).unwrap().unwrap();
        assert_eq!(c.display_alias.as_deref(), Some("Bob"));

        upsert_contact(
            ns,
            SavedContact {
                public_key_hex: pk.to_string(),
                display_alias: Some("".to_string()),
                last_message_preview: None,
                last_message_at_ms: None,
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: true,
                is_blocked: false,
                chat_room_exit_at_ms: None,
            },
        )
        .unwrap();
        let c = find_by_public_key(ns, pk).unwrap().unwrap();
        assert_eq!(c.display_alias, None);
    }

    #[test]
    fn set_contact_trust_updates_flags() {
        let td = TempDir::new().unwrap();
        let ns = "dev.contacts.trust";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let _id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();

        upsert_contact(
            ns,
            SavedContact {
                public_key_hex: REMOTE_PK.to_string(),
                display_alias: None,
                last_message_preview: None,
                last_message_at_ms: None,
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: false,
                is_blocked: false,
                chat_room_exit_at_ms: None,
            },
        )
        .unwrap();

        let c = set_contact_trust(ns, REMOTE_PK, Some(true), None).unwrap();
        assert!(c.is_known);
        assert!(!c.is_blocked);

        let c = set_contact_trust(ns, REMOTE_PK, None, Some(true)).unwrap();
        assert!(c.is_blocked);
    }

    #[test]
    fn upsert_and_find_ed25519_prefixed_identity() {
        let td = TempDir::new().unwrap();
        let ns = "dev.contacts.ed25519";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let _id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let (_remote_ks, remote) =
            create_keystore_v1_with_algorithm("pw2", IdentityAlgorithm::Ed25519, None).unwrap();
        let wire = remote.identity_wire();

        upsert_contact(
            ns,
            SavedContact {
                public_key_hex: wire.clone(),
                display_alias: Some("Ed peer".to_string()),
                last_message_preview: None,
                last_message_at_ms: None,
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: true,
                is_blocked: false,
                chat_room_exit_at_ms: None,
            },
        )
        .unwrap();

        let found = find_by_public_key(ns, &wire).unwrap().unwrap();
        assert_eq!(found.public_key_hex, wire);
        assert_eq!(found.display_alias.as_deref(), Some("Ed peer"));
    }

    #[test]
    fn refresh_thread_preview_uses_latest_created_at_in_transcript() {
        use crate::c_ffi::configure_android_data_directory;
        use crate::dm_transcript_store::{StoredChatLine, append_if_new};

        let _guard = crate::c_ffi::test_storage_isolation_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let td = TempDir::new().unwrap();
        configure_android_data_directory(td.path().to_str().unwrap());
        let ns = "dev.contacts.preview";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let _id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        upsert_contact(
            ns,
            SavedContact {
                public_key_hex: REMOTE_PK.to_string(),
                display_alias: None,
                last_message_preview: Some("stale".to_string()),
                last_message_at_ms: Some(1000),
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: true,
                is_blocked: false,
                chat_room_exit_at_ms: None,
            },
        )
        .unwrap();
        append_if_new(
            ns,
            REMOTE_PK,
            StoredChatLine {
                local_id: "a".into(),
                text: "older inbound".into(),
                outgoing: false,
                from: Some("peer".into()),
                message_id: Some("mid-old".into()),
                delivery: "pending".into(),
                created_at_ms: Some(1000),
                received_at_ms: Some(1000),
                read_ack_sent: false,
            },
        )
        .unwrap();
        append_if_new(
            ns,
            REMOTE_PK,
            StoredChatLine {
                local_id: "b".into(),
                text: "newer outbound".into(),
                outgoing: true,
                from: None,
                message_id: Some("mid-new".into()),
                delivery: "pending".into(),
                created_at_ms: Some(2000),
                received_at_ms: None,
                read_ack_sent: false,
            },
        )
        .unwrap();
        refresh_thread_preview_from_transcript(ns, REMOTE_PK, None).unwrap();
        let c = find_by_public_key(ns, REMOTE_PK).unwrap().unwrap();
        assert_eq!(c.last_message_preview.as_deref(), Some("newer outbound"));
        assert_eq!(c.last_message_at_ms, Some(2000));
    }
}
