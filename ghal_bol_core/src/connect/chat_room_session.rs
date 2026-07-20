//! Chat room session clock (identity-wire keyed).

use std::sync::atomic::{AtomicI64, Ordering};

use super::session::{chrono_now_ms, SessionState};
use super::types::SessionPeer;
use super::ui_session::{app_ack_read_enabled, app_ui_visible, live_foreground_peer};
use crate::p2p::native_log;

static CHAT_ROOM_SESSION_AT_MS: AtomicI64 = AtomicI64::new(0);

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

pub(crate) fn begin_chat_room_session(session: &SessionState, peer: &SessionPeer) {
    let now = chrono_now_ms();
    set_chat_room_session_at_ms(now);
    let Some(ns) = app_namespace(session) else {
        return;
    };
    if let Err(e) = crate::contacts_v1::sync_chat_room_exit_at_ms(ns, peer, now) {
        native_log::debug("read_ack", format!("chat room session begin {peer}: {e}"));
    }
}


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
        native_log::debug("read_ack", format!("chat room freeze on inactive: {e}"));
    } else {
        native_log::info("read_ack", format!("chat room frozen on inactive pk={fg_pk} exit_at_ms={at}"));
    }
    clear_chat_room_session();
}

pub(crate) fn tick_chat_room_session_if_active(session: &SessionState) {
    let Some(fg_pk) = live_foreground_peer() else {
        return;
    };
    if !app_ui_visible() || !app_ack_read_enabled() {
        return;
    };
    let now = chrono_now_ms();
    set_chat_room_session_at_ms(now);
    let Some(ns) = app_namespace(session) else {
        return;
    };
    let _ = crate::contacts_v1::sync_chat_room_exit_at_ms(ns, &fg_pk, now);
}

pub(crate) fn read_ack_cutoff_ms(session: &SessionState, peer: &SessionPeer) -> i64 {
    if super::ui_session::is_live_foreground_peer(peer) {
        let live = chat_room_session_at_ms();
        if live > 0 {
            return live;
        }
        return chrono_now_ms();
    }
    let Some(ns) = app_namespace(session) else {
        return chrono_now_ms();
    };
    crate::contacts_v1::chat_room_exit_at_ms(ns, peer)
        .ok()
        .flatten()
        .filter(|t| *t > 0)
        .unwrap_or_else(chrono_now_ms)
}
