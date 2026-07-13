//! In-process native DM worker (shared by FFI and the Unix-socket daemon).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::dm_transport::DmDialAddr;
use crate::connect::{
    identity_wire_for_libp2p_peer, new_msg_id_for_ffi,
};
use serde_json::Value;

use crate::call_sig_v1::CallSigKind;
use crate::call_state;
use crate::dm_event_handler::{
    apply_p2p_event_json, clear_p2p_handler_context, active_app_namespace, set_p2p_handler_context,
};
use crate::contacts_v1::{clear_unread, is_valid_public_key_hex};
use crate::msg_v1::MsgKind;
use crate::p2p::{
    DEFAULT_GOSSIP_TOPIC, DmPeer, GossipChatConfig, GossipChatEvent, OutboundCmd, native_log,
    live_foreground_peer_for_catchup, libp2p_peer_for_contact_identity, queue_read_ack_catchup, run_gossip_chat_node_with_std_io,
    set_drop_pending_call_invite_hook, sync_foreground_peer_now,
};
use crate::session_runtime::unlocked_identity_clone;

struct P2pHolder {
    out_tx: std::sync::mpsc::Sender<OutboundCmd>,
    stop: Arc<AtomicBool>,
    join: std::thread::JoinHandle<()>,
    events_rx: std::sync::mpsc::Receiver<GossipChatEvent>,
}

fn p2p_mx() -> &'static Mutex<Option<P2pHolder>> {
    static P: OnceLock<Mutex<Option<P2pHolder>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

fn pending_p2p_events_mx() -> &'static Mutex<VecDeque<GossipChatEvent>> {
    static P: OnceLock<Mutex<VecDeque<GossipChatEvent>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn register_dm_peer_throttle_mx() -> &'static RwLock<std::collections::HashMap<String, i64>> {
    static S: OnceLock<RwLock<std::collections::HashMap<String, i64>>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(std::collections::HashMap::new()))
}

fn clear_pending_p2p_events() {
    if let Ok(mut q) = pending_p2p_events_mx().lock() {
        q.clear();
    }
}

static LAST_POLL_MAINTENANCE_MS: AtomicI64 = AtomicI64::new(0);
const POLL_MAINTENANCE_INTERVAL_MS: i64 = 5_000;

fn maybe_maintain_poll_queue(now_ms: i64) {
    let last = LAST_POLL_MAINTENANCE_MS.load(Ordering::Relaxed);
    if now_ms.saturating_sub(last) < POLL_MAINTENANCE_INTERVAL_MS {
        return;
    }
    LAST_POLL_MAINTENANCE_MS.store(now_ms, Ordering::Relaxed);
    purge_stale_pending_call_invites(now_ms);
    if call_state::expire_stale_ringing(now_ms) {
        crate::incoming_call_notify::dismiss_incoming_call();
    }
}

/// Drop buffered `invite` poll events so a late UI poll cannot ring after remote hangup.
pub fn drop_pending_call_invite(call_id: &str) {
    let cid = call_id.trim();
    if cid.is_empty() {
        return;
    }
    let Ok(mut q) = pending_p2p_events_mx().lock() else {
        return;
    };
    q.retain(|ev| {
        !matches!(
            ev,
            GossipChatEvent::CallSignal {
                call_id: c,
                signal,
                ..
            } if c == cid && signal == "invite"
        )
    });
}

/// Drop invite poll events older than [call_state::MAX_LIVE_CALL_INVITE_AGE_MS].
pub fn purge_stale_pending_call_invites(now_ms: i64) {
    let Ok(mut q) = pending_p2p_events_mx().lock() else {
        return;
    };
    q.retain(|ev| match ev {
        GossipChatEvent::CallSignal {
            signal,
            created_at_ms,
            ..
        } if signal == "invite" => call_state::call_invite_is_live(*created_at_ms, now_ms),
        _ => true,
    });
}

/// Dismiss OS incoming-call alert in `:p2p` / daemon (Linux libnotify, Android full-screen).
pub fn p2p_dismiss_incoming_call_alert() -> Value {
    let _ = call_state::expire_stale_ringing(call_state::now_ms());
    crate::incoming_call_notify::dismiss_incoming_call();
    json_ok(serde_json::json!({ "ok": true }))
}

/// Tear down native voice/video and send hangup when the UI session ends (privacy invariant).
pub fn p2p_force_end_active_call(reason: &str) -> Value {
    native_log::info("call", &format!("force_end_active_call reason={reason}"));
    let had_active = crate::p2p::call_active::snapshot().is_some()
        || call_state::first_incoming_ringing().is_some();

    let out_tx = {
        let g = match p2p_mx().lock() {
            Ok(g) => g,
            Err(_) => {
                crate::p2p::call_active::clear();
                call_state::clear_all_calls();
                crate::incoming_call_notify::dismiss_incoming_call();
                return json_ok(serde_json::json!({
                    "ok": true,
                    "ended": had_active,
                    "reason": reason,
                }));
            }
        };
        g.as_ref().map(|h| h.out_tx.clone())
    };

    if let Some(out_tx) = out_tx {
        let mut hangup_targets = std::collections::HashSet::new();
        if let Some(s) = crate::p2p::call_active::snapshot() {
            hangup_targets.insert((s.peer_public_key_hex.clone(), s.call_id.clone()));
            let _ = out_tx.send(OutboundCmd::CallMediaStop {
                call_id: s.call_id.clone(),
            });
            let _ = out_tx.send(OutboundCmd::CallVideoStop {
                call_id: s.call_id.clone(),
            });
        }
        if let Ok(g) = call_state::store_for_teardown() {
            for (pk, call_id) in g {
                hangup_targets.insert((pk, call_id));
            }
        }
        for (pk, call_id) in hangup_targets {
            drop_pending_call_invite(&call_id);
            let _ = out_tx.send(OutboundCmd::SendCallSignal {
                recipient_public_key_hex: pk,
                call_id,
                signal_kind: CallSigKind::Hangup,
                payload: serde_json::json!({}),
                signal_id: new_msg_id_for_ffi(),
            });
        }
    }

    crate::p2p::call_active::clear();
    call_state::clear_all_calls();
    crate::incoming_call_notify::dismiss_incoming_call();
    json_ok(serde_json::json!({
        "ok": true,
        "ended": had_active,
        "reason": reason,
    }))
}

/// Consume daemon incoming-call wake marker (Linux notification tap → present UI).
pub fn p2p_take_incoming_call_wake() -> Value {
    let wake = crate::daemon::take_incoming_call_wake();
    json_ok(serde_json::json!({ "ok": true, "wake": wake }))
}

/// Consume daemon unlock wake marker (login after reboot → present unlock UI).
pub fn p2p_take_unlock_wake() -> Value {
    let wake = crate::daemon::take_unlock_wake();
    json_ok(serde_json::json!({ "ok": true, "wake": wake }))
}

