//! In-process native DM worker (shared by FFI and the Unix-socket daemon).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
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
    run_gossip_chat_node_with_std_io, sync_foreground_peer_now,
    DmPeer, GossipChatConfig, GossipChatEvent, OutboundCmd, DEFAULT_GOSSIP_TOPIC,
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
    let Some(url) = config
        .get("coord_base_url")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let tls = config
        .get("coord_insecure_tls")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    crate::coord_runtime::set_coord_base_url(url, tls);
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

/// Hint the swarm loop that the OS default network changed (Android callback or desktop).
pub fn p2p_notify_network_change() -> Value {
    if !p2p_holder_alive() {
        return json_ok(serde_json::json!({ "ok": false, "running": false }));
    }
    crate::p2p::notify_network_change();
    json_ok(serde_json::json!({ "ok": true }))
}

pub fn p2p_stop() {
    native_log::info("p2p", "p2p_stop requested");
    crate::coord_runtime::stop_coord_presence();
    crate::p2p::set_app_ack_read_enabled(false);
    stop_p2p_node(Duration::from_secs(3));
    call_state::clear_all_calls();
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
    if out_tx
        .send(OutboundCmd::SetForegroundPeer { peer_id })
        .is_err()
    {
        return json_err("p2p send failed (node stopped?)");
    }
    json_ok(serde_json::json!({ "ok": true }))
}

pub fn p2p_poll_event() -> Option<Value> {
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
                return Some(j);
            }
        }
        if kind == "peer_identified" {
            return Some(j);
        }
        return Some(j);
    }
}
