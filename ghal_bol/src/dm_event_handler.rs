//! Apply P2P DM events to contacts + transcript (moved from Flutter [P2pEventBridge]).

use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use crate::app_paths::{chat_transcript_v1_path, contacts_v1_path, storage_config_for_namespace};
use crate::c_ffi::ffi_unlocked_identity_clone;
use crate::contacts_v1::{
    SavedContact, find_by_peer_id, find_by_public_key, is_valid_public_key_hex,
    merge_discovered_peer_id, record_inbound_preview, upsert_contact,
};
use crate::dm_transcript_store::{
    StoredChatLine, append_if_new, load_merged, patch_outgoing_delivery,
};
use crate::flow_log::{self, short_hex};
use crate::public_key_util::same_contact_pk;
use crate::storage::base_data_dir;

struct HandlerState {
    app_namespace: String,
}

fn state_mx() -> &'static Mutex<Option<HandlerState>> {
    static S: OnceLock<Mutex<Option<HandlerState>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

pub fn set_p2p_handler_context(app_namespace: &str) {
    if let Ok(mut g) = state_mx().lock() {
        *g = Some(HandlerState {
            app_namespace: app_namespace.trim().to_string(),
        });
        let ns = app_namespace.trim();
        flow_log::info(
            "DM/store",
            format!("handler context set app_namespace={ns}"),
        );
        let cfg = storage_config_for_namespace(ns);
        if let Ok(base) = base_data_dir(&cfg) {
            flow_log::info("Storage", format!("base_data_dir={}", base.display()));
        }
        if let Ok(p) = contacts_v1_path(&cfg) {
            flow_log::info("Storage", format!("contacts_path={}", p.display()));
        }
        if let Ok(p) = chat_transcript_v1_path(&cfg) {
            flow_log::info("Storage", format!("transcript_path={}", p.display()));
        }
    }
}

pub fn clear_p2p_handler_context() {
    if let Ok(mut g) = state_mx().lock() {
        if g.is_some() {
            flow_log::info("DM/store", "handler context cleared");
        }
        *g = None;
    }
}

/// Active `app_namespace` from unlock / `p2p_start` (coord prefs, etc.).
pub fn active_app_namespace() -> Option<String> {
    state_mx()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.app_namespace.clone()))
}

fn public_key_hex_from_event(ev: &Value) -> String {
    for key in ["sender_public_key_hex", "public_key_hex"] {
        let Some(s) = ev.get(key).and_then(|v| v.as_str()) else {
            continue;
        };
        let t = s.trim().to_lowercase();
        if is_valid_public_key_hex(&t) {
            return t;
        }
    }
    String::new()
}

fn conversation_key_from_event(ev: &Value) -> String {
    public_key_hex_from_event(ev)
}

/// Canonical transcript view key for poll/UI sync (66-hex pk when known).
pub fn transcript_poll_view_key(app_namespace: &str, ev: &Value) -> Option<String> {
    let kind = ev.get("kind")?.as_str()?;
    let raw = match kind {
        "dm_message" => {
            let pk = public_key_hex_from_event(ev);
            if is_valid_public_key_hex(&pk) {
                pk
            } else {
                let from = ev.get("from").and_then(|v| v.as_str()).unwrap_or("").trim();
                if from.is_empty() {
                    return None;
                }
                find_by_peer_id(app_namespace, from)
                    .ok()
                    .flatten()
                    .map(|c| {
                        if c.has_public_key() {
                            c.public_key_hex.trim().to_ascii_lowercase()
                        } else {
                            c.conversation_key()
                        }
                    })?
            }
        }
        "peer_identified" => {
            let pk = public_key_hex_from_event(ev);
            if !is_valid_public_key_hex(&pk) {
                return None;
            }
            pk
        }
        _ => return None,
    };
    let view = crate::dm_transcript_store::transcript_view_key(app_namespace, &raw);
    if view.is_empty() { None } else { Some(view) }
}

fn dm_ack_sender_matches(sender_pk: &str, contact: &SavedContact) -> bool {
    is_valid_public_key_hex(sender_pk)
        && contact.has_public_key()
        && contact.public_key_hex == sender_pk
}

fn outbound_delivery_for_ack(msg_kind: &str) -> &'static str {
    if msg_kind.trim() == "ack_read" {
        "read"
    } else {
        "delivered"
    }
}