/// Delivery-server path writes stores directly; enqueue here so `p2p_poll` can emit `stores_updated`.
pub fn enqueue_delivery_gossip_event(ev: GossipChatEvent) {
    if let Ok(mut q) = pending_p2p_events_mx().lock() {
        q.push_back(ev);
    }
}

fn drain_holder_events_into_pending(h: &P2pHolder) {
    let Ok(mut q) = pending_p2p_events_mx().lock() else {
        return;
    };
    while let Ok(ev) = h.events_rx.try_recv() {
        q.push_back(ev);
    }
}

fn poll_next_p2p_event() -> Option<GossipChatEvent> {
    if let Ok(g) = p2p_mx().lock() {
        if let Some(h) = g.as_ref() {
            if let Ok(ev) = h.events_rx.try_recv() {
                return Some(ev);
            }
        }
    }
    pending_p2p_events_mx()
        .lock()
        .ok()
        .and_then(|mut q| q.pop_front())
}

fn clear_p2p_holder() {
    if let Ok(mut g) = p2p_mx().lock() {
        if let Some(h) = g.take() {
            drain_holder_events_into_pending(&h);
        }
    }
}

pub fn p2p_holder_alive() -> bool {
    let Ok(g) = p2p_mx().lock() else {
        return false;
    };
    let Some(h) = g.as_ref() else {
        return false;
    };
    !h.join.is_finished()
}

fn stop_p2p_node(wait: Duration) {
    let join = {
        let Ok(mut g) = p2p_mx().lock() else {
            return;
        };
        let Some(h) = g.take() else {
            return;
        };
        h.stop.store(true, Ordering::SeqCst);
        drop(h.out_tx);
        h.join
    };
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = join.join();
        let _ = done_tx.send(());
    });
    let _ = done_rx.recv_timeout(wait);
}

pub fn gossip_event_json(ev: GossipChatEvent) -> Value {
    match ev {
        GossipChatEvent::Listening(ma) => serde_json::json!({
            "kind": "listening",
            "multiaddr": ma.to_string(),
        }),
        GossipChatEvent::PeerConnected(p) => {
            let pk = identity_wire_for_libp2p_peer(&p).unwrap_or_else(|| p.to_string());
            serde_json::json!({
                "kind": "peer_connected",
                "peer_id": p.to_string(),
                "public_key_hex": pk,
            })
        }
        GossipChatEvent::PeerDisconnected(p) => {
            let pk = identity_wire_for_libp2p_peer(&p).unwrap_or_else(|| p.to_string());
            serde_json::json!({
                "kind": "peer_disconnected",
                "peer_id": p.to_string(),
                "public_key_hex": pk,
            })
        }
        GossipChatEvent::DialFailed { peer, error } => serde_json::json!({
            "kind": "dial_failed",
            "peer": peer.and_then(|p| identity_wire_for_libp2p_peer(&p)),
            "error": error,
        }),
        GossipChatEvent::DmMessage {
            from,
            id,
            msg_kind,
            text,
            ref_id,
            sender_public_key_hex,
            created_at_ms,
            received_at_ms,
        } => {
            let mut j = serde_json::json!({
            "kind": "dm_message",
            "from": from.to_string(),
            "id": id,
            "msg_kind": msg_kind,
            "text": text,
            "ref_id": ref_id,
            "sender_public_key_hex": sender_public_key_hex,
            "created_at_ms": created_at_ms,
        });
            if let Some(at) = received_at_ms.filter(|t| *t > 0) {
                j["received_at_ms"] = serde_json::json!(at);
            }
            j
        }
        GossipChatEvent::PeerIdentified {
            peer_id,
            public_key_hex,
        } => serde_json::json!({
            "kind": "peer_identified",
            "peer_id": peer_id.to_string(),
            "public_key_hex": public_key_hex,
        }),
        GossipChatEvent::ChatReady { peer_id } => {
            let pk = identity_wire_for_libp2p_peer(&peer_id)
                .unwrap_or_else(|| peer_id.to_string());
            serde_json::json!({
                "kind": "chat_ready",
                "peer_id": peer_id.to_string(),
                "public_key_hex": pk,
            })
        }
        GossipChatEvent::SendFailed { message_id, error } => serde_json::json!({
            "kind": "send_failed",
            "message_id": message_id,
            "error": error,
        }),
        GossipChatEvent::OutboundSent { message_id } => serde_json::json!({
            "kind": "outbound_sent",
            "message_id": message_id,
        }),
        GossipChatEvent::NativeLog {
            level,
            tag,
            message,
        } => serde_json::json!({
            "kind": "native_log",
            "level": level,
            "tag": tag,
            "message": message,
        }),
        GossipChatEvent::CallSignal {
            from,
            id,
            call_id,
            signal,
            sender_public_key_hex,
            created_at_ms,
            payload,
        } => serde_json::json!({
            "kind": "call_signal",
            "from": from.to_string(),
            "id": id,
            "call_id": call_id,
            "signal": signal,
            "sender_public_key_hex": sender_public_key_hex,
            "created_at_ms": created_at_ms,
            "payload": payload,
        }),
        GossipChatEvent::CallSignalSent {
            call_id,
            signal,
            recipient_public_key_hex,
        } => serde_json::json!({
            "kind": "call_signal_sent",
            "call_id": call_id,
            "signal": signal,
            "recipient_public_key_hex": recipient_public_key_hex,
        }),
        GossipChatEvent::CallMedia {
            call_id,
            peer_public_key_hex,
            state,
            camera_on,
            remote_video_on,
            reason,
        } => serde_json::json!({
            "kind": "call_media",
            "call_id": call_id,
            "peer_public_key_hex": peer_public_key_hex,
            "state": state,
            "camera_on": camera_on,
            "remote_video_on": remote_video_on,
            "reason": reason,
        }),
        GossipChatEvent::NodeReady => serde_json::json!({ "kind": "node_ready" }),
        GossipChatEvent::NodeStopped { error } => serde_json::json!({
            "kind": "node_stopped",
            "error": error,
        }),
    }
}

fn json_err(msg: impl AsRef<str>) -> Value {
    serde_json::json!({ "ok": false, "error": msg.as_ref() })
}

fn json_ok(v: Value) -> Value {
    v
}

fn recipient_identity_from_config(config: &Value) -> Option<String> {
    config
        .get("recipient_public_key_hex")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| is_valid_public_key_hex(s))
        .map(str::to_string)
}

fn parse_dm_peers(v: &Value) -> Vec<DmPeer> {
    let mut out = Vec::new();
    let Some(arr) = v.get("dm_peers").and_then(|x| x.as_array()) else {
        return out;
    };
    for item in arr {
        let pk = item
            .get("public_key_hex")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| crate::contacts_v1::is_valid_public_key_hex(s));
        if let Some(pk) = pk {
            if let Ok(dm) = DmPeer::from_public_key_hex(pk.to_string()) {
                out.push(dm);
            }
        }
    }
    out
}

