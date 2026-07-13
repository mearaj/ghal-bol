//! Delivery-server text path (replaces P2P DM when `GHAL_BOL_DELIVERY_URL` is set).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;

use serde_json::{Value, json};

use crate::contacts_v1::{is_valid_public_key_hex, record_thread_message_preview};
use crate::delivery_client::{blocking_upload_text, connect_and_auth, ws_url_from_base, DeliverySession};
use crate::delivery_read_acks::{delivery_read_ack_upkeep, try_read_ack_after_inbound_async};
use crate::delivery_msg_v1::open_text_from_envelope;
use crate::dm_event_handler::{apply_p2p_event_json, persist_inbound_text_on_wire};
use crate::dm_event_handler::active_app_namespace;
use crate::dm_transcript_store::{StoredChatLine, append_if_new, load_merged, patch_outgoing_delivery};
use crate::p2p::native_log;
use crate::p2p::GossipChatEvent;
use crate::p2p_runtime::enqueue_delivery_gossip_event;
use crate::peer_id_util::peer_id_from_identity_wire;
use crate::session_runtime::unlocked_identity_clone;

#[derive(Clone, Debug, Default)]
struct DeliveryMirror {
    connected: bool,
    last_error: Option<String>,
    quota: Value,
    policy: Value,
    mailbox_rows: Vec<Value>,
}

fn mirror_mx() -> &'static Mutex<DeliveryMirror> {
    static M: OnceLock<Mutex<DeliveryMirror>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(DeliveryMirror::default()))
}

fn stop_flag() -> &'static AtomicBool {
    static S: OnceLock<AtomicBool> = OnceLock::new();
    S.get_or_init(|| AtomicBool::new(true))
}

fn worker_mx() -> &'static Mutex<Option<JoinHandle<()>>> {
    static W: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(None))
}

const DELIVERY_OUTBOX_RESEND_INTERVAL_MS: i64 = 1_000;
const DELIVERY_OUTBOX_MAX_PER_TICK: usize = 16;

fn outbox_last_attempt_mx() -> &'static Mutex<HashMap<String, i64>> {
    static M: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn outbox_in_flight_mx() -> &'static Mutex<HashSet<String>> {
    static M: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashSet::new()))
}

fn delivery_url_mx() -> &'static Mutex<Option<String>> {
    static U: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    U.get_or_init(|| Mutex::new(None))
}

/// Set delivery base URL from UI `p2p_start` config (`delivery_url` field).
pub fn set_delivery_url(url: Option<&str>) {
    let val = url.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if let Ok(mut g) = delivery_url_mx().lock() {
        *g = val;
    }
}