/// Persist inbound text as soon as the DM stream delivers it (must not wait for UI poll).
pub fn persist_inbound_text_on_wire(
    from_peer_id: &str,
    message_id: &str,
    text: &str,
    sender_public_key_hex: &str,
    created_at_ms: i64,
) -> bool {
    let ev = serde_json::json!({
        "kind": "dm_message",
        "from": from_peer_id,
        "id": message_id,
        "msg_kind": "text",
        "text": text,
        "sender_public_key_hex": sender_public_key_hex,
        "created_at_ms": created_at_ms,
    });
    apply_p2p_event_json(&ev)
}

/// Returns `true` when contacts or transcript were updated (UI should refresh).
pub fn apply_p2p_event_json(ev: &Value) -> bool {
    let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let Ok(mut g) = state_mx().lock() else {
        return false;
    };
    let Some(st) = g.as_mut() else {
        if kind == "dm_message" || kind == "peer_identified" {
            flow_log::warn(
                "DM/store",
                "event ignored: handler context not set (p2p_start?)",
            );
        }
        return false;
    };
    let ns = st.app_namespace.clone();
    drop(g);

    match kind {
        "peer_identified" => {
            let pk = public_key_hex_from_event(ev);
            if is_valid_public_key_hex(&pk) {
                flow_log::info(
                    "DM/store",
                    format!("merge discovered pk={}", short_hex(&pk)),
                );
                let _ = merge_discovered_peer_id(&ns, &pk, "");
                return true;
            }
            flow_log::warn(
                "DM/store",
                format!(
                    "peer_identified skipped: pk_valid={}",
                    is_valid_public_key_hex(&pk)
                ),
            );
        }
        "dm_message" => {
            let msg_kind = ev.get("msg_kind").and_then(|v| v.as_str()).unwrap_or("");
            if msg_kind == "text" {
                return apply_inbound_text(&ns, ev);
            }
            if msg_kind == "ack_received" || msg_kind == "ack_read" {
                return apply_inbound_ack(&ns, ev, msg_kind);
            }
            flow_log::warn(
                "DM/store",
                format!("dm_message ignored: unknown msg_kind={msg_kind}"),
            );
        }
        _ => {}
    }
    false
}

/// Transcript keys for poll replay — pk + peer-id buckets (DESIGN.md § Transcript threads).
pub(crate) fn inbound_transcript_lookup_keys(
    ns: &str,
    conv_key: &str,
    sender_pk: &str,
    wire_peer_id: &str,
) -> Vec<String> {
    let mut keys = Vec::new();
    let mut push = |k: &str| {
        let t = k.trim();
        if !t.is_empty() && !keys.iter().any(|x| x == t) {
            keys.push(t.to_string());
        }
    };
    push(conv_key);
    if is_valid_public_key_hex(sender_pk) {
        push(sender_pk);
    }
    if !wire_peer_id.is_empty() {
        push(wire_peer_id);
    }
    if let Ok(Some(c)) = find_by_public_key(ns, sender_pk) {
        push(&c.conversation_key());
    }
    if let Ok(Some(c)) = find_by_peer_id(ns, wire_peer_id) {
        push(&c.conversation_key());
        if c.has_public_key() {
            push(&c.public_key_hex);
        }
    }
    keys
}

fn inbound_text_already_in_transcript(
    ns: &str,
    msg_id: &str,
    conv_key: &str,
    sender_pk: &str,
    wire_peer_id: &str,
) -> bool {
    if msg_id.is_empty() {
        return false;
    }
    let keys = inbound_transcript_lookup_keys(ns, conv_key, sender_pk, wire_peer_id);
    let rows = load_merged(ns, &keys, Some(wire_peer_id)).unwrap_or_default();
    rows.iter()
        .any(|r| !r.outgoing && r.message_id.as_deref().map(str::trim) == Some(msg_id))
}

fn contact_is_blocked(ns: &str, sender_pk: &str, from_key: &str) -> bool {
    if is_valid_public_key_hex(sender_pk) {
        if let Ok(Some(c)) = find_by_public_key(ns, sender_pk) {
            return c.is_blocked;
        }
    }
    if let Ok(Some(c)) = find_by_peer_id(ns, from_key) {
        return c.is_blocked;
    }
    false
}