fn apply_coord_from_config(config: &Value) {
    let tls = config
        .get("coord_insecure_tls")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let urls = crate::coord_runtime::coord_urls_from_json_value(config);
    if !urls.is_empty() {
        crate::coord_runtime::set_coord_base_urls(&urls, tls);
    }
}

pub fn p2p_dial_bootstrap_peers(addrs: &[DmDialAddr]) -> Value {
    if !p2p_holder_alive() {
        return json_err("p2p not running");
    }
    dial_bootstrap_on_running_node(addrs.to_vec());
    json_ok(serde_json::json!({ "ok": true, "count": addrs.len() }))
}

fn dial_bootstrap_on_running_node(_addrs: Vec<DmDialAddr>) {
    // Native connect uses mDNS + coord bridge — no libp2p bootstrap dials.
}

fn apply_delivery_from_p2p_config(config: &Value) {
    crate::rustls_init::ensure_rustls_crypto_provider();
    match config.get("delivery_url").and_then(|v| v.as_str()) {
        Some(url) if !url.trim().is_empty() => {
            crate::delivery_runtime::set_delivery_url(Some(url));
            if crate::text_transport::wan_text_via_delivery_server() {
                crate::delivery_runtime::delivery_start();
            }
        }
        _ => {
            crate::delivery_runtime::set_delivery_url(None);
            crate::delivery_runtime::delivery_stop();
        }
    }
}

/// Start native connect DM node in a background thread.
pub fn p2p_start(config: &Value) -> Value {
    crate::rustls_init::ensure_rustls_crypto_provider();
    set_drop_pending_call_invite_hook(drop_pending_call_invite);
    // Stale notification-tap marker must not fire presentWindow on next UI login.
    crate::daemon::clear_incoming_call_wake();
    apply_coord_from_config(config);
    let topic = config
        .get("topic")
        .and_then(|t| t.as_str())
        .unwrap_or(DEFAULT_GOSSIP_TOPIC)
        .to_string();
    let peers: Vec<DmDialAddr> = config
        .get("bootstrap_peers")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .filter_map(DmDialAddr::parse)
                .collect()
        })
        .unwrap_or_default();
    let dm_peers = parse_dm_peers(config);

    let ident = match unlocked_identity_clone() {
        Ok(i) => i,
        Err(e) => return json_err(e),
    };
    if !ident.p2p_ready() {
        return json_err("identity cannot start p2p on this build");
    }

    let mut gossip_cfg = match GossipChatConfig::from_unlocked_identity(topic, &ident) {
        Ok(c) => c,
        Err(e) => return json_err(e),
    };
    let bootstrap_for_hot_dial = peers.clone();
    gossip_cfg.dm_peers = dm_peers;
    gossip_cfg.transcript_path = config
        .get("transcript_path")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    gossip_cfg.app_namespace = config
        .get("app_namespace")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if p2p_holder_alive() {
        native_log::info(
            "p2p",
            format!(
                "p2p_start already_running — refresh handler + dm_peers={} ns={}",
                gossip_cfg.dm_peers.len(),
                gossip_cfg.app_namespace.as_deref().unwrap_or("(none)")
            ),
        );
        if let Some(ns) = gossip_cfg.app_namespace.as_deref() {
            set_p2p_handler_context(ns);
            crate::incoming_call_notify::set_desktop_app_id(ns);
        }
        for dm in &gossip_cfg.dm_peers {
            let pk = dm.identity_wire.trim();
            if !crate::contacts_v1::is_valid_public_key_hex(pk) {
                continue;
            }
            let _ = p2p_register_dm_peer(pk);
        }
        dial_bootstrap_on_running_node(bootstrap_for_hot_dial);
        if crate::coord_runtime::coord_is_configured() {
            crate::p2p::notify_relay_refresh();
        }
        apply_delivery_from_p2p_config(config);
        return json_ok(serde_json::json!({ "ok": true, "already_running": true }));
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_t = Arc::clone(&stop);
    let (out_tx, out_rx) = std::sync::mpsc::channel::<OutboundCmd>();
    let (ev_tx, ev_rx) = std::sync::mpsc::channel::<GossipChatEvent>();
    let ev_tx_log = ev_tx.clone();
    native_log::set_sink(Some(Box::new(move |line| {
        if !native_log_should_forward_to_ui(&line) {
            return;
        }
        let _ = ev_tx_log.send(GossipChatEvent::NativeLog {
            level: line.level.to_string(),
            tag: line.tag,
            message: line.message,
        });
    })));
    native_log::info(
        "p2p",
        format!(
            "node starting identity={} dm_peers={}",
            ident.identity_wire(),
            gossip_cfg.dm_peers.len(),
        ),
    );

    if let Some(ns) = gossip_cfg.app_namespace.as_deref() {
        set_p2p_handler_context(ns);
        crate::incoming_call_notify::set_desktop_app_id(ns);
    }

    clear_pending_p2p_events();
    apply_delivery_from_p2p_config(config);
    {
        let contacts: Vec<String> = gossip_cfg
            .dm_peers
            .iter()
            .map(|p| p.identity_wire.clone())
            .collect();
        if let Err(e) = crate::connect::connect_start(&ident.identity_wire(), &contacts) {
            native_log::warn("connect", format!("connect_start failed: {e}"));
        }
    }
    let join = std::thread::spawn(move || {
        let default_panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            native_log::error("p2p", format!("panic: {info}"));
            default_panic_hook(info);
        }));
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let msg = format!("tokio runtime build failed: {e}");
                native_log::error("p2p", msg.clone());
                let _ = ev_tx.send(GossipChatEvent::NodeStopped { error: Some(msg) });
                native_log::set_sink(None);
                clear_p2p_holder();
                return;
            }
        };
        let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.block_on(crate::connect::run_connect_node_with_std_io(
                gossip_cfg,
                ident,
                out_rx,
                ev_tx.clone(),
                stop_t,
            ))
        }));
        native_log::set_sink(None);
        let stop_err = match run_result {
            Ok(Ok(())) => {
                native_log::info("p2p", "node stopped cleanly");
                None
            }
            Ok(Err(e)) => {
                let msg = format!("{e}");
                native_log::error("p2p", format!("node ended: {msg}"));
                Some(msg)
            }
            Err(panic) => {
                let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                    format!("panic: {s}")
                } else if let Some(s) = panic.downcast_ref::<String>() {
                    format!("panic: {s}")
                } else {
                    "panic in p2p worker".to_string()
                };
                native_log::error("p2p", msg.clone());
                Some(msg)
            }
        };
        let _ = ev_tx.send(GossipChatEvent::NodeStopped { error: stop_err });
        clear_p2p_holder();
    });

    let holder = P2pHolder {
        out_tx,
        stop,
        join,
        events_rx: ev_rx,
    };

    match p2p_mx().lock() {
        Ok(mut g) => {
            if g.is_some() {
                drop(g);
                stop_p2p_node(Duration::from_secs(3));
                let Ok(mut g2) = p2p_mx().lock() else {
                    return json_err("p2p mutex poisoned");
                };
                if g2.is_some() {
                    return json_err("p2p still stopping (try again)");
                }
                *g2 = Some(holder);
            } else {
                *g = Some(holder);
            }
        }
        Err(_) => return json_err("p2p mutex poisoned"),
    }

    json_ok(serde_json::json!({ "ok": true }))
}

