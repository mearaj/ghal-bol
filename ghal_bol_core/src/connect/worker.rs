//! Main connect worker loop — mDNS + TCP + Noise + mux.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio::select;

use super::lan_discovery::{LanDiscovery, LanDiscoveryEvent, spawn_event_forwarder};
use super::outbound::process_outbound_cmds;
use super::outbox_acks::run_ack_upkeep;
use super::peer_session::{PeerSessionRegistry, SessionWriters, accept_inbound_tcp, dial_peer_tcp};
use super::session::{SessionState, chrono_now_ms};
use super::types::{
    ConnectConfig, ConnectError, GossipChatEvent, MAX_OUTBOUND_CMDS_PER_TICK, OutboundCmd,
    SessionPeer,
};
use crate::dm_transport::DmDialAddr;
use crate::p2p::native_log;

static LAST_BRIDGE_PENDING_POLL_MS: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(0);
const BRIDGE_PENDING_POLL_MS: i64 = 3_000;
/// After a failed accept, wait before retrying the same bridge_id (avoids log storms).
const BRIDGE_ACCEPT_RETRY_MS: i64 = 30_000;
static BRIDGE_ACCEPT_BACKOFF: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
/// bridge_ids currently in `accept_bridge_pending` — do not spawn duplicates.
static BRIDGE_ACCEPT_INFLIGHT: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

pub struct ConnectWorkerState {
    pub registry: Arc<PeerSessionRegistry>,
    pub session: Arc<SessionState>,
    pub identity: crate::DecryptedIdentity,
    pub events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
}

