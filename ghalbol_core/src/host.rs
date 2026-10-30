//! In-process API for the Makepad UI (`ghalbol_app`).
//!
//! The desktop shell links this crate and calls these functions directly.

use serde_json::{Value, json};

use crate::contacts_v1::{self, SavedContact};
use crate::coord_runtime::parse_coord_urls;
use crate::p2p_runtime::{self, p2p_poll_event, p2p_start, p2p_stop, p2p_sync_ui_session};
use crate::preferences_v1;
use crate::session_runtime::{self, lock_identity, unlock_identity};
use crate::app_paths::storage_config_for_namespace;
use crate::storage::keystore_v1_file_exists;

fn cfg(app_namespace: &str) -> crate::storage::StorageConfig {
    storage_config_for_namespace(app_namespace)
}

/// Install namespace (data directory and desktop id): the app's baked `GHALBOL_APP_ID`
/// (`com.ghalbol` / `com.ghalbol.debug`), else the build profile.
pub fn default_app_namespace() -> String {
    std::env::var("GHALBOL_APP_ID")
        .ok()
        .map(|id| id.trim().to_string())
        .filter(|id| crate::storage::sanitize_namespace(id).is_ok())
        .unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                "com.ghalbol.debug"
            } else {
                crate::storage::ANDROID_LIBRARY_NAMESPACE
            }
            .to_string()
        })
}

/// Hand this process's Java VM and Activity to native audio, camera, and HTTPS.
/// The Makepad app calls this once on Android. A second call is ignored.
#[cfg(target_os = "android")]
pub fn install_android_context(vm: *mut std::ffi::c_void, activity: *mut std::ffi::c_void) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if vm.is_null() || activity.is_null() {
        return;
    }
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        ndk_context::initialize_android_context(vm, activity);
    }
    crate::call_media::set_android_audio_ready();
    crate::rustls_init::ensure_rustls_crypto_provider();
    let vm = unsafe { jni::JavaVM::from_raw(vm.cast()) };
    let _ = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let activity_ref = unsafe { jni::objects::JObject::from_raw(env, activity as jni::sys::jobject) };
        let context = env.new_local_ref(&activity_ref)?;
        // `activity` is Makepad's global ref. Dropping `activity_ref` would delete it.
        std::mem::forget(activity_ref);
        rustls_platform_verifier::android::init_with_env(env, context)?;
        Ok(())
    });
    if let Ok(dir) = crate::incoming_call_android::android_files_dir() {
        crate::app_paths::configure_android_data_directory(&dir);
    }
}

pub fn keystore_exists(app_namespace: &str) -> bool {
    let cfg = cfg(app_namespace);
    keystore_v1_file_exists(&cfg).unwrap_or(false)
}

#[derive(Clone, Debug)]
pub struct UnlockedSession {
    pub app_namespace: String,
    pub public_key_hex: String,
    pub identity_wire: String,
}

pub fn unlock(app_namespace: &str, password: &str) -> Result<UnlockedSession, String> {
    let v = unlock_identity(app_namespace, password);
    if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        let err = v
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("unlock failed");
        return Err(err.to_string());
    }
    let public_key_hex = v
        .get("public_key_hex")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let identity_wire = v
        .get("libp2p_peer_id")
        .and_then(|x| x.as_str())
        .unwrap_or(&public_key_hex)
        .to_string();
    Ok(UnlockedSession {
        app_namespace: app_namespace.to_string(),
        public_key_hex,
        identity_wire,
    })
}

pub fn lock() {
    p2p_stop();
    #[cfg(target_os = "android")]
    crate::android_network::hold_multicast_lock(false);
    lock_identity();
}

pub fn session_unlocked() -> bool {
    session_runtime::session_unlocked()
}

#[derive(Clone, Debug)]
pub struct RosterEntry {
    pub public_key_hex: String,
    pub title: String,
    pub preview: String,
    pub unread: u32,
    pub is_known: bool,
    pub is_blocked: bool,
    pub availability: String,
}

pub fn list_roster(app_namespace: &str) -> Result<Vec<RosterEntry>, String> {
    let list = contacts_v1::list_contacts(app_namespace).map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .filter(|c| c.has_public_key())
        .map(roster_entry)
        .collect())
}

pub fn unlock_with_options(
    app_namespace: &str,
    password: &str,
    create_algorithm: Option<&str>,
    import_secret_hex: Option<&str>,
) -> Result<UnlockedSession, String> {
    if password.is_empty() {
        return Err("App password is required.".into());
    }
    if keystore_exists(app_namespace) {
        return unlock(app_namespace, password);
    }
    let algorithm = match create_algorithm {
        Some(id) => crate::IdentityAlgorithm::from_wire_id(id)?,
        None => crate::IdentityAlgorithm::Secp256k1,
    };
    let ident = if let Some(secret) = import_secret_hex.map(str::trim).filter(|s| !s.is_empty()) {
        crate::import_identity_from_secret_hex_v1_with_algorithm(
            &cfg(app_namespace),
            password,
            secret,
            algorithm,
        )
        .map_err(|e| e.to_string())?
    } else {
        crate::create_or_unlock_identity_v1_with_algorithm(
            &cfg(app_namespace),
            password,
            Some(algorithm),
        )
        .map_err(|e| e.to_string())?
    };
    install_unlocked(app_namespace, ident)
}

pub fn import_keystore(
    app_namespace: &str,
    password: &str,
    keystore_json: &str,
) -> Result<UnlockedSession, String> {
    if password.is_empty() {
        return Err("App password is required.".into());
    }
    let ident = crate::import_keystore_from_json_v1(
        &cfg(app_namespace),
        password,
        keystore_json,
    )
    .map_err(|e| e.to_string())?;
    install_unlocked(app_namespace, ident)
}

pub fn delete_identity(app_namespace: &str, password: &str) -> Result<(), String> {
    unlock(app_namespace, password)?;
    crate::delete_stored_identity_v1(&cfg(app_namespace)).map_err(|e| e.to_string())?;
    lock();
    Ok(())
}