pub fn p2p_is_running() -> Value {
    json_ok(serde_json::json!({ "ok": true, "running": p2p_holder_alive() }))
}

pub fn p2p_stop() {
    native_log::info("p2p", "p2p_stop requested");
    crate::connect::connect_stop();
    crate::delivery_runtime::delivery_stop();
    clear_ui_session_snapshot();
    crate::coord_runtime::stop_coord_presence();
    crate::p2p::set_app_ack_read_enabled(false);
    stop_p2p_node(Duration::from_secs(3));
    call_state::clear_all_calls();
    crate::incoming_call_notify::dismiss_incoming_call();
    clear_p2p_handler_context();
    native_log::set_sink(None);
}

/// Voice-call signaling. JSON:
/// `{ "recipient_public_key_hex": "<identity wire>", "call_id": "...", "signal": "invite|...", "payload": {} }`
pub fn p2p_call_signal(config: &Value) -> Value {
    let recipient = match recipient_identity_from_config(config) {
        Some(s) => s,
        None => return json_err("recipient_public_key_hex required (valid identity wire)"),
    };
    let call_id = match config
        .get("call_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => return json_err("call_id required"),
    };
    let signal_s = match config.get("signal").and_then(|x| x.as_str()) {
        Some(s) => s,
        None => return json_err("signal required"),
    };
    let signal_kind = match CallSigKind::parse_wire(signal_s) {
        Ok(k) => k,
        Err(e) => return json_err(e),
    };
    let payload = config
        .get("payload")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let signal_id = config
        .get("signal_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(new_msg_id_for_ffi);

    let out_tx = {
        let g = match p2p_mx().lock() {
            Ok(g) => g,
            Err(_) => return json_err("p2p mutex poisoned"),
        };
        let Some(h) = g.as_ref() else {
            return json_err("p2p not running");
        };
        h.out_tx.clone()
    };
    if out_tx
        .send(OutboundCmd::SendCallSignal {
            recipient_public_key_hex: recipient,
            call_id: call_id.clone(),
            signal_kind,
            payload,
            signal_id: signal_id.clone(),
        })
        .is_err()
    {
        return json_err("p2p send failed (node stopped?)");
    }
    json_ok(serde_json::json!({
        "ok": true,
        "call_id": call_id,
        "signal_id": signal_id,
        "queued": true,
    }))
}

/// Native voice **media** control plane. One entrypoint, `action`-dispatched, so
/// the FFI/RPC surface stays small. See `docs/GHAL_BOL_CALL_NATIVE_V2.md`.
///
/// - `{"action":"start","call_id":..,"recipient_public_key_hex":..}` — open media.
/// - `{"action":"stop","call_id":..}` — tear down media.
/// - `{"action":"set_mic_muted","call_id":..,"muted":bool}` — mute/unmute mic.
/// - `{"action":"set_speaker","call_id":..,"speaker_on":bool}` — Android speakerphone.
pub fn p2p_call_media(config: &Value) -> Value {
    let action = config
        .get("action")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .unwrap_or("");
    let call_id = match config
        .get("call_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => return json_err("call_id required"),
    };

    let cmd = match action {
        "start" => {
            let recipient = match recipient_identity_from_config(config) {
                Some(s) => s,
                None => return json_err("recipient_public_key_hex required (valid identity wire)"),
            };
            OutboundCmd::CallMediaStart {
                call_id: call_id.clone(),
                peer_public_key_hex: recipient,
            }
        }
        "stop" => OutboundCmd::CallMediaStop {
            call_id: call_id.clone(),
        },
        "set_mic_muted" => {
            let muted = config
                .get("muted")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            OutboundCmd::CallMediaSetMicMuted {
                call_id: call_id.clone(),
                muted,
            }
        }
        "set_speaker" => {
            let speaker_on = config
                .get("speaker_on")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            OutboundCmd::CallMediaSetSpeaker {
                call_id: call_id.clone(),
                speaker_on,
            }
        }
        other => return json_err(format!("unknown call media action: {other}")),
    };

    let out_tx = {
        let g = match p2p_mx().lock() {
            Ok(g) => g,
            Err(_) => return json_err("p2p mutex poisoned"),
        };
        let Some(h) = g.as_ref() else {
            return json_err("p2p not running");
        };
        h.out_tx.clone()
    };
    if out_tx.send(cmd).is_err() {
        return json_err("p2p send failed (node stopped?)");
    }
    json_ok(serde_json::json!({
        "ok": true,
        "call_id": call_id,
        "action": action,
        "queued": true,
    }))
}

/// Read-only transcript merge — same process as `:p2p` poll writes (avoids UI FFI file races).
pub fn p2p_transcript_load_merged(config: &Value) -> Value {
    let ns = config
        .get("app_namespace")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(ns) = ns else {
        return json_err("app_namespace required");
    };
    let keys: Vec<String> = config
        .get("conversation_keys")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if keys.is_empty() {
        return json_err("conversation_keys required");
    }
    let from_peer = config
        .get("match_inbound_from_peer_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match crate::dm_transcript_store::thread_view(ns, &keys, from_peer) {
        Ok(view) => json_ok(serde_json::json!({
            "ok": true,
            "revision": view.revision,
            "lines": view.lines.iter().map(|l| l.to_json()).collect::<Vec<_>>(),
        })),
        Err(e) => json_err(format!("{e}")),
    }
}

/// Snapshot of native call signaling/media (UI re-sync when the app was backgrounded or killed).
pub fn p2p_call_status(_config: &Value) -> Value {
    let now = call_state::now_ms();
    if call_state::expire_stale_ringing(now) {
        crate::incoming_call_notify::dismiss_incoming_call();
    }
    if let Some(s) = crate::p2p::call_active::snapshot() {
        return json_ok(serde_json::json!({
            "ok": true,
            "active": true,
            "call_id": s.call_id,
            "peer_public_key_hex": s.peer_public_key_hex,
            "voice_active": s.voice_active,
            "video_active": s.video_active,
            "camera_on": s.camera_on,
            "remote_video_on": s.remote_video_on,
        }));
    }
    if let Some((pk, call_id)) = crate::call_state::first_incoming_ringing() {
        let ring_age_ms = call_state::incoming_ring_age_ms(now).unwrap_or(0);
        return json_ok(serde_json::json!({
            "ok": true,
            "active": false,
            "ringing": true,
            "phase": "incoming_ringing",
            "call_id": call_id,
            "peer_public_key_hex": pk,
            "ring_age_ms": ring_age_ms,
        }));
    }
    json_ok(serde_json::json!({
        "ok": true,
        "active": false,
    }))
}

/// Native **video** control plane (parallel to [`p2p_call_media`]).
///
/// - `{"action":"start","call_id":..,"recipient_public_key_hex":..}` — open video.
/// - `{"action":"stop","call_id":..}` — tear down video.
/// - `{"action":"set_camera_enabled","call_id":..,"enabled":bool}` — camera on/off.
pub fn p2p_call_video(config: &Value) -> Value {
    let action = config
        .get("action")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .unwrap_or("");
    let call_id = match config
        .get("call_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => return json_err("call_id required"),
    };

    let cmd = match action {
        "start" => {
            let recipient = match recipient_identity_from_config(config) {
                Some(s) => s,
                None => return json_err("recipient_public_key_hex required (valid identity wire)"),
            };
            let camera_enabled = config
                .get("camera_enabled")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            OutboundCmd::CallVideoStart {
                call_id: call_id.clone(),
                peer_public_key_hex: recipient,
                camera_enabled,
            }
        }
        "stop" => OutboundCmd::CallVideoStop {
            call_id: call_id.clone(),
        },
        "set_camera_enabled" => {
            let enabled = config
                .get("enabled")
                .and_then(|x| x.as_bool())
                .unwrap_or(true);
            OutboundCmd::CallVideoSetCameraEnabled {
                call_id: call_id.clone(),
                enabled,
            }
        }
        "capture_backend" => {
            return json_ok(serde_json::json!({
                "ok": true,
                "backend": crate::call_video::desktop_capture_backend(),
            }));
        }
        other => return json_err(format!("unknown call video action: {other}")),
    };

    let out_tx = {
        let g = match p2p_mx().lock() {
            Ok(g) => g,
            Err(_) => return json_err("p2p mutex poisoned"),
        };
        let Some(h) = g.as_ref() else {
            return json_err("p2p not running");
        };
        h.out_tx.clone()
    };
    if out_tx.send(cmd).is_err() {
        return json_err("p2p send failed (node stopped?)");
    }
    json_ok(serde_json::json!({
        "ok": true,
        "call_id": call_id,
        "action": action,
        "queued": true,
    }))
}

/// Push one I420 camera frame from the Flutter UI into the desktop video engine.
/// Used on Linux/macOS/Windows where the daemon cannot open the webcam directly.
pub fn p2p_call_video_push_camera_frame(config: &Value) -> Value {
    use base64::Engine as _;
    let call_id = config
        .get("call_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let width = config.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    let height = config.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    let b64 = match config.get("data_base64").and_then(|x| x.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return json_err("data_base64 required"),
    };
    let data = match base64::engine::general_purpose::STANDARD.decode(b64) {
        Ok(d) => d,
        Err(e) => return json_err(format!("data_base64 decode: {e}")),
    };
    // `format`: `i420` (default, already planar) or packed `rgba`/`bgra` straight from
    // the camera — packed is converted to I420 natively (no Dart per-pixel loop).
    let format = config
        .get("format")
        .and_then(|x| x.as_str())
        .unwrap_or("i420")
        .to_ascii_lowercase();
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        let frame = match format.as_str() {
            "rgba" | "bgra" => {
                let stride = config
                    .get("stride")
                    .and_then(|x| x.as_u64())
                    .map(|s| s as usize)
                    .unwrap_or((width as usize) * 4);
                match crate::call_video::packed_to_i420(
                    &data,
                    stride,
                    width,
                    height,
                    format == "rgba",
                ) {
                    Some(f) => f,
                    None => return json_err("packed frame: bad dimensions/stride"),
                }
            }
            _ => crate::call_video::RawVideoFrame {
                width,
                height,
                data,
            },
        };
        crate::call_video::push_camera_frame(frame);
        return json_ok(serde_json::json!({
            "ok": true,
            "call_id": call_id,
            "accepted": true,
        }));
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (call_id, width, height, data, format);
        json_err("push camera frame only supported on desktop")
    }
}

