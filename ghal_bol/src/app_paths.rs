//! Shared on-disk paths for app data (`ghal_bol/` under the app data root).
//! Android package id remains **`com.ghalbol`** — this folder is not the package name.

use std::path::PathBuf;

use crate::storage::{base_data_dir, StorageConfig, KeystoreStorageError};

/// `{data_dir}/ghal_bol/` — app-private storage (contacts, transcript, prefs).
pub fn ui_data_dir(cfg: &StorageConfig) -> Result<PathBuf, KeystoreStorageError> {
    let mut p = base_data_dir(cfg)?;
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