pub fn add_contact(
    app_namespace: &str,
    public_key_hex: &str,
    alias: Option<&str>,
) -> Result<(), String> {
    let contact = SavedContact {
        public_key_hex: public_key_hex.trim().to_string(),
        display_alias: alias.map(str::to_string),
        availability_status: None,
        last_message_preview: None,
        last_message_at_ms: None,
        unread_count: 0,
        created_at_ms: None,
        updated_at_ms: None,
        is_known: true,
        is_blocked: false,
        chat_room_exit_at_ms: None,
    };
    let saved = contacts_v1::upsert_contact(app_namespace, contact).map_err(|e| e.to_string())?;
    let _ = p2p_runtime::p2p_register_dm_peer(&saved.public_key_hex);
    Ok(())
}

/// Rename (or clear) a contact's display alias. Empty alias clears the name.
pub fn rename_contact(
    app_namespace: &str,
    public_key_hex: &str,
    alias: &str,
) -> Result<(), String> {
    let pk = public_key_hex.trim();
    if !contacts_v1::is_valid_public_key_hex(pk) {
        return Err("public key is not valid".into());
    }
    let contact = SavedContact {
        public_key_hex: pk.to_string(),
        display_alias: Some(alias.to_string()),
        availability_status: None,
        last_message_preview: None,
        last_message_at_ms: None,
        unread_count: 0,
        created_at_ms: None,
        updated_at_ms: None,
        is_known: true,
        is_blocked: false,
        chat_room_exit_at_ms: None,
    };
    let _ = contacts_v1::upsert_contact(app_namespace, contact).map_err(|e| e.to_string())?;
    Ok(())
}