/// Texture registration: shm path + display dimensions for GPU render (no pixels in JSON).
pub fn p2p_call_video_texture(config: &Value) -> Value {
    let call_id = match config
        .get("call_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => return json_err("call_id required"),
    };
    let track = config
        .get("track")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .unwrap_or("remote");
    match crate::call_video::texture_shm_info(&call_id, track) {
        Some(info) => json_ok(serde_json::json!({
            "ok": true,
            "ready": true,
            "shm_path": info.path,
            "width": info.width,
            "height": info.height,
            "generation": info.generation,
        })),
        None => json_ok(serde_json::json!({
            "ok": true,
            "ready": false,
        })),
    }
}

/// Render pull: latest decoded frame for `call_id` if newer than `since_generation`.
/// Returns the frame as base64 I420 plus its dimensions and monotonic `generation`
/// (the UI passes the last `generation` back so duplicates are skipped). I420 keeps
/// the payload ~⅔ the size of RGB and matches the camera/codec format.
pub fn p2p_call_video_frame(config: &Value) -> Value {
    use base64::Engine as _;
    let call_id = match config
        .get("call_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => return json_err("call_id required"),
    };
    let since = config
        .get("since_generation")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let track = config
        .get("track")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .unwrap_or("remote");
    // `rgba` (default): native I420→RGBA conversion so the Flutter UI isolate does no
    // per-pixel work — it feeds the bytes straight to `decodeImageFromPixels`. `i420`
    // keeps the raw planar payload for callers that convert themselves.
    let want_rgba = config
        .get("format")
        .and_then(|x| x.as_str())
        .map(|f| f.eq_ignore_ascii_case("rgba"))
        .unwrap_or(true);
    // Downscale display pulls (default 360 px longest edge) — full-res encode/send
    // is unchanged; this only shrinks the UI poll payload (~4× less base64/decode).
    let max_edge = config
        .get("max_edge")
        .and_then(|x| x.as_u64())
        .unwrap_or(360) as u32;
    let pulled = match track {
        "local" => crate::call_video::latest_local_preview(&call_id, since),
        _ => crate::call_video::latest_decoded_frame(&call_id, since),
    };
    match pulled {
        Some((frame, generation)) => {
            let (format, bytes, out_w, out_h) = if want_rgba {
                let (rgba, w, h) = crate::call_video::i420_to_rgba_max_edge(&frame, max_edge);
                ("rgba", rgba, w, h)
            } else {
                ("i420", frame.data.clone(), frame.width, frame.height)
            };
            json_ok(serde_json::json!({
                "ok": true,
                "has_frame": true,
                "width": out_w,
                "height": out_h,
                "generation": generation,
                "format": format,
                "data_base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
            }))
        }
        None => json_ok(serde_json::json!({ "ok": true, "has_frame": false })),
    }
}