fn apply_inbound_text(ns: &str, ev: &Value) -> bool {
    let my_pk = ffi_unlocked_identity_clone()
        .ok()
        .map(|id| id.public_key_hex())
        .unwrap_or_default();
    let sender_pk = public_key_hex_from_event(ev);
    let from_key = conversation_key_from_event(ev);
    let wire_peer_id = ev.get("from").and_then(|v| v.as_str()).unwrap_or("").trim();
    let text = ev.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let msg_id = ev.get("id").and_then(|v| v.as_str()).unwrap_or("").trim();
    if text.is_empty() || from_key.is_empty() {
        flow_log::warn(
            "DM/store",
            format!("inbound text dropped: empty text or from (id={msg_id})"),
        );
        return false;
    }
    if is_valid_public_key_hex(&my_pk) && sender_pk == my_pk {
        flow_log::info(
            "DM/store",
            format!("inbound text ignored: own message id={msg_id}"),
        );
        return false;
    }
    if contact_is_blocked(ns, &sender_pk, wire_peer_id) {
        flow_log::info(
            "DM/store",
            format!(
                "inbound text ignored: blocked sender pk={}",
                short_hex(&sender_pk)
            ),
        );
        return false;
    }

    let mut skip_unread = false;
    if let Some(live) = crate::p2p::live_foreground_peer_pk() {
        if same_contact_pk(&live, &from_key) || same_contact_pk(&live, &sender_pk) {
            skip_unread = true;
        }
    }

    let contact = if is_valid_public_key_hex(&sender_pk) {
        find_by_public_key(ns, &sender_pk).ok().flatten()
    } else {
        None
    }
    .or_else(|| find_by_peer_id(ns, &from_key).ok().flatten());
    let conv_key = contact
        .as_ref()
        .map(|c| c.conversation_key())
        .filter(|k| !k.is_empty())
        .unwrap_or_else(|| {
            if is_valid_public_key_hex(&sender_pk) {
                sender_pk.clone()
            } else {
                from_key.clone()
            }
        });

    // Wire persists on receive; poll replays the same event — bump unread/append only once per id.
    let poll_replay =
        inbound_text_already_in_transcript(ns, msg_id, &conv_key, &sender_pk, wire_peer_id);
    if poll_replay {
        skip_unread = true;
    }
    if poll_replay {
        flow_log::info(
            "DM/store",
            format!("inbound text replay id={msg_id} conv={conv_key} — skip unread/append"),
        );
    }

    let mut changed = false;
    if is_valid_public_key_hex(&sender_pk) {
        if find_by_public_key(ns, &sender_pk).ok().flatten().is_none() {
            flow_log::info(
                "DM/store",
                format!(
                    "auto-create contact from inbound text pk={}",
                    short_hex(&sender_pk)
                ),
            );
            let _ = upsert_contact(
                ns,
                SavedContact {
                    public_key_hex: sender_pk.clone(),
                    display_alias: None,
                    last_message_preview: None,
                    last_message_at_ms: None,
                    unread_count: 0,
                    created_at_ms: None,
                    updated_at_ms: None,
                    is_known: false,
                    is_blocked: false,
                },
            );
        }
        let at = ev
            .get("created_at_ms")
            .and_then(|v| v.as_i64())
            .filter(|&t| t > 0)
            .unwrap_or_else(now_ms);
        if !poll_replay {
            let bump_unread = !skip_unread;
            let _ = record_inbound_preview(ns, &sender_pk, text, bump_unread, Some(at));
            changed = true;
        }
    }

    let created_at_ms = ev
        .get("created_at_ms")
        .and_then(|v| v.as_i64())
        .filter(|&t| t > 0)
        .unwrap_or_else(now_ms);
    let local_id = format!("bg-{created_at_ms}-{}", from_key.len());
    let line_from = if !wire_peer_id.is_empty() {
        wire_peer_id.to_string()
    } else {
        from_key.clone()
    };
    let line = StoredChatLine {
        local_id,
        text: text.to_string(),
        outgoing: false,
        from: Some(line_from.clone()),
        message_id: ev.get("id").and_then(|v| v.as_str()).map(str::to_string),
        delivery: "pending".to_string(),
        created_at_ms: Some(created_at_ms),
        read_ack_sent: false,
    };
    if poll_replay {
        return changed;
    }
    match append_if_new(ns, &conv_key, line) {
        Ok(()) => {
            flow_log::info(
                "DM/store",
                format!(
                    "transcript append inbound id={msg_id} conv={conv_key} from={line_from} len={}",
                    text.len()
                ),
            );
            changed = true;
        }
        Err(e) => {
            flow_log::warn(
                "DM/store",
                format!("transcript append failed id={msg_id} conv={conv_key}: {e}"),
            );
        }
    }
    changed
}