/// Display name embedded in invitations (`global_alias`). Empty clears.
pub fn local_display_name(app_namespace: &str) -> String {
    preferences_v1::local_display_name_get(&cfg(app_namespace))
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub fn set_local_display_name(app_namespace: &str, name: &str) -> Result<(), String> {
    preferences_v1::local_display_name_set(&cfg(app_namespace), name).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn mark_known(app_namespace: &str, public_key_hex: &str) -> Result<(), String> {
    contacts_v1::set_contact_trust(app_namespace, public_key_hex, Some(true), Some(false))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn contacts_version() -> u64 {
    contacts_v1::contacts_change_version()
}

pub fn remove_contact(app_namespace: &str, public_key_hex: &str) -> Result<(), String> {
    let pk = public_key_hex.trim();
    if !contacts_v1::is_valid_public_key_hex(pk) {
        return Err("public key is not valid".into());
    }
    let contact = contacts_v1::SavedContact {
        public_key_hex: pk.to_string(),
        display_alias: None,
        availability_status: None,
        last_message_preview: None,
        last_message_at_ms: None,
        unread_count: 0,
        created_at_ms: None,
        updated_at_ms: None,
        is_known: false,
        is_blocked: false,
        chat_room_exit_at_ms: None,
    };
    contacts_v1::remove_contact(app_namespace, &contact).map_err(|e| e.to_string())
}

pub fn set_blocked(app_namespace: &str, public_key_hex: &str, blocked: bool) -> Result<(), String> {
    contacts_v1::set_contact_trust(app_namespace, public_key_hex, None, Some(blocked))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn invite_links(public_key_hex: &str, alias: Option<&str>) -> Result<(String, String), String> {
    let map = crate::connect_invite_v1::build_connect_invite_wire_map(
        crate::p2p::DEFAULT_GOSSIP_TOPIC,
        public_key_hex,
        alias,
    )?;
    let https = crate::connect_invite_v1::connect_invite_https_uri_from_wire_map(&map)?;
    let app = crate::connect_invite_v1::connect_invite_app_uri_from_wire_map(&map)?;
    Ok((https, app))
}

/// Invite URL from the Android view intent, once per distinct link.
pub fn take_view_invite() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        let url = crate::incoming_call_android::view_intent_url()?;
        let url = url.trim().to_string();
        if url.is_empty() {
            return None;
        }
        use std::sync::Mutex;
        static SEEN: Mutex<String> = Mutex::new(String::new());
        let mut seen = SEEN.lock().ok()?;
        if *seen == url {
            return None;
        }
        *seen = url.clone();
        return Some(url);
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

pub fn accept_invite(app_namespace: &str, raw: &str) -> Result<String, String> {
    let value = crate::connect_invite_v1::parse_connect_invite_uri(raw.trim())?;
    let wire = crate::connect_invite_v1::identity_wire_from_invite(&value)?;
    let alias = crate::connect_invite_v1::global_alias_from_invite(&value);
    add_contact(app_namespace, &wire, alias.as_deref())?;
    Ok(wire)
}

pub fn availability_status() -> String {
    p2p_runtime::p2p_get_availability_status()
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn set_availability_status(status: &str) -> Result<(), String> {
    let v = p2p_runtime::p2p_set_availability_status(status);
    if v.get("ok").and_then(|x| x.as_bool()) == Some(false) {
        return Err(v
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("could not save status")
            .to_string());
    }
    Ok(())
}

fn json_error(v: &Value, fallback: &str) -> Result<(), String> {
    if v.get("ok").and_then(|x| x.as_bool()) == Some(false) {
        return Err(v
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or(fallback)
            .to_string());
    }
    Ok(())
}

pub fn clear_unread(app_namespace: &str, public_key_hex: &str) {
    let _ = contacts_v1::clear_unread(app_namespace, public_key_hex);
}

pub fn fetch_attachment(peer_public_key_hex: &str, offer_id: &str) -> Result<String, String> {
    let v = p2p_runtime::p2p_attachment_fetch(&json!({
        "peer_public_key_hex": peer_public_key_hex,
        "offer_id": offer_id,
    }));
    json_error(&v, "could not download attachment")?;
    if v.get("downloading").and_then(|x| x.as_bool()) == Some(true) {
        return Ok("Downloading".into());
    }
    Ok(v.get("local_path")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string())
}

pub fn send_attachment(peer_public_key_hex: &str, file_path: &str) -> Result<(), String> {
    note_outbound_trust(peer_public_key_hex);
    let cfg = json!({ "file_path": file_path.trim() });
    json_error(
        &p2p_runtime::p2p_send_attachment(peer_public_key_hex, &cfg),
        "could not send attachment",
    )
}

pub fn start_voice_call(peer_public_key_hex: &str) -> Result<String, String> {
    let call_id = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let cfg = json!({
        "recipient_public_key_hex": peer_public_key_hex,
        "call_id": call_id,
        "signal": "invite",
        "payload": { "media": "audio", "voice_engine": "native_v2" },
    });
    json_error(&p2p_runtime::p2p_call_signal(&cfg), "could not start call")?;
    if let Ok(mut g) = active_call().lock() {
        *g = Some((call_id.clone(), peer_public_key_hex.to_string()));
    }
    let _ = start_call_media();
    Ok(call_id)
}

pub fn accept_incoming_call() -> Result<(), String> {
    let status = p2p_runtime::p2p_call_status(&json!({}));
    if !call_is_incoming_ringing() {
        return Err("No incoming call".into());
    }
    let call_id = status
        .get("call_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let peer = status
        .get("peer_public_key_hex")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if call_id.is_empty() || peer.is_empty() {
        return Err("No incoming call".into());
    }
    let cfg = json!({
        "recipient_public_key_hex": peer,
        "call_id": call_id,
        "signal": "accept",
        "payload": { "media": "audio", "voice_engine": "native_v2" },
    });
    json_error(&p2p_runtime::p2p_call_signal(&cfg), "could not accept call")?;
    if let Ok(mut g) = active_call().lock() {
        *g = Some((call_id, peer));
    }
    let _ = start_call_media();
    Ok(())
}

pub fn start_video_call(peer_public_key_hex: &str) -> Result<String, String> {
    let call_id = start_voice_call(peer_public_key_hex)?;
    let cfg = json!({
        "action": "start",
        "call_id": call_id,
        "recipient_public_key_hex": peer_public_key_hex,
        "camera_enabled": true,
    });
    json_error(&p2p_runtime::p2p_call_video(&cfg), "could not start video")?;
    Ok(call_id)
}

pub fn set_speaker(on: bool) -> Result<(), String> {
    crate::call_media::set_desktop_speaker_on(on);
    #[cfg(target_os = "android")]
    {
        let (call_id, _) = current_call()?;
        let cfg = json!({
            "action": "set_speaker",
            "call_id": call_id,
            "speaker_on": on,
        });
        json_error(&p2p_runtime::p2p_call_media(&cfg), "could not change speaker")?;
    }
    Ok(())
}

pub fn set_mic_muted(muted: bool) -> Result<(), String> {
    let (call_id, _) = current_call()?;
    let cfg = json!({
        "action": "set_mic_muted",
        "call_id": call_id,
        "muted": muted,
    });
    json_error(&p2p_runtime::p2p_call_media(&cfg), "could not change mute")
}

fn current_call() -> Result<(String, String), String> {
    active_call()
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .ok_or_else(|| "No active call".to_string())
}

fn start_call_media() -> Result<(), String> {
    let (call_id, peer) = current_call()?;
    let cfg = json!({
        "action": "start",
        "call_id": call_id,
        "recipient_public_key_hex": peer,
    });
    json_error(&p2p_runtime::p2p_call_media(&cfg), "could not start call audio")
}

pub fn end_call() -> Result<(), String> {
    let (call_id, peer) = active_call()
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .or_else(|| {
            let status = p2p_runtime::p2p_call_status(&json!({}));
            let call_id = status.get("call_id").and_then(|v| v.as_str())?.to_string();
            let peer = status
                .get("peer_public_key_hex")
                .and_then(|v| v.as_str())?
                .to_string();
            Some((call_id, peer))
        })
        .ok_or_else(|| "No active call".to_string())?;
    let cfg = json!({
        "recipient_public_key_hex": peer,
        "call_id": call_id,
        "signal": "hangup",
        "payload": {},
    });
    let _ = p2p_runtime::p2p_call_signal(&cfg);
    let _ = p2p_runtime::p2p_call_media(&json!({ "action": "stop", "call_id": call_id }));
    let _ = p2p_runtime::p2p_call_video(&json!({ "action": "stop", "call_id": call_id }));
    let _ = p2p_runtime::p2p_force_end_active_call("ui_hangup");
    if let Ok(mut g) = active_call().lock() {
        *g = None;
    }
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android"))]
    crate::call_pictures::clear_call_pictures();
    Ok(())
}

/// Local camera PNG and remote video PNG for the open call.
pub fn call_picture_pngs() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android"))]
    {
        if let Ok((call_id, _)) = current_call() {
            return crate::call_pictures::call_picture_pngs(&call_id);
        }
    }
    (None, None)
}

pub fn start_qr_scan() -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        return crate::desktop_media::start_qr_scan();
    }
    #[cfg(target_os = "android")]
    {
        return crate::android_qr::start_qr_scan();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android")))]
    Err("QR camera scan is not available on this device".into())
}

pub fn take_qr_scan() -> Option<Result<String, String>> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        return crate::desktop_media::take_qr_scan();
    }
    #[cfg(target_os = "android")]
    {
        return crate::android_qr::take_qr_scan();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android")))]
    None
}

/// Latest QR-scan camera preview as raw RGBA, if a new frame is ready.
pub fn take_qr_preview_rgba() -> Option<(u32, u32, Vec<u8>)> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        return crate::desktop_media::take_qr_preview_rgba();
    }
    #[cfg(target_os = "android")]
    {
        return crate::android_qr::take_qr_preview_rgba();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android")))]
    None
}

/// Stop an in-flight QR camera scan.
pub fn stop_qr_scan() {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        crate::desktop_media::stop_qr_scan();
    }
    #[cfg(target_os = "android")]
    {
        crate::android_qr::stop_qr_scan();
    }
}

/// Android: write the keystore backup to app files and the public Downloads folder.
#[cfg(target_os = "android")]
pub fn android_save_backup_json(json: &str) -> Result<String, String> {
    crate::incoming_call_android::save_keystore_backup(json)
}

/// Faster UI poll while the Scan sheet needs live camera frames.
pub fn set_qr_scan_ui_active(active: bool) {
    QR_SCAN_UI_ACTIVE.store(active, std::sync::atomic::Ordering::Relaxed);
}

pub fn qr_scan_ui_active() -> bool {
    QR_SCAN_UI_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

static QR_SCAN_UI_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Decode an invite QR from a picture: a path on desktop, a picker `content://` URI on Android.
pub fn decode_qr_file(path: &str) -> Result<String, String> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        return crate::desktop_media::decode_qr_file(path);
    }
    #[cfg(target_os = "android")]
    {
        let (_, bytes) = read_picked_file(path, MAX_PICKED_PICTURE_BYTES)?;
        return crate::qr_luma::decode_image_bytes(&bytes);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android")))]
    {
        let _ = path;
        Err("QR picture decode is not available on this device".into())
    }
}

/// Largest picture accepted for QR decode from the picker.
#[cfg(target_os = "android")]
const MAX_PICKED_PICTURE_BYTES: usize = 32 * 1024 * 1024;

/// Largest keystore backup accepted from the picker (real ones are under 1 KB).
pub const MAX_PICKED_BACKUP_BYTES: usize = 1024 * 1024;

/// A file chosen in the system picker, as (display name, bytes). Desktop pickers return a
/// path; Android returns a `content://` document URI, read through the content resolver.
pub fn read_picked_file(picked: &str, max_bytes: usize) -> Result<(String, Vec<u8>), String> {
    let picked = picked.trim();
    #[cfg(target_os = "android")]
    if picked.starts_with("content://") {
        return crate::incoming_call_android::read_content_uri(picked, max_bytes);
    }
    let path = std::path::Path::new(picked);
    let len = std::fs::metadata(path)
        .map_err(|e| format!("could not open {picked}: {e}"))?
        .len();
    if len > max_bytes as u64 {
        return Err(format!(
            "The chosen file is larger than {} MB",
            max_bytes / (1024 * 1024)
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("could not read {picked}: {e}"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok((name, bytes))
}

/// Android: send a file picked as a `content://` URI. The bytes are copied into app
/// storage first, because the transcript keeps the sender's local path for **Open**.
#[cfg(target_os = "android")]
pub fn send_picked_attachment(
    app_namespace: &str,
    peer_public_key_hex: &str,
    picked: &str,
) -> Result<(), String> {
    let (name, bytes) = read_picked_file(picked, crate::attach_v1::MAX_FILE_SIZE_BYTES as usize)?;
    let path = crate::attach_v1::stage_picked_file(app_namespace, &name, &bytes)?;
    send_attachment(peer_public_key_hex, &path.display().to_string())
}

pub fn call_banner() -> String {
    let status = p2p_runtime::p2p_call_status(&json!({}));
    let peer = status
        .get("peer_public_key_hex")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let peer_label = peer_display_name(peer);
    // Incoming ring only — never treat the caller's outbound dial as Accept UI.
    if status.get("incoming").and_then(|v| v.as_bool()) == Some(true)
        || (status.get("ringing").and_then(|v| v.as_bool()) == Some(true)
            && status.get("phase").and_then(|v| v.as_str()) == Some("incoming_ringing"))
    {
        return format!("Incoming call · {peer_label}");
    }
    if status.get("phase").and_then(|v| v.as_str()) == Some("outgoing_ringing") {
        return format!("Calling {peer_label}…");
    }
    if status.get("active").and_then(|v| v.as_bool()) == Some(true) {
        let voice = status.get("voice_active").and_then(|v| v.as_bool()) == Some(true);
        let video = call_shows_video_from(&status);
        let e2e = " · End-to-end encrypted";
        return if video {
            format!("Video call · {peer_label}{e2e}")
        } else if voice {
            format!("Voice call · {peer_label}{e2e}")
        } else {
            format!("Connecting · {peer_label}")
        };
    }
    String::new()
}

/// Peer display name for the open call (alias, else short key).
pub fn call_peer_label() -> String {
    let status = p2p_runtime::p2p_call_status(&json!({}));
    let peer = status
        .get("peer_public_key_hex")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if peer.is_empty() {
        String::new()
    } else {
        peer_display_name(peer)
    }
}

/// True when local/remote video should be shown (not on plain voice dials).
pub fn call_shows_video() -> bool {
    let status = p2p_runtime::p2p_call_status(&json!({}));
    call_shows_video_from(&status)
}

fn call_shows_video_from(status: &Value) -> bool {
    status.get("video_active").and_then(|v| v.as_bool()) == Some(true)
        || status.get("camera_on").and_then(|v| v.as_bool()) == Some(true)
        || status.get("remote_video_on").and_then(|v| v.as_bool()) == Some(true)
}

/// Contact alias, else a short key — call banner and message notifications.
pub(crate) fn peer_display_name(peer_pk_hex: &str) -> String {
    let pk = peer_pk_hex.trim();
    if pk.is_empty() {
        return String::new();
    }
    if let Some(ns) = crate::dm_event_handler::active_app_namespace() {
        if let Ok(rows) = contacts_v1::list_contacts(&ns) {
            if let Some(c) = rows
                .into_iter()
                .find(|c| c.public_key_hex.eq_ignore_ascii_case(pk))
            {
                if let Some(alias) = c
                    .display_alias
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    return alias.to_string();
                }
            }
        }
    }
    short_key(pk)
}

/// True only while this device is the **callee** with an unanswered invite.
/// The caller's dial must never show Accept.
pub fn call_is_incoming_ringing() -> bool {
    let status = p2p_runtime::p2p_call_status(&json!({}));
    status.get("incoming").and_then(|v| v.as_bool()) == Some(true)
        || (status.get("ringing").and_then(|v| v.as_bool()) == Some(true)
            && status.get("phase").and_then(|v| v.as_str()) == Some("incoming_ringing"))
}

/// What mouse / Android / Escape back should do while call chrome is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallBackNav {
    /// No call to handle.
    None,
    /// Dialing or connected — consume back; do not hang up.
    Block,
    /// Incoming ring — decline / hang up.
    End,
}

pub fn call_back_nav() -> CallBackNav {
    if call_is_incoming_ringing() {
        return CallBackNav::End;
    }
    let status = p2p_runtime::p2p_call_status(&json!({}));
    if status.get("phase").and_then(|v| v.as_str()) == Some("outgoing_ringing")
        || status.get("active").and_then(|v| v.as_bool()) == Some(true)
    {
        return CallBackNav::Block;
    }
    CallBackNav::None
}

pub fn delivery_summary() -> String {
    let status = crate::delivery_runtime::delivery_connection_status();
    let list = crate::delivery_runtime::delivery_mailbox_list(true);
    let connected = status.get("connected").and_then(|v| v.as_bool()).unwrap_or(false);
    let url = status
        .get("delivery_url")
        .and_then(|v| v.as_str())
        .unwrap_or("(not set)");
    let err = status
        .get("last_error")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let quota = status.get("quota").cloned().unwrap_or(Value::Null);
    let used = quota
        .get("used_bytes")
        .or_else(|| quota.get("used"))
        .and_then(|v| v.as_u64());
    let alloc = quota
        .get("allocated_bytes")
        .or_else(|| quota.get("allocated"))
        .and_then(|v| v.as_u64());
    let quota_line = match (used, alloc) {
        (Some(u), Some(a)) if a > 0 => {
            format!(
                "Quota: {} / {} ({:.0}%)\n",
                format_bytes(u),
                format_bytes(a),
                (u as f64 / a as f64) * 100.0
            )
        }
        _ => String::new(),
    };
    let rows = list
        .pointer("/snapshot/rows")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let list_err = list.get("error").and_then(|v| v.as_str()).unwrap_or("");
    let mut body = format!(
        "Delivery mailbox\nConnected: {connected}\nURL: {url}\n{quota_line}Stored messages: {}\n",
        rows.len()
    );
    if !err.is_empty() {
        body.push_str(err);
        body.push('\n');
    }
    if !list_err.is_empty() {
        body.push_str(list_err);
        body.push('\n');
    }
    for row in rows.iter().take(40) {
        let mid = row
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let state = row.get("state").and_then(|v| v.as_str()).unwrap_or("?");
        let size = row
            .get("size_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let exp = row
            .get("expires_at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        body.push_str(&format!(
            "· {mid}  {state}  {}  expires_ms={exp}\n",
            format_bytes(size)
        ));
    }
    if rows.len() > 40 {
        body.push_str(&format!("…and {} more\n", rows.len() - 40));
    }
    body.push_str(
        "\nQueued rows can be extended (Refresh then Extend). Expired rows need Tap to resend in chat.",
    );
    body
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

/// Extend TTL for a still-queued mailbox message (default +7 days).
pub fn extend_mailbox_ttl(message_id: &str) -> Result<(), String> {
    let v = crate::delivery_runtime::delivery_extend_ttl(message_id, 7 * 24 * 3600);
    if v.get("ok").and_then(|x| x.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(v
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("extend failed")
            .to_string())
    }
}

/// Resend a failed/expired outbound text as a **new** mailbox upload (docs: no extend after expiry).
pub fn resend_text(peer_public_key_hex: &str, old_message_id: &str) -> Result<(), String> {
    resend_failed(peer_public_key_hex, old_message_id)
}

/// Resend a failed/expired outbound text, voice note, or attachment as a **new** upload.
pub fn resend_failed(peer_public_key_hex: &str, old_message_id: &str) -> Result<(), String> {
    let peer = peer_public_key_hex.trim();
    let mid = old_message_id.trim();
    if peer.is_empty() || mid.is_empty() {
        return Err("missing peer or message id".into());
    }
    let ns = crate::dm_event_handler::active_app_namespace()
        .ok_or_else(|| "not unlocked".to_string())?;
    let lines = crate::dm_transcript_store::load_merged(&ns, &[peer.to_string()], None)
        .map_err(|e| e.to_string())?;
    let line = lines
        .into_iter()
        .find(|l| {
            l.outgoing
                && l.message_id.as_deref().unwrap_or("") == mid
                && (l.delivery == "failed" || l.delivery == "pending")
        })
        .ok_or_else(|| "no failed message to resend".to_string())?;
    let kind = line.msg_kind.trim();
    if kind.is_empty() || kind == "text" {
        let text = line.text.trim();
        if text.is_empty() {
            return Err("message body empty".into());
        }
        return send_text(peer, text);
    }
    if kind == "voice" {
        let path = line
            .audio_path
            .as_deref()
            .or(line.local_path.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "voice file missing for resend".to_string())?;
        let opus = std::fs::read(path).map_err(|e| format!("read voice: {e}"))?;
        let duration_ms = line.duration_ms.unwrap_or(1).max(1);
        return send_voice_note(peer, duration_ms, opus);
    }
    if kind == "attachment_offer" {
        let path = line
            .local_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "attachment file missing for resend".to_string())?;
        return send_attachment(peer, path);
    }
    Err(format!("cannot resend msg_kind={kind}"))
}

/// Identity algorithm of the unlocked session (`secp256k1`, `ed25519`, …).
pub fn unlocked_algorithm() -> Option<String> {
    crate::session_runtime::unlocked_identity_clone()
        .ok()
        .map(|id| id.algorithm().wire_id().to_string())
}

/// One-shot quota warning for the chat chrome (clears after read).
pub fn take_delivery_quota_warning() -> Option<String> {
    crate::delivery_runtime::take_quota_warning()
}

/// Flip the in-call camera (front ↔ back on Android; no-op elsewhere when unsupported).
pub fn flip_call_camera() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        return crate::call_video::android_video::flip_camera();
    }
    #[cfg(not(target_os = "android"))]
    {
        Err("Camera flip is only available on Android during a video call".into())
    }
}

/// Short line for chat chrome when text is queued but the mailbox is down.
/// `None` when delivery is unused or currently connected.
pub fn delivery_offline_hint() -> Option<String> {
    let status = crate::delivery_runtime::delivery_connection_status();
    let url = status
        .get("delivery_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if url.is_empty() {
        return None;
    }
    if status.get("connected").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    let err = status
        .get("last_error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if err.is_empty() {
        Some(
            "Delivery offline — message queued; peer will get it when the mailbox is reachable."
                .into(),
        )
    } else {
        Some(format!("Delivery offline — message queued. {err}"))
    }
}

pub fn app_log_text() -> String {
    let lines = crate::p2p::native_log::recent_lines();
    if lines.is_empty() {
        "No log lines yet.".into()
    } else {
        lines.join("\n")
    }
}

pub fn reveal_secret(app_namespace: &str, password: &str) -> Result<String, String> {
    let (hex, algo) = crate::reveal_secret_key_hex_v1(&cfg(app_namespace), password)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Anyone with this key and your password can control this identity.\n\nAlgorithm: {}\n\n{hex}",
        algo.wire_id()
    ))
}

