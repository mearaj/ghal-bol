use std::fs;
use std::io;
use std::path::PathBuf;

use directories::ProjectDirs;
use thiserror::Error;

use crate::keystore_v1::{
    DecryptedIdentity, KeystoreError, KeystoreV1, create_keystore_v1, create_keystore_v1_from_secret,
    parse_secret_key_hex, secret_key_hex_from_identity, unlock_keystore_v1,
};

/// Product / packaging id for **`ghal_bol`** (Rust keystore storage root via `directories`).
///
/// Matches the Flutter **`ghal_bol_ui`** Android `applicationId` / iOS bundle id **`com.ghalbol`**.
/// Use this symbol from JNI / FFI when aligning native paths.
pub const ANDROID_LIBRARY_NAMESPACE: &str = "com.ghalbol";

/// Configuration for where `ghal_bol` stores its encrypted keystore blob.
#[derive(Clone, Debug)]
pub struct StorageConfig {
    /// Logical namespace for isolating installs (dev/prod flavors, multiple apps, etc.).
    ///
    /// This is appended under the per-platform app data directory.
    pub app_namespace: String,

    /// Optional override root directory (useful for Android internal storage path or tests).
    pub override_data_dir: Option<PathBuf>,
}

impl StorageConfig {
    pub fn new(app_namespace: impl Into<String>) -> Self {
        Self { app_namespace: app_namespace.into(), override_data_dir: None }
    }

    pub fn with_override_data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.override_data_dir = Some(dir.into());
        self
    }
}

#[derive(Debug, Error)]
pub enum KeystoreStorageError {
    #[error("keystore crypto: {0}")]
    Crypto(#[from] KeystoreError),

    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("cannot determine platform data directory")]
    NoDataDir,

    #[error("invalid app_namespace")]
    BadNamespace,

    #[error("ANDROID_LIBRARY_NAMESPACE must be qualifier.org.application (three dot-separated segments)")]
    InvalidLibraryNamespace,
}

#[derive(Clone, Debug)]
pub struct StoredKeystore {
    pub path: PathBuf,
    pub keystore: KeystoreV1,
}

pub(crate) fn sanitize_namespace(ns: &str) -> Result<(), KeystoreStorageError> {
    let ns = ns.trim();
    if ns.is_empty() || ns.len() > 200 {
        return Err(KeystoreStorageError::BadNamespace);
    }
    // Keep it path-safe.
    if ns.contains("..") || ns.contains('/') || ns.contains('\\') {
        return Err(KeystoreStorageError::BadNamespace);
    }
    Ok(())
}

/// Resolves [`ProjectDirs`] from [`ANDROID_LIBRARY_NAMESPACE`] (`com.ghalbol`).
pub fn project_dirs_for_library() -> Result<ProjectDirs, KeystoreStorageError> {
    return ProjectDirs::from_path(PathBuf::from(ANDROID_LIBRARY_NAMESPACE))
    .ok_or(KeystoreStorageError::NoDataDir);
}

pub fn base_data_dir(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    if let Some(d) = &cfg.override_data_dir {
        return Ok(d.clone());
    }

    let dirs = project_dirs_for_library()?;
    Ok(dirs.data_local_dir().to_path_buf())
}

/// Directory for identity + prefs under [base_data_dir].
///
/// When [StorageConfig::app_namespace] matches [ANDROID_LIBRARY_NAMESPACE], the base path
/// is already `…/com.ghalbol/` — do not append the namespace segment again.
pub fn namespace_data_dir(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    sanitize_namespace(&cfg.app_namespace)?;
    let mut p = base_data_dir(cfg)?;
    let ns = cfg.app_namespace.trim();
    if ns != ANDROID_LIBRARY_NAMESPACE {
        p.push(ns);
    }
    Ok(p)
}

pub fn keystore_v1_path(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    let mut p = namespace_data_dir(cfg)?;
    p.push("keystore_v1.json");
    Ok(p)
}

/// Whether the encrypted keystore file is present (does not verify password or JSON).
pub fn keystore_v1_file_exists(cfg: &StorageConfig) -> Result<bool, KeystoreStorageError> {
    let path = keystore_v1_path(cfg)?;
    Ok(path.exists())
}

