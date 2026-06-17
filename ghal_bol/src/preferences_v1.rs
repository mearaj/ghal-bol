//! App-level preferences (non-secret) stored next to the keystore under the same [`StorageConfig`].
//!
//! v1 file: **`{namespace_data_dir}/preferences_v1.json`** (see [`crate::storage::namespace_data_dir`]).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::DecryptedIdentity;
use crate::storage::{KeystoreStorageError, StorageConfig};

const PREFERENCES_FORMAT_VERSION: u64 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct PreferencesV1 {
    pub format_version: u64,
    #[serde(default)]
    pub peer_display_aliases: HashMap<String, String>,
    #[serde(default)]
    pub coord_base_url: Option<String>,
    #[serde(default)]
    pub coord_insecure_tls: bool,
}

impl Default for PreferencesV1 {
    fn default() -> Self {
        Self {
            format_version: PREFERENCES_FORMAT_VERSION,
            peer_display_aliases: HashMap::new(),
            coord_base_url: None,
            coord_insecure_tls: false,
        }
    }
}

pub fn preferences_v1_path(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    let mut p = crate::storage::namespace_data_dir(cfg)?;
    p.push("preferences_v1.json");
    Ok(p)
}

fn load_preferences_v1(cfg: &StorageConfig) -> Result<PreferencesV1, KeystoreStorageError> {
    let path = preferences_v1_path(cfg)?;
    if !path.exists() {
        return Ok(PreferencesV1::default());
    }
    let s = fs::read_to_string(&path)?;
    let mut p: PreferencesV1 = serde_json::from_str(&s).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("preferences json: {e}"))
    })?;
    if p.format_version != PREFERENCES_FORMAT_VERSION {
        return Err(KeystoreStorageError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported preferences format_version",
        )));
    }
    p.peer_display_aliases = p
        .peer_display_aliases
        .into_iter()
        .map(|(k, v)| (k.trim().to_lowercase(), v))
        .collect();
    Ok(p)
}

fn save_preferences_v1(
    cfg: &StorageConfig,
    prefs: &PreferencesV1,
) -> Result<(), KeystoreStorageError> {
    let path = preferences_v1_path(cfg)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(prefs).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("preferences encode: {e}"),
        )
    })?;
    fs::write(&tmp_path, json.as_bytes())?;
    fs::rename(&tmp_path, &path)?;
    Ok(())
}

fn session_public_key_hex_lower(ident: &DecryptedIdentity) -> String {
    ident.public_key_hex().to_lowercase()
}

fn verify_public_key_matches_session(
    ident: &DecryptedIdentity,
    public_key_hex: &str,
) -> Result<(), String> {
    let want = public_key_hex.trim().to_lowercase();
    let cur = session_public_key_hex_lower(ident);
    if want != cur {
        return Err("public_key_hex does not match unlocked identity".to_string());
    }
    Ok(())
}

/// Trim / single-line / max length — display hint only (not cryptographic).
pub fn sanitize_peer_display_alias(raw: &str) -> Option<String> {
    let mut t = raw.trim().to_string();
    if t.is_empty() {
        return None;
    }
    for ch in ['\n', '\r', '\t'] {
        t = t.replace(ch, " ");
    }
    const MAX: usize = 64;
    if t.len() > MAX {
        t.truncate(MAX);
        while !t.is_empty() && t.ends_with(' ') {
            t.pop();
        }
    }
    let t = t.trim().to_string();
    if t.is_empty() { None } else { Some(t) }
}

/// Read stored display alias for the **current unlocked** identity (must match [public_key_hex]).
pub fn peer_display_alias_get(
    cfg: &StorageConfig,
    session: &DecryptedIdentity,
    public_key_hex: &str,
) -> Result<Option<String>, KeystoreStorageError> {
    verify_public_key_matches_session(session, public_key_hex).map_err(|e| {
        KeystoreStorageError::Io(io::Error::new(io::ErrorKind::PermissionDenied, e))
    })?;
    let key = session_public_key_hex_lower(session);
    let prefs = load_preferences_v1(cfg)?;
    Ok(prefs.peer_display_aliases.get(&key).cloned())
}

/// Set or clear display alias for the **current unlocked** identity.
pub fn peer_display_alias_set(
    cfg: &StorageConfig,
    session: &DecryptedIdentity,
    public_key_hex: &str,
    alias_raw_utf8: &str,
) -> Result<Option<String>, KeystoreStorageError> {
    verify_public_key_matches_session(session, public_key_hex).map_err(|e| {
        KeystoreStorageError::Io(io::Error::new(io::ErrorKind::PermissionDenied, e))
    })?;
    let key = session_public_key_hex_lower(session);
    let mut prefs = load_preferences_v1(cfg)?;
    if let Some(s) = sanitize_peer_display_alias(alias_raw_utf8) {
        prefs.peer_display_aliases.insert(key, s.clone());
        save_preferences_v1(cfg, &prefs)?;
        Ok(Some(s))
    } else {
        prefs.peer_display_aliases.remove(&key);
        save_preferences_v1(cfg, &prefs)?;
        Ok(None)
    }
}

pub fn coord_settings_get(
    cfg: &StorageConfig,
) -> Result<(Option<String>, bool), KeystoreStorageError> {
    let prefs = load_preferences_v1(cfg)?;
    Ok((prefs.coord_base_url.clone(), prefs.coord_insecure_tls))
}

pub fn coord_settings_set(
    cfg: &StorageConfig,
    base_url: &str,
    insecure_tls: bool,
) -> Result<(), KeystoreStorageError> {
    let t = base_url.trim().trim_end_matches('/').to_string();
    if t.is_empty() {
        return Err(KeystoreStorageError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "coord base url empty",
        )));
    }
    let mut prefs = load_preferences_v1(cfg)?;
    prefs.coord_base_url = Some(t);
    prefs.coord_insecure_tls = insecure_tls;
    save_preferences_v1(cfg, &prefs)
}

/// Removes `preferences_v1.json` for [cfg] if present.
pub fn delete_preferences_v1_file(cfg: &StorageConfig) -> Result<(), KeystoreStorageError> {
    let path = preferences_v1_path(cfg)?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_or_unlock_identity_v1;
    use tempfile::TempDir;

    #[test]
    fn roundtrip_alias() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new("dev.prefs").with_override_data_dir(td.path());
        let id = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        assert!(peer_display_alias_get(&cfg, &id, "not_hex").is_err());

        let sig = id.public_key_hex();
        assert_eq!(peer_display_alias_get(&cfg, &id, &sig).unwrap(), None);

        let stored = peer_display_alias_set(&cfg, &id, &sig, "  Alice  ").unwrap();
        assert_eq!(stored.as_deref(), Some("Alice"));

        assert_eq!(
            peer_display_alias_get(&cfg, &id, &sig).unwrap().as_deref(),
            Some("Alice")
        );

        let _ = peer_display_alias_set(&cfg, &id, &sig, "   ").unwrap();
        assert_eq!(peer_display_alias_get(&cfg, &id, &sig).unwrap(), None);
    }
}