pub fn export_keystore(app_namespace: &str) -> Result<String, String> {
    crate::export_keystore_json_v1(&cfg(app_namespace)).map_err(|e| e.to_string())
}

pub fn change_password(
    app_namespace: &str,
    old_password: &str,
    new_password: &str,
) -> Result<(), String> {
    let ident = crate::change_password_v1(
        &cfg(app_namespace),
        old_password,
        new_password,
    )
    .map_err(|e| e.to_string())?;
    install_unlocked(app_namespace, ident)?;
    Ok(())
}

fn active_call() -> &'static std::sync::Mutex<Option<(String, String)>> {
    static CALL: std::sync::OnceLock<std::sync::Mutex<Option<(String, String)>>> =
        std::sync::OnceLock::new();
    CALL.get_or_init(|| std::sync::Mutex::new(None))
}

fn roster_entry(c: SavedContact) -> RosterEntry {
    let title = c
        .display_alias
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| short_key(&c.public_key_hex));
    let preview = c
        .last_message_preview
        .unwrap_or_default()
        .trim()
        .to_string();
    RosterEntry {
        public_key_hex: c.public_key_hex,
        title,
        preview,
        unread: c.unread_count,
        is_known: c.is_known,
        is_blocked: c.is_blocked,
        availability: c.availability_status.unwrap_or_default(),
    }
}

