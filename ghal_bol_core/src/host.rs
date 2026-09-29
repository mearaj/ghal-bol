//! In-process API for the Makepad UI (`ghal_bol_app`).
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

/// Debug installs share the Makepad debug namespace so an existing keystore unlocks.
pub fn default_app_namespace() -> &'static str {
    if cfg!(debug_assertions) {
        "com.ghalbol.debug"
    } else {
        crate::storage::ANDROID_LIBRARY_NAMESPACE
    }
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
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    crate::desktop_media::clear_call_pictures();
    Ok(())
}

/// Local camera PNG and remote video PNG for the open call.
pub fn call_picture_pngs() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        if let Ok((call_id, _)) = current_call() {
            return crate::desktop_media::call_picture_pngs(&call_id);
        }
    }
    (None, None)
}

pub fn start_qr_scan() -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        return crate::desktop_media::start_qr_scan();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err("QR camera scan is not available on this device".into())
}

pub fn take_qr_scan() -> Option<Result<String, String>> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        return crate::desktop_media::take_qr_scan();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    None
}

pub fn decode_qr_file(path: &str) -> Result<String, String> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        return crate::desktop_media::decode_qr_file(path);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err("QR picture decode is not available on this device".into())
    }
}

pub fn call_banner() -> String {
    let status = p2p_runtime::p2p_call_status(&json!({}));
    if status.get("ringing").and_then(|v| v.as_bool()) == Some(true) {
        let peer = status
            .get("peer_public_key_hex")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return format!("Incoming voice call from {}", short_key(peer));
    }
    if status.get("active").and_then(|v| v.as_bool()) == Some(true) {
        let voice = status.get("voice_active").and_then(|v| v.as_bool()) == Some(true);
        return if voice {
            "Voice call connected".into()
        } else {
            "Calling…".into()
        };
    }
    String::new()
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
    let status = p2p_runtime::p2p_call_status(&json!({}));
    if status.get("ringing").and_then(|v| v.as_bool()) == Some(true) {
        return CallBackNav::End;
    }
    if status.get("active").and_then(|v| v.as_bool()) == Some(true) {
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
    let rows = list
        .pointer("/snapshot/rows")
        .and_then(|v| v.as_array())
        .map(|rows| rows.len())
        .unwrap_or(0);
    let list_err = list.get("error").and_then(|v| v.as_str()).unwrap_or("");
    format!(
        "Delivery mailbox\nConnected: {connected}\nURL: {url}\nStored messages: {rows}\n{err}{list_err}"
    )
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
}

pub struct TranscriptPage {
    pub lines: Vec<ChatLine>,
    pub has_more: bool,
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

/// `Ok(None)` means recording started. `Ok(Some)` is the finished note.
pub fn voice_note_toggle() -> Result<Option<(u32, Vec<u8>)>, String> {
    crate::voice_note::voice_note_toggle()
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
    #[cfg(not(target_os = "android"))]
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
    for key in ["GHAL_BOL_DELIVERY_URL_LOCAL", "GHAL_BOL_DELIVERY_URL"] {
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
    let insecure_env = std::env::var("GHAL_BOL_COORD_INSECURE_TLS")
        .ok()
        .map(|s| {
            let t = s.trim();
            t == "1" || t.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false);
    if let Ok(raw) = std::env::var("GHAL_BOL_COORD_URLS") {
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
