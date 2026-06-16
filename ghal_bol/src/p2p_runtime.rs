//! In-process native DM worker (shared by FFI and the Unix-socket daemon).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use std::time::Duration;

use crate::dm_transport::DmDialAddr;
use crate::peer_id_util::peer_id_from_secp256k1_public_key_hex;
use crate::peer_id_util::secp256k1_public_key_hex_from_peer_id;
use libp2p::Multiaddr;
use serde_json::Value;

use crate::session_runtime::unlocked_identity_clone;
use crate::dm_event_handler::{apply_p2p_event_json, clear_p2p_handler_context, set_foreground_peer,
    set_p2p_handler_context};
use crate::call_sig_v1::CallSigKind;
use crate::call_state;
use crate::msg_v1::MsgKind;
use crate::p2p::{
    native_log, queue_read_ack_catchup,
    run_gossip_chat_node_with_std_io, set_drop_pending_call_invite_hook,
    sync_foreground_peer_now, DmPeer, GossipChatConfig, GossipChatEvent, OutboundCmd,
    DEFAULT_GOSSIP_TOPIC,
};

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
    q.retain(|ev| {
        match ev {
            GossipChatEvent::CallSignal { signal, created_at_ms, .. } if signal == "invite" => {
                call_state::call_invite_is_live(*created_at_ms, now_ms)
            }
            _ => true,
        }
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
                signal_id: crate::p2p::chat_server::new_msg_id_for_ffi(),
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
            let pk = secp256k1_public_key_hex_from_peer_id(&p).unwrap_or_else(|| p.to_string());
            serde_json::json!({
                "kind": "peer_connected",
                "peer_id": p.to_string(),
                "public_key_hex": pk,
            })
        }
        GossipChatEvent::PeerDisconnected(p) => {
            let pk = secp256k1_public_key_hex_from_peer_id(&p).unwrap_or_else(|| p.to_string());
            serde_json::json!({
                "kind": "peer_disconnected",
                "peer_id": p.to_string(),
                "public_key_hex": pk,
            })
        }
        GossipChatEvent::DialFailed { peer, error } => serde_json::json!({
            "kind": "dial_failed",
            "peer": peer.and_then(|p| secp256k1_public_key_hex_from_peer_id(&p)),
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
        } => serde_json::json!({
            "kind": "dm_message",
            "from": from.to_string(),
            "id": id,
            "msg_kind": msg_kind,
            "text": text,
            "ref_id": ref_id,
            "sender_public_key_hex": sender_public_key_hex,
            "created_at_ms": created_at_ms,
        }),
        GossipChatEvent::PeerIdentified {
            peer_id,
            public_key_hex,
        } => serde_json::json!({
            "kind": "peer_identified",
            "peer_id": peer_id.to_string(),
            "public_key_hex": public_key_hex,
        }),
        GossipChatEvent::ChatReady { peer_id } => {
            let pk = secp256k1_public_key_hex_from_peer_id(&peer_id)
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
        GossipChatEvent::NativeLog { level, tag, message } => serde_json::json!({
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

fn parse_dm_peers(v: &Value) -> Vec<DmPeer> {
    let mut out = Vec::new();
    let Some(arr) = v.get("dm_peers").and_then(|x| x.as_array()) else {
        return out;
    };
    for item in arr {
        let Some(pk) = item
            .get("public_key_hex")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| s.len() == 66)
        else {
            continue;
        };
        if let Ok(dm) = DmPeer::from_public_key_hex(pk.to_string()) {
            out.push(dm);
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

fn dial_bootstrap_on_running_node(addrs: Vec<DmDialAddr>) {
    if addrs.is_empty() {
        return;
    }
    let out_tx = match p2p_mx().lock() {
        Ok(g) => g.as_ref().map(|h| h.out_tx.clone()),
        Err(_) => None,
    };
    let Some(out_tx) = out_tx else {
        return;
    };
    let mas: Vec<Multiaddr> = addrs
        .iter()
        .filter_map(|a| a.to_multiaddr_string().parse().ok())
        .collect();
    let _ = out_tx.send(OutboundCmd::DialBootstrapPeers { addrs: mas });
}

/// Start libp2p DM node in a background thread.
pub fn p2p_start(config: &Value) -> Value {
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

    let mut gossip_cfg = match GossipChatConfig::from_unlocked_identity(topic, &ident) {
        Ok(c) => c,
        Err(e) => return json_err(format!("{e}")),
    };
    let bootstrap_for_hot_dial = peers.clone();
    gossip_cfg.bootstrap_peers = peers
        .iter()
        .filter_map(|a| a.to_multiaddr_string().parse().ok())
        .collect();
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
            let pk = dm
                .public_key_hex
                .as_deref()
                .unwrap_or("")
                .trim();
            if pk.len() != 66 {
                continue;
            }
            let _ = p2p_register_dm_peer(pk);
        }
        dial_bootstrap_on_running_node(bootstrap_for_hot_dial);
        if crate::coord_runtime::coord_is_configured() {
            crate::p2p::notify_relay_refresh();
        }
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
            "node starting local_peer={} dm_peers={} bootstrap={}",
            gossip_cfg.keypair.public().to_peer_id(),
            gossip_cfg.dm_peers.len(),
            gossip_cfg.bootstrap_peers.len()
        ),
    );

    if let Some(ns) = gossip_cfg.app_namespace.as_deref() {
        set_p2p_handler_context(ns);
        crate::incoming_call_notify::set_desktop_app_id(ns);
    }

    clear_pending_p2p_events();
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
                let _ = ev_tx.send(GossipChatEvent::NodeStopped {
                    error: Some(msg),
                });
                native_log::set_sink(None);
                clear_p2p_holder();
                return;
            }
        };
        let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.block_on(run_gossip_chat_node_with_std_io(
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
        let _ = ev_tx.send(GossipChatEvent::NodeStopped {
            error: stop_err,
        });
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
    crate::coord_runtime::stop_coord_presence();
    crate::p2p::set_app_ack_read_enabled(false);
    stop_p2p_node(Duration::from_secs(3));
    call_state::clear_all_calls();
    crate::incoming_call_notify::dismiss_incoming_call();
    clear_p2p_handler_context();
    native_log::set_sink(None);
}

/// Voice-call signaling. JSON:
/// `{ "recipient_public_key_hex": "<66-hex>", "call_id": "...", "signal": "invite|...", "payload": {} }`
pub fn p2p_call_signal(config: &Value) -> Value {
    let recipient = match config
        .get("recipient_public_key_hex")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| s.len() == 66)
    {
        Some(s) => s.to_string(),
        None => return json_err("recipient_public_key_hex required (66 hex)"),
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
    let payload = config.get("payload").cloned().unwrap_or(Value::Object(Default::default()));
    let signal_id = config
        .get("signal_id")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| crate::p2p::chat_server::new_msg_id_for_ffi());

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
            let recipient = match config
                .get("recipient_public_key_hex")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| s.len() == 66)
            {
                Some(s) => s.to_string(),
                None => return json_err("recipient_public_key_hex required (66 hex)"),
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
            let recipient = match config
                .get("recipient_public_key_hex")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| s.len() == 66)
            {
                Some(s) => s.to_string(),
                None => return json_err("recipient_public_key_hex required (66 hex)"),
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
            let enabled = config.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
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
            _ => crate::call_video::RawVideoFrame { width, height, data },
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
    if wait_for_wire {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        if out_tx
            .send(OutboundCmd::SendText {
                recipient_public_key_hex: recipient,
                text,
                message_id: message_id.clone(),
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
            done: None,
        })
        .is_err()
    {
        native_log::warn("outbound", format!("send_text failed: node stopped? msg_id={message_id}"));
        return json_err("p2p send failed (node stopped?)");
    }
    json_ok(serde_json::json!({
        "ok": true,
        "message_id": message_id,
        "queued": true,
    }))
}

pub fn p2p_send_text_dm(recipient: &str, text: &str) -> Value {
    let message_id = crate::p2p::chat_server::new_msg_id_for_ffi();
    enqueue_send_text_dm(message_id, recipient.to_string(), text.to_string(), false)
}

pub fn p2p_requeue_outbound_dm(message_id: &str, recipient: &str, text: &str) -> Value {
    if message_id.trim().is_empty() {
        return json_err("message_id required");
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
    if pk_trim.len() == 66 {
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
    native_log::info(
        "session",
        format!("register_dm_peer pk={pk_short}"),
    );
    if out_tx
        .send(OutboundCmd::RegisterDmPeer {
            peer_id: None,
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
    if let Ok(g) = p2p_mx().lock() {
        if let Some(h) = g.as_ref() {
            if enabled {
                if let Some(peer) = crate::p2p::live_foreground_peer_for_catchup() {
                    queue_read_ack_catchup(&h.out_tx, peer);
                }
            }
            // Leave backlog drain runs only from `SetForegroundPeer(None)` — not here.
            // Queuing drain on every gate-off flooded priority-0 outbound and starved SendText.
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
        .filter(|s| s.len() == 66);
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

/// DM + libp2p connectivity — forward to Flutter App log (`Native/…` tags in export).
fn native_log_should_forward_to_ui(line: &native_log::NativeLogLine) -> bool {
    if line.level == "warn" || line.level == "error" {
        return true;
    }
    let tag = line.tag.as_str();
    let connectivity = matches!(
        tag,
        "flow" | "net" | "p2p" | "swarm" | "kad" | "listen" | "relay" | "coord" | "mdns"
            | "dial" | "autonat" | "dcutr" | "upnp" | "stream"
    );
    if connectivity {
        return line.level == "info" || (line.level == "debug" && native_log::verbose_enabled());
    }
    let dm_flow = matches!(
        tag,
        "DM/store" | "Contacts" | "Transcript" | "outbound" | "outbox" | "delivery_ack"
            | "read_ack" | "session" | "Storage"
    );
    if dm_flow && (line.level == "info" || line.level == "debug") {
        return true;
    }
    false
}

pub fn p2p_set_foreground_peer(public_key_hex: Option<&str>) -> Value {
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
    set_foreground_peer(pk.clone());
    sync_foreground_peer_now(pk.clone());
    let peer_id = pk.as_ref().and_then(|hex| {
        peer_id_from_secp256k1_public_key_hex(hex)
            .ok()
            .and_then(|s| s.parse().ok())
    });
    let generation = crate::p2p::chat_server::bump_foreground_peer_cmd_gen();
    if out_tx
        .send(OutboundCmd::SetForegroundPeer {
            peer_id,
            generation,
        })
        .is_err()
    {
        return json_err("p2p send failed (node stopped?)");
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
        obj.insert(
            "conversation_key".to_string(),
            Value::String(view_key),
        );
        obj.insert(
            "transcript_revision".to_string(),
            Value::Number(rev.into()),
        );
    }
}

pub fn p2p_poll_event() -> Option<Value> {
    maybe_maintain_poll_queue(call_state::now_ms());
    let now = call_state::now_ms();
    loop {
        let Some(ev) = poll_next_p2p_event() else {
            return None;
        };
        if let GossipChatEvent::Listening(ma) = &ev {
            if let Some(dm) = DmDialAddr::parse(&ma.to_string()) {
                crate::coord_runtime::on_listen_dm_addr(&dm);
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
                // DESIGN.md: duplicate/no-op acks do not change stores — drain, no UI event.
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