fn install_unlocked(
    app_namespace: &str,
    ident: crate::DecryptedIdentity,
) -> Result<UnlockedSession, String> {
    let public_key_hex = ident.public_key_hex();
    let identity_wire = ident.identity_wire();
    crate::session_runtime::install_unlocked_identity(ident).map_err(|e| e.to_string())?;
    crate::set_p2p_handler_context(app_namespace);
    crate::delivery_runtime::delivery_start();
    Ok(UnlockedSession {
        app_namespace: app_namespace.to_string(),
        public_key_hex,
        identity_wire,
    })
}

fn short_key(pk: &str) -> String {
    let t = pk.trim();
    if t.len() <= 16 {
        t.to_string()
    } else {
        format!("{}…{}", &t[..8], &t[t.len() - 4..])
    }
}

/// Start native connect in this process and register saved contacts.
pub fn start_network(app_namespace: &str) -> Result<Value, String> {
    let roster = contacts_v1::list_contacts(app_namespace).map_err(|e| e.to_string())?;
    let dm_peers: Vec<Value> = roster
        .into_iter()
        .filter(|c| c.has_public_key())
        .map(|c| json!({ "public_key_hex": c.public_key_hex }))
        .collect();
    let (coord_urls, insecure) = coord_for_namespace(app_namespace);
    let mut cfg = json!({
        "app_namespace": app_namespace,
        "bootstrap_peers": [],
        "dm_peers": dm_peers,
        "coord_base_urls": coord_urls,
        "coord_insecure_tls": insecure,
    });
    if let Some(url) = delivery_url_from_env() {
        cfg["delivery_url"] = Value::String(url);
    }
    let result = p2p_start(&cfg);
    if result.get("ok").and_then(|x| x.as_bool()) == Some(false) {
        let err = result
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("p2p_start failed");
        return Err(err.to_string());
    }
    let _ = p2p_sync_ui_session(true, None);
    #[cfg(target_os = "linux")]
    crate::linux_desktop_launch::install_user_autostart();
    #[cfg(target_os = "android")]
    crate::android_network::hold_multicast_lock(true);
    Ok(result)
}

