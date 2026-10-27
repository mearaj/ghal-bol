//! Shared on-disk paths for app data under the same namespace root as the keystore.
//!
//! Linux: `~/.local/share/com.ghalbol.debug/ghalbol/` (debug) or `…/com.ghalbol/ghalbol/`.
//! Android: `{app_makepad}/{namespace}/ghalbol/` (debug) or `{app_makepad}/ghalbol/` (release).
//!
//! Builds before the `ghal_bol` → `ghalbol` rename used a `ghal_bol/` folder. [`ui_data_dir`]
//! moves it once so existing contacts, transcripts, voice notes, and attachments stay visible.

use std::fs;
use std::path::{Path, PathBuf};
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

pub(crate) fn resolved_storage_config(ns: &str) -> StorageConfig {
    let mut cfg = StorageConfig::new(ns.to_owned());
    if let Ok(g) = android_data_dir_mx().lock() {
        if let Some(dir) = g.as_ref() {
            cfg = cfg.with_override_data_dir(dir.clone());
        }
    }
    cfg
}

const UI_DATA_DIR_NAME: &str = "ghalbol";
/// Store folder name written before the `ghal_bol` → `ghalbol` rename.
const LEGACY_UI_DATA_DIR_NAME: &str = "ghal_bol";
const UI_STORE_FILES: [&str; 2] = ["contacts_v1.json", "chat_transcript_v1.json"];

/// `{namespace_data_dir}/ghalbol/` — contacts, transcript (user-owned; no transport cache).
pub fn ui_data_dir(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    let ns_dir = namespace_data_dir(cfg)?;
    Ok(adopt_legacy_ui_data_dir(&ns_dir, ns_dir.join(UI_DATA_DIR_NAME)))
}

fn has_ui_stores(dir: &Path) -> bool {
    UI_STORE_FILES.iter().any(|f| dir.join(f).exists())
}

/// Move a pre-rename `ghal_bol/` folder into `ghalbol/` the first time stores are resolved.
///
/// A post-rename build may already have created an empty `ghalbol/`; legacy entries are moved
/// into it unless it already holds contacts or a transcript. On Unix a `ghal_bol → ghalbol`
/// symlink is left behind so absolute voice/attachment paths saved in transcript rows still
/// open. Returns the folder to use — the legacy one only when the move failed.
fn adopt_legacy_ui_data_dir(ns_dir: &Path, current: PathBuf) -> PathBuf {
    static MIGRATE: Mutex<()> = Mutex::new(());
    let legacy = ns_dir.join(LEGACY_UI_DATA_DIR_NAME);
    // `symlink_metadata` does not follow links: a migrated `ghal_bol` symlink is not a dir.
    let legacy_is_dir = fs::symlink_metadata(&legacy).is_ok_and(|m| m.is_dir());
    if !legacy_is_dir || has_ui_stores(&current) {
        return current;
    }
    let _guard = MIGRATE.lock().unwrap_or_else(|e| e.into_inner());
    if !fs::symlink_metadata(&legacy).is_ok_and(|m| m.is_dir()) {
        return current;
    }
    let moved = if current.exists() {
        fs::read_dir(&legacy)
            .map(|entries| {
                for entry in entries.flatten() {
                    let target = current.join(entry.file_name());
                    if !target.exists() {
                        let _ = fs::rename(entry.path(), target);
                    }
                }
            })
            .is_ok()
            && fs::remove_dir(&legacy).is_ok()
    } else {
        fs::rename(&legacy, &current).is_ok()
    };
    if !moved {
        return if has_ui_stores(&legacy) { legacy } else { current };
    }
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(UI_DATA_DIR_NAME, &legacy);
    current
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
/// Order: `GHALBOL_APP_NAMESPACE` (when that keystore exists), then release, then debug.
pub fn detect_keystore_app_namespace() -> Option<String> {
    if let Ok(raw) = std::env::var("GHALBOL_APP_NAMESPACE") {
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
                .join("ghalbol")
                .join("contacts_v1.json")
        );
    }

    #[test]
    fn pre_rename_store_folder_is_adopted() {
        let td = TempDir::new().unwrap();
        let ns_dir = td.path().join("com.ghalbol.debug");
        let legacy = ns_dir.join("ghal_bol");
        fs::create_dir_all(legacy.join("voice")).unwrap();
        fs::write(legacy.join("contacts_v1.json"), "{}").unwrap();
        fs::write(legacy.join("voice").join("m1.opus"), "opus").unwrap();
        let cfg = StorageConfig::new("com.ghalbol.debug").with_override_data_dir(td.path());

        let contacts = contacts_v1_path(&cfg).unwrap();
        assert_eq!(contacts, ns_dir.join("ghalbol").join("contacts_v1.json"));
        assert_eq!(fs::read_to_string(&contacts).unwrap(), "{}");
        // Old absolute voice paths keep resolving through the symlink.
        #[cfg(unix)]
        assert_eq!(
            fs::read_to_string(legacy.join("voice").join("m1.opus")).unwrap(),
            "opus"
        );
        // Second resolve is a no-op.
        assert_eq!(contacts_v1_path(&cfg).unwrap(), contacts);
    }

    #[test]
    fn pre_rename_store_folder_merges_into_empty_new_folder() {
        let td = TempDir::new().unwrap();
        let legacy = td.path().join("ghal_bol");
        let current = td.path().join("ghalbol");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(current.join("attach")).unwrap();
        fs::write(legacy.join("chat_transcript_v1.json"), "[]").unwrap();
        let cfg = StorageConfig::new(ANDROID_LIBRARY_NAMESPACE).with_override_data_dir(td.path());

        let transcript = chat_transcript_v1_path(&cfg).unwrap();
        assert_eq!(transcript, current.join("chat_transcript_v1.json"));
        assert_eq!(fs::read_to_string(&transcript).unwrap(), "[]");
        assert!(current.join("attach").is_dir());
    }

    #[test]
    fn new_store_folder_with_data_is_left_alone() {
        let td = TempDir::new().unwrap();
        let legacy = td.path().join("ghal_bol");
        let current = td.path().join("ghalbol");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(legacy.join("contacts_v1.json"), "old").unwrap();
        fs::write(current.join("contacts_v1.json"), "new").unwrap();
        let cfg = StorageConfig::new(ANDROID_LIBRARY_NAMESPACE).with_override_data_dir(td.path());

        let contacts = contacts_v1_path(&cfg).unwrap();
        assert_eq!(fs::read_to_string(&contacts).unwrap(), "new");
        assert_eq!(fs::read_to_string(legacy.join("contacts_v1.json")).unwrap(), "old");
    }

    #[test]
    fn release_namespace_ui_data_under_package_root() {
        let td = TempDir::new().unwrap();
        let cfg = StorageConfig::new(ANDROID_LIBRARY_NAMESPACE).with_override_data_dir(td.path());
        let contacts = contacts_v1_path(&cfg).unwrap();
        assert_eq!(
            contacts,
            td.path().join("ghalbol").join("contacts_v1.json")
        );
    }
}
