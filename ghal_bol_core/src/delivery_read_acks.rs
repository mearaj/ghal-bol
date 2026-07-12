//! Delivery-server read receipts (`inbox.read` / `message.read_to_sender`).
//!
//! Mirrors P2P read-ack policy (DESIGN.md): in-room only for **new** mail; leave backlog
//! for inbound accepted while the room was open. All wire I/O here — Flutter displays transcript only.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use crate::contacts_v1::is_valid_public_key_hex;
use crate::delivery_client::blocking_inbox_read;
use crate::dm_event_handler::active_app_namespace;
use crate::dm_transcript_store::{StoredChatLine, load_merged, patch_inbound_read_ack_sent_global};
use crate::p2p::chat_room_session_at_ms;
use crate::p2p::may_send_read_ack_for_contact_pk;
use crate::p2p::native_log;

#[derive(Clone, Debug)]
struct PendingDeliveryReadAck {
    message_id: String,
    sender_wire: String,
    queued_at_ms: i64,
}

fn pending_mx() -> &'static Mutex<Vec<PendingDeliveryReadAck>> {
    static M: OnceLock<Mutex<Vec<PendingDeliveryReadAck>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(Vec::new()))
}

fn in_flight_mx() -> &'static Mutex<HashSet<String>> {
    static M: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashSet::new()))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn normalize_pk(pk: &str) -> String {
    pk.trim().to_ascii_lowercase()
}

fn read_ack_cutoff_ms(sender_pk: &str) -> i64 {
    if may_send_read_ack_for_contact_pk(sender_pk) {
        let live = chat_room_session_at_ms();
        if live > 0 {
            return live;
        }
        return now_ms();
    }
    let Some(ns) = active_app_namespace() else {
        return now_ms();
    };
    crate::contacts_v1::chat_room_exit_at_ms(&ns, sender_pk)
        .ok()
        .flatten()
        .filter(|t| *t > 0)
        .unwrap_or_else(now_ms)
}

fn inbound_received_at_for_row(row: &StoredChatLine) -> Option<i64> {
    row.received_at_ms.or(row.created_at_ms)
}

fn lookup_keys_for_sender(ns: &str, sender_pk: &str) -> Vec<String> {
    crate::dm_event_handler::inbound_transcript_lookup_keys(ns, sender_pk, sender_pk, sender_pk)
}

fn enqueue_pending(message_id: &str, sender_wire: &str) {
    let mid = message_id.trim();
    let sender = normalize_pk(sender_wire);
    if mid.is_empty() || !is_valid_public_key_hex(&sender) {
        return;
    }
    let Ok(mut q) = pending_mx().lock() else {
        return;
    };
    if q.iter().any(|p| p.message_id == mid && p.sender_wire == sender) {
        return;
    }
    q.push(PendingDeliveryReadAck {
        message_id: mid.to_string(),
        sender_wire: sender,
        queued_at_ms: now_ms(),
    });
}

fn dequeue_pending(message_id: &str, sender_wire: &str) {
    let mid = message_id.trim();
    let sender = normalize_pk(sender_wire);
    let Ok(mut q) = pending_mx().lock() else {
        return;
    };
    q.retain(|p| !(p.message_id == mid && p.sender_wire == sender));
}

fn claim_in_flight(message_id: &str) -> bool {
    let Ok(mut g) = in_flight_mx().lock() else {
        return false;
    };
    g.insert(message_id.trim().to_string())
}

fn release_in_flight(message_id: &str) {
    if let Ok(mut g) = in_flight_mx().lock() {
        g.remove(message_id.trim());
    }
}

fn patch_read_ack_sent(message_id: &str) -> bool {
    let Some(ns) = active_app_namespace() else {
        return false;
    };
    patch_inbound_read_ack_sent_global(&ns, message_id).is_ok()
}

fn send_inbox_read(ws_url: &str, message_id: &str, sender_pk: &str) -> bool {
    if !claim_in_flight(message_id) {
        return false;
    }
    let result = blocking_inbox_read(ws_url, message_id, sender_pk);
    release_in_flight(message_id);
    match result {
        Ok(()) => {
            let _ = patch_read_ack_sent(message_id);
            dequeue_pending(message_id, sender_pk);
            native_log::info(
                "delivery_read",
                format!("inbox.read sent mid={message_id} sender={sender_pk}"),
            );
            true
        }
        Err(e) => {
            native_log::warn(
                "delivery_read",
                format!("inbox.read failed mid={message_id} sender={sender_pk}: {e}"),
            );
            if let Ok(mut q) = pending_mx().lock() {
                for p in q.iter_mut() {
                    if p.message_id == message_id.trim() && p.sender_wire == sender_pk {
                        p.queued_at_ms = now_ms();
                    }
                }
            }
            false
        }
    }
}

/// Seed + drain read acks when hub opens room or read gate turns on (delivery mode).
pub fn queue_delivery_read_catchup(sender_pk: &str) {
    if !crate::delivery_runtime::delivery_mode_enabled() {
        return;
    }
    let pk = normalize_pk(sender_pk);
    if !is_valid_public_key_hex(&pk) {
        return;
    }
    seed_read_acks_for_sender(&pk);
    drain_pending_read_acks(true);
}

