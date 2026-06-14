//! Shared on-disk paths for app data under the same namespace root as the keystore.
//!
//! Linux: `~/.local/share/com.ghalbol.debug/ghal_bol/` (debug) or `…/com.ghalbol/ghal_bol/`.
//! Android: `{app_flutter}/{namespace}/ghal_bol/` (debug) or `{app_flutter}/ghal_bol/` (release).

use std::path::PathBuf;

use crate::storage::{namespace_data_dir, StorageConfig, KeystoreStorageError};

/// `{namespace_data_dir}/ghal_bol/` — contacts, transcript, relay cache.
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
    crate::c_ffi::resolved_storage_config(app_namespace)
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