pub fn delivery_url() -> Option<String> {
    delivery_url_mx()
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .or_else(|| {
            std::env::var("GHAL_BOL_DELIVERY_URL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

pub fn delivery_mode_enabled() -> bool {
    delivery_url().is_some()
}

pub fn delivery_start() {
    if !delivery_mode_enabled() {
        return;
    }
    let Some(url) = delivery_url() else {
        return;
    };
    if let Ok(mut g) = worker_mx().lock() {
        if g.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        if let Some(h) = g.take() {
            stop_flag().store(true, Ordering::SeqCst);
            let _ = h.join();
        }
    }
    stop_flag().store(false, Ordering::SeqCst);
    let ws_url = ws_url_from_base(&url);
    let ws_url_log = ws_url.clone();
    let handle = std::thread::Builder::new()
        .name("ghal_bol_delivery".into())
        .spawn(move || delivery_worker_loop(ws_url))
        .expect("delivery worker thread");
    if let Ok(mut g) = worker_mx().lock() {
        *g = Some(handle);
    }
    native_log::info(
        "delivery",
        format!("delivery worker started url={ws_url_log}"),
    );
}

pub fn delivery_stop() {
    stop_flag().store(true, Ordering::SeqCst);
    if let Ok(mut g) = worker_mx().lock() {
        if let Some(h) = g.take() {
            let _ = h.join();
        }
    }
    if let Ok(mut m) = mirror_mx().lock() {
        m.connected = false;
    }
}

fn delivery_worker_loop(ws_url: String) {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            set_mirror_error(&format!("tokio runtime: {e}"));
            return;
        }
    };
    rt.block_on(async {
        while !stop_flag().load(Ordering::SeqCst) {
            let ident = match unlocked_identity_clone() {
                Ok(i) => i,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            match connect_and_auth(&ws_url, &ident).await {
                Ok(mut session) => {
                    update_mirror_connected(&session.quota, &session.policy);
                    delivery_outbox_upkeep();
                    loop {
                        if stop_flag().load(Ordering::SeqCst) {
                            break;
                        }
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(1),
                            session.recv_push(),
                        )
                        .await
                        {
                            Ok(Ok(Some(frame))) => {
                                handle_push_frame(&ident, &mut session, &frame).await
                            }
                            Ok(Ok(None)) | Err(_) => {
                                delivery_read_ack_upkeep();
                                delivery_outbox_upkeep();
                            }
                            Ok(Err(e)) => {
                                set_mirror_error(&e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    set_mirror_error(&e);
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        }
    });
}

async fn handle_push_frame(
    ident: &crate::DecryptedIdentity,
    session: &mut DeliverySession,
    frame: &Value,
) {
    let ty = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match ty {
        "message.inbound" => {
            let envelope = match frame.get("envelope") {
                Some(e) => e,
                None => {
                    native_log::warn("delivery", "message.inbound missing envelope");
                    return;
                }
            };
            match open_text_from_envelope(ident, envelope) {
                Ok((message_id, sender_wire, text)) => {
                    let now = now_ms();
                    let created = envelope
                        .get("created_at_ms")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(now);
                    let updated = persist_inbound_text_on_wire(
                        "",
                        &message_id,
                        &text,
                        &sender_wire,
                        created,
                        now,
                    );
                    if updated {
                        enqueue_dm_poll(
                            &sender_wire,
                            &message_id,
                            "text",
                            Some(text),
                            created,
                            Some(now),
                        );
                    }
                    if let Err(e) = session.inbox_ack(&message_id, &sender_wire).await {
                        native_log::warn(
                            "delivery",
                            format!("inbox.ack failed message_id={message_id}: {e}"),
                        );
                    }
                    try_read_ack_after_inbound_async(session, &message_id, &sender_wire).await;
                }
                Err(e) => {
                    native_log::warn("delivery", format!("message.inbound decrypt failed: {e}"));
                }
            }
        }
        "message.ack_to_sender" => {
            let message_id = frame.get("message_id").and_then(|v| v.as_str()).unwrap_or("");
            let recipient = frame
                .get("recipient_wire")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ev = json!({
                "kind": "dm_message",
                "msg_kind": "ack_received",
                "ref_id": message_id,
                "id": format!("ack_received:{message_id}"),
                "sender_public_key_hex": recipient,
                "public_key_hex": recipient,
            });
            let _ = apply_p2p_event_json(&ev);
            enqueue_delivery_ack_poll(recipient, message_id, "ack_received");
        }
        "message.read_to_sender" => {
            let message_id = frame.get("message_id").and_then(|v| v.as_str()).unwrap_or("");
            let recipient = frame
                .get("recipient_wire")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ev = json!({
                "kind": "dm_message",
                "msg_kind": "ack_read",
                "ref_id": message_id,
                "id": format!("ack_read:{message_id}"),
                "sender_public_key_hex": recipient,
                "public_key_hex": recipient,
            });
            let _ = apply_p2p_event_json(&ev);
            enqueue_delivery_ack_poll(recipient, message_id, "ack_read");
        }
        "message.expired" => {
            // UI mailbox sync picks up expired rows on next list refresh.
        }
        "quota.status" | "quota.warning" => {
            if let Ok(mut m) = mirror_mx().lock() {
                m.quota = frame.clone();
            }
        }
        _ => {}
    }
}

fn update_mirror_connected(quota: &Value, policy: &Value) {
    if let Ok(mut m) = mirror_mx().lock() {
        m.connected = true;
        m.last_error = None;
        m.quota = quota.clone();
        m.policy = policy.clone();
    }
}

fn set_mirror_error(err: &str) {
    if let Ok(mut m) = mirror_mx().lock() {
        m.connected = false;
        m.last_error = Some(err.to_string());
    }
    native_log::warn("delivery", format!("delivery worker: {err}"));
}

fn peer_id_for_sender_wire(sender_wire: &str) -> Option<String> {
    peer_id_from_identity_wire(sender_wire)
        .ok()
        .or_else(|| crate::p2p::libp2p_peer_for_contact_identity(sender_wire))
}

fn enqueue_dm_poll(
    sender_wire: &str,
    message_id: &str,
    msg_kind: &str,
    text: Option<String>,
    created_at_ms: i64,
    received_at_ms: Option<i64>,
) {
    let Some(from) = peer_id_for_sender_wire(sender_wire) else {
        native_log::warn(
            "delivery",
            format!("poll enqueue skipped: no peer id for sender wire"),
        );
        return;
    };
    enqueue_delivery_gossip_event(GossipChatEvent::DmMessage {
        from,
        id: message_id.to_string(),
        msg_kind: msg_kind.to_string(),
        text,
        ref_id: None,
        sender_public_key_hex: sender_wire.to_string(),
        created_at_ms,
        received_at_ms,
    });
}

fn enqueue_delivery_ack_poll(sender_wire: &str, ref_message_id: &str, msg_kind: &str) {
    let Some(from) = peer_id_for_sender_wire(sender_wire) else {
        native_log::warn(
            "delivery",
            format!("ack poll enqueue skipped: no peer id for sender wire"),
        );
        return;
    };
    let mid = ref_message_id.trim();
    if mid.is_empty() {
        return;
    }
    enqueue_delivery_gossip_event(GossipChatEvent::DmMessage {
        from,
        id: format!("{msg_kind}:{mid}"),
        msg_kind: msg_kind.to_string(),
        text: None,
        ref_id: Some(mid.to_string()),
        sender_public_key_hex: sender_wire.to_string(),
        created_at_ms: now_ms(),
        received_at_ms: None,
    });
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn append_outbound_transcript(
    ns: &str,
    recipient_wire: &str,
    message_id: &str,
    text: &str,
) {
    let conv_key = recipient_wire.trim().to_string();
    if !is_valid_public_key_hex(&conv_key) {
        return;
    }
    let now = now_ms();
    let line = StoredChatLine {
        local_id: message_id.to_string(),
        text: text.to_string(),
        outgoing: true,
        from: None,
        message_id: Some(message_id.to_string()),
        delivery: "pending".to_string(),
        created_at_ms: Some(now),
        received_at_ms: None,
        read_ack_sent: false,
    };
    let _ = append_if_new(ns, &conv_key, line);
    let _ = record_thread_message_preview(ns, &conv_key, text, false, Some(now));
}

fn mark_outbox_attempt(message_id: &str, now: i64) {
    if let Ok(mut m) = outbox_last_attempt_mx().lock() {
        m.insert(message_id.to_string(), now);
    }
}

fn outbox_due_for_retry(message_id: &str, now: i64) -> bool {
    let Ok(m) = outbox_last_attempt_mx().lock() else {
        return true;
    };
    match m.get(message_id) {
        None => true,
        Some(last) => now.saturating_sub(*last) >= DELIVERY_OUTBOX_RESEND_INTERVAL_MS,
    }
}

fn try_claim_outbox_upload(message_id: &str) -> bool {
    let Ok(mut g) = outbox_in_flight_mx().lock() else {
        return false;
    };
    if g.contains(message_id) {
        return false;
    }
    g.insert(message_id.to_string());
    true
}

fn release_outbox_upload(message_id: &str) {
    if let Ok(mut g) = outbox_in_flight_mx().lock() {
        g.remove(message_id);
    }
}

fn apply_delivery_upload_ok(recipient: &str, message_id: &str, resp: &Value) {
    if let Some(ns) = active_app_namespace() {
        let _ = patch_outgoing_delivery(&ns, recipient, message_id, "sent");
    }
    enqueue_delivery_gossip_event(GossipChatEvent::OutboundSent {
        message_id: message_id.to_string(),
    });
    if let Ok(mut m) = mirror_mx().lock() {
        if let Some(q) = resp.get("quota") {
            m.quota = q.clone();
        }
    }
    native_log::info("delivery", format!("upload ok message_id={message_id}"));
}

fn spawn_delivery_upload(recipient: String, text: String, message_id: String) {
    let Some(url) = delivery_url() else {
        return;
    };
    if !try_claim_outbox_upload(&message_id) {
        return;
    }
    mark_outbox_attempt(&message_id, now_ms());
    std::thread::Builder::new()
        .name("delivery_upload".into())
        .spawn(move || {
            let result =
                blocking_upload_text(&url, &recipient, &text, &message_id, None);
            release_outbox_upload(&message_id);
            match result {
                Ok(resp) => apply_delivery_upload_ok(&recipient, &message_id, &resp),
                Err(e) => {
                    native_log::warn(
                        "delivery",
                        format!("upload failed message_id={message_id}: {e}"),
                    );
                }
            }
        })
        .ok();
}

/// Retry outbound transcript rows stuck at `delivery=pending` (failed one-shot uploads).
pub fn delivery_outbox_upkeep() {
    if !delivery_mode_enabled() {
        return;
    }
    let Some(ns) = active_app_namespace() else {
        return;
    };
    let contacts = match crate::contacts_v1::list_contacts(&ns) {
        Ok(c) => c,
        Err(_) => return,
    };
    let now = now_ms();
    let mut due = Vec::new();
    for contact in contacts {
        let pk = contact.public_key_hex.trim();
        if !is_valid_public_key_hex(pk) {
            continue;
        }
        let rows = match load_merged(&ns, &[pk.to_string()], None) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for row in rows {
            if !row.outgoing || row.delivery != "pending" {
                continue;
            }
            let Some(mid) = row.message_id.as_deref().map(str::trim).filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !outbox_due_for_retry(mid, now) {
                continue;
            }
            due.push((pk.to_string(), mid.to_string(), row.text.clone()));
        }
    }
    if due.is_empty() {
        return;
    }
    native_log::debug(
        "delivery",
        format!("outbox resync {} pending upload(s)", due.len()),
    );
    for (recipient, message_id, text) in due.into_iter().take(DELIVERY_OUTBOX_MAX_PER_TICK) {
        spawn_delivery_upload(recipient, text, message_id);
    }
}

pub fn delivery_send_text_dm(recipient: &str, text: &str, message_id: &str) -> Value {
    if delivery_url().is_none() {
        return json!({ "ok": false, "error": "GHAL_BOL_DELIVERY_URL not set" });
    }
    let recipient_trim = recipient.trim().to_lowercase();
    if let Ok(ident) = unlocked_identity_clone() {
        let my_pk = ident.public_key_hex().trim().to_lowercase();
        if recipient_trim == my_pk {
            return json!({ "ok": false, "error": "cannot send DM to own identity" });
        }
    }
    if let Some(ns) = active_app_namespace() {
        append_outbound_transcript(&ns, recipient, message_id, text);
    }
    spawn_delivery_upload(
        recipient.to_string(),
        text.to_string(),
        message_id.to_string(),
    );
    json!({
        "ok": true,
        "message_id": message_id,
        "delivery": true,
        "queued": true,
    })
}

pub fn delivery_connection_status() -> Value {
    if let Ok(m) = mirror_mx().lock() {
        return json!({
            "ok": true,
            "connected": m.connected,
            "last_error": m.last_error,
            "quota": m.quota,
            "policy": m.policy,
            "delivery_url": delivery_url(),
        });
    }
    json!({ "ok": true, "connected": false, "delivery_url": delivery_url() })
}

pub fn delivery_quota_status() -> Value {
    let status = delivery_connection_status();
    json!({
        "ok": true,
        "quota": status.get("quota").cloned().unwrap_or(Value::Null),
        "connected": status.get("connected"),
    })
}

pub fn delivery_mailbox_list(include_expired: bool) -> Value {
    let Some(url) = delivery_url() else {
        return json!({ "ok": false, "error": "GHAL_BOL_DELIVERY_URL not set" });
    };
    let ident = match unlocked_identity_clone() {
        Ok(i) => i,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let ws_url = ws_url_from_base(&url);
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    match rt.block_on(async {
        let mut session = connect_and_auth(&ws_url, &ident).await?;
        session.mailbox_list(include_expired).await
    }) {
        Ok(snapshot) => {
            if let Ok(mut m) = mirror_mx().lock() {
                m.mailbox_rows = snapshot
                    .get("rows")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
            }
            json!({ "ok": true, "snapshot": snapshot })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

pub fn delivery_extend_ttl(message_id: &str, extend_secs: u64) -> Value {
    let Some(url) = delivery_url() else {
        return json!({ "ok": false, "error": "GHAL_BOL_DELIVERY_URL not set" });
    };
    let ident = match unlocked_identity_clone() {
        Ok(i) => i,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let ws_url = ws_url_from_base(&url);
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    match rt.block_on(async {
        let mut session = connect_and_auth(&ws_url, &ident).await?;
        session.extend_ttl(&ident, message_id, extend_secs).await
    }) {
        Ok(resp) => json!({ "ok": true, "result": resp }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

pub fn delivery_resend_message(message_id: &str) -> Value {
    let Some(ns) = active_app_namespace() else {
        return json!({ "ok": false, "error": "app namespace not set" });
    };
    let mid = message_id.trim();
    if mid.is_empty() {
        return json!({ "ok": false, "error": "message_id required" });
    }
    // Scan merged transcripts for outbound row with this id.
    let contacts = match crate::contacts_v1::list_contacts(&ns) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    for contact in contacts {
        let pk = contact.public_key_hex.trim();
        if !is_valid_public_key_hex(pk) {
            continue;
        }
        let rows = match load_merged(&ns, &[pk.to_string()], None) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for row in rows {
            if !row.outgoing {
                continue;
            }
            if row.message_id.as_deref() != Some(mid) {
                continue;
            }
            return delivery_send_text_dm(pk, &row.text, mid);
        }
    }
    json!({ "ok": false, "error": "outbound message not found locally" })
}