pub async fn run_connect_node_with_std_io(
    config: ConnectConfig,
    identity: crate::DecryptedIdentity,
    outbound_rx: std::sync::mpsc::Receiver<OutboundCmd>,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    stop: Arc<AtomicBool>,
) -> Result<(), ConnectError> {
    let ident_wire = identity.identity_wire();
    native_log::info("connect", "starting native connect worker");

    let session = Arc::new(
        SessionState::new(
            identity.clone(),
            &config.dm_peers,
            config.app_namespace.clone(),
        )
        .map_err(ConnectError::Other)?,
    );

    let registry = Arc::new(PeerSessionRegistry::new(ident_wire.clone()));

    let listener = TcpListener::bind("0.0.0.0:0")
        .await
        .map_err(ConnectError::Io)?;
    let local_addr = listener.local_addr().map_err(ConnectError::Io)?;
    let listen_port = local_addr.port();
    let listen_str = format!("{}:{}", local_addr.ip(), local_addr.port());
    let _ = events_tx.send(GossipChatEvent::Listening(listen_str.clone()));
    native_log::info(
        "connect",
        format!(
            "TCP listen {listen_str} pattern={}",
            super::noise_session::NOISE_PATTERN
        ),
    );

    let mut discovery = LanDiscovery::new().map_err(ConnectError::Other)?;
    for p in &config.dm_peers {
        let _ = discovery.register_contact(&p.identity_wire);
    }
    let _ = discovery.publish_listener(&ident_wire, local_addr.port());
    let discovery = Arc::new(std::sync::Mutex::new(discovery));
    let (mdns_tx, mut mdns_rx) = tokio::sync::mpsc::unbounded_channel::<LanDiscoveryEvent>();
    spawn_event_forwarder(Arc::clone(&discovery), mdns_tx);

    let _ = events_tx.send(GossipChatEvent::NodeReady);

    let worker = Arc::new(ConnectWorkerState {
        registry: Arc::clone(&registry),
        session: Arc::clone(&session),
        identity: identity.clone(),
        events_tx: events_tx.clone(),
    });

    let events_opt = Some(events_tx.clone());
    let mut upkeep = tokio::time::interval(std::time::Duration::from_secs(1));
    upkeep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }

        if super::notify::take_connect_upkeep_notify() {
            run_connect_upkeep(
                &identity,
                listen_port,
                Arc::clone(&registry),
                Arc::clone(&session),
                events_tx.clone(),
            );
        }

        process_outbound_cmds(
            Arc::clone(&worker),
            &events_opt,
            &outbound_rx,
            MAX_OUTBOUND_CMDS_PER_TICK,
        )
        .await;

        while let Ok(ev) = mdns_rx.try_recv() {
            match ev {
                LanDiscoveryEvent::Discovered {
                    identity_commitment,
                    host,
                    port,
                } => {
                    let peer_wire = discovery
                        .lock()
                        .ok()
                        .and_then(|d| d.wire_for_commitment(&identity_commitment));
                    if let Some(peer) = peer_wire {
                        native_log::info(
                            "connect",
                            format!("mdns discovered {host}:{port} peer={peer}"),
                        );
                        let reg = Arc::clone(&registry);
                        let sess = Arc::clone(&session);
                        let id = identity.clone();
                        let ev = events_tx.clone();
                        tokio::spawn(dial_peer_tcp(reg, sess, id, ev, peer, host, port));
                    }
                }
                LanDiscoveryEvent::Expired {
                    identity_commitment,
                } => {
                    if let Some(peer) = discovery
                        .lock()
                        .ok()
                        .and_then(|d| d.wire_for_commitment(&identity_commitment))
                    {
                        native_log::info("connect", format!("mdns expired {peer}"));
                        registry.remove_session(&peer);
                        session.set_peer_on_local_lan(&peer, false);
                    }
                }
            }
        }

        let connected: Vec<SessionPeer> = registry
            .writers
            .read()
            .ok()
            .map(|g| g.keys().cloned().collect())
            .unwrap_or_default();
        if !connected.is_empty() {
            let sess = Arc::clone(&session);
            let writers: SessionWriters = Arc::clone(&registry.writers);
            let ev = events_tx.clone();
            let peers = connected.clone();
            tokio::spawn(async move {
                super::outbox_acks::resync_pending_outbox(sess, writers, peers, Some(ev)).await;
            });
            run_ack_upkeep(
                Arc::clone(&session),
                Arc::clone(&registry.writers),
                &connected,
            )
            .await;
        }

        select! {
            biased;
            accept = listener.accept() => {
                if let Ok((stream, addr)) = accept {
                    let reg = Arc::clone(&registry);
                    let sess = Arc::clone(&session);
                    let id = identity.clone();
                    let ev = events_tx.clone();
                    tokio::spawn(accept_inbound_tcp(reg, sess, id, ev, stream, addr));
                }
            }
            _ = upkeep.tick() => {
                run_connect_upkeep(
                    &identity,
                    listen_port,
                    Arc::clone(&registry),
                    Arc::clone(&session),
                    events_tx.clone(),
                );
                super::chat_room_session::tick_chat_room_session_if_active(session.as_ref());
                let connected: Vec<SessionPeer> = registry
                    .writers
                    .read()
                    .ok()
                    .map(|g| g.keys().cloned().collect())
                    .unwrap_or_default();
                if !connected.is_empty() {
                    run_delivery_ack_upkeep(
                        Arc::clone(&session),
                        Arc::clone(&registry.writers),
                        &connected,
                    )
                    .await;
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
        }
    }

    let _ = events_tx.send(GossipChatEvent::NodeStopped { error: None });
    Ok(())
}

async fn run_delivery_ack_upkeep(
    session: Arc<SessionState>,
    writers: SessionWriters,
    connected_peers: &[SessionPeer],
) {
    super::outbox_acks::run_ack_upkeep_limited_delivery(session, writers, connected_peers).await;
}

pub fn mark_peer_connected(
    session: &SessionState,
    peer: &SessionPeer,
    events_tx: &std::sync::mpsc::Sender<GossipChatEvent>,
) {
    session.set_peer_connected(peer, true);
    session.set_stream_ready(peer, true);
    if session.emit_identified_once(peer) {
        let _ = events_tx.send(GossipChatEvent::PeerIdentified {
            peer_id: peer.clone(),
            public_key_hex: peer.clone(),
        });
    }
    if session.emit_chat_ready_once(peer) {
        let _ = events_tx.send(GossipChatEvent::ChatReady {
            peer_id: peer.clone(),
        });
    }
    let _ = events_tx.send(GossipChatEvent::PeerConnected(peer.clone()));
}