/// First launch on the desktop: install login start, and ask for the password if a keystore exists.
pub fn desktop_session_start(app_namespace: &str) {
    #[cfg(target_os = "linux")]
    {
        crate::linux_desktop_launch::install_user_autostart();
        if keystore_exists(app_namespace) && !session_unlocked() {
            crate::linux_desktop_launch::notify_unlock_needed();
        }
    }
    #[cfg(target_os = "android")]
    if keystore_exists(app_namespace) && !session_unlocked() {
        crate::incoming_call_android::notify_unlock_needed();
    }
    let _ = app_namespace;
}

/// Play the incoming-call tone once per ring, a ringback while dialing, and stop when the banner clears.
pub fn note_call_banner(line: &str) {
    #[cfg(target_os = "linux")]
    {
        use std::sync::atomic::{AtomicU8, Ordering};
        static PHASE: AtomicU8 = AtomicU8::new(0);
        let next = if line.starts_with("Incoming") {
            1
        } else if line.starts_with("Calling") {
            2
        } else {
            0
        };
        let prev = PHASE.swap(next, Ordering::Relaxed);
        if next != 0 && prev != next {
            if next == 1 {
                crate::linux_desktop_launch::play_incoming_ring();
            } else {
                crate::linux_desktop_launch::play_ringback();
            }
        }
    }
    let _ = line;
}

pub fn set_open_room(public_key_hex: Option<&str>) {
    if let Some(peer) = public_key_hex {
        crate::message_notify::clear_for_peer(peer);
    }
    let _ = p2p_sync_ui_session(true, public_key_hex);
}

/// Android background: the open room stays, and new read acks stop until the app is visible again.
/// Linux does not call this for a brief focus change.
pub fn set_app_visible(visible: bool) {
    let room = crate::p2p::live_foreground_peer_pk();
    let _ = p2p_sync_ui_session(visible, room.as_deref());
}

fn note_outbound_trust(peer_public_key_hex: &str) {
    if let Some(ns) = crate::dm_event_handler::active_app_namespace() {
        let _ = mark_known(&ns, peer_public_key_hex);
    }
}

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub outgoing: bool,
    pub text: String,
    pub delivery: String,
    pub kind: String,
    pub duration_ms: u32,
    pub audio_path: String,
    pub local_path: String,
    pub message_id: String,
    /// Attachment size when known (offer / downloaded), else 0.
    pub size_bytes: u64,
}

pub struct TranscriptPage {
    pub lines: Vec<ChatLine>,
    pub has_more: bool,
}

/// Monotonic in-memory revision of the peer's transcript (bumped on every store
/// mutation: inbound text, outbound queue, acks). The UI compares this each poll
/// and reloads the open room only when it changed — cheap, and independent of
/// whether the originating event was surfaced through `poll_event`.
pub fn transcript_revision(app_namespace: &str, peer_public_key_hex: &str) -> u64 {
    crate::dm_transcript_store::thread_revision_for_keys(
        app_namespace,
        &[peer_public_key_hex.to_string()],
    )
}

