//! One peer TCP session: Noise XX + channel mux read/write loops.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use bytes::{BufMut, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use super::channel_mux::{CHANNEL_CALL_AUDIO, CHANNEL_KEEPALIVE, CHANNEL_MSG, MUX_HEADER_LEN};
use super::frames::{dispatch_mux_payload, on_session_ready, WireDispatchCtx};
use super::noise_session::{transport_static_secret_for_identity, ConnectNoiseSession};
use super::session::{chrono_now_ms, SessionState};
use super::types::{GossipChatEvent, SessionPeer};
use super::worker::{mark_peer_connected, mark_peer_disconnected};
use crate::p2p::native_log;

pub(crate) type SessionWriters =
    Arc<RwLock<HashMap<SessionPeer, mpsc::UnboundedSender<SessionWireItem>>>>;

#[derive(Debug)]
pub(crate) enum SessionWireItem {
    Channel0(Vec<u8>),
    Mux { channel: u32, payload: Vec<u8> },
}

struct ActiveSession {
    is_outbound: bool,
    abort: Arc<AtomicBool>,
}

pub(crate) struct PeerSessionRegistry {
    pub writers: SessionWriters,
    sessions: RwLock<HashMap<SessionPeer, ActiveSession>>,
    dial_inflight: RwLock<HashSet<SessionPeer>>,
    pub local_identity_wire: String,
}

impl PeerSessionRegistry {
    pub fn new(local_identity_wire: String) -> Self {
        Self {
            writers: Arc::new(RwLock::new(HashMap::new())),
            sessions: RwLock::new(HashMap::new()),
            dial_inflight: RwLock::new(HashSet::new()),
            local_identity_wire,
        }
    }

    pub fn writer_open(&self, peer: &SessionPeer) -> bool {
        self.writers
            .read()
            .ok()
            .is_some_and(|g| g.contains_key(peer))
    }

    pub fn try_begin_dial(&self, peer: &SessionPeer) -> bool {
        if self.writer_open(peer) {
            return false;
        }
        let Ok(mut g) = self.dial_inflight.write() else {
            return false;
        };
        if g.contains(peer) {
            return false;
        }
        g.insert(peer.clone());
        true
    }

    pub fn end_dial(&self, peer: &SessionPeer) {
        if let Ok(mut g) = self.dial_inflight.write() {
            g.remove(peer);
        }
    }

    pub fn has_session(&self, peer: &SessionPeer) -> bool {
        self.sessions
            .read()
            .ok()
            .is_some_and(|g| g.contains_key(peer))
    }

    fn should_drop_new_outbound(&self, peer_wire: &SessionPeer) -> bool {
        self.has_session(peer_wire) && self.local_identity_wire.as_str() < peer_wire.as_str()
    }

    pub fn register_session(
        &self,
        peer: SessionPeer,
        is_outbound: bool,
        abort: Arc<AtomicBool>,
    ) {
        if let Ok(mut g) = self.sessions.write() {
            g.insert(
                peer,
                ActiveSession {
                    is_outbound,
                    abort,
                },
            );
        }
    }

    pub fn remove_session(&self, peer: &SessionPeer) {
        if let Ok(mut g) = self.sessions.write() {
            g.remove(peer);
        }
        if let Ok(mut w) = self.writers.write() {
            w.remove(peer);
        }
        self.end_dial(peer);
    }

    pub fn abort_outbound_duplicate(&self, peer: &SessionPeer) {
        if self.local_identity_wire.as_str() >= peer.as_str() {
            return;
        }
        if let Ok(g) = self.sessions.read() {
            if let Some(s) = g.get(peer) {
                if s.is_outbound {
                    s.abort.store(true, Ordering::SeqCst);
                }
            }
        }
    }
}

pub(crate) fn queue_frame_for_peer(
    writers: &SessionWriters,
    peer: &SessionPeer,
    frame: Vec<u8>,
) -> Result<(), String> {
    let tx = writers
        .read()
        .map_err(|_| "writers lock poisoned".to_string())?
        .get(peer)
        .cloned()
        .ok_or_else(|| "no connect session to peer yet — wait until connected".to_string())?;
    tx.send(SessionWireItem::Channel0(frame))
        .map_err(|_| "connect session closed".to_string())
}

