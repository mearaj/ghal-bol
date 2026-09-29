//! Shared on-disk paths for app data under the same namespace root as the keystore.
//!
//! Linux: `~/.local/share/com.ghalbol.debug/ghal_bol/` (debug) or `…/com.ghalbol/ghal_bol/`.
//! Android: `{app_makepad}/{namespace}/ghal_bol/` (debug) or `{app_makepad}/ghal_bol/` (release).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::storage::{KeystoreStorageError, StorageConfig, namespace_data_dir};

fn android_data_dir_mx() -> &'static Mutex<Option<PathBuf>> {
    static D: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(None))
}

/// Point keystore and chat stores at an Android files directory (or a test temp dir).
#[cfg(any(target_os = "android", test))]
pub(crate) fn configure_android_data_directory(path: &str) {
    if let Ok(mut g) = android_data_dir_mx().lock() {
        *g = Some(PathBuf::from(path));
    }
}

/// Drop the test/Android override so later tests use the normal data directory.
#[cfg(test)]
pub(crate) fn clear_test_data_directory() {
    if let Ok(mut g) = android_data_dir_mx().lock() {
        *g = None;
    }
}

/// Serialize tests that mutate the process-global data root.
#[cfg(test)]
pub(crate) fn test_storage_isolation_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

/// Android files root when the platform configured one.
#[cfg(target_os = "android")]
pub(crate) fn optional_android_data_dir() -> Option<PathBuf> {
    android_data_dir_mx().lock().ok().and_then(|g| g.clone())
}

pub(crate) fn resolved_storage_config(ns: &str) -> StorageConfig {
    let mut cfg = StorageConfig::new(ns.to_owned());
    if let Ok(g) = android_data_dir_mx().lock() {
        if let Some(dir) = g.as_ref() {
            cfg = cfg.with_override_data_dir(dir.clone());
        }
    }
    cfg
}

/// `{namespace_data_dir}/ghal_bol/` — contacts, transcript (user-owned; no transport cache).
pub fn ui_data_dir(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    let mut p = namespace_data_dir(cfg)?;
    p.push("ghal_bol");
    Ok(p)
}

pub fn contacts_v1_path(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    let mut p = ui_data_dir(cfg)?;
    p.push("contacts_v1.json");
    Ok(p)
}

pub fn chat_transcript_v1_path(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    let mut p = ui_data_dir(cfg)?;
    p.push("chat_transcript_v1.json");
    Ok(p)
}

pub fn storage_config_for_namespace(app_namespace: &str) -> StorageConfig {
    resolved_storage_config(app_namespace)
}

/// Resolve the installed app namespace when the daemon starts without an unlocked session.
///
/// Order: `GHAL_BOL_APP_NAMESPACE` (when that keystore exists), then release, then debug.
pub fn detect_keystore_app_namespace() -> Option<String> {
    if let Ok(raw) = std::env::var("GHAL_BOL_APP_NAMESPACE") {
        let ns = raw.trim();
        if !ns.is_empty() {
            let cfg = StorageConfig::new(ns);
            if crate::keystore_v1_file_exists(&cfg).unwrap_or(false) {
                return Some(ns.to_string());
            }
        }
    }
    for ns in [
        crate::storage::ANDROID_LIBRARY_NAMESPACE,
        "com.ghalbol.debug",
    ] {
        let cfg = StorageConfig::new(ns);
        if crate::keystore_v1_file_exists(&cfg).unwrap_or(false) {
            return Some(ns.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{ANDROID_LIBRARY_NAMESPACE, StorageConfig};
    use tempfile::TempDir;

    /// Android-style override: chat stores live under the same namespace dir as keystore.
    #[test]
    fn ui_data_dir_under_namespace_root_not_package_base() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new("com.ghalbol.debug").with_override_data_dir(td.path());
        let contacts = contacts_v1_path(&cfg).unwrap();
        assert_eq!(
            contacts,
            td.path()
                .join("com.ghalbol.debug")
                .join("ghal_bol")
                .join("contacts_v1.json")
        );
    }

    #[test]
    fn release_namespace_ui_data_under_package_root() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new(ANDROID_LIBRARY_NAMESPACE).with_override_data_dir(td.path());
        let contacts = contacts_v1_path(&cfg).unwrap();
        assert_eq!(
            contacts,
            td.path().join("ghal_bol").join("contacts_v1.json")
        );
    }
}
