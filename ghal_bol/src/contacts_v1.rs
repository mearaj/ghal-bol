//! Persisted 1:1 contacts (`contacts_v1.json`) — same format as Flutter [ContactStore].

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde_json::Value;
use thiserror::Error;

use crate::app_paths::{contacts_v1_path, storage_config_for_namespace};
use crate::flow_log::{self, short_hex};
use crate::dm_transport::normalize_contact_pk;
use crate::public_key_util::{legacy_public_key_from_peer_id_str, secp256k1_public_key_from_hex};
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
            display_alias: obj.get("display_alias").and_then(|v| v.as_str()).map(str::to_string),
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
        })
    }
}

pub fn is_valid_public_key_hex(hex_s: &str) -> bool {
    secp256k1_public_key_from_hex(hex_s).is_ok()
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

fn with_store<T>(app_namespace: &str, f: impl FnOnce(&Path, HashMap<String, Vec<SavedContact>>) -> Result<(T, HashMap<String, Vec<SavedContact>>), ContactsError>) -> Result<T, ContactsError> {
    let _guard = io_chain().lock().map_err(|_| ContactsError::Io(std::io::Error::other("contacts io mutex poisoned")))?;
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
    let pk = contact.public_key_hex.trim();
    if is_valid_public_key_hex(pk) {
        return list.iter().position(|c| c.public_key_hex == pk);
    }
    None
}

pub fn list_contacts(app_namespace: &str) -> Result<Vec<SavedContact>, ContactsError> {
    let cfg = storage_config_for_namespace(app_namespace);
    let path = contacts_v1_path(&cfg)?;
    let all = read_all(&path)?;
    let mut list = all.get(app_namespace).cloned().unwrap_or_default();
    list.sort_by(|a, b| {
        let ta = a.last_message_at_ms.or(a.updated_at_ms).or(a.created_at_ms).unwrap_or(0);
        let tb = b.last_message_at_ms.or(b.updated_at_ms).or(b.created_at_ms).unwrap_or(0);
        tb.cmp(&ta)
    });
    Ok(list)
}

pub fn find_by_public_key(app_namespace: &str, public_key_hex: &str) -> Result<Option<SavedContact>, ContactsError> {
    let pk = public_key_hex.trim().to_lowercase();
    if !is_valid_public_key_hex(&pk) {
        return Ok(None);
    }
    Ok(list_contacts(app_namespace)?
        .into_iter()
        .find(|c| c.public_key_hex == pk))
}

/// Lookup by public key hex, or legacy libp2p PeerId string on disk (migrated to pk).
pub fn find_by_peer_id(app_namespace: &str, conversation_key: &str) -> Result<Option<SavedContact>, ContactsError> {
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

pub fn upsert_contact(app_namespace: &str, contact: SavedContact) -> Result<SavedContact, ContactsError> {
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
            contact.public_key_hex.trim().to_lowercase()
        } else {
            base.public_key_hex.clone()
        };
        let next = SavedContact {
            public_key_hex: pk_norm,
            display_alias: contact.display_alias.clone().or(base.display_alias.clone()),
            last_message_preview: contact.last_message_preview.clone().or(base.last_message_preview.clone()),
            last_message_at_ms: contact.last_message_at_ms.or(base.last_message_at_ms),
            unread_count: contact.unread_count,
            created_at_ms: base.created_at_ms.or(Some(now)),
            updated_at_ms: Some(now),
            is_known: contact.is_known || base.is_known,
            is_blocked: contact.is_blocked || base.is_blocked,
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
        if c.last_message_at_ms.is_some_and(|t| at < t) {
            return Ok(((), all));
        }
        let p = truncate_preview(preview);
        list[i] = SavedContact {
            public_key_hex: c.public_key_hex.clone(),
            display_alias: c.display_alias.clone(),
            last_message_preview: Some(p),
            last_message_at_ms: Some(at),
            // DESIGN.md: unread only when not foreground; viewing the room clears the badge.
            unread_count: if mark_unread {
                c.unread_count.saturating_add(1)
            } else {
                0
            },
            created_at_ms: c.created_at_ms,
            updated_at_ms: Some(now_ms()),
            is_known: c.is_known,
            is_blocked: c.is_blocked,
        };
        Ok(((), all))
    })
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
    use crate::storage::{create_or_unlock_identity_v1, StorageConfig};
    use tempfile::TempDir;

    #[test]
    fn record_inbound_preview_clears_unread_when_not_marking() {
        let td = TempDir::new().unwrap();
        let ns = "dev.contacts.unread";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let other_pk = id.public_key_hex();

        upsert_contact(
            ns,
            SavedContact {
                public_key_hex: other_pk.to_string(),
                display_alias: None,
                last_message_preview: None,
                last_message_at_ms: None,
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: true,
                is_blocked: false,
            },
        )
        .unwrap();

        record_inbound_preview(ns, &other_pk, "hello", true, Some(1000)).unwrap();
        let c = find_by_public_key(ns, &other_pk).unwrap().unwrap();
        assert_eq!(c.unread_count, 1);

        record_inbound_preview(ns, &other_pk, "seen in room", false, Some(2000)).unwrap();
        let c = find_by_public_key(ns, &other_pk).unwrap().unwrap();
        assert_eq!(c.unread_count, 0);
        assert_eq!(
            c.last_message_preview.as_deref(),
            Some("seen in room")
        );
    }

    #[test]
    fn set_contact_trust_updates_flags() {
        let td = TempDir::new().unwrap();
        let ns = "dev.contacts.trust";
        let cfg = StorageConfig::new(ns).with_override_data_dir(td.path());
        let id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let other_pk = id.public_key_hex();

        upsert_contact(
            ns,
            SavedContact {
                public_key_hex: other_pk.to_string(),
                display_alias: None,
                last_message_preview: None,
                last_message_at_ms: None,
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: false,
                is_blocked: false,
            },
        )
        .unwrap();

        let c = set_contact_trust(ns, &other_pk, Some(true), None).unwrap();
        assert!(c.is_known);
        assert!(!c.is_blocked);

        let c = set_contact_trust(ns, &other_pk, None, Some(true)).unwrap();
        assert!(c.is_blocked);
    }
}