/// Look up existing timestamp for outbound message (Flutter saves before calling Rust).
fn existing_outbound_created_at_ms(recipient: &str, message_id: &str) -> Option<i64> {
    if let Some(ns) = active_app_namespace() {
        if let Ok(rows) = crate::dm_transcript_store::load_merged(&ns, &[recipient.to_string()], None) {
            for row in rows {
                if row.outgoing && row.message_id.as_deref() == Some(message_id) {
                    return row.created_at_ms;
                }
            }
        }
    }
    None
}

fn enqueue_send_text_dm(
    message_id: String,
    recipient: String,
    text: String,
    wait_for_wire: bool,
) -> Value {
    let recipient_trim = recipient.trim().to_lowercase();
    if let Ok(ident) = unlocked_identity_clone() {
        let my_pk = ident.public_key_hex().trim().to_lowercase();
        if recipient_trim == my_pk {
            native_log::warn(
                "outbound",
                format!("send_text rejected: recipient is own identity msg_id={message_id}"),
            );
            return json_err("cannot send DM to your own identity (scan the other device's QR)");
        }
    }
    let out_tx = {
        let g = match p2p_mx().lock() {
            Ok(g) => g,
            Err(_) => return json_err("p2p mutex poisoned"),
        };
        let Some(h) = g.as_ref() else {
            return json_err("p2p not running");
        };
        h.out_tx.clone()
    };
    // Preserve existing timestamp if Flutter already saved the message to transcript.
    // This ensures timestamps are immutable - once set, never changed.
    let created_at_ms = existing_outbound_created_at_ms(&recipient_trim, &message_id).unwrap_or_else(now_ms);
    if wait_for_wire {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        if out_tx
            .send(OutboundCmd::SendText {
                recipient_public_key_hex: recipient,
                text,
                message_id: message_id.clone(),
                created_at_ms,
                done: Some(done_tx),
            })
            .is_err()
        {
            return json_err("p2p send failed (node stopped?)");
        }
        return match done_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => json_ok(serde_json::json!({ "ok": true, "message_id": message_id })),
            Ok(Err(e)) => json_ok(serde_json::json!({
                "ok": true,
                "message_id": message_id,
                "queued": true,
                "detail": e,
            })),
            Err(_) => json_ok(serde_json::json!({
                "ok": true,
                "message_id": message_id,
                "queued": true,
                "detail": "send pending (outbox will retry)",
            })),
        };
    }
    let recipient_log = recipient.trim();
    let pk_short = if recipient_log.len() >= 16 {
        format!(
            "{}…{}",
            &recipient_log[..8],
            &recipient_log[recipient_log.len() - 8..]
        )
    } else {
        recipient_log.to_string()
    };
    native_log::info(
        "outbound",
        format!(
            "enqueue send_text msg_id={message_id} recipient_pk={pk_short} text_len={}",
            text.len()
        ),
    );
    if out_tx
        .send(OutboundCmd::SendText {
            recipient_public_key_hex: recipient,
            text,
            message_id: message_id.clone(),
            created_at_ms,
            done: None,
        })
        .is_err()
    {
        native_log::warn(
            "outbound",
            format!("send_text failed: node stopped? msg_id={message_id}"),
        );
        return json_err("p2p send failed (node stopped?)");
    }
    json_ok(serde_json::json!({
        "ok": true,
        "message_id": message_id,
        "queued": true,
    }))
}

fn maybe_mirror_text_on_lan_fast_path(
    message_id: &str,
    recipient: &str,
    text: &str,
) {
    if !crate::text_transport::lan_fast_path_enabled() {
        return;
    }
    if !crate::p2p::contact_has_lan_p2p_text_path(recipient) {
        return;
    }
    let _ = enqueue_send_text_dm(
        message_id.to_string(),
        recipient.to_string(),
        text.to_string(),
        false,
    );
}

pub fn p2p_send_text_dm(recipient: &str, text: &str) -> Value {
    let message_id = new_msg_id_for_ffi();
    if crate::text_transport::delivery_primary_text() {
        let result =
            crate::delivery_runtime::delivery_send_text_dm(recipient, text, &message_id);
        maybe_mirror_text_on_lan_fast_path(&message_id, recipient, text);
        return result;
    }
    if !crate::p2p::contact_has_lan_p2p_text_path(recipient) {
        return json_err(
            "GHAL_BOL_DELIVERY_URL not set — WAN text requires delivery server; peer not on LAN",
        );
    }
    enqueue_send_text_dm(message_id, recipient.to_string(), text.to_string(), false)
}

pub fn p2p_requeue_outbound_dm(message_id: &str, recipient: &str, text: &str) -> Value {
    if message_id.trim().is_empty() {
        return json_err("message_id required");
    }
    if crate::text_transport::delivery_primary_text() {
        let mid = message_id.trim();
        let result = crate::delivery_runtime::delivery_send_text_dm(recipient, text, mid);
        maybe_mirror_text_on_lan_fast_path(mid, recipient, text);
        return result;
    }
    if !crate::p2p::contact_has_lan_p2p_text_path(recipient) {
        return json_err(
            "GHAL_BOL_DELIVERY_URL not set — WAN text requires delivery server; peer not on LAN",
        );
    }
    enqueue_send_text_dm(
        message_id.trim().to_string(),
        recipient.to_string(),
        text.to_string(),
        false,
    )
}

pub fn p2p_send_ack_dm(recipient: &str, ref_id: &str, ack_kind: &str) -> Value {
    let ack_kind = match ack_kind {
        "ack_received" => MsgKind::AckReceived,
        "ack_read" => MsgKind::AckRead,
        "ack_request" => MsgKind::AckRequest,
        other => return json_err(format!("unknown ack kind: {other}")),
    };
    let out_tx = {
        let g = match p2p_mx().lock() {
            Ok(g) => g,
            Err(_) => return json_err("p2p mutex poisoned"),
        };
        let Some(h) = g.as_ref() else {
            return json_err("p2p not running");
        };
        h.out_tx.clone()
    };
    if out_tx
        .send(OutboundCmd::SendAck {
            recipient_public_key_hex: recipient.to_string(),
            ref_id: ref_id.to_string(),
            ack_kind,
        })
        .is_err()
    {
        return json_err("p2p send failed (node stopped?)");
    }
    json_ok(serde_json::json!({ "ok": true }))
}