pub(crate) fn queue_mux_for_peer(
    writers: &SessionWriters,
    peer: &SessionPeer,
    channel: u32,
    payload: Vec<u8>,
) -> Result<(), String> {
    let tx = writers
        .read()
        .map_err(|_| "writers lock poisoned".to_string())?
        .get(peer)
        .cloned()
        .ok_or_else(|| "no connect session to peer yet".to_string())?;
    tx.send(SessionWireItem::Mux { channel, payload })
        .map_err(|_| "connect session closed".to_string())
}

pub(crate) async fn dial_peer_tcp(
    registry: Arc<PeerSessionRegistry>,
    session: Arc<SessionState>,
    identity: crate::DecryptedIdentity,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    peer_wire: SessionPeer,
    host: String,
    port: u16,
) {
    if !registry.try_begin_dial(&peer_wire) {
        return;
    }
    native_log::info("connect", format!("dial {peer_wire} at {host}:{port}"));
    let stream = match TcpStream::connect((host.as_str(), port)).await {
        Ok(s) => s,
        Err(e) => {
            registry.end_dial(&peer_wire);
            native_log::warn("connect", format!("dial failed {peer_wire}: {e}"));
            let _ = events_tx.send(GossipChatEvent::DialFailed {
                peer: Some(peer_wire),
                error: e.to_string(),
            });
            return;
        }
    };
    start_session(
        registry,
        session,
        identity,
        events_tx,
        peer_wire,
        true,
        stream,
    )
    .await;
}

pub(crate) async fn accept_inbound_tcp(
    registry: Arc<PeerSessionRegistry>,
    session: Arc<SessionState>,
    identity: crate::DecryptedIdentity,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    stream: TcpStream,
    remote_addr: std::net::SocketAddr,
) {
    native_log::info("connect", format!("inbound TCP from {remote_addr}"));
    start_session(
        registry,
        session,
        identity,
        events_tx,
        String::new(),
        false,
        stream,
    )
    .await;
}

async fn start_session(
    registry: Arc<PeerSessionRegistry>,
    session: Arc<SessionState>,
    identity: crate::DecryptedIdentity,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    expected_peer: SessionPeer,
    is_outbound: bool,
    stream: TcpStream,
) {
    let (read, write) = tokio::io::split(stream);
    start_session_io(
        registry,
        session,
        identity,
        events_tx,
        expected_peer,
        is_outbound,
        read,
        write,
    )
    .await;
}