pub fn load_keystore_v1(cfg: &StorageConfig) -> Result<Option<StoredKeystore>, KeystoreStorageError> {
    let path = keystore_v1_path(cfg)?;
    if !path.exists() {
        return Ok(None);
    }
    let s = fs::read_to_string(&path)?;
    let ks = KeystoreV1::from_json_str(&s)?;
    Ok(Some(StoredKeystore { path, keystore: ks }))
}

pub fn save_keystore_v1(cfg: &StorageConfig, ks: &KeystoreV1) -> Result<PathBuf, KeystoreStorageError> {
    let path = keystore_v1_path(cfg)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("json.tmp");
    let json = ks.to_json_string()?;
    fs::write(&tmp_path, json.as_bytes())?;
    fs::rename(&tmp_path, &path)?;
    Ok(path)
}

/// High-level helper for UI:
/// - if no keystore exists: create it with `password`, persist it, and return decrypted identity
/// - if keystore exists: unlock it with `password` and return decrypted identity
pub fn create_or_unlock_identity_v1(
    cfg: &StorageConfig,
    password: &str,
) -> Result<DecryptedIdentity, KeystoreStorageError> {
    if let Some(stored) = load_keystore_v1(cfg)? {
        let id = unlock_keystore_v1(password, &stored.keystore)?;
        return Ok(id);
    }

    let (ks, id) = create_keystore_v1(password, None)?;
    let _ = save_keystore_v1(cfg, &ks)?;
    Ok(id)
}

/// First-time setup: encrypt [secret_key_hex] (64 hex chars) with [password] and persist.
/// Fails if a keystore already exists (delete first).
pub fn import_identity_from_secret_hex_v1(
    cfg: &StorageConfig,
    password: &str,
    secret_key_hex: &str,
) -> Result<DecryptedIdentity, KeystoreStorageError> {
    if keystore_v1_file_exists(cfg)? {
        return Err(KeystoreStorageError::Crypto(KeystoreError::Invalid(
            "keystore already exists; delete identity before import",
        )));
    }
    let secret = parse_secret_key_hex(secret_key_hex)?;
    let (ks, id) = create_keystore_v1_from_secret(password, &secret, None)?;
    let _ = save_keystore_v1(cfg, &ks)?;
    Ok(id)
}

/// Verify [password] and return the 64-char secret key hex (sensitive).
pub fn reveal_secret_key_hex_v1(
    cfg: &StorageConfig,
    password: &str,
) -> Result<String, KeystoreStorageError> {
    let stored = load_keystore_v1(cfg)?
        .ok_or(KeystoreStorageError::Crypto(KeystoreError::Invalid("no keystore on disk")))?;
    let id = unlock_keystore_v1(password, &stored.keystore)?;
    Ok(secret_key_hex_from_identity(&id))
}

/// Export encrypted `keystore_v1.json` contents (password still required to decrypt elsewhere).
pub fn export_keystore_json_v1(cfg: &StorageConfig) -> Result<String, KeystoreStorageError> {
    let stored = load_keystore_v1(cfg)?
        .ok_or(KeystoreStorageError::Crypto(KeystoreError::Invalid("no keystore on disk")))?;
    Ok(stored.keystore.to_json_string()?)
}

/// Restore from exported JSON when no keystore exists; [password] must unlock the blob.
pub fn import_keystore_from_json_v1(
    cfg: &StorageConfig,
    password: &str,
    keystore_json: &str,
) -> Result<DecryptedIdentity, KeystoreStorageError> {
    if keystore_v1_file_exists(cfg)? {
        return Err(KeystoreStorageError::Crypto(KeystoreError::Invalid(
            "keystore already exists; delete identity before import",
        )));
    }
    let ks = KeystoreV1::from_json_str(keystore_json)?;
    let id = unlock_keystore_v1(password, &ks)?;
    let _ = save_keystore_v1(cfg, &ks)?;
    Ok(id)
}

