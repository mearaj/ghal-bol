//! Live chat-room session clock + per-contact `chat_room_exit_at_ms` sync (DESIGN.md).
//!
//! While A has the room open with B (foreground + read gate + UI visible), A's session
//! timestamp and B's contact field advance together ~1/s. On leave/switch/inactive, B's
//! field freezes; read-ack seeding uses `received_at_ms <= chat_room_exit_at_ms`.

use std::sync::atomic::{AtomicI64, Ordering};

use libp2p::PeerId;

use super::chrono_now_ms;
use super::prelude::*;
use super::{SessionState, app_ack_read_enabled, app_ui_visible, is_live_foreground_peer, live_foreground_peer};

static CHAT_ROOM_SESSION_AT_MS: AtomicI64 = AtomicI64::new(0);

#[inline]
pub fn chat_room_session_at_ms() -> i64 {
    CHAT_ROOM_SESSION_AT_MS.load(Ordering::Relaxed)
}

fn set_chat_room_session_at_ms(at_ms: i64) {
    CHAT_ROOM_SESSION_AT_MS.store(at_ms, Ordering::Relaxed);
}

pub(crate) fn clear_chat_room_session() {
    CHAT_ROOM_SESSION_AT_MS.store(0, Ordering::Relaxed);
}

fn app_namespace(session: &SessionState) -> Option<&str> {
    session
        .app_namespace
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
}

fn peer_public_key_hex(session: &SessionState, peer: PeerId) -> Option<String> {
    session
        .dm_peer_for_libp2p(peer)
        .and_then(|d| d.public_key_hex.clone())
}

/// Start / continue in-room session: session clock + foreground contact mirror the same value.
pub(crate) fn begin_chat_room_session(session: &SessionState, peer: PeerId) {
    let now = chrono_now_ms();
    set_chat_room_session_at_ms(now);
    let Some(ns) = app_namespace(session) else {
        return;
    };
    let Some(pk) = peer_public_key_hex(session, peer) else {
        return;
    };
    if let Err(e) = crate::contacts_v1::sync_chat_room_exit_at_ms(ns, &pk, now) {
        native_log::debug(
            "read_ack",
            format!("chat room session begin {peer}: contact sync failed: {e}"),
        );
    }
}

/// Freeze per-contact cutoff from the live session clock (leave / switch / UI inactive).
pub(crate) fn freeze_chat_room_for_peer(session: &SessionState, peer: PeerId) {
    let at = chat_room_session_at_ms();
    if at <= 0 {
        return;
    }
    let Some(ns) = app_namespace(session) else {
        return;
    };
    let Some(pk) = peer_public_key_hex(session, peer) else {
        return;
    };
    if let Err(e) = crate::contacts_v1::sync_chat_room_exit_at_ms(ns, &pk, at) {
        native_log::debug(
            "read_ack",
            format!("chat room freeze {peer}: contact sync failed: {e}"),
        );
        return;
    }
    native_log::info(
        "read_ack",
        format!("chat room frozen for {pk} exit_at_ms={at}"),
    );
}

/// Freeze the foreground contact when UI backgrounds (no `SessionState` handle needed).
pub fn freeze_open_chat_room_session() {
    let Some(fg_pk) = live_foreground_peer() else {
        clear_chat_room_session();
        return;
    };
    let at = chat_room_session_at_ms();
    if at <= 0 {
        clear_chat_room_session();
        return;
    };
    let Some(ns) = crate::dm_event_handler::active_app_namespace() else {
        clear_chat_room_session();
        return;
    };
    if let Err(e) = crate::contacts_v1::sync_chat_room_exit_at_ms(&ns, &fg_pk, at) {
        native_log::debug(
            "read_ack",
            format!("chat room freeze on inactive: contact sync failed: {e}"),
        );
    } else {
        native_log::info(
            "read_ack",
            format!("chat room frozen on inactive pk={fg_pk} exit_at_ms={at}"),
        );
    }
    clear_chat_room_session();
}

/// ~1 s heartbeat while the read gate is open for the foreground peer.
pub(crate) fn tick_chat_room_session_if_active(session: &SessionState) {
    let Some(fg_pk) = live_foreground_peer() else {
        return;
    };
    if !app_ui_visible() || !app_ack_read_enabled() {
        return;
    }
    let Some(peer_id) = super::libp2p_peer_for_contact_identity(&fg_pk) else {
        return;
    };
    if !is_live_foreground_peer(peer_id) {
        return;
    }
    let now = chrono_now_ms();
    set_chat_room_session_at_ms(now);
    let Some(ns) = app_namespace(session) else {
        return;
    };
    let _ = crate::contacts_v1::sync_chat_room_exit_at_ms(ns, &fg_pk, now);
}

/// Read-ack cutoff for seeding: live session while in-room; frozen contact field after leave.
pub(crate) fn read_ack_cutoff_ms(session: &SessionState, peer: PeerId) -> i64 {
    if is_live_foreground_peer(peer) {
        let live = chat_room_session_at_ms();
        if live > 0 {
            return live;
        }
        return chrono_now_ms();
    }
    let Some(ns) = app_namespace(session) else {
        return chrono_now_ms();
    };
    let Some(pk) = peer_public_key_hex(session, peer) else {
        return chrono_now_ms();
    };
    crate::contacts_v1::chat_room_exit_at_ms(ns, &pk)
        .ok()
        .flatten()
        .filter(|t| *t > 0)
        .unwrap_or_else(chrono_now_ms)
}