pub fn mark_peer_disconnected(
    session: &SessionState,
    peer: &SessionPeer,
    events_tx: &std::sync::mpsc::Sender<GossipChatEvent>,
) {
    session.set_peer_connected(peer, false);
    session.set_stream_ready(peer, false);
    session.clear_transport_kem_for_peer(peer);
    let _ = events_tx.send(GossipChatEvent::PeerDisconnected(peer.clone()));
}

fn run_connect_upkeep(
    identity: &crate::DecryptedIdentity,
    listen_port: u16,
    registry: Arc<PeerSessionRegistry>,
    session: Arc<SessionState>,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
) {
    crate::p2p::network_transport::refresh_os_network_truth();
    let mut net = crate::p2p::network_transport::detect_local_network_profile();
    crate::p2p::network_transport::merge_os_network_truth(&mut net);
    if let Some(pub_ip) = net.primary_public_ipv4 {
        if let Some(addr) = DmDialAddr::parse(&format!("{pub_ip}:{listen_port}")) {
            crate::coord_runtime::on_listen_dm_addr(&addr);
        }
    }
    crate::coord_runtime::coord_register_tick(&[]);

    let now = chrono_now_ms();
    let last = LAST_BRIDGE_PENDING_POLL_MS.load(std::sync::atomic::Ordering::Relaxed);
    if now.saturating_sub(last) < BRIDGE_PENDING_POLL_MS {
        return;
    }
    LAST_BRIDGE_PENDING_POLL_MS.store(now, std::sync::atomic::Ordering::Relaxed);
    if !crate::coord_runtime::coord_is_configured() {
        return;
    }
    let ident_wire = identity.identity_wire();
    let reg = Arc::clone(&registry);
    let sess = Arc::clone(&session);
    let id = identity.clone();
    let ev = events_tx;
    tokio::spawn(async move {
        let pending = match tokio::task::spawn_blocking(move || {
            super::bridge_ws::poll_bridge_pending_blocking(&ident_wire)
        })
        .await
        {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                native_log::debug("bridge", format!("pending poll: {e}"));
                return;
            }
            Err(e) => {
                native_log::debug("bridge", format!("pending poll task: {e}"));
                return;
            }
        };
        for item in pending {
            let caller = item.caller_identity_wire.clone();
            // Skip only when a live writer exists — a stale registry entry must not
            // block WAN bridge accept (cell↔Wi‑Fi calls).
            if super::peer_session::writer_open_for_peer(&reg.writers, &caller) {
                continue;
            }
            let now_ms = chrono_now_ms();
            if let Ok(g) = BRIDGE_ACCEPT_BACKOFF.lock() {
                if g.get(&item.bridge_id).is_some_and(|until| *until > now_ms) {
                    continue;
                }
            }
            {
                let Ok(mut inflight) = BRIDGE_ACCEPT_INFLIGHT.lock() else {
                    continue;
                };
                if !inflight.insert(item.bridge_id.clone()) {
                    continue; // already accepting this bridge_id
                }
            }
            native_log::info(
                "bridge",
                format!("accepting pending bridge call_id={}", item.call_id),
            );
            let bridge_id = item.bridge_id.clone();
            let accept_result = super::bridge_ws::accept_bridge_pending(
                Arc::clone(&reg),
                Arc::clone(&sess),
                id.clone(),
                ev.clone(),
                caller,
                item,
            )
            .await;
            if let Ok(mut inflight) = BRIDGE_ACCEPT_INFLIGHT.lock() {
                inflight.remove(&bridge_id);
            }
            if let Err(e) = accept_result {
                native_log::warn("bridge", format!("accept pending bridge failed: {e}"));
                if let Ok(mut g) = BRIDGE_ACCEPT_BACKOFF.lock() {
                    g.insert(bridge_id, now_ms + BRIDGE_ACCEPT_RETRY_MS);
                    if g.len() > 64 {
                        g.retain(|_, until| *until > now_ms);
                    }
                }
            }
        }
    });
}