pub fn p2p_register_dm_peer(public_key_hex: &str) -> Value {
    let out_tx = {
        let g = match p2p_mx().lock() {
            Ok(g) => g,
            Err(_) => return json_err("p2p mutex poisoned"),
        };
        let Some(h) = g.as_ref() else {
            return json_err("p2p not running");
        };
        h.out_tx.clone()
    };
    let pk_trim = public_key_hex.trim();
    // UI can call this repeatedly while processing lots of inbound events / transcript patches.
    // That burst causes repeated coord lookups + dial churn and can destabilize the DM stream.
    // Keep it idempotent: enqueue at most once per peer per short window.
    let now = now_ms();
    if is_valid_public_key_hex(pk_trim) {
        if let Ok(mut m) = register_dm_peer_throttle_mx().write() {
            if let Some(last) = m.get(pk_trim).copied() {
                if now.saturating_sub(last) < 1_200 {
                    return json_ok(serde_json::json!({ "ok": true, "throttled": true }));
                }
            }
            m.insert(pk_trim.to_string(), now);
        }
    }
    let pk_short = if pk_trim.len() >= 16 {
        format!("{}…{}", &pk_trim[..8], &pk_trim[pk_trim.len() - 8..])
    } else {
        pk_trim.to_string()
    };
    native_log::info("session", format!("register_dm_peer pk={pk_short}"));
    if out_tx
        .send(OutboundCmd::RegisterDmPeer {
            public_key_hex: public_key_hex.to_string(),
        })
        .is_err()
    {
        native_log::warn("session", "register_dm_peer failed: node stopped?");
        return json_err("p2p register failed (node stopped?)");
    }
    json_ok(serde_json::json!({ "ok": true }))
}

pub fn p2p_set_app_ack_read_enabled(enabled: bool) -> Value {
    native_log::info("session", format!("app_ack_read_enabled={enabled}"));
    crate::p2p::set_app_ack_read_enabled(enabled);
    if enabled {
        if let Some(peer) = crate::p2p::live_foreground_peer_for_catchup() {
            if crate::text_transport::wan_text_via_delivery_server() {
                crate::delivery_read_acks::queue_delivery_read_catchup(&peer);
            }
            if crate::text_transport::lan_p2p_ack_mirror_enabled(&peer) {
                if let Ok(g) = p2p_mx().lock() {
                    if let Some(h) = g.as_ref() {
                        queue_read_ack_catchup(&h.out_tx, peer);
                    }
                }
            }
        }
    }
    json_ok(serde_json::json!({ "ok": true, "enabled": enabled }))
}

pub fn p2p_set_app_ui_visible(visible: bool) -> Value {
    native_log::info("session", format!("app_ui_visible={visible}"));
    crate::p2p::set_app_ui_visible(visible);
    json_ok(serde_json::json!({ "ok": true, "visible": visible }))
}

/// Atomically sync integrator UI state → native read-receipt policy.
///
/// The coord server and relay never see this; only `ghal_bol` uses it to decide when
/// `ack_read` is allowed for **new** inbound mail. Leave backlog drain still runs via
/// `SetForegroundPeer(null)` when [room_public_key_hex] is cleared.
///
/// Close order: room `None` → foreground leave drain → read gate off.
/// Open order: ui visible + room set → read gate on → foreground peer (enter catch-up).
pub fn p2p_sync_ui_session(ui_visible: bool, room_public_key_hex: Option<&str>) -> Value {
    let room = room_public_key_hex
        .map(str::trim)
        .filter(|s| is_valid_public_key_hex(s));
    if ui_session_snapshot_unchanged(ui_visible, room) {
        return json_ok(serde_json::json!({
            "ok": true,
            "unchanged": true,
            "ui_visible": ui_visible,
            "room": room,
            "read_receipts": ui_visible && room.is_some(),
        }));
    }
    record_ui_session_snapshot(ui_visible, room);
    native_log::info(
        "session",
        format!(
            "sync_ui_session ui_visible={ui_visible} room={}",
            room.map(|_| "<pk>").unwrap_or("(none)")
        ),
    );
    crate::p2p::set_app_ui_visible(ui_visible);
    if room.is_none() {
        let r = p2p_set_foreground_peer(None);
        if r.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            return r;
        }
        crate::p2p::set_app_ack_read_enabled(false);
        return json_ok(serde_json::json!({
            "ok": true,
            "ui_visible": ui_visible,
            "room": null,
            "read_receipts": false,
        }));
    }
    if ui_visible {
        crate::p2p::set_app_ack_read_enabled(true);
        let r = p2p_set_foreground_peer(room);
        // Android inactive keeps foreground pk but turns read gate off. On resumed the room is
        // unchanged so SetForegroundPeer is skipped — still seed + drain ack_read backlog.
        queue_read_catchup_for_room(room);
        if r.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            return json_ok(serde_json::json!({
                "ok": true,
                "ui_visible": true,
                "room": room,
                "read_receipts": true,
            }));
        }
        return r;
    }
    // Room still open in UI stack but app not interactive (Android inactive, etc.).
    crate::p2p::set_app_ack_read_enabled(false);
    json_ok(serde_json::json!({
        "ok": true,
        "ui_visible": false,
        "room": room,
        "read_receipts": false,
    }))
}

/// Linux read-gate nudge — re-run in-room `ack_read` catch-up without re-issuing `SetForegroundPeer`.
pub fn p2p_nudge_read_catchup() -> Value {
    if let Some(peer) = live_foreground_peer_for_catchup() {
        if crate::text_transport::wan_text_via_delivery_server() {
            crate::delivery_read_acks::queue_delivery_read_catchup(&peer);
        }
        if crate::text_transport::lan_p2p_ack_mirror_enabled(&peer) {
            if let Ok(g) = p2p_mx().lock() {
                if let Some(h) = g.as_ref() {
                    queue_read_ack_catchup(&h.out_tx, peer);
                }
            }
        }
    }
    json_ok(serde_json::json!({ "ok": true }))
}

fn ui_session_snapshot_mx() -> &'static Mutex<Option<(bool, Option<String>)>> {
    static S: OnceLock<Mutex<Option<(bool, Option<String>)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn ui_session_snapshot_unchanged(ui_visible: bool, room: Option<&str>) -> bool {
    let Ok(g) = ui_session_snapshot_mx().lock() else {
        return false;
    };
    let Some((was_visible, was_room)) = g.as_ref() else {
        return false;
    };
    *was_visible == ui_visible && was_room.as_deref() == room
}

fn record_ui_session_snapshot(ui_visible: bool, room: Option<&str>) {
    if let Ok(mut g) = ui_session_snapshot_mx().lock() {
        *g = Some((ui_visible, room.map(str::to_string)));
    }
}

fn clear_ui_session_snapshot() {
    if let Ok(mut g) = ui_session_snapshot_mx().lock() {
        *g = None;
    }
}

/// DM + libp2p connectivity — forward to Flutter App log (`Native/…` tags in export).
fn native_log_should_forward_to_ui(line: &native_log::NativeLogLine) -> bool {
    if line.level == "warn" || line.level == "error" {
        return true;
    }
    let tag = line.tag.as_str();
    let connectivity = matches!(
        tag,
        "flow"
            | "net"
            | "p2p"
            | "swarm"
            | "kad"
            | "listen"
            | "relay"
            | "coord"
            | "mdns"
            | "dial"
            | "autonat"
            | "dcutr"
            | "upnp"
            | "stream"
    );
    if connectivity {
        return line.level == "info" || (line.level == "debug" && native_log::verbose_enabled());
    }
    let dm_flow = matches!(
        tag,
        "DM/store"
            | "Contacts"
            | "Transcript"
            | "outbound"
            | "outbox"
            | "delivery_ack"
            | "delivery"
            | "read_ack"
            | "session"
            | "Storage"
    );
    if dm_flow && (line.level == "info" || line.level == "debug") {
        return true;
    }
    false
}