fn apply_inbound_ack(ns: &str, ev: &Value, msg_kind: &str) -> bool {
    let ref_id = ev
        .get("ref_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let from_key = conversation_key_from_event(ev);
    let sender_pk = public_key_hex_from_event(ev);
    if ref_id.is_empty() || from_key.is_empty() {
        flow_log::warn("DM/store", "inbound ack dropped: missing ref_id or from");
        return false;
    }

    let contact = if is_valid_public_key_hex(&sender_pk) {
        find_by_public_key(ns, &sender_pk).ok().flatten()
    } else {
        None
    }
    .or_else(|| find_by_peer_id(ns, &from_key).ok().flatten());
    let Some(contact) = contact else {
        flow_log::warn(
            "DM/store",
            format!(
                "inbound ack ignored: no contact for from={from_key} ref={ref_id} kind={msg_kind}"
            ),
        );
        return false;
    };
    if !dm_ack_sender_matches(&sender_pk, &contact) {
        flow_log::warn(
            "DM/store",
            format!(
                "inbound ack rejected: sender pk={} does not match contact ref={ref_id}",
                short_hex(&sender_pk)
            ),
        );
        return false;
    }

    if is_valid_public_key_hex(&sender_pk) {
        let _ = merge_discovered_peer_id(ns, &sender_pk, "");
    }

    let conv = contact.conversation_key();
    let wire_peer_id = ev.get("from").and_then(|v| v.as_str()).unwrap_or("").trim();
    let lookup_keys = inbound_transcript_lookup_keys(ns, &conv, &sender_pk, wire_peer_id);
    let rows = load_merged(ns, &lookup_keys, Some(wire_peer_id)).unwrap_or_default();
    let has_outgoing = rows
        .iter()
        .any(|r| r.outgoing && r.message_id.as_deref() == Some(ref_id));
    let has_inbound = rows
        .iter()
        .any(|r| !r.outgoing && r.message_id.as_deref() == Some(ref_id));

    if msg_kind == "ack_read" && has_outgoing {
        let delivery = outbound_delivery_for_ack(msg_kind);
        match patch_outgoing_delivery(ns, &conv, ref_id, delivery) {
            Ok(true) => {
                flow_log::info(
                    "DM/store",
                    format!("patch outbound delivery={delivery} ref={ref_id} conv={conv}"),
                );
                return true;
            }
            Ok(false) => return false,
            Err(e) => {
                flow_log::warn(
                    "DM/store",
                    format!("patch outbound read failed ref={ref_id}: {e}"),
                );
                return false;
            }
        }
    }

    if msg_kind == "ack_received" {
        if has_outgoing {
            match patch_outgoing_delivery(ns, &conv, ref_id, "delivered") {
                Ok(true) => {
                    flow_log::info(
                        "DM/store",
                        format!("patch outbound delivered ref={ref_id} conv={conv}"),
                    );
                    return true;
                }
                Ok(false) => return false,
                Err(e) => {
                    flow_log::warn(
                        "DM/store",
                        format!("patch outbound delivered failed ref={ref_id}: {e}"),
                    );
                    return false;
                }
            }
        }
        // Inbound read-receipt confirm is owned by the wire path (`mark_read_ack_confirmed`
        // after `has_pending_read_ack`). Poll replay must not set `read_ack_sent` alone.
    }
    flow_log::warn(
        "DM/store",
        format!(
            "inbound ack no matching row: kind={msg_kind} ref={ref_id} conv={conv} has_out={has_outgoing} has_in={has_inbound}"
        ),
    );
    false
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
    use crate::c_ffi::configure_android_data_directory;
    use crate::contacts_v1::find_by_public_key;
    use crate::storage::{StorageConfig, create_or_unlock_identity_v1};
    use serde_json::json;
    use tempfile::TempDir;

    const PK_A: &str = "0305b1b0d27745e0a38a7254ea100abc38857b51ded2ac7ea88d3063fb8da21784";

    struct IsolatedStore {
        _temp: TempDir,
        // Held for the whole test: the data dir + handler-context namespace are
        // process-global, so two of these tests running in parallel would clobber
        // each other's store (flaky unread counts). Serialize them.
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    fn isolated_store_lock() -> &'static std::sync::Mutex<()> {
        static L: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        L.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn isolated_store(ns: &str) -> IsolatedStore {
        let guard = isolated_store_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let temp = TempDir::new().unwrap();
        configure_android_data_directory(temp.path().to_str().unwrap());
        let cfg = StorageConfig::new(ns).with_override_data_dir(temp.path());
        let _ = create_or_unlock_identity_v1(&cfg, "pw");
        set_p2p_handler_context(ns);
        IsolatedStore {
            _temp: temp,
            _guard: guard,
        }
    }

    #[test]
    fn conversation_key_from_event_uses_sender_pk_not_libp2p_from() {
        let ev = json!({
            "sender_public_key_hex": PK_A,
            "from": "12D3KooWMustNotBecomeConversationKey"
        });
        assert_eq!(conversation_key_from_event(&ev), PK_A);
    }

    #[test]
    fn conversation_key_empty_when_only_peer_id_present() {
        let ev = json!({
            "from": "12D3KooWOnlyWireId",
            "peer_id": "12D3KooWOnlyWireId"
        });
        assert!(conversation_key_from_event(&ev).is_empty());
    }

    #[test]
    fn apply_inbound_text_poll_replay_peer_id_bucket_no_double_unread() {
        const NS: &str = "test.unread.peer_id_bucket";
        let _store = isolated_store(NS);
        const PEER_ID: &str = "16Uiu2HAmTestPeerIdBucketReplay";
        let line = StoredChatLine {
            local_id: "bg-1".to_string(),
            text: "hello".to_string(),
            outgoing: false,
            from: Some(PEER_ID.to_string()),
            message_id: Some("peer-bucket-1".to_string()),
            delivery: "pending".to_string(),
            created_at_ms: Some(1000),
            read_ack_sent: false,
        };
        append_if_new(NS, PEER_ID, line).unwrap();
        let ev = json!({
            "kind": "dm_message",
            "msg_kind": "text",
            "id": "peer-bucket-1",
            "from": PEER_ID,
            "text": "hello",
            "sender_public_key_hex": PK_A,
            "created_at_ms": 1000
        });
        assert!(!apply_p2p_event_json(&ev));
        let c = find_by_public_key(NS, PK_A).unwrap().unwrap();
        assert_eq!(c.unread_count, 0);
        clear_p2p_handler_context();
    }

    #[test]
    fn apply_inbound_text_poll_replay_does_not_double_unread() {
        const NS: &str = "test.unread.wire.poll";
        let _store = isolated_store(NS);
        let ev = json!({
            "kind": "dm_message",
            "msg_kind": "text",
            "id": "wire-poll-1",
            "text": "hello",
            "sender_public_key_hex": PK_A,
            "created_at_ms": 1000
        });
        // Host had zero contacts — first inbound auto-creates contact + transcript.
        assert!(apply_p2p_event_json(&ev));
        assert!(!apply_p2p_event_json(&ev));
        let c = find_by_public_key(NS, PK_A).unwrap().unwrap();
        assert_eq!(c.unread_count, 1);
        clear_p2p_handler_context();
    }

    #[test]
    fn apply_inbound_text_out_of_order_still_counts_three_unread() {
        const NS: &str = "test.unread.order";
        let _store = isolated_store(NS);
        for (id, at, text) in [
            ("m3", 3000, "third"),
            ("m1", 1000, "first"),
            ("m2", 2000, "second"),
        ] {
            let ev = json!({
                "kind": "dm_message",
                "msg_kind": "text",
                "id": id,
                "text": text,
                "sender_public_key_hex": PK_A,
                "created_at_ms": at
            });
            assert!(apply_p2p_event_json(&ev));
        }
        let c = find_by_public_key(NS, PK_A).unwrap().unwrap();
        assert_eq!(c.unread_count, 3);
        assert_eq!(c.last_message_preview.as_deref(), Some("third"));
        clear_p2p_handler_context();
    }
}
