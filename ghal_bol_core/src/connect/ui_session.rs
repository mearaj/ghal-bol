use super::chat_room_session::freeze_open_chat_room_session;
use super::notify::{drop_pending_call_invite, notify_dm_presence_wake};
use super::session::SessionState;
use super::types::{GossipChatEvent, OutboundCmd, READ_ACK_CATCHUP_THROTTLE_MS, SessionPeer};
use crate::dm_transport::ContactPk;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

static LIVE_FOREGROUND_PEER: OnceLock<RwLock<Option<ContactPk>>> = OnceLock::new();
static LAST_ROOM_PEER: OnceLock<RwLock<Option<ContactPk>>> = OnceLock::new();

#[inline]
pub(crate) fn platform_incoming_call_show(peer_pk: &str, call_id: &str) {
    if app_ui_visible() {
        return;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::incoming_call_notify::show_incoming_call(peer_pk, call_id);
}

#[inline]
pub(crate) fn platform_incoming_call_dismiss() {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::incoming_call_notify::dismiss_incoming_call();
}

pub(crate) fn on_local_call_signal_sent(call_id: &str, kind: crate::call_sig_v1::CallSigKind) {
    match kind {
        crate::call_sig_v1::CallSigKind::Hangup | crate::call_sig_v1::CallSigKind::Reject => {
            platform_incoming_call_dismiss();
            drop_pending_call_invite(call_id);
        }
        crate::call_sig_v1::CallSigKind::Accept => {
            platform_incoming_call_dismiss();
        }
        _ => {}
    }
}

fn live_foreground_peer_mx() -> &'static RwLock<Option<ContactPk>> {
    LIVE_FOREGROUND_PEER.get_or_init(|| RwLock::new(None))
}

pub(crate) fn last_room_peer_mx() -> &'static RwLock<Option<ContactPk>> {
    LAST_ROOM_PEER.get_or_init(|| RwLock::new(None))
}

pub fn last_room_peer() -> Option<ContactPk> {
    last_room_peer_mx().read().ok().and_then(|g| g.clone())
}

static FOREGROUND_PEER_CMD_GEN: AtomicU64 = AtomicU64::new(0);

pub fn bump_foreground_peer_cmd_gen() -> u64 {
    FOREGROUND_PEER_CMD_GEN.fetch_add(1, Ordering::SeqCst) + 1
}

pub fn sync_foreground_peer_now(peer: Option<ContactPk>) {
    if let Ok(mut g) = live_foreground_peer_mx().write() {
        *g = peer.clone();
    }
    if let Some(p) = peer {
        if let Ok(mut last) = last_room_peer_mx().write() {
            *last = Some(p);
        }
    }
}

pub(crate) fn live_foreground_peer() -> Option<ContactPk> {
    live_foreground_peer_mx()
        .read()
        .ok()
        .and_then(|g| g.clone())
}

pub fn live_foreground_peer_pk() -> Option<ContactPk> {
    live_foreground_peer()
}

pub(crate) fn is_live_foreground_peer(peer: &SessionPeer) -> bool {
    live_foreground_peer().is_some_and(|pk| crate::public_key_util::same_contact_pk(&pk, peer))
}

pub(crate) fn emit_call_media(
    tx: &Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    call_id: &str,
    peer_public_key_hex: &str,
    state: &str,
    reason: Option<&str>,
) {
    let Some(tx) = tx else {
        return;
    };
    let snap = crate::p2p::call_active::snapshot();
    let (camera_on, remote_video_on) = snap
        .as_ref()
        .filter(|s| s.call_id == call_id)
        .map(|s| (s.camera_on, s.remote_video_on))
        .unwrap_or((false, false));
    let _ = tx.send(GossipChatEvent::CallMedia {
        call_id: call_id.to_string(),
        peer_public_key_hex: peer_public_key_hex.trim().to_string(),
        state: state.to_string(),
        camera_on,
        remote_video_on,
        reason: reason.map(str::to_string),
    });
}

pub fn live_foreground_peer_for_catchup() -> Option<ContactPk> {
    live_foreground_peer()
}

static APP_ACK_READ_ENABLED: OnceLock<AtomicBool> = OnceLock::new();
static APP_UI_VISIBLE: OnceLock<AtomicBool> = OnceLock::new();

fn app_ui_visible_mx() -> &'static AtomicBool {
    APP_UI_VISIBLE.get_or_init(|| AtomicBool::new(false))
}

pub fn set_app_ui_visible(visible: bool) {
    let was = app_ui_visible_mx().swap(visible, Ordering::SeqCst);
    if visible {
        platform_incoming_call_dismiss();
        if !was {
            notify_dm_presence_wake();
        }
    } else if was {
        freeze_open_chat_room_session();
    }
}

pub fn app_ui_visible() -> bool {
    app_ui_visible_mx().load(Ordering::SeqCst)
}

fn app_ack_read_enabled_mx() -> &'static AtomicBool {
    APP_ACK_READ_ENABLED.get_or_init(|| AtomicBool::new(false))
}

pub fn may_send_read_ack_for_contact_pk(sender_pk: &str) -> bool {
    app_ui_visible()
        && app_ack_read_enabled()
        && live_foreground_peer()
            .is_some_and(|f| crate::public_key_util::same_contact_pk(&f, sender_pk))
}

pub(crate) fn may_send_in_room_read_ack(_session: &SessionState, peer: &SessionPeer) -> bool {
    is_live_foreground_peer(peer) && may_send_read_ack_for_contact_pk(peer)
}

pub(crate) fn inbound_suppresses_hub_unread(sender_pk: &str, from_key: &str) -> bool {
    if !app_ui_visible() || !app_ack_read_enabled() {
        return false;
    }
    let Some(live) = live_foreground_peer() else {
        return false;
    };
    crate::public_key_util::same_contact_pk(&live, from_key)
        || crate::public_key_util::same_contact_pk(&live, sender_pk)
}

pub fn set_app_ack_read_enabled(enabled: bool) {
    app_ack_read_enabled_mx().store(enabled, Ordering::SeqCst);
}

pub fn queue_read_ack_catchup(out_tx: &std::sync::mpsc::Sender<OutboundCmd>, peer: ContactPk) {
    if !app_ui_visible()
        || !app_ack_read_enabled()
        || !live_foreground_peer().is_some_and(|f| f == peer)
    {
        return;
    }
    let _ = out_tx.send(OutboundCmd::RunReadAckCatchup {
        identity_wire: peer,
    });
}

fn read_catchup_throttle_mx() -> &'static RwLock<HashMap<String, i64>> {
    static M: OnceLock<RwLock<HashMap<String, i64>>> = OnceLock::new();
    M.get_or_init(|| RwLock::new(HashMap::new()))
}

pub(crate) fn read_ack_catchup_throttled(peer: &SessionPeer, now_ms: i64) -> bool {
    let Ok(mut m) = read_catchup_throttle_mx().write() else {
        return false;
    };
    let last = m.get(peer).copied().unwrap_or(0);
    if now_ms.saturating_sub(last) < READ_ACK_CATCHUP_THROTTLE_MS {
        return true;
    }
    m.insert(peer.clone(), now_ms);
    false
}

pub(crate) fn app_ack_read_enabled() -> bool {
    app_ack_read_enabled_mx().load(Ordering::SeqCst)
}