pub fn load_transcript(
    app_namespace: &str,
    peer_public_key_hex: &str,
    limit: usize,
) -> Result<TranscriptPage, String> {
    let cfg = json!({
        "app_namespace": app_namespace,
        "conversation_keys": [peer_public_key_hex],
        "limit": limit.max(1),
    });
    let v = p2p_runtime::p2p_transcript_load_merged(&cfg);
    if v.get("ok").and_then(|x| x.as_bool()) == Some(false) {
        let err = v
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("transcript load failed");
        return Err(err.to_string());
    }
    let lines = v
        .get("lines")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let has_more = v.get("has_more").and_then(|x| x.as_bool()).unwrap_or(false);
    Ok(TranscriptPage {
        has_more,
        lines: lines
            .into_iter()
            .map(|line| ChatLine {
                outgoing: line.get("outgoing").and_then(|x| x.as_bool()).unwrap_or(false),
                text: line
                    .get("text")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                delivery: line
                    .get("delivery")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: line
                    .get("msg_kind")
                    .and_then(|x| x.as_str())
                    .unwrap_or("text")
                    .to_string(),
                duration_ms: line
                    .get("duration_ms")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0) as u32,
                audio_path: line
                    .get("audio_path")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                local_path: line
                    .get("local_path")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                message_id: line
                    .get("message_id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                size_bytes: line
                    .get("size_bytes")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0),
            })
            .collect(),
    })
}

pub fn send_voice_note(peer_public_key_hex: &str, duration_ms: u32, opus: Vec<u8>) -> Result<(), String> {
    note_outbound_trust(peer_public_key_hex);
    let v = p2p_runtime::p2p_send_voice_dm(peer_public_key_hex, duration_ms, opus);
    if v.get("ok").and_then(|x| x.as_bool()) == Some(false) {
        let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("voice send failed");
        return Err(err.to_string());
    }
    Ok(())
}

pub fn voice_note_recording() -> bool {
    crate::voice_note::voice_note_recording()
}

/// Elapsed ms of the in-progress recording, or `None` when idle.
pub fn voice_note_elapsed_ms() -> Option<u32> {
    crate::voice_note::voice_note_elapsed_ms()
}

/// Drop the in-progress recording without sending.
pub fn voice_note_cancel() -> Result<(), String> {
    crate::voice_note::voice_note_cancel()
}

/// `Ok(None)` means recording started. `Ok(Some)` is the finished note.
pub fn voice_note_toggle() -> Result<Option<(u32, Vec<u8>)>, String> {
    crate::voice_note::voice_note_toggle()
}

/// Copy a downloaded attachment into the user's Downloads folder.
/// Returns a short status string suitable for the chat status line.
pub fn save_attachment(local_path: &str) -> Result<String, String> {
    let src = local_path.trim();
    if src.is_empty() || !std::path::Path::new(src).is_file() {
        return Err("Attachment file is missing".into());
    }
    let name = std::path::Path::new(src)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment")
        .to_string();
    #[cfg(target_os = "android")]
    {
        return crate::incoming_call_android::save_file_to_downloads(src, &name);
    }
    #[cfg(not(target_os = "android"))]
    {
        let dest_dir = {
            #[cfg(target_os = "windows")]
            {
                std::env::var_os("USERPROFILE")
                    .map(std::path::PathBuf::from)
                    .map(|p| p.join("Downloads"))
                    .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Downloads")))
                    .ok_or_else(|| "USERPROFILE/HOME unset".to_string())?
            }
            #[cfg(not(target_os = "windows"))]
            {
                let home = std::env::var_os("HOME").ok_or_else(|| "HOME unset".to_string())?;
                std::path::PathBuf::from(home).join("Downloads")
            }
        };
        std::fs::create_dir_all(&dest_dir).map_err(|e| format!("Downloads: {e}"))?;
        let mut dest = dest_dir.join(&name);
        if dest.exists() {
            let stem = dest
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("attachment");
            let ext = dest
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_default();
            for i in 1..1000 {
                let candidate = dest_dir.join(format!("{stem}-{i}{ext}"));
                if !candidate.exists() {
                    dest = candidate;
                    break;
                }
            }
        }
        std::fs::copy(src, &dest).map_err(|e| format!("Could not save attachment: {e}"))?;
        Ok(format!("Saved to {}", dest.display()))
    }
}

pub fn play_voice_file(path: &str) -> Result<String, String> {
    if crate::voice_note::play_opus_file(path)? {
        Ok("Playing".into())
    } else {
        Ok("Stopped".into())
    }
}

pub fn open_local_file(path: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() || !std::path::Path::new(path).is_file() {
        return Err("That file is not on this device yet.".into());
    }
    #[cfg(target_os = "android")]
    {
        return crate::incoming_call_android::open_file(path);
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", path])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(all(
        not(target_os = "android"),
        not(target_os = "windows"),
        not(target_os = "macos")
    ))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub fn ui_locale(app_namespace: &str) -> Option<String> {
    preferences_v1::ui_locale_get(&cfg(app_namespace)).ok().flatten()
}

pub fn set_ui_locale(app_namespace: &str, locale: &str) -> Result<(), String> {
    preferences_v1::ui_locale_set(&cfg(app_namespace), locale).map_err(|e| e.to_string())
}

pub fn send_text(peer_public_key_hex: &str, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("message is empty".into());
    }
    note_outbound_trust(peer_public_key_hex);
    let v = p2p_runtime::p2p_send_text_dm(peer_public_key_hex, text);
    if v.get("ok").and_then(|x| x.as_bool()) == Some(false) {
        let err = v
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("send failed");
        return Err(err.to_string());
    }
    Ok(())
}

pub fn poll_event() -> Option<Value> {
    if crate::desktop_wake::take_incoming_call_wake() {
        return Some(json!({ "kind": "incoming_call_wake" }));
    }
    if crate::desktop_wake::take_unlock_wake() {
        return Some(json!({ "kind": "unlock_wake" }));
    }
    p2p_poll_event()
}