/// Removes `keystore_v1.json` and `preferences_v1.json` for [cfg] if they exist.
pub fn delete_stored_identity_v1(cfg: &StorageConfig) -> Result<(), KeystoreStorageError> {
    let kp = keystore_v1_path(cfg)?;
    if kp.exists() {
        fs::remove_file(&kp)?;
    }
    let tmp = kp.with_extension("json.tmp");
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }
    crate::preferences_v1::delete_preferences_v1_file(cfg)?;
    Ok(())
}

/// Drop a partial or failed first-time setup so the user can retry with a new app password.
///
/// Safe to call when create/import did not complete successfully; removes keystore if present.
pub fn reset_first_time_identity_v1(cfg: &StorageConfig) -> Result<(), KeystoreStorageError> {
    delete_stored_identity_v1(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn keystore_v1_file_exists_false_until_saved() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new("dev.exists")
            .with_override_data_dir(td.path());
        assert!(!keystore_v1_file_exists(&cfg).unwrap());
        let _ = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        assert!(keystore_v1_file_exists(&cfg).unwrap());
    }

    #[test]
    fn create_persists_then_unlocks() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new("dev.test")
            .with_override_data_dir(td.path());

        let id1 = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let id2 = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        assert_eq!(id1.public_key_hex(), id2.public_key_hex());
        assert_eq!(id1.public_key_hex(), id2.public_key_hex());
    }

    #[test]
    fn reset_first_time_removes_keystore() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new("dev.reset")
            .with_override_data_dir(td.path());
        let _ = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        assert!(keystore_v1_file_exists(&cfg).unwrap());
        reset_first_time_identity_v1(&cfg).unwrap();
        assert!(!keystore_v1_file_exists(&cfg).unwrap());
    }

    #[test]
    fn import_secret_hex_persists() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new("dev.import")
            .with_override_data_dir(td.path());
        let sk = {
            let (_ks, id) = create_keystore_v1("old", None).unwrap();
            id.secp256k1_secret().secret_bytes()
        };
        let hex = hex::encode(sk);
        let id = import_identity_from_secret_hex_v1(&cfg, "newpw", &hex).unwrap();
        let id2 = create_or_unlock_identity_v1(&cfg, "newpw").unwrap();
        assert_eq!(id.public_key_hex(), id2.public_key_hex());
    }

    #[test]
    fn export_import_keystore_json_roundtrip() {
        let td = TempDir::new().unwrap();
        let cfg_a = StorageConfig::new("dev.export")
            .with_override_data_dir(td.path().join("a"));
        let cfg_b = StorageConfig::new("dev.export")
            .with_override_data_dir(td.path().join("b"));
        let id1 = create_or_unlock_identity_v1(&cfg_a, "pw").unwrap();
        let json = export_keystore_json_v1(&cfg_a).unwrap();
        let id2 = import_keystore_from_json_v1(&cfg_b, "pw", &json).unwrap();
        assert_eq!(id1.public_key_hex(), id2.public_key_hex());
    }

    #[test]
    fn wrong_password_on_existing_keystore_fails() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new("dev.test")
            .with_override_data_dir(td.path());

        let _ = create_or_unlock_identity_v1(&cfg, "pw").unwrap();
        let err = create_or_unlock_identity_v1(&cfg, "nope").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("decrypt failed") || msg.contains("kdf failed"));
    }

    #[test]
    fn android_library_namespace_matches_flutter_application_id() {
        assert_eq!(ANDROID_LIBRARY_NAMESPACE, "com.ghalbol");
        let _dirs = project_dirs_for_library().expect("valid library namespace");
    }

    #[test]
    fn default_namespace_keystore_lives_at_base_not_nested() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new(ANDROID_LIBRARY_NAMESPACE)
            .with_override_data_dir(td.path());
        let path = keystore_v1_path(&cfg).unwrap();
        assert_eq!(path, td.path().join("keystore_v1.json"));
    }

    #[test]
    fn other_namespace_keystore_has_subdir() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new("dev.test")
            .with_override_data_dir(td.path());
        let path = keystore_v1_path(&cfg).unwrap();
        assert_eq!(path, td.path().join("dev.test").join("keystore_v1.json"));
    }
}