/// Leave / switch room: flush read backlog for the contact we left (frozen cutoff).
pub fn dispatch_delivery_leave_drain(left_pk: &str) {
    if !crate::delivery_runtime::delivery_mode_enabled() {
        return;
    }
    let pk = normalize_pk(left_pk);
    if !is_valid_public_key_hex(&pk) {
        return;
    }
    seed_read_acks_for_sender(&pk);
    drain_pending_read_acks(true);
}

/// After inbound delivery ack — send read receipt on the worker session when read gate is open.
pub async fn try_read_ack_after_inbound_async(
    session: &mut crate::delivery_client::DeliverySession,
    message_id: &str,
    sender_wire: &str,
) {
    if !crate::delivery_runtime::delivery_mode_enabled() {
        return;
    }
    let sender = normalize_pk(sender_wire);
    if !is_valid_public_key_hex(&sender) {
        return;
    }
    if !may_send_read_ack_for_contact_pk(&sender) {
        return;
    }
    if !claim_in_flight(message_id) {
        return;
    }
    let result = session.inbox_read(message_id, &sender).await;
    release_in_flight(message_id);
    match result {
        Ok(()) => {
            let _ = patch_read_ack_sent(message_id);
            dequeue_pending(message_id, &sender);
            native_log::info(
                "delivery_read",
                format!("inbox.read sent mid={message_id} sender={sender}"),
            );
        }
        Err(e) => {
            native_log::warn(
                "delivery_read",
                format!("inbox.read failed mid={message_id} sender={sender}: {e}"),
            );
        }
    }
}

/// ~1s upkeep hook from the delivery worker receive loop.
pub fn delivery_read_ack_upkeep() {
    if !crate::delivery_runtime::delivery_mode_enabled() {
        return;
    }
    drain_pending_read_acks(false);
}

fn seed_read_acks_for_sender(sender_pk: &str) {
    let Some(ns) = active_app_namespace() else {
        return;
    };
    let cutoff = read_ack_cutoff_ms(sender_pk);
    let keys = lookup_keys_for_sender(&ns, sender_pk);
    let Ok(rows) = load_merged(&ns, &keys, None) else {
        return;
    };
    let mut seeded = 0usize;
    for row in rows {
        if row.outgoing || row.read_ack_sent {
            continue;
        }
        let Some(received_at) = inbound_received_at_for_row(&row) else {
            continue;
        };
        if received_at > cutoff {
            continue;
        }
        let Some(mid) = row
            .message_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        enqueue_pending(mid, sender_pk);
        seeded += 1;
    }
    if seeded > 0 {
        native_log::info(
            "delivery_read",
            format!("seeded {seeded} pending inbox.read for {sender_pk} cutoff_ms={cutoff}"),
        );
    }
}

fn drain_pending_read_acks(force: bool) {
    let Some(url) = crate::delivery_runtime::delivery_url() else {
        return;
    };
    let ws_url = crate::delivery_client::ws_url_from_base(&url);
    let pending: Vec<PendingDeliveryReadAck> = pending_mx()
        .lock()
        .ok()
        .map(|q| q.clone())
        .unwrap_or_default();
    let now = now_ms();
    const RETRY_MS: i64 = 1000;
    for item in pending {
        if !force && now.saturating_sub(item.queued_at_ms) < RETRY_MS {
            continue;
        }
        if !should_attempt_read_ack(&item.sender_wire, &item.message_id) {
            continue;
        }
        let _ = send_inbox_read(&ws_url, &item.message_id, &item.sender_wire);
    }
}

fn should_attempt_read_ack(sender_pk: &str, message_id: &str) -> bool {
    let Some(ns) = active_app_namespace() else {
        return false;
    };
    if patch_already_sent(&ns, sender_pk, message_id) {
        dequeue_pending(message_id, sender_pk);
        return false;
    }
    if may_send_read_ack_for_contact_pk(sender_pk) {
        return true;
    }
    let cutoff = read_ack_cutoff_ms(sender_pk);
    row_eligible_for_leave_read(sender_pk, message_id, cutoff)
}

fn patch_already_sent(ns: &str, sender_pk: &str, message_id: &str) -> bool {
    let keys = lookup_keys_for_sender(ns, sender_pk);
    let Ok(rows) = load_merged(ns, &keys, None) else {
        return false;
    };
    rows.iter().any(|r| {
        !r.outgoing
            && r.read_ack_sent
            && r.message_id.as_deref().map(str::trim) == Some(message_id.trim())
    })
}

fn row_eligible_for_leave_read(sender_pk: &str, message_id: &str, cutoff_ms: i64) -> bool {
    let Some(ns) = active_app_namespace() else {
        return false;
    };
    let keys = lookup_keys_for_sender(&ns, sender_pk);
    let Ok(rows) = load_merged(&ns, &keys, None) else {
        return false;
    };
    rows.iter().any(|r| {
        if r.outgoing || r.read_ack_sent {
            return false;
        }
        if r.message_id.as_deref().map(str::trim) != Some(message_id.trim()) {
            return false;
        }
        inbound_received_at_for_row(r)
            .is_some_and(|t| t <= cutoff_ms)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pk_lowercases() {
        assert_eq!(normalize_pk("  ABCD "), "abcd");
    }
}