fn queue_read_catchup_for_room(room: Option<&str>) {
    let Some(peer_pk) = room
        .map(str::trim)
        .filter(|s| is_valid_public_key_hex(s))
        .map(str::to_string)
    else {
        return;
    };
    if crate::text_transport::wan_text_via_delivery_server() {
        crate::delivery_read_acks::queue_delivery_read_catchup(&peer_pk);
    }
    if crate::text_transport::lan_p2p_ack_mirror_enabled(&peer_pk) {
        if let Ok(g) = p2p_mx().lock() {
            if let Some(h) = g.as_ref() {
                queue_read_ack_catchup(&h.out_tx, peer_pk);
            }
        }
    }
}

fn foreground_room_unchanged(public_key_hex: Option<&str>) -> bool {
    let want = public_key_hex
        .map(str::trim)
        .filter(|s| is_valid_public_key_hex(s))
        .map(str::to_lowercase);
    match (want, live_foreground_peer_for_catchup()) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b.to_ascii_lowercase(),
        _ => false,
    }
}

pub fn p2p_set_foreground_peer(public_key_hex: Option<&str>) -> Value {
    if foreground_room_unchanged(public_key_hex) {
        return json_ok(serde_json::json!({ "ok": true, "unchanged": true }));
    }
    let label = public_key_hex
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("(none)");
    native_log::info("session", format!("set_foreground_peer request pk={label}"));
    let out_tx = {
        let g = match p2p_mx().lock() {
            Ok(g) => g,
            Err(_) => return json_err("p2p mutex poisoned"),
        };
        let Some(h) = g.as_ref() else {
            return json_err("p2p not running");
        };
        h.out_tx.clone()
    };
    let pk = public_key_hex
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let prev = live_foreground_peer_for_catchup();
    sync_foreground_peer_now(pk.clone());
    if let Some(ref new_pk) = pk {
        let changed = prev.as_deref().map(str::to_ascii_lowercase)
            != Some(new_pk.to_ascii_lowercase());
        if changed {
            if let Some(ns) = active_app_namespace() {
                let _ = clear_unread(&ns, new_pk);
            }
        }
    }
    let prev_pk = live_foreground_peer_for_catchup();
    let generation = crate::p2p::bump_foreground_peer_cmd_gen();
    if out_tx
        .send(OutboundCmd::SetForegroundPeer {
            identity_wire: pk.clone(),
            generation,
        })
        .is_err()
    {
        return json_err("p2p send failed (node stopped?)");
    }
    if crate::text_transport::wan_text_via_delivery_server() {
        let leaving = match (&prev_pk, &pk) {
            (Some(_prev), None) => true,
            (Some(prev), Some(new)) => !prev.eq_ignore_ascii_case(new),
            _ => false,
        };
        if leaving {
            if let Some(left) = prev_pk.as_ref() {
                crate::delivery_read_acks::dispatch_delivery_leave_drain(left);
            }
        }
        if let Some(new_pk) = pk.as_ref() {
            crate::delivery_read_acks::queue_delivery_read_catchup(new_pk);
        }
    }
    json_ok(serde_json::json!({ "ok": true }))
}

fn enrich_transcript_poll_fields(j: &mut Value) {
    let Some(ns) = crate::dm_event_handler::active_app_namespace() else {
        return;
    };
    let Some(view_key) = crate::dm_event_handler::transcript_poll_view_key(&ns, j) else {
        return;
    };
    let rev = crate::dm_transcript_store::thread_revision_for_view(&ns, &view_key);
    if let Some(obj) = j.as_object_mut() {
        obj.insert("conversation_key".to_string(), Value::String(view_key));
        obj.insert("transcript_revision".to_string(), Value::Number(rev.into()));
    }
}

pub fn p2p_poll_event() -> Option<Value> {
    maybe_maintain_poll_queue(call_state::now_ms());
    let now = call_state::now_ms();
    loop {
        let Some(ev) = poll_next_p2p_event() else {
            return None;
        };
        if let GossipChatEvent::Listening(addr) = &ev {
            if let Some((host, port)) = addr.rsplit_once(':') {
                if let Ok(p) = port.parse::<u16>() {
                    crate::coord_runtime::on_listen_dm_addr(&DmDialAddr::new(
                        host.to_string(),
                        p,
                    ));
                }
            }
        }
        let mut j = gossip_event_json(ev.clone());
        let stores_updated = apply_p2p_event_json(&j);
        if stores_updated {
            if let Some(obj) = j.as_object_mut() {
                obj.insert("stores_updated".to_string(), Value::Bool(true));
            }
            enrich_transcript_poll_fields(&mut j);
            if let Some(kind) = j.get("kind").and_then(|v| v.as_str()) {
                native_log::info("DM/store", format!("stores_updated after {kind}"));
            }
            return Some(j);
        }
        let Some(kind) = j.get("kind").and_then(|v| v.as_str()) else {
            return Some(j);
        };
        if kind == "dm_message" {
            let mk = j
                .get("msg_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if mk == "ack_received" || mk == "ack_read" {
                if stores_updated {
                    if let Some(obj) = j.as_object_mut() {
                        obj.insert("stores_updated".to_string(), Value::Bool(true));
                    }
                    enrich_transcript_poll_fields(&mut j);
                    return Some(j);
                }
                // Delivery worker may have patched transcript before this poll replay.
                if crate::text_transport::wan_text_via_delivery_server() {
                    if let Some(obj) = j.as_object_mut() {
                        obj.insert("stores_updated".to_string(), Value::Bool(true));
                    }
                    enrich_transcript_poll_fields(&mut j);
                    return Some(j);
                }
                continue;
            }
            if mk == "text" {
                // Wire path persists before this poll event; apply is often a no-op replay but
                // contacts on disk already have the new unread — UI must reload the roster.
                if let Some(obj) = j.as_object_mut() {
                    obj.insert("stores_updated".to_string(), Value::Bool(true));
                }
                enrich_transcript_poll_fields(&mut j);
                return Some(j);
            }
        }
        if kind == "peer_identified" {
            return Some(j);
        }
        if kind == "outbound_sent" && crate::text_transport::wan_text_via_delivery_server() {
            if let Some(obj) = j.as_object_mut() {
                obj.insert("stores_updated".to_string(), Value::Bool(true));
            }
            return Some(j);
        }
        if kind == "call_signal" {
            let signal = j.get("signal").and_then(|v| v.as_str()).unwrap_or("");
            if signal == "invite" {
                let created = j.get("created_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                if !call_state::call_invite_is_live(created, now) {
                    continue;
                }
            }
        }
        return Some(j);
    }
}