pub(crate) async fn start_session_io<R, W>(
    registry: Arc<PeerSessionRegistry>,
    session: Arc<SessionState>,
    identity: crate::DecryptedIdentity,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    expected_peer: SessionPeer,
    is_outbound: bool,
    read: R,
    write: W,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let transport_sk = transport_static_secret_for_identity(&identity);
    let hs = if is_outbound {
        ConnectNoiseSession::initiator(&identity, &transport_sk, read, write).await
    } else {
        ConnectNoiseSession::responder(&identity, &transport_sk, read, write).await
    };
    let (mut noise, remote_wire, read, write) = match hs {
        Ok(v) => v,
        Err(e) => {
            if is_outbound {
                registry.end_dial(&expected_peer);
            }
            native_log::warn("connect", format!("noise handshake failed: {e}"));
            if is_outbound {
                let _ = events_tx.send(GossipChatEvent::DialFailed {
                    peer: Some(expected_peer),
                    error: e,
                });
            }
            return;
        }
    };
    let peer_wire = match super::types::session_peer_from_identity_wire(&remote_wire) {
        Ok(w) => w,
        Err(e) => {
            if is_outbound {
                registry.end_dial(&expected_peer);
            }
            native_log::warn("connect", format!("remote identity invalid: {e}"));
            return;
        }
    };
    if is_outbound && !expected_peer.is_empty() && peer_wire != expected_peer {
        native_log::warn(
            "connect",
            format!("dial identity mismatch expected={expected_peer} got={peer_wire}"),
        );
    }
    session.register_dm_peer_key(&peer_wire);
    if is_outbound && registry.should_drop_new_outbound(&peer_wire) {
        registry.end_dial(&peer_wire);
        native_log::info(
            "connect",
            format!("drop outbound duplicate to {peer_wire} (inbound wins)"),
        );
        return;
    }
    registry.abort_outbound_duplicate(&peer_wire);

    let (wire_tx, mut wire_rx) = mpsc::unbounded_channel::<SessionWireItem>();
    let abort_flag = Arc::new(AtomicBool::new(false));
    registry.register_session(peer_wire.clone(), is_outbound, Arc::clone(&abort_flag));
    if let Ok(mut g) = registry.writers.write() {
        g.insert(peer_wire.clone(), wire_tx);
    }
    session.set_peer_on_local_lan(&peer_wire, true);
    mark_peer_connected(session.as_ref(), &peer_wire, &events_tx);
    registry.end_dial(&peer_wire);

    let reg_c = Arc::clone(&registry);
    let sess_c = Arc::clone(&session);
    let ev_c = events_tx.clone();
    let peer_c = peer_wire.clone();
    let dispatch_ctx = WireDispatchCtx {
        session: Arc::clone(&session),
        events_tx: Some(events_tx.clone()),
        peer: peer_wire.clone(),
        writers: Arc::clone(&registry.writers),
    };

    let abort_c = Arc::clone(&abort_flag);
    tokio::spawn(async move {
        let mut read = read;
        let mut write = write;
        let mut last_write = chrono_now_ms();

        loop {
            if abort_c.load(Ordering::SeqCst) {
                break;
            }
            tokio::select! {
                biased;
                item = wire_rx.recv() => {
                    let Some(item) = item else { break };
                    let (channel, payload) = match item {
                        SessionWireItem::Channel0(frame) => (CHANNEL_MSG, frame),
                        SessionWireItem::Mux { channel, payload } => (channel, payload),
                    };
                    if write_noise_mux(&mut noise, &mut write, channel, &payload).await.is_err() {
                        break;
                    }
                    last_write = chrono_now_ms();
                }
                read_res = read_noise_mux(&mut noise, &mut read) => {
                    match read_res {
                        Ok((channel, payload)) => {
                            if channel == CHANNEL_KEEPALIVE {
                                if payload.is_empty() {
                                    let _ = write_noise_mux(
                                        &mut noise,
                                        &mut write,
                                        CHANNEL_KEEPALIVE,
                                        &[0x01],
                                    )
                                    .await;
                                }
                                continue;
                            }
                            dispatch_mux_payload(channel, &payload, &dispatch_ctx).await;
                        }
                        Err(_) => break,
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(20)) => {
                    if chrono_now_ms().saturating_sub(last_write) >= 20_000 {
                        if write_noise_mux(
                            &mut noise,
                            &mut write,
                            CHANNEL_KEEPALIVE,
                            &[],
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                        last_write = chrono_now_ms();
                    }
                }
            }
        }

        reg_c.remove_session(&peer_c);
        sess_c.set_peer_on_local_lan(&peer_c, false);
        mark_peer_disconnected(sess_c.as_ref(), &peer_c, &ev_c);
        native_log::info(
            "connect",
            format!("session closed {peer_c} outbound={is_outbound}"),
        );
    });

    on_session_ready(
        session,
        registry.writers.clone(),
        peer_wire,
        Some(events_tx),
    )
    .await;
}

async fn write_noise_mux<W: AsyncWrite + Unpin>(
    noise: &mut ConnectNoiseSession,
    write: &mut W,
    channel: u32,
    payload: &[u8],
) -> Result<(), String> {
    let mut buf = BytesMut::with_capacity(MUX_HEADER_LEN + payload.len());
    buf.put_u32(channel);
    buf.put_u32(payload.len() as u32);
    buf.extend_from_slice(payload);
    noise.write(&buf, write).await
}

async fn read_noise_mux<R: AsyncRead + Unpin>(
    noise: &mut ConnectNoiseSession,
    read: &mut R,
) -> Result<(u32, Vec<u8>), String> {
    let plaintext = noise.read(read).await?;
    if plaintext.len() < MUX_HEADER_LEN {
        return Err("mux frame too short".into());
    }
    let channel = u32::from_be_bytes(plaintext[0..4].try_into().unwrap());
    let len = u32::from_be_bytes(plaintext[4..8].try_into().unwrap()) as usize;
    if plaintext.len() < MUX_HEADER_LEN + len {
        return Err("mux frame truncated".into());
    }
    Ok((
        channel,
        plaintext[MUX_HEADER_LEN..MUX_HEADER_LEN + len].to_vec(),
    ))
}

pub(crate) fn writer_open_for_peer(writers: &SessionWriters, peer: &SessionPeer) -> bool {
    writers
        .read()
        .ok()
        .is_some_and(|g| g.contains_key(peer))
}