fn delivery_url_from_env() -> Option<String> {
    for key in ["GHALBOL_DELIVERY_URL_LOCAL", "GHALBOL_DELIVERY_URL"] {
        if let Ok(raw) = std::env::var(key) {
            let t = raw.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn coord_for_namespace(app_namespace: &str) -> (Vec<String>, bool) {
    let insecure_env = std::env::var("GHALBOL_COORD_INSECURE_TLS")
        .ok()
        .map(|s| {
            let t = s.trim();
            t == "1" || t.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false);
    if let Ok(raw) = std::env::var("GHALBOL_COORD_URLS") {
        let urls = parse_coord_urls(raw.trim());
        if !urls.is_empty() {
            return (urls, insecure_env);
        }
    }
    let cfg = cfg(app_namespace);
    if let Ok((url, insecure_pref)) = preferences_v1::coord_settings_get(&cfg) {
        if let Some(url) = url {
            let urls = parse_coord_urls(&url);
            if !urls.is_empty() {
                return (urls, insecure_env || insecure_pref);
            }
        }
    }
    (Vec::new(), insecure_env)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Isolate {
        _lock: std::sync::MutexGuard<'static, ()>,
        _dir: tempfile::TempDir,
    }

    impl Isolate {
        fn new() -> Self {
            let lock = crate::app_paths::test_storage_isolation_lock()
                .lock()
                .unwrap();
            let dir = tempfile::TempDir::new().unwrap();
            crate::app_paths::configure_android_data_directory(dir.path().to_str().unwrap());
            Self { _lock: lock, _dir: dir }
        }
    }

    impl Drop for Isolate {
        fn drop(&mut self) {
            let _ = lock();
            crate::app_paths::clear_test_data_directory();
        }
    }

    #[test]
    fn identity_invite_password_and_locale() {
        let _iso = Isolate::new();
        let ns = "test.host.production";
        assert!(!keystore_exists(ns));
        let session = unlock_with_options(ns, "correct horse", Some("secp256k1"), None).unwrap();
        assert!(keystore_exists(ns));
        assert!(session_unlocked());
        assert!(session.public_key_hex.len() >= 64);

        let revealed = reveal_secret(ns, "correct horse").unwrap();
        assert!(revealed.contains("secp256k1"));

        let exported = export_keystore(ns).unwrap();
        assert!(exported.contains("keystore") || exported.starts_with('{'));

        let (https, app) = invite_links(&session.public_key_hex, Some("Ada")).unwrap();
        assert!(https.contains("ghalbol.com/connect/"));
        assert!(app.starts_with("ghalbol://connect/"));

        let other = "test.host.production.peer";
        let joined = accept_invite(other, &app).unwrap();
        assert_eq!(joined, session.public_key_hex);
        let roster = list_roster(other).unwrap();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].title, "Ada");
        assert!(roster[0].is_known);
        set_blocked(other, &joined, true).unwrap();
        assert!(list_roster(other).unwrap()[0].is_blocked);

        change_password(ns, "correct horse", "new horse").unwrap();
        assert!(reveal_secret(ns, "new horse").is_ok());
        assert!(reveal_secret(ns, "correct horse").is_err());

        set_ui_locale(ns, "hi").unwrap();
        assert_eq!(ui_locale(ns).as_deref(), Some("hi"));

        delete_identity(ns, "new horse").unwrap();
        assert!(!keystore_exists(ns));
        assert!(!session_unlocked());
    }

    const PEER: &str = "0305b1b0d27745e0a38a7254ea100abc38857b51ded2ac7ea88d3063fb8da21784";

    fn unknown_peer(ns: &str) {
        contacts_v1::upsert_contact(
            ns,
            SavedContact {
                public_key_hex: PEER.into(),
                display_alias: None,
                availability_status: None,
                last_message_preview: None,
                last_message_at_ms: None,
                unread_count: 0,
                created_at_ms: None,
                updated_at_ms: None,
                is_known: false,
                is_blocked: true,
                chat_room_exit_at_ms: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn outbound_send_marks_an_unknown_contact_known() {
        let _iso = Isolate::new();
        let ns = "test.host.trust";
        unlock_with_options(ns, "pw", Some("secp256k1"), None).unwrap();
        unknown_peer(ns);
        assert!(send_text(PEER, "hello").is_err());
        let row = list_roster(ns).unwrap().into_iter().next().unwrap();
        assert!(row.is_known);
        assert!(!row.is_blocked);
        lock();
        assert!(!session_unlocked());
    }

    #[test]
    fn availability_presets_persist_and_cap_at_64() {
        let _iso = Isolate::new();
        let ns = "test.host.status";
        unlock_with_options(ns, "pw", Some("secp256k1"), None).unwrap();
        set_availability_status("Busy").unwrap();
        assert_eq!(availability_status(), "Busy");
        set_availability_status("").unwrap();
        assert!(availability_status().is_empty());
        let long = "x".repeat(80);
        set_availability_status(&long).unwrap();
        assert_eq!(availability_status().len(), 64);
    }

    #[test]
    fn transcript_page_keeps_delivery_and_has_more() {
        let _iso = Isolate::new();
        let ns = "test.host.transcript";
        unlock_with_options(ns, "pw", Some("secp256k1"), None).unwrap();
        for i in 0..4 {
            crate::dm_transcript_store::append_if_new(
                ns,
                PEER,
                crate::dm_transcript_store::StoredChatLine {
                    local_id: format!("l{i}"),
                    text: format!("m{i}"),
                    outgoing: true,
                    from: None,
                    message_id: Some(format!("mid{i}")),
                    delivery: if i == 3 { "read" } else { "pending" }.into(),
                    created_at_ms: Some(1000 + i),
                    received_at_ms: None,
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
        let page = load_transcript(ns, PEER, 2).unwrap();
        assert!(page.has_more);
        assert_eq!(page.lines.len(), 2);
        assert_eq!(page.lines[1].text, "m3");
        assert_eq!(page.lines[1].delivery, "read");
        let full = load_transcript(ns, PEER, 10).unwrap();
        assert!(!full.has_more);
        assert_eq!(full.lines.len(), 4);
    }

    #[test]
    fn hidden_room_does_not_send_a_new_read_ack() {
        let _iso = Isolate::new();
        crate::p2p::sync_foreground_peer_now(Some(PEER.into()));
        let _ = p2p_sync_ui_session(true, Some(PEER));
        let hidden = p2p_sync_ui_session(false, Some(PEER));
        assert_eq!(hidden.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(hidden.get("read_receipts").and_then(|v| v.as_bool()), Some(false));
        assert!(!crate::p2p::may_send_read_ack_for_contact_pk(PEER));
        assert_eq!(
            crate::p2p::live_foreground_peer_pk().as_deref(),
            Some(PEER)
        );
        crate::p2p::sync_foreground_peer_now(None);
        crate::p2p::set_app_ack_read_enabled(false);
        crate::p2p::set_app_ui_visible(true);
    }

    #[test]
    fn speaker_switch_changes_desktop_playback() {
        set_speaker(false).unwrap();
        assert!(!crate::call_media::desktop_speaker_on());
        set_speaker(true).unwrap();
        assert!(crate::call_media::desktop_speaker_on());
    }
}
