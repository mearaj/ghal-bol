//! libp2p **direct-message** node: **QUIC/TCP**, **relay**, **mDNS**, **Kademlia DHT**, and **`/ghal-bol/msg/1.0.0`** streams.
//!
//! Gossipsub was removed; 1:1 chat uses signed **`ghal_bol_msg_v1`** frames on libp2p streams.
//!
//! ## Messaging model (`docs/GOTIGIN_DM_MSG_V1.md`)
//!
//! - **Receiver:** off-room → `ack_received` only for new mail; in-room → `ack_received` then `ack_read`.
//!   Do not clear the read-ack queue on leave (retry in-room backlog).
//! - **Sender:** resend **text** from outbox until peer `ack_received` or `ack_read` (never `ack_request`).
//! - **Transcript is authoritative** for what to resend on each upkeep tick.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use crate::dm_transport::ContactPk;

/// UI foreground peer — updated synchronously from FFI before the outbox cmd is processed.
static LIVE_FOREGROUND_PEER: OnceLock<RwLock<Option<ContactPk>>> = OnceLock::new();
static LAST_ROOM_PEER: OnceLock<RwLock<Option<ContactPk>>> = OnceLock::new();
static NETWORK_CHANGE_NOTIFY: AtomicBool = AtomicBool::new(false);

/// Android connectivity / default-network change — swarm loop re-runs handover recovery.
pub fn notify_network_change() {
    NETWORK_CHANGE_NOTIFY.store(true, Ordering::SeqCst);
}

pub(crate) fn take_network_change_notify() -> bool {
    NETWORK_CHANGE_NOTIFY.swap(false, Ordering::SeqCst)
}

fn live_foreground_peer_mx() -> &'static RwLock<Option<ContactPk>> {
    LIVE_FOREGROUND_PEER.get_or_init(|| RwLock::new(None))
}

fn last_room_peer_mx() -> &'static RwLock<Option<ContactPk>> {
    LAST_ROOM_PEER.get_or_init(|| RwLock::new(None))
}

pub fn last_room_peer() -> Option<ContactPk> {
    last_room_peer_mx().read().ok().and_then(|g| g.clone())
}

/// Match Flutter room open/close immediately (avoids 1–2 spurious `ack_read` while leaving).
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

fn live_foreground_peer() -> Option<ContactPk> {
    live_foreground_peer_mx().read().ok().and_then(|g| g.clone())
}

pub fn live_foreground_peer_for_catchup() -> Option<ContactPk> {
    live_foreground_peer()
}

/// UI visibility gate (protonet: read state only while chatroom is active / app visible).
/// When false: inbound text gets `ack_received` only; no `ack_read` enqueue, seed, or upkeep.
static APP_ACK_READ_ENABLED: OnceLock<AtomicBool> = OnceLock::new();

fn app_ack_read_enabled_mx() -> &'static AtomicBool {
    APP_ACK_READ_ENABLED.get_or_init(|| AtomicBool::new(true))
}

/// Called from FFI when the app backgrounds or UI is torn down.
pub fn set_app_ack_read_enabled(enabled: bool) {
    app_ack_read_enabled_mx().store(enabled, Ordering::SeqCst);
}

pub fn queue_read_ack_catchup(out_tx: &std::sync::mpsc::Sender<OutboundCmd>, peer: ContactPk) {
    if !app_ack_read_enabled() || !live_foreground_peer().is_some_and(|f| f == peer) {
        return;
    }
    let Ok(pid) = peer_id_from_secp256k1_public_key_hex(&peer) else {
        return;
    };
    let Ok(peer_id) = pid.parse::<PeerId>() else {
        return;
    };
    let _ = out_tx.send(OutboundCmd::RunReadAckCatchup { peer_id });
}

fn spawn_leave_read_ack_drain(
    session: Arc<SessionState>,
    writers: StreamWriters,
    left: PeerId,
) {
    if let Some(pk) = secp256k1_public_key_hex_from_peer_id(&left) {
        seed_read_acks_for_peer_from_transcript(session.as_ref(), left);
        native_log::info(
            "read_ack",
            format!(
                "chat room leave {pk} — drain ack_read for in-room backlog (new mail: recv only)"
            ),
        );
    }
    tokio::spawn(async move {
        read_ack_catchup_for_peer(session, writers, left, false, false).await;
    });
}

fn app_ack_read_enabled() -> bool {
    app_ack_read_enabled_mx().load(Ordering::SeqCst)
}

use futures::StreamExt;
use libp2p::core::ConnectedPoint;
use libp2p::identity::PeerId;
use libp2p::multiaddr::Protocol;
use libp2p::noise;
use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::swarm::dial_opts::{DialOpts, PeerCondition};
use libp2p::swarm::{DialError, NetworkBehaviour, Swarm, SwarmEvent};
use libp2p::tcp;
use libp2p::yamux;
use libp2p::Multiaddr;
use libp2p::StreamProtocol;
use libp2p::SwarmBuilder;
use libp2p_stream as stream;
use rand_core::{OsRng, RngCore};
use thiserror::Error;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use tokio::select;
use tokio::sync::mpsc;
use tokio::time::{self, MissedTickBehavior};

use super::dht_bootstrap::{
    bootstrap_kad, decode_addr_record, expand_listen_addresses, kad_lookup_peer,
    new_kademlia_behaviour, peer_id_from_multiaddr,
    peer_id_from_record_key, resolve_public_dht_bootnodes, seed_kad_routing_table,
};
use super::native_log;
use crate::call_sig_v1::{
    build_call_envelope, call_envelope_from_frame, call_envelope_to_frame_bytes,
    frame_wire_share, parse_call_envelope, CallSigKind, CALL_SHARE,
};
use crate::call_state;
use crate::msg_v1::{
    build_ack_envelope, build_text_envelope, envelope_to_frame_bytes,
    frame_bytes_to_envelope, parse_envelope, MsgKind, ParsedMsg, MSG_SHARE,
    STREAM_PROTOCOL,
};
use crate::peer_id_util::{
    secp256k1_public_hex_matches_peer_id, peer_id_from_secp256k1_public_key_hex, secp256k1_public_key_hex_from_peer_id,
};

/// Kept for connect-invite `topic` metadata; **not** used for transport (streams replace gossipsub).
pub const DEFAULT_GOSSIP_TOPIC: &str = "ghal-bol-chat";

#[derive(Clone, Debug)]
pub struct DmPeer {
    pub peer_id: PeerId,
    pub public_key_hex: Option<String>,
}

impl DmPeer {
    pub fn from_public_key_hex(public_key_hex: String) -> Result<Self, String> {
        let pid_str = peer_id_from_secp256k1_public_key_hex(&public_key_hex)?;
        let peer_id: PeerId = pid_str
            .parse()
            .map_err(|e| format!("peer id parse: {e}"))?;
        Ok(Self {
            peer_id,
            public_key_hex: Some(public_key_hex),
        })
    }

    pub fn peer_id_only(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            public_key_hex: None,
        }
    }

    pub fn has_send_keys(&self) -> bool {
        self.public_key_hex
            .as_ref()
            .is_some_and(|s| s.len() == 66)
    }
}

#[derive(Clone, Debug)]
pub struct GossipChatConfig {
    pub topic: String,
    pub keypair: libp2p_identity::Keypair,
    pub bootstrap_peers: Vec<Multiaddr>,
    pub dm_peers: Vec<DmPeer>,
    /// Flutter `chat_transcript_v1.json` path for restoring pending outbound on start.
    pub transcript_path: Option<String>,
    pub app_namespace: Option<String>,
}

impl GossipChatConfig {
    pub fn new(topic: impl Into<String>, keypair: libp2p_identity::Keypair) -> Self {
        Self {
            topic: topic.into(),
            keypair,
            bootstrap_peers: Vec::new(),
            dm_peers: Vec::new(),
            transcript_path: None,
            app_namespace: None,
        }
    }

    pub fn from_unlocked_identity(
        topic: impl Into<String>,
        id: &crate::DecryptedIdentity,
    ) -> Result<Self, crate::Libp2pIdentityError> {
        Ok(Self {
            topic: topic.into(),
            keypair: id.to_libp2p_keypair()?,
            bootstrap_peers: Vec::new(),
            dm_peers: Vec::new(),
            transcript_path: None,
            app_namespace: None,
        })
    }
}

#[derive(Clone, Debug)]
pub enum GossipChatEvent {
    Listening(Multiaddr),
    PeerConnected(PeerId),
    /// DM libp2p connection closed (stream may still reopen on next upkeep).
    PeerDisconnected(PeerId),
    DialFailed {
        peer: Option<PeerId>,
        error: String,
    },
    DmMessage {
        from: PeerId,
        id: String,
        msg_kind: String,
        text: Option<String>,
        ref_id: Option<String>,
        sender_public_key_hex: String,
        created_at_ms: i64,
    },
    /// Remote contact keys are known (invite, libp2p PeerId, or verified on first DM frame).
    PeerIdentified {
        peer_id: PeerId,
        public_key_hex: String,
    },
    /// Outbound chat stream is open — safe to send text to this libp2p peer.
    ChatReady {
        peer_id: PeerId,
    },
    /// Outbound text could not be sent (see [message_id]).
    SendFailed {
        message_id: String,
        error: String,
    },
    /// Encrypted DM frame was written to the open chat stream (single-tick in UI).
    OutboundSent {
        message_id: String,
    },
    /// Internal libp2p / chat-server diagnostic (surfaced in Flutter as `native_log`).
    NativeLog {
        level: String,
        tag: String,
        message: String,
    },
    /// Voice-call signaling frame (`ghal_bol_call_v1`).
    CallSignal {
        from: PeerId,
        id: String,
        call_id: String,
        signal: String,
        sender_public_key_hex: String,
        created_at_ms: i64,
        payload: serde_json::Value,
    },
    /// Swarm is listening and processing outbound commands.
    NodeReady,
    /// Background node thread exited (see [error] when startup or run failed).
    NodeStopped {
        error: Option<String>,
    },
}

#[derive(Debug, Error)]
pub enum ChatServerError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("keystore → libp2p identity: {0}")]
    Libp2pIdentity(#[from] crate::Libp2pIdentityError),

    #[error("multiaddr: {0}")]
    Multiaddr(String),

    #[error("swarm transport: {0}")]
    Transport(String),

    #[error("listen: {0}")]
    Listen(String),
}

#[derive(Clone)]
pub enum OutboundCmd {
    SendText {
        recipient_public_key_hex: String,
        text: String,
        message_id: String,
        /// When set, the node reports whether the frame actually left on a chat stream.
        done: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    },
    SendAck {
        recipient_public_key_hex: String,
        ref_id: String,
        ack_kind: MsgKind,
    },
    /// Register remote keys on the running node (invite QR or hot-register).
    RegisterDmPeer {
        peer_id: Option<PeerId>,
        public_key_hex: String,
    },
    /// UI opened a chat with this contact (or closed when `None`).
    SetForegroundPeer {
        peer_id: Option<PeerId>,
    },
    /// Hub enabled read receipts after foreground was already set.
    RunReadAckCatchup {
        peer_id: PeerId,
    },
    /// Dial invite/coord bootstrap addrs on a running node.
    DialBootstrapPeers {
        addrs: Vec<Multiaddr>,
    },
    /// Outbound voice-call signaling (`ghal_bol_call_v1`).
    SendCallSignal {
        recipient_public_key_hex: String,
        call_id: String,
        signal_kind: CallSigKind,
        payload: serde_json::Value,
        signal_id: String,
    },
    /// Re-publish local listen addrs to the DHT (e.g. after `p2p_start` `already_running`).
    FlushKadPublish,
}

/// Outbound text kept until the peer’s **`ack_received`** or **`ack_read`** (read implies delivery).
#[derive(Clone)]
struct PendingOutbound {
    message_id: String,
    peer_id: PeerId,
    recipient_public_key_hex: String,
    text: String,
    created_at_ms: i64,
    last_send_ms: i64,
    /// False until the frame is actually written to the chat stream.
    on_wire: bool,
}

/// DM upkeep ticker: resend unacked outbound about once per second.
const OUTBOX_RESEND_INTERVAL_MS: i64 = 1_000;
/// Read-receipt retries per upkeep tick (queued until peer confirms).
const READ_ACK_UPKEEP_MAX_OPS_PER_TICK: usize = 64;
const ACK_BURST_MAX_OPS_PER_PASS: usize = 64;
const ACK_BURST_MAX_ROUNDS: usize = 1;
const MAX_PENDING_READ_ACKS: usize = 16_384;
const HISTORY_REPLAY_SPACING_MS: u64 = 8;

#[derive(Clone)]
struct PendingReadAck {
    peer_id: PeerId,
    inbound_id: String,
    recipient_public_key_hex: String,
}

#[derive(Clone)]
struct PendingDeliveryAck {
    peer_id: PeerId,
    inbound_id: String,
    recipient_public_key_hex: String,
}

#[derive(Clone)]
struct PendingCallSignal {
    recipient_public_key_hex: String,
    call_id: String,
    signal_kind: CallSigKind,
    payload: serde_json::Value,
    signal_id: String,
    frame: Vec<u8>,
    peer_id: PeerId,
}

fn is_transient_outbound_error(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("connecting to peer")
        || e.contains("chat stream opening")
        || e.contains("chat stream not ready")
        || e.contains("wait until connected")
        || e.contains("open_stream")
        || e.contains("broken pipe")
        || e.contains("connection reset")
        || e.contains("stream closed")
}

fn notify_outbound_on_wire(
    session: &SessionState,
    message_id: &str,
    now_ms: i64,
    events_tx: &Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if !session.mark_outbox_sent(message_id, now_ms) {
        return;
    }
    if let Some(tx) = events_tx {
        let _ = tx.send(GossipChatEvent::OutboundSent {
            message_id: message_id.to_string(),
        });
    }
}
/// Cap outbound work per swarm/poll tick (avoid unbounded drain in one select turn).
const MAX_OUTBOUND_CMDS_PER_TICK: usize = 64;
const SEEN_INBOUND_MAX: usize = 2_048;

struct SessionState {
    identity: crate::DecryptedIdentity,
    my_public_key_hex: String,
    peers: RwLock<PeerTables>,
    connected: RwLock<HashSet<PeerId>>,
    /// Messages we sent that are not yet `ack_received` by the peer.
    outbox: RwLock<HashMap<String, PendingOutbound>>,
    /// Dedupe inbound `text` emits (retries / duplicate frames).
    seen_inbound_ids: RwLock<HashMap<String, i64>>,
    /// FFI/UI: `peer_identified` at most once per remote libp2p peer.
    identified_emitted: RwLock<HashSet<PeerId>>,
    /// FFI/UI: `chat_ready` at most once per remote libp2p peer.
    chat_ready_emitted: RwLock<HashSet<PeerId>>,
    /// Dialable addresses we have published to the DHT (accumulated across listeners).
    published_listen: RwLock<Vec<Multiaddr>>,
    /// Debounce DHT `put_record` (avoid relay-addr storms that never replicate).
    kad_publish_fingerprint: RwLock<String>,
    kad_last_publish_ms: RwLock<i64>,
    /// Log `DHT record not found` at most once per remote peer per session.
    kad_not_found_logged: RwLock<HashSet<PeerId>>,
    /// Throttle routed dials / stream-open attempts (ms since epoch).
    routed_dial_attempt_ms: RwLock<HashMap<PeerId, i64>>,
    stream_open_log_emitted: RwLock<HashSet<PeerId>>,
    /// Prevent concurrent open_stream storms per peer (causes "receiver is gone"/oneshot canceled).
    stream_open_inflight: RwLock<HashSet<PeerId>>,
    /// Public IPFS DHT bootstrap peer ids (for logging + relay reservation).
    bootstrap_peer_ids: HashSet<PeerId>,
    relay_reserve_requested: RwLock<HashSet<PeerId>>,
    /// Throttle `listen_on(/p2p-circuit)` attempts per relay peer.
    /// Repeated listen attempts create large listen/behaviour churn and can delay WAN readiness.
    relay_reserve_last_attempt_ms: RwLock<HashMap<PeerId, i64>>,
    /// Remote multiaddr per connected public DHT bootstrap (relay reservation retries).
    bootstrap_relay_addr: RwLock<HashMap<PeerId, Multiaddr>>,
    dht_bootstrap_dial_logged: RwLock<HashSet<PeerId>>,
    /// At least one public DHT bootstrap peer has a live libp2p connection.
    any_bootstrap_connected: AtomicBool,
    /// Throttle repeated coord lookups per contact public key (UI can spam register/send bursts).
    last_coord_lookup_ms: RwLock<HashMap<String, i64>>,
    /// Backoff coord lookups when peer isn't registered yet (HTTP 404 peer_not_on_server).
    /// Key: recipient public_key_hex.
    coord_lookup_backoff: RwLock<HashMap<String, CoordLookupBackoff>>,
    kad_empty_closest_log_ms: RwLock<i64>,
    bootstrap_dial_err_log_ms: RwLock<HashMap<PeerId, i64>>,
    /// Peers we rejected on connect (public DHT noise); suppress disconnect logs.
    incidental_rejects: RwLock<HashSet<PeerId>>,
    /// Inbound texts needing `ack_read` while foreground chat is open (retried until confirmed).
    pending_read_acks: RwLock<VecDeque<PendingReadAck>>,
    /// Inbound texts whose `ack_received` failed to send (retried until stream is ready).
    pending_delivery_acks: RwLock<VecDeque<PendingDeliveryAck>>,
    /// Inbound message ids for which the peer sent `ack_received` after our `ack_read`.
    read_ack_confirmed: RwLock<HashSet<String>>,
    /// Inbound message ids for which we already sent `ack_received` (wire retries must not re-send).
    delivery_ack_sent: RwLock<HashSet<String>>,
    /// Call signaling frames waiting for DM stream (same transient errors as text send).
    pending_call_signals: RwLock<VecDeque<PendingCallSignal>>,
    foreground_peer: RwLock<Option<PeerId>>,
    transcript_path: Option<String>,
    app_namespace: Option<String>,
    /// History replay once per remote peer per session (avoids reordering the open chat).
    history_replay_done: RwLock<HashSet<PeerId>>,
    network_profile: RwLock<super::dht_bootstrap::LocalNetworkProfile>,
    /// Fast relay/coord/bootstrap loop after Wi‑Fi ↔ mobile (or OS connectivity callback).
    wan_recovery_active: AtomicBool,
    wan_recovery_started_ms: RwLock<i64>,
    /// Rate-limit diagnostic logs for dial skips (avoid log storms).
    dial_skip_log_ms: RwLock<HashMap<PeerId, i64>>,
}

#[derive(Clone, Copy, Debug)]
struct CoordLookupBackoff {
    next_allowed_ms: i64,
    step_ms: i64,
}

struct PeerTables {
    by_peer_id: HashMap<PeerId, DmPeer>,
}

impl PeerTables {
    fn retain_invalid_dm_peer_ids(&mut self) {
        self.by_peer_id.retain(|peer, dm| {
            if dm.has_send_keys() {
                return true;
            }
            secp256k1_public_key_hex_from_peer_id(peer).is_some()
        });
    }
}

impl SessionState {
    fn new(
        identity: crate::DecryptedIdentity,
        dm_peers_list: &[DmPeer],
        bootstrap_peer_ids: HashSet<PeerId>,
        transcript_path: Option<String>,
        app_namespace: Option<String>,
        network_profile: super::dht_bootstrap::LocalNetworkProfile,
    ) -> Result<Self, ChatServerError> {
        let my_public_key_hex = identity.public_key_hex();
        let mut tables = PeerTables {
            by_peer_id: HashMap::new(),
        };
        for p in dm_peers_list {
            if let Some(pk) = p
                .public_key_hex
                .as_deref()
                .map(str::trim)
                .filter(|s| s.len() == 66)
            {
                if let Ok(dm) = DmPeer::from_public_key_hex(pk.to_string()) {
                    tables.by_peer_id.insert(dm.peer_id, dm);
                    continue;
                }
            }
            if let Some(pk) = secp256k1_public_key_hex_from_peer_id(&p.peer_id) {
                tables
                    .by_peer_id
                    .insert(p.peer_id, DmPeer {
                        peer_id: p.peer_id,
                        public_key_hex: Some(pk),
                    });
            } else {
                native_log::debug(
                    "session",
                    format!(
                        "skip dm peer {} at start: not secp256k1 identity (relay/DHT nodes are not contacts)",
                        p.peer_id
                    ),
                );
            }
        }
        tables.retain_invalid_dm_peer_ids();
        Ok(Self {
            identity,
            my_public_key_hex,
            peers: RwLock::new(tables),
            connected: RwLock::new(HashSet::new()),
            outbox: RwLock::new(HashMap::new()),
            seen_inbound_ids: RwLock::new(HashMap::new()),
            identified_emitted: RwLock::new(HashSet::new()),
            chat_ready_emitted: RwLock::new(HashSet::new()),
            published_listen: RwLock::new(Vec::new()),
            kad_publish_fingerprint: RwLock::new(String::new()),
            kad_last_publish_ms: RwLock::new(0),
            kad_not_found_logged: RwLock::new(HashSet::new()),
            routed_dial_attempt_ms: RwLock::new(HashMap::new()),
            stream_open_log_emitted: RwLock::new(HashSet::new()),
            stream_open_inflight: RwLock::new(HashSet::new()),
            bootstrap_peer_ids,
            relay_reserve_requested: RwLock::new(HashSet::new()),
            relay_reserve_last_attempt_ms: RwLock::new(HashMap::new()),
            bootstrap_relay_addr: RwLock::new(HashMap::new()),
            dht_bootstrap_dial_logged: RwLock::new(HashSet::new()),
            any_bootstrap_connected: AtomicBool::new(false),
            last_coord_lookup_ms: RwLock::new(HashMap::new()),
            coord_lookup_backoff: RwLock::new(HashMap::new()),
            kad_empty_closest_log_ms: RwLock::new(0),
            bootstrap_dial_err_log_ms: RwLock::new(HashMap::new()),
            incidental_rejects: RwLock::new(HashSet::new()),
            pending_read_acks: RwLock::new(VecDeque::new()),
            pending_delivery_acks: RwLock::new(VecDeque::new()),
            read_ack_confirmed: RwLock::new(HashSet::new()),
            delivery_ack_sent: RwLock::new(HashSet::new()),
            pending_call_signals: RwLock::new(VecDeque::new()),
            foreground_peer: RwLock::new(None),
            transcript_path,
            app_namespace,
            history_replay_done: RwLock::new(HashSet::new()),
            network_profile: RwLock::new(network_profile),
            wan_recovery_active: AtomicBool::new(false),
            wan_recovery_started_ms: RwLock::new(0),
            dial_skip_log_ms: RwLock::new(HashMap::new()),
        })
    }

    fn should_log_dial_skip(&self, peer: PeerId, now_ms: i64, min_interval_ms: i64) -> bool {
        let Ok(mut m) = self.dial_skip_log_ms.write() else {
            return true;
        };
        let last = m.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < min_interval_ms {
            return false;
        }
        m.insert(peer, now_ms);
        true
    }

    fn diag_ctx(&self) -> String {
        let profile = self.network_profile_snapshot().mode_label().to_string();
        let coord_cfg = crate::coord_runtime::coord_is_configured();
        let coord_reg = crate::coord_runtime::coord_is_registered();
        let boot = self.any_bootstrap_connected.load(Ordering::Relaxed);
        let wan_recovery = self.wan_recovery_active.load(Ordering::Relaxed);
        let outbox = self.outbox.read().ok().map(|m| m.len()).unwrap_or(0);
        let pending_delivery = self
            .pending_delivery_acks
            .read()
            .ok()
            .map(|q| q.len())
            .unwrap_or(0);
        let pending_read = self
            .pending_read_acks
            .read()
            .ok()
            .map(|q| q.len())
            .unwrap_or(0);
        let relay_listen = self
            .published_listen_snapshot()
            .iter()
            .any(|ma| super::dht_bootstrap::is_relay_circuit_multiaddr(ma));
        format!(
            "profile={profile} coord_cfg={coord_cfg} coord_reg={coord_reg} bootstrap_ok={boot} relay_listen={relay_listen} wan_recovery={wan_recovery} outbox={outbox} pending_delivery_acks={pending_delivery} pending_read_acks={pending_read}"
        )
    }

    fn try_begin_stream_open(&self, peer: PeerId) -> bool {
        let Ok(mut g) = self.stream_open_inflight.write() else {
            return true;
        };
        g.insert(peer)
    }

    fn end_stream_open(&self, peer: PeerId) {
        let Ok(mut g) = self.stream_open_inflight.write() else {
            return;
        };
        g.remove(&peer);
    }

    fn begin_wan_recovery(&self) {
        self.wan_recovery_active.store(true, Ordering::Relaxed);
        if let Ok(mut t) = self.wan_recovery_started_ms.write() {
            *t = chrono_now_ms();
        }
    }

    fn refresh_bootstrap_connected_flag(&self, swarm: &Swarm<ChatBehaviour>) {
        let any = self
            .bootstrap_peer_ids
            .iter()
            .any(|p| swarm.is_connected(p));
        self.any_bootstrap_connected
            .store(any, Ordering::Relaxed);
    }

    fn network_profile_snapshot(&self) -> super::dht_bootstrap::LocalNetworkProfile {
        self.network_profile
            .read()
            .ok()
            .map(|p| *p)
            .unwrap_or_default()
    }

    /// Re-detect interfaces; returns `(old_mode, new_mode)` when dial/coord strategy should change.
    fn refresh_network_path_if_changed(&self) -> Option<(String, String)> {
        let new = super::dht_bootstrap::detect_local_network_profile();
        let Ok(mut cur) = self.network_profile.write() else {
            return None;
        };
        let old_key = super::dht_bootstrap::network_handover_key(&*cur);
        let new_key = super::dht_bootstrap::network_handover_key(&new);
        if old_key == new_key {
            return None;
        }
        let old_mode = cur.mode_label().to_string();
        *cur = new;
        let new_mode = cur.mode_label().to_string();
        Some((old_mode, new_mode))
    }

    /// Mobile/CGNAT without active Wi‑Fi LAN — prefer coord/relay; skip blind peerstore dials.
    /// Wi‑Fi + RFC1918 keeps routed dials enabled so LAN/mDNS paths stay smooth.
    fn prefers_mobile_coord_strategy(&self) -> bool {
        self.network_profile_snapshot()
            .avoid_blind_routed_dial()
    }

    fn should_coord_lookup_pk(&self, pk_hex: &str, now_ms: i64, min_interval_ms: i64) -> bool {
        let pk = pk_hex.trim();
        if pk.len() != 66 {
            return false;
        }
        if let Ok(m) = self.coord_lookup_backoff.read() {
            if let Some(b) = m.get(pk) {
                if now_ms < b.next_allowed_ms {
                    return false;
                }
            }
        }
        let Ok(mut m) = self.last_coord_lookup_ms.write() else {
            return true;
        };
        let last = m.get(pk).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < min_interval_ms {
            return false;
        }
        m.insert(pk.to_string(), now_ms);
        true
    }

    fn note_coord_lookup_not_found(&self, pk_hex: &str, now_ms: i64) {
        let pk = pk_hex.trim();
        if pk.len() != 66 {
            return;
        }
        let Ok(mut m) = self.coord_lookup_backoff.write() else {
            return;
        };
        let prev = m.get(pk).copied();
        // Fast initial retries, then back off hard (coord can't return what isn't registered).
        let next_step = match prev {
            None => 2_000,
            Some(p) => (p.step_ms.saturating_mul(2)).clamp(2_000, 30_000),
        };
        m.insert(
            pk.to_string(),
            CoordLookupBackoff {
                next_allowed_ms: now_ms.saturating_add(next_step),
                step_ms: next_step,
            },
        );
    }

    fn clear_coord_lookup_backoff(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if pk.len() != 66 {
            return;
        }
        if let Ok(mut m) = self.coord_lookup_backoff.write() {
            m.remove(pk);
        }
    }

    fn is_kept_peer(&self, peer: PeerId) -> bool {
        self.is_dm_contact(peer) || self.is_bootstrap_peer(peer)
    }

    fn mark_incidental_reject(&self, peer: PeerId) {
        if let Ok(mut g) = self.incidental_rejects.write() {
            g.insert(peer);
        }
    }

    fn consume_incidental_reject(&self, peer: PeerId) -> bool {
        self.incidental_rejects
            .write()
            .ok()
            .is_some_and(|mut g| g.remove(&peer))
    }

    fn should_log_bootstrap_dial_err(&self, peer: PeerId, now_ms: i64) -> bool {
        let Ok(mut g) = self.bootstrap_dial_err_log_ms.write() else {
            return true;
        };
        let last = g.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < 30_000 {
            return false;
        }
        g.insert(peer, now_ms);
        true
    }

    fn note_bootstrap_connected(&self) {
        self.any_bootstrap_connected.store(true, Ordering::Relaxed);
    }

    fn should_log_kad_empty_closest(&self, now_ms: i64) -> bool {
        let Ok(mut g) = self.kad_empty_closest_log_ms.write() else {
            return true;
        };
        if now_ms.saturating_sub(*g) < 15_000 {
            return false;
        }
        *g = now_ms;
        true
    }

    fn is_bootstrap_peer(&self, peer: PeerId) -> bool {
        self.bootstrap_peer_ids.contains(&peer)
    }

    /// Someone who added us via QR dials in — accept without a reciprocal contact row.
    ///
    /// Returns `Some(public_key_hex)` when we can immediately learn keys from the libp2p `PeerId`.
    fn register_inbound_dialer_if_needed(
        &self,
        peer: PeerId,
        endpoint: &ConnectedPoint,
    ) -> Option<String> {
        if self.is_kept_peer(peer) {
            return None;
        }
        if !matches!(endpoint, ConnectedPoint::Listener { .. }) {
            return None;
        }
        if self.ensure_dm_peer_from_libp2p(peer) {
            native_log::info(
                "session",
                format!(
                    "accepted inbound dialer {peer} (libp2p identity → DM keys; stream protocol only)"
                ),
            );
            return self.dm_peer_for_libp2p(peer).and_then(|d| d.public_key_hex);
        }
        None
    }

    /// Registered DM contact (invite or inbound dial) — not a public-DHT incidental peer.
    fn is_dm_contact(&self, peer: PeerId) -> bool {
        self.should_dial_libp2p_peer(peer)
    }

    fn should_routed_dial(&self, peer: PeerId, now_ms: i64, min_interval_ms: i64) -> bool {
        let Ok(mut g) = self.routed_dial_attempt_ms.write() else {
            return true;
        };
        let last = g.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < min_interval_ms {
            return false;
        }
        g.insert(peer, now_ms);
        true
    }

    fn log_stream_open_once(&self, peer: PeerId) -> bool {
        let Ok(mut g) = self.stream_open_log_emitted.write() else {
            return true;
        };
        g.insert(peer)
    }

    fn log_kad_not_found_once(&self, peer: PeerId) -> bool {
        let Ok(mut g) = self.kad_not_found_logged.write() else {
            return true;
        };
        g.insert(peer)
    }

    /// Returns true when new dialable addresses were added (caller should republish to DHT).
    fn merge_published_listen(&self, addrs: Vec<Multiaddr>) -> bool {
        let Ok(mut v) = self.published_listen.write() else {
            return false;
        };
        v.retain(|ma| super::dht_bootstrap::is_kad_publish_tcp_multiaddr(ma));
        let before = v.len();
        for ma in addrs {
            if !super::dht_bootstrap::is_kad_publish_tcp_multiaddr(&ma) {
                continue;
            }
            if !v.iter().any(|x| x == &ma) {
                v.push(ma);
            }
        }
        v.len() > before
    }

    fn published_listen_snapshot(&self) -> Vec<Multiaddr> {
        self.published_listen
            .read()
            .ok()
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    /// One debounced DHT publish per address-set change (TCP addrs only).
    fn flush_kad_publish(&self, kad: &mut super::dht_bootstrap::KadBehaviour, local_peer: &PeerId, force: bool) {
        const MIN_INTERVAL_MS: i64 = 5_000;
        let snapshot = self.published_listen_snapshot();
        let tcp = if crate::coord_runtime::wan_discovery_via_coord_only() {
            super::dht_bootstrap::tcp_dm_publish_addrs_coord_mode(snapshot)
        } else {
            super::dht_bootstrap::tcp_dm_publish_addrs(snapshot)
        };
    if tcp.is_empty() {
        native_log::warn(
            "kad",
            format!("publish skipped for {local_peer}: no TCP listen addrs yet"),
        );
        return;
    }
        let fp = super::dht_bootstrap::kad_publish_fingerprint(&tcp);
        let now = chrono_now_ms();
        let Ok(mut last_ms) = self.kad_last_publish_ms.write() else {
            return;
        };
        let Ok(mut prev_fp) = self.kad_publish_fingerprint.write() else {
            return;
        };
        if !force && *prev_fp == fp && now.saturating_sub(*last_ms) < MIN_INTERVAL_MS {
            return;
        }
        *prev_fp = fp;
        *last_ms = now;
        native_log::info(
            "kad",
            format!("publish {local_peer} ({} tcp dial addr(s))", tcp.len()),
        );
        super::dht_bootstrap::kad_publish_peer_record(kad, local_peer, tcp);
    }

    fn try_emit_peer_identified(
        &self,
        peer: PeerId,
        public_key_hex: String,
        events_tx: &Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    ) {
        let first = self
            .identified_emitted
            .write()
            .ok()
            .is_some_and(|mut g| g.insert(peer));
        if !first {
            return;
        }
        if let Some(tx) = events_tx {
            let _ = tx.send(GossipChatEvent::PeerIdentified {
                peer_id: peer,
                public_key_hex,
            });
        }
    }

    fn track_outbound(&self, pending: PendingOutbound) {
        let mut entry = pending;
        // Eligible for resync on the next upkeep tick (do not wait a full resend interval).
        entry.last_send_ms = chrono_now_ms().saturating_sub(OUTBOX_RESEND_INTERVAL_MS);
        if let Ok(mut g) = self.outbox.write() {
            g.insert(entry.message_id.clone(), entry);
        }
    }

    fn complete_outbound(&self, message_id: &str) {
        let id = message_id.trim();
        if id.is_empty() {
            return;
        }
        if let Ok(mut g) = self.outbox.write() {
            g.remove(id);
        }
    }

    fn outbox_due_for_resend(&self, now_ms: i64) -> Vec<PendingOutbound> {
        let Ok(g) = self.outbox.read() else {
            return Vec::new();
        };
        let mut due: Vec<PendingOutbound> = g
            .values()
            .filter(|p| {
                now_ms.saturating_sub(p.last_send_ms) >= OUTBOX_RESEND_INTERVAL_MS
            })
            .cloned()
            .collect();
        due.sort_by_key(|p| p.last_send_ms);
        due
    }

    /// Returns true the first time this message is marked on-wire.
    fn mark_outbox_sent(&self, message_id: &str, now_ms: i64) -> bool {
        let Ok(mut g) = self.outbox.write() else {
            return false;
        };
        if let Some(p) = g.get_mut(message_id) {
            let first_wire = !p.on_wire;
            p.on_wire = true;
            p.last_send_ms = now_ms;
            return first_wire;
        }
        false
    }

    fn mark_outbox_send_failed(&self, message_id: &str, now_ms: i64) {
        let Ok(mut g) = self.outbox.write() else {
            return;
        };
        if let Some(p) = g.get_mut(message_id) {
            p.on_wire = false;
            p.last_send_ms = now_ms;
        }
    }

    fn outbox_contains(&self, message_id: &str) -> bool {
        let id = message_id.trim();
        self.outbox
            .read()
            .ok()
            .is_some_and(|g| g.contains_key(id))
    }

    fn remember_inbound_id(&self, message_id: &str, now_ms: i64) -> bool {
        let id = message_id.trim();
        if id.is_empty() {
            return true;
        }
        let Ok(mut g) = self.seen_inbound_ids.write() else {
            return true;
        };
        if g.contains_key(id) {
            return false;
        }
        if g.len() >= SEEN_INBOUND_MAX {
            let trim = SEEN_INBOUND_MAX / 8;
            let mut oldest: Vec<(String, i64)> = g
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            oldest.sort_by_key(|(_, ts)| *ts);
            for (k, _) in oldest.into_iter().take(trim) {
                g.remove(&k);
            }
        }
        g.insert(id.to_string(), now_ms);
        true
    }

    fn has_seen_inbound_id(&self, message_id: &str) -> bool {
        let id = message_id.trim();
        if id.is_empty() {
            return false;
        }
        self.seen_inbound_ids
            .read()
            .ok()
            .is_some_and(|g| g.contains_key(id))
    }

    fn note_connected(&self, peer: PeerId) {
        if let Ok(mut g) = self.connected.write() {
            g.insert(peer);
        }
    }

    fn note_disconnected(&self, peer: &PeerId) {
        if let Ok(mut g) = self.connected.write() {
            g.remove(peer);
        }
        if let Ok(mut g) = self.chat_ready_emitted.write() {
            g.remove(peer);
        }
    }

    fn connected_peers(&self) -> Vec<PeerId> {
        self.connected
            .read()
            .ok()
            .map(|g| g.iter().copied().collect())
            .unwrap_or_default()
    }

    /// libp2p PeerIds for configured DM contacts (for Kademlia lookups).
    fn dm_peer_ids(&self) -> Vec<PeerId> {
        self.peers
            .read()
            .ok()
            .map(|t| t.by_peer_id.keys().copied().collect())
            .unwrap_or_default()
    }

    fn dm_public_keys(&self) -> Vec<String> {
        self.peers
            .read()
            .ok()
            .map(|t| {
                t.by_peer_id
                    .values()
                    .filter_map(|d| d.public_key_hex.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Only dial mDNS/DHT peers we already know from the invite (never random LAN nodes).
    /// Requires a 66-char secp256k1 `public_key_hex` — never bare `peer_id_only` captures.
    fn should_dial_libp2p_peer(&self, peer: PeerId) -> bool {
        let Ok(tables) = self.peers.read() else {
            return false;
        };
        tables
            .by_peer_id
            .get(&peer)
            .is_some_and(|dm| dm.has_send_keys())
    }

    /// Target PeerId to open `/ghal-bol/msg/1.0.0` for this contact.
    ///
    /// Always derived from the 66-hex secp256k1 public key. A stale
    /// `peer_id` stored beside the key must never override the cryptographic identity.
    fn resolve_send_peer(&self, signing_pk_hex: &str) -> Option<PeerId> {
        let pk = signing_pk_hex.trim();
        if pk.len() != 66 {
            return None;
        }
        let peer_id: PeerId = peer_id_from_secp256k1_public_key_hex(pk)
            .ok()
            .and_then(|s| s.parse().ok())?;
        self.ensure_dm_peer(pk, peer_id);
        Some(peer_id)
    }

    /// Fill `public_key_hex` from libp2p PeerId when this network uses secp256k1 identities.
    fn ensure_dm_peer_from_libp2p(&self, peer: PeerId) -> bool {
        if self
            .dm_peer_for_libp2p(peer)
            .is_some_and(|d| d.has_send_keys())
        {
            return true;
        }
        let Some(pk) = secp256k1_public_key_hex_from_peer_id(&peer) else {
            // Ed25519 relay/DHT peers must never become DM rows (no `/ghal-bol/msg/1.0.0`).
            return false;
        };
        self.ensure_dm_peer(&pk, peer);
        true
    }

    fn register_dm_peer_key(&self, peer_id_hint: Option<PeerId>, public_key_hex: &str) {
        let pk = public_key_hex.trim();
        if pk.len() != 66 {
            if let Some(pid) = peer_id_hint {
                self.ensure_dm_peer_from_libp2p(pid);
            }
            return;
        }
        let Some(derived) = peer_id_from_secp256k1_public_key_hex(pk)
            .ok()
            .and_then(|s| s.parse::<PeerId>().ok())
        else {
            return;
        };
        if let Some(hint) = peer_id_hint {
            if hint != derived {
                native_log::warn(
                    "session",
                    format!("dm peer id corrected {hint} -> {derived} (public key is authoritative)"),
                );
            }
        }
        self.ensure_dm_peer(pk, derived);
        self.purge_invalid_dm_peer_ids();
    }

    fn ensure_dm_peer(&self, public_key_hex: &str, libp2p_peer: PeerId) {
        let pk = public_key_hex.trim();
        if pk.len() != 66 {
            return;
        }
        if !secp256k1_public_hex_matches_peer_id(pk, &libp2p_peer) {
            native_log::warn(
                "session",
                format!("reject dm keys for {libp2p_peer}: public key does not match peer id"),
            );
            return;
        }
        let mut tables = match self.peers.write() {
            Ok(g) => g,
            Err(_) => return,
        };
        let stale: Vec<PeerId> = tables
            .by_peer_id
            .iter()
            .filter(|(pid, dm)| {
                **pid != libp2p_peer && dm.public_key_hex.as_deref() == Some(pk)
            })
            .map(|(pid, _)| *pid)
            .collect();
        for pid in stale {
            tables.by_peer_id.remove(&pid);
        }
        tables.retain_invalid_dm_peer_ids();
        let entry = tables
            .by_peer_id
            .entry(libp2p_peer)
            .or_insert_with(|| DmPeer::peer_id_only(libp2p_peer));
        entry.public_key_hex = Some(pk.to_string());
    }

    /// Drop `peer_id_only` rows and non-secp256k1 libp2p ids left from old inbound captures.
    fn purge_invalid_dm_peer_ids(&self) {
        let Ok(mut tables) = self.peers.write() else {
            return;
        };
        tables.retain_invalid_dm_peer_ids();
    }

    fn dm_peer(&self, signing_pk_hex: &str) -> Option<DmPeer> {
        let pk = signing_pk_hex.trim();
        let tables = self.peers.read().ok()?;
        tables.by_peer_id.values().find_map(|dm| {
            if dm.public_key_hex.as_deref() == Some(pk) {
                Some(dm.clone())
            } else {
                None
            }
        })
    }

    fn dm_peer_for_libp2p(&self, peer: PeerId) -> Option<DmPeer> {
        self.peers.read().ok()?.by_peer_id.get(&peer).cloned()
    }

    fn dm_peer_for_conversation_key(&self, key: &str) -> Option<DmPeer> {
        let key = key.trim();
        if key.len() == 66 {
            return self.dm_peer(key);
        }
        if let Ok(pid) = key.parse::<PeerId>() {
            return self.dm_peer_for_libp2p(pid);
        }
        None
    }

    fn set_foreground_peer(&self, peer: Option<PeerId>) {
        if let Ok(mut g) = self.foreground_peer.write() {
            *g = peer;
        }
        let pk = peer.and_then(|p| secp256k1_public_key_hex_from_peer_id(&p));
        sync_foreground_peer_now(pk);
    }

    fn current_foreground_peer(&self) -> Option<PeerId> {
        self.foreground_peer.read().ok().and_then(|g| *g)
    }

    fn is_foreground_peer(&self, peer: PeerId) -> bool {
        self.current_foreground_peer().is_some_and(|f| f == peer)
    }

    fn pending_read_ack_len(&self) -> usize {
        self.pending_read_acks
            .read()
            .map(|q| q.len())
            .unwrap_or(0)
    }

    fn pending_delivery_ack_len(&self) -> usize {
        self.pending_delivery_acks
            .read()
            .map(|q| q.len())
            .unwrap_or(0)
    }

    fn enqueue_read_ack(&self, peer_id: PeerId, inbound_id: &str, recipient_signing: &str) {
        let id = inbound_id.trim().to_string();
        if id.is_empty() {
            return;
        }
        if self.is_read_ack_confirmed(&id) {
            return;
        }
        let Ok(mut q) = self.pending_read_acks.write() else {
            return;
        };
        if q.len() >= MAX_PENDING_READ_ACKS {
            q.pop_front();
        }
        if q.iter().any(|p| p.inbound_id == id) {
            return;
        }
        q.push_back(PendingReadAck {
            peer_id,
            inbound_id: id,
            recipient_public_key_hex: recipient_signing.trim().to_string(),
        });
    }

    fn is_read_ack_confirmed(&self, inbound_id: &str) -> bool {
        let id = inbound_id.trim();
        if id.is_empty() {
            return false;
        }
        self.read_ack_confirmed
            .read()
            .ok()
            .is_some_and(|s| s.contains(id))
    }

    fn has_pending_read_ack(&self, inbound_id: &str) -> bool {
        let id = inbound_id.trim();
        if id.is_empty() || self.is_read_ack_confirmed(id) {
            return false;
        }
        self.pending_read_acks
            .read()
            .ok()
            .is_some_and(|q| q.iter().any(|p| p.inbound_id == id))
    }

    fn mark_read_ack_confirmed(&self, inbound_id: &str) {
        let id = inbound_id.trim();
        if id.is_empty() || self.is_read_ack_confirmed(id) {
            return;
        }
        if let Ok(mut s) = self.read_ack_confirmed.write() {
            s.insert(id.to_string());
            if s.len() > SEEN_INBOUND_MAX {
                let trim = SEEN_INBOUND_MAX / 8;
                let mut keys: Vec<String> = s.iter().cloned().collect();
                keys.sort();
                for k in keys.into_iter().take(trim) {
                    s.remove(&k);
                }
            }
        }
        if let Ok(mut q) = self.pending_read_acks.write() {
            q.retain(|p| p.inbound_id != id);
        }
        if let (Some(path), Some(ns)) = (&self.transcript_path, &self.app_namespace) {
            let path = path.trim();
            let ns = ns.trim();
            if !path.is_empty() && !ns.is_empty() {
                let _ = crate::dm_transcript_v1::mark_inbound_read_ack_sent(
                    Path::new(path),
                    ns,
                    id,
                );
            }
        }
    }

    /// Queued read receipts (from in-room or post-enter backlog) — retried until sender confirms.
    fn read_acks_due_for_upkeep(&self, limit: usize) -> Vec<PendingReadAck> {
        let Ok(q) = self.pending_read_acks.read() else {
            return Vec::new();
        };
        let confirmed = self.read_ack_confirmed.read().ok();
        let mut out = Vec::new();
        for item in q.iter() {
            if out.len() >= limit {
                break;
            }
            if confirmed
                .as_ref()
                .is_some_and(|s| s.contains(&item.inbound_id))
            {
                continue;
            }
            out.push(item.clone());
        }
        out
    }

    fn enqueue_delivery_ack(&self, peer_id: PeerId, inbound_id: &str, recipient_signing: &str) {
        let id = inbound_id.trim().to_string();
        if id.is_empty() {
            return;
        }
        let Ok(mut q) = self.pending_delivery_acks.write() else {
            return;
        };
        if q.len() >= MAX_PENDING_READ_ACKS {
            q.pop_front();
        }
        if q.iter().any(|p| p.inbound_id == id) {
            return;
        }
        q.push_back(PendingDeliveryAck {
            peer_id,
            inbound_id: id,
            recipient_public_key_hex: recipient_signing.trim().to_string(),
        });
    }

    fn dequeue_delivery_ack(&self, inbound_id: &str) {
        let id = inbound_id.trim();
        if id.is_empty() {
            return;
        }
        if let Ok(mut q) = self.pending_delivery_acks.write() {
            q.retain(|p| p.inbound_id != id);
        }
    }

    fn is_delivery_ack_sent(&self, inbound_id: &str) -> bool {
        let id = inbound_id.trim();
        if id.is_empty() {
            return false;
        }
        self.delivery_ack_sent
            .read()
            .ok()
            .is_some_and(|s| s.contains(id))
    }

    fn mark_delivery_ack_sent(&self, inbound_id: &str) {
        let id = inbound_id.trim().to_string();
        if id.is_empty() {
            return;
        }
        let Ok(mut s) = self.delivery_ack_sent.write() else {
            return;
        };
        if s.len() >= SEEN_INBOUND_MAX {
            let trim = SEEN_INBOUND_MAX / 8;
            let drop: Vec<String> = s.iter().take(trim).cloned().collect();
            for k in drop {
                s.remove(&k);
            }
        }
        s.insert(id);
    }

    fn delivery_acks_due_for_upkeep(&self, limit: usize) -> Vec<PendingDeliveryAck> {
        let Ok(q) = self.pending_delivery_acks.read() else {
            return Vec::new();
        };
        q.iter().take(limit).cloned().collect()
    }

    fn enqueue_pending_call_signal(&self, item: PendingCallSignal) {
        const MAX: usize = 128;
        let Ok(mut q) = self.pending_call_signals.write() else {
            return;
        };
        if q.len() >= MAX {
            q.pop_front();
        }
        q.push_back(item);
    }

    fn drain_pending_call_signals(&self, limit: usize) -> Vec<PendingCallSignal> {
        let Ok(mut q) = self.pending_call_signals.write() else {
            return Vec::new();
        };
        let n = limit.min(q.len());
        q.drain(0..n).collect()
    }

    fn requeue_pending_call_signal_front(&self, item: PendingCallSignal) {
        const MAX: usize = 128;
        let Ok(mut q) = self.pending_call_signals.write() else {
            return;
        };
        if q.len() >= MAX {
            return;
        }
        q.push_front(item);
    }

}

fn emit_chat_ready_if_can_send(
    session: Arc<SessionState>,
    peer: PeerId,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if !session.is_dm_contact(peer) || !writer_open_for_peer(&writers, peer) {
        return;
    }
    let first = session
        .chat_ready_emitted
        .write()
        .ok()
        .is_some_and(|mut g| g.insert(peer));
    if !first {
        return;
    }
    if let Some(tx) = events_tx.clone() {
        let _ = tx.send(GossipChatEvent::ChatReady { peer_id: peer });
    }
    let session2 = Arc::clone(&session);
    let writers2 = Arc::clone(&writers);
    tokio::spawn(async move {
        if let (Some(path), Some(ns)) = (
            &session2.transcript_path,
            &session2.app_namespace,
        ) {
            transcript_sync_outbound_tick(session2.as_ref(), Path::new(path), ns.trim());
        }
        resync_pending_outbox(
            session2.clone(),
            writers2.clone(),
            vec![peer],
            events_tx.clone(),
            None,
        )
        .await;
        flush_pending_call_signals(
            session2.clone(),
            Arc::clone(&writers2),
            vec![peer],
            events_tx.clone(),
        )
        .await;
        // Fast read-receipt catch-up when this peer is the open chat (not for background connects).
        if app_ack_read_enabled() && session2.is_foreground_peer(peer) {
            read_ack_catchup_for_peer(session2.clone(), writers2.clone(), peer, false, true).await;
        }
        let first_replay = session2
            .history_replay_done
            .write()
            .ok()
            .is_some_and(|mut g| g.insert(peer));
        if first_replay {
            tokio::time::sleep(Duration::from_secs(2)).await;
            replay_conversation_history(session2, writers2, peer).await;
        }
    });
}

/// After connect, resend transcript rows still marked pending (does not flood delivered history).
async fn replay_conversation_history(
    session: Arc<SessionState>,
    writers: StreamWriters,
    peer: PeerId,
) {
    let (path, ns) = match (&session.transcript_path, &session.app_namespace) {
        (Some(p), Some(n)) if !p.trim().is_empty() && !n.trim().is_empty() => {
            (p.clone(), n.trim().to_string())
        }
        _ => return,
    };
    if !writer_open_for_peer(&writers, peer) {
        return;
    }
    let dm = match session.dm_peer_for_libp2p(peer) {
        Some(d) => d,
        None => return,
    };
    let recipient_pk = match dm.public_key_hex.as_deref() {
        Some(s) if s.len() == 66 => s,
        _ => return,
    };
    let Ok(rows) = crate::dm_transcript_v1::pending_outbound_rows(Path::new(&path), &ns) else {
        return;
    };
    let peer_s = peer.to_string();
    let mut sent = 0usize;
    for row in rows {
        let ck = row.conversation_key.as_str();
        if ck != peer_s && ck != recipient_pk {
            continue;
        }
        if !writer_open_for_peer(&writers, peer) {
            break;
        }
        let pending = PendingOutbound {
            message_id: row.message_id.clone(),
            peer_id: peer,
            recipient_public_key_hex: recipient_pk.to_string(),
            text: row.text.clone(),
            created_at_ms: if row.created_at_ms > 0 {
                row.created_at_ms
            } else {
                chrono_now_ms()
            },
            last_send_ms: chrono_now_ms(),
            on_wire: false,
        };
        let Ok(frame) = build_pending_outbound_frame(session.as_ref(), &pending) else {
            continue;
        };
        if send_frame_to_peer(peer, frame, Arc::clone(&writers))
            .await
            .is_ok()
        {
            sent += 1;
        }
        tokio::time::sleep(Duration::from_millis(HISTORY_REPLAY_SPACING_MS)).await;
    }
    if sent > 0 {
        native_log::debug(
            "history",
            format!("replayed {sent} pending outbound line(s) to {peer}"),
        );
    }
}

fn writer_open_for_peer(writers: &StreamWriters, peer: PeerId) -> bool {
    writers
        .lock()
        .ok()
        .is_some_and(|g| g.contains_key(&peer))
}

type StreamWriters = Arc<Mutex<HashMap<PeerId, mpsc::UnboundedSender<Vec<u8>>>>>;

fn send_frame_on_open_stream(
    peer: PeerId,
    frame: Vec<u8>,
    writers: &StreamWriters,
) -> Result<(), String> {
    let tx = {
        let g = writers
            .lock()
            .map_err(|_| "writers mutex poisoned".to_string())?;
        g.get(&peer).cloned()
    };
    let Some(tx) = tx else {
        return Err(
            "no chat stream to peer yet — wait until connected".to_string(),
        );
    };
    tx.send(frame)
        .map_err(|_| "chat stream closed".to_string())
}

#[derive(NetworkBehaviour)]
pub struct ChatBehaviour {
    pub relay: libp2p::relay::client::Behaviour,
    pub dcutr: libp2p::dcutr::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub autonat: libp2p::autonat::Behaviour,
    pub upnp: Toggle<libp2p::upnp::tokio::Behaviour>,
    pub mdns: Toggle<libp2p::mdns::tokio::Behaviour>,
    pub kademlia: super::dht_bootstrap::KadBehaviour,
    pub stream: stream::Behaviour,
}

fn chat_behaviour(
    key: &libp2p::identity::Keypair,
    relay: libp2p::relay::client::Behaviour,
) -> ChatBehaviour {
    let local_peer_id = key.public().to_peer_id();
    let identify_cfg = libp2p::identify::Config::new_with_signed_peer_record(
        "/ghal-bol/1.0.0".to_string(),
        key,
    )
    .with_agent_version(format!("ghal_bol/{}", env!("CARGO_PKG_VERSION")))
    .with_push_listen_addr_updates(true);
    let mdns = match libp2p::mdns::tokio::Behaviour::new(
        libp2p::mdns::Config::default(),
        local_peer_id,
    ) {
        Ok(b) => {
            native_log::info("mdns", "enabled");
            Toggle::from(Some(b))
        }
        Err(e) => {
            native_log::warn("mdns", format!("disabled: {e}"));
            Toggle::from(None)
        }
    };
    let upnp = Toggle::from(Some(libp2p::upnp::tokio::Behaviour::default()));
    native_log::info(
        "p2p",
        "behaviours: relay+dcutr+identify+autonat+upnp+mdns+kad",
    );
    ChatBehaviour {
        relay,
        dcutr: libp2p::dcutr::Behaviour::new(local_peer_id),
        identify: libp2p::identify::Behaviour::new(identify_cfg),
        autonat: libp2p::autonat::Behaviour::new(local_peer_id, libp2p::autonat::Config::default()),
        upnp,
        mdns,
        kademlia: new_kademlia_behaviour(local_peer_id),
        stream: stream::Behaviour::new(),
    }
}

/// Shorter on Android so dead Wi‑Fi TCP does not block bootstrap redial for minutes.
#[cfg(target_os = "android")]
const SWARM_IDLE_CONNECTION_TIMEOUT_SECS: u64 = 45;

#[cfg(not(target_os = "android"))]
const SWARM_IDLE_CONNECTION_TIMEOUT_SECS: u64 = 300;

/// Phones: TCP+noise only (no QUIC/TLS stack) — avoids common Android libp2p build failures.
#[cfg(target_os = "android")]
fn build_swarm(config: &GossipChatConfig) -> Result<Swarm<ChatBehaviour>, ChatServerError> {
    native_log::info("p2p", "swarm transport: android tcp+noise");
    let swarm = SwarmBuilder::with_existing_identity(config.keypair.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| ChatServerError::Transport(format!("tcp: {e}")))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| ChatServerError::Transport(format!("relay client: {e}")))?
        .with_behaviour(|key, relay| Ok(chat_behaviour(key, relay)))
        .map_err(|e| ChatServerError::Transport(format!("behaviour: {e}")))?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(Duration::from_secs(
                SWARM_IDLE_CONNECTION_TIMEOUT_SECS,
            ))
        })
        .build();
    Ok(swarm)
}

#[cfg(not(target_os = "android"))]
fn build_swarm(config: &GossipChatConfig) -> Result<Swarm<ChatBehaviour>, ChatServerError> {
    // TCP uses noise only (same as Android) so phones and desktop can DM on LAN/coord TCP.
    native_log::info("p2p", "swarm transport: tcp+noise+quic");
    let swarm = SwarmBuilder::with_existing_identity(config.keypair.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| ChatServerError::Transport(e.to_string()))?
        .with_quic()
        .with_dns()
        .map_err(|e| ChatServerError::Transport(format!("dns: {e}")))?
        .with_relay_client(
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| ChatServerError::Transport(format!("relay client: {e}")))?
        .with_behaviour(|key, relay| Ok(chat_behaviour(key, relay)))
        .map_err(|e| ChatServerError::Transport(e.to_string()))?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(Duration::from_secs(
                SWARM_IDLE_CONNECTION_TIMEOUT_SECS,
            ))
        })
        .build();
    Ok(swarm)
}

fn listen_swarm_transports(swarm: &mut Swarm<ChatBehaviour>) -> Result<(), ChatServerError> {
    listen_ephemeral(swarm, "/ip4/0.0.0.0/tcp/0")?;
    #[cfg(not(target_os = "android"))]
    {
        listen_ephemeral(swarm, "/ip4/0.0.0.0/udp/0/quic-v1")?;
        listen_ephemeral(swarm, "/ip6/::/udp/0/quic-v1")?;
        listen_ephemeral(swarm, "/ip6/::/tcp/0")?;
    }
    Ok(())
}

fn parse_ma(s: &str) -> Result<Multiaddr, ChatServerError> {
    s.parse()
        .map_err(|e: libp2p::multiaddr::Error| ChatServerError::Multiaddr(e.to_string()))
}

fn dial_opts_peer_hint(ma: &Multiaddr) -> Option<PeerId> {
    peer_id_from_multiaddr(ma)
}

/// Prefer TCP on LAN; QUIC bootstrap/mDNS dials often time out on phones.
pub(crate) fn is_tcp_multiaddr(ma: &Multiaddr) -> bool {
    ma.iter().any(|p| matches!(p, Protocol::Tcp(_)))
}

pub(crate) fn is_quic_multiaddr(ma: &Multiaddr) -> bool {
    ma.to_string().contains("quic-v1")
}

/// Prefer TCP relay reservation addresses (Android DM transport is TCP-only).
fn tcp_relay_reservation_addr(relay: PeerId, relay_addr: &Multiaddr) -> Option<Multiaddr> {
    if is_tcp_multiaddr(relay_addr) && !is_quic_multiaddr(relay_addr) {
        return Some(relay_addr.clone());
    }
    for p in relay_addr.iter() {
        match p {
            Protocol::Ip4(ip) => {
                let s = format!("/ip4/{ip}/tcp/4001/p2p/{relay}");
                if let Ok(ma) = s.parse::<Multiaddr>() {
                    return Some(ma);
                }
            }
            Protocol::Ip6(ip) => {
                let s = format!("/ip6/{ip}/tcp/4001/p2p/{relay}");
                if let Ok(ma) = s.parse::<Multiaddr>() {
                    return Some(ma);
                }
            }
            _ => {}
        }
    }
    None
}

/// One dial per bootstrap peer via peerstore (DNS/tcp multiaddrs).
fn dial_public_dht_bootnodes(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    nodes: &[(PeerId, Multiaddr)],
) {
    for (peer, ma) in nodes {
        if swarm.is_connected(peer) {
            continue;
        }
        if !super::dht_bootstrap::is_trusted_bootstrap_dial_addr(ma) {
            continue;
        }
        swarm
            .behaviour_mut()
            .kademlia
            .add_address(peer, ma.clone());
        let first_log = session
            .dht_bootstrap_dial_logged
            .write()
            .ok()
            .is_some_and(|mut g| g.insert(*peer));
        if first_log {
            native_log::info("dial", format!("public DHT bootstrap {peer} via {ma}"));
        }
        if let Err(e) = swarm.dial(ma.clone()) {
            native_log::debug("dial", format!("public DHT bootstrap {peer} {ma}: {e}"));
        }
    }
}

/// After a network handover: drop zombie bootstrap TCP (was blocking redial for up to idle timeout).
fn redial_public_dht_bootnodes(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    nodes: &[(PeerId, Multiaddr)],
) {
    session
        .any_bootstrap_connected
        .store(false, Ordering::Relaxed);
    for (peer, ma) in nodes {
        if !super::dht_bootstrap::is_trusted_bootstrap_dial_addr(ma) {
            continue;
        }
        if swarm.is_connected(peer) {
            let _ = swarm.disconnect_peer_id(*peer);
            session.note_disconnected(peer);
        }
        swarm
            .behaviour_mut()
            .kademlia
            .add_address(peer, ma.clone());
        native_log::info("dial", format!("public DHT bootstrap redial {peer} via {ma}"));
        if let Err(e) = swarm.dial(ma.clone()) {
            native_log::debug("dial", format!("public DHT bootstrap redial {peer} {ma}: {e}"));
        }
    }
}

fn disconnect_peers_for_handover(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    let mut peers: HashSet<PeerId> = HashSet::new();
    peers.extend(session.dm_peer_ids());
    peers.extend(session.bootstrap_peer_ids.iter().copied());
    if let Ok(g) = session.connected.read() {
        peers.extend(g.iter().copied());
    }
    for peer in peers {
        if swarm.is_connected(&peer) {
            let _ = swarm.disconnect_peer_id(peer);
        }
        session.note_disconnected(&peer);
    }
    session
        .any_bootstrap_connected
        .store(false, Ordering::Relaxed);
    if let Ok(mut g) = session.dht_bootstrap_dial_logged.write() {
        g.clear();
    }
}

/// Request a relay reservation on a connected bootstrap (NAT traversal for phones).
fn try_relay_reservation(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
    relay_addr: &Multiaddr,
    force: bool,
) {
    if !session.is_bootstrap_peer(relay) || relay_addr.to_string().contains("/p2p-circuit") {
        return;
    }
    let now_ms = chrono_now_ms();
    // If we are already listening on this circuit, do not re-issue listens.
    let already_listening = swarm.listeners().any(|ma| {
        ma.to_string().contains("/p2p-circuit")
            && ma.to_string().contains(&format!("/p2p/{relay}"))
    });
    if already_listening {
        return;
    }
    // Throttle repeated listen attempts per relay. 1s storms significantly degrade performance.
    if !force {
        if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
            if let Some(last) = m.get(&relay).copied() {
                if now_ms.saturating_sub(last) < 10_000 {
                    return;
                }
            }
            m.insert(relay, now_ms);
        }
    } else if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
        m.insert(relay, now_ms);
    }

    let Ok(mut g) = session.relay_reserve_requested.write() else {
        return;
    };
    if !force && !g.insert(relay) {
        return;
    }
    g.insert(relay);
    drop(g);
    let Some(mut listen_ma) = tcp_relay_reservation_addr(relay, relay_addr) else {
        native_log::warn(
            "relay",
            format!("no TCP reservation addr for {relay} from {relay_addr}"),
        );
        return;
    };
    if !listen_ma.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
        if !listen_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
            listen_ma.push(Protocol::P2p(relay));
        }
        listen_ma.push(Protocol::P2pCircuit);
    }
    match swarm.listen_on(listen_ma.clone()) {
        Ok(_) => native_log::info("relay", format!("reserving circuit on {relay} via {listen_ma}")),
        Err(e) => native_log::warn("relay", format!("relay reserve listen {listen_ma}: {e}")),
    }
}

/// Poll/UI only needs TCP dialable listen addrs (LAN or relay circuit), not every relay transport variant.
fn should_emit_listening_event(addr: &Multiaddr) -> bool {
    super::dht_bootstrap::is_kad_publish_tcp_multiaddr(addr)
        || super::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr(addr)
}

/// Coord registration must be based on what libp2p is *actually* listening on.
/// Using only the cached `published_listen` snapshot can temporarily drop relay circuits
/// during churn, which makes coord think we have "no endpoints" and flaps registration.
fn coord_register_listen_snapshot(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
) -> Vec<Multiaddr> {
    let mut out = session.published_listen_snapshot();
    for ma in swarm.listeners() {
        if !out.iter().any(|e| e == ma) {
            out.push(ma.clone());
        }
    }
    out
}

/// Drop stale relay/LAN listen addrs after a network handover.
fn clear_wan_listen_state_for_handover(session: &SessionState) {
    if let Ok(mut v) = session.published_listen.write() {
        v.retain(|ma| !super::dht_bootstrap::is_relay_circuit_multiaddr(ma));
        if crate::coord_runtime::wan_discovery_via_coord_only() {
            v.retain(|ma| {
                !super::dht_bootstrap::ipv4_from_ma_str(&ma.to_string())
                    .is_some_and(|ip| ip.is_private())
            });
        }
    }
    if let Ok(mut g) = session.relay_reserve_requested.write() {
        g.clear();
    }
    if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
        m.clear();
    }
    if let Ok(mut fp) = session.kad_publish_fingerprint.write() {
        fp.clear();
    }
    if let Ok(mut last) = session.kad_last_publish_ms.write() {
        *last = 0;
    }
}

fn listen_ready_for_node(
    session: &SessionState,
    coord_mode: bool,
    swarm: &Swarm<ChatBehaviour>,
) -> bool {
    if coord_mode {
        return swarm
            .listeners()
            .any(super::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr);
    }
    let snap = session.published_listen_snapshot();
    if snap
        .iter()
        .any(super::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr)
    {
        return true;
    }
    !super::dht_bootstrap::tcp_dm_publish_addrs(snap).is_empty()
}

fn try_wan_relay_recovery(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return;
    }
    if listen_ready_for_node(session, true, swarm) {
        return;
    }
    retry_stalled_relay_reservations(swarm, session, false);
}

fn wan_recovery_satisfied(session: &SessionState, swarm: &Swarm<ChatBehaviour>) -> bool {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return true;
    }
    listen_ready_for_node(session, true, swarm) && crate::coord_runtime::coord_is_registered()
}

fn finish_wan_recovery_if_ready(session: &SessionState, swarm: &Swarm<ChatBehaviour>) {
    if !session.wan_recovery_active.load(Ordering::Relaxed) {
        return;
    }
    if wan_recovery_satisfied(session, swarm) {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        native_log::info("net", "WAN recovery complete — relay circuit + coord registered");
    }
}

fn run_wan_recovery_pass(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    public_dht: &[(PeerId, Multiaddr)],
) {
    if !session.wan_recovery_active.load(Ordering::Relaxed) {
        return;
    }
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        return;
    }
    if !listen_ready_for_node(session, true, swarm) {
        session.refresh_bootstrap_connected_flag(swarm);
        let started = session
            .wan_recovery_started_ms
            .read()
            .ok()
            .map(|t| *t)
            .unwrap_or(0);
        let stale_bootstrap = session.any_bootstrap_connected.load(Ordering::Relaxed)
            && chrono_now_ms().saturating_sub(started) >= WAN_RECOVERY_BOOTSTRAP_STALE_MS;
        if !session.any_bootstrap_connected.load(Ordering::Relaxed) || stale_bootstrap {
            if stale_bootstrap {
                native_log::info(
                    "net",
                    "WAN recovery: bootstrap connected but no relay — forcing bootstrap redial",
                );
                if let Ok(mut t) = session.wan_recovery_started_ms.write() {
                    *t = chrono_now_ms();
                }
            }
            redial_public_dht_bootnodes(swarm, session, public_dht);
        } else {
            retry_stalled_relay_reservations(swarm, session, true);
        }
    }
    let listen = coord_register_listen_snapshot(swarm, session);
    crate::coord_runtime::coord_register_tick(&listen);
    finish_wan_recovery_if_ready(session, swarm);
}

fn handle_network_path_change(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    public_dht: &[(PeerId, Multiaddr)],
    old_mode: &str,
    new_mode: &str,
) {
    native_log::info("net", format!("network path changed {old_mode} -> {new_mode}"));

    // Conservative handover:
    // - LAN/Wi‑Fi should NOT be disrupted by aggressive WAN recovery.
    // - Only perform WAN reset when coord is configured and we are on a mobile/CGNAT path.
    let net = session.network_profile_snapshot();
    let coord_only = crate::coord_runtime::wan_discovery_via_coord_only();
    if !coord_only || net.has_active_lan() {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        // Still rebuild endpoints based on current listens (e.g. UPnP/autonat changes).
        crate::coord_runtime::rebuild_coord_endpoints_from_listen(
            &session.published_listen_snapshot(),
        );
        return;
    }

    native_log::info("net", "WAN handover: resetting relay/coord state");
    clear_wan_listen_state_for_handover(session);
    crate::coord_runtime::coord_invalidate_presence_on_network_change();
    crate::coord_runtime::rebuild_coord_endpoints_from_listen(&session.published_listen_snapshot());

    // Do not tear down DM streams unless we have to — only drop bootstrap/Zombie TCP to relays.
    for peer in session.bootstrap_peer_ids.iter().copied() {
        if swarm.is_connected(&peer) {
            let _ = swarm.disconnect_peer_id(peer);
        }
        session.note_disconnected(&peer);
    }
    session.any_bootstrap_connected.store(false, Ordering::Relaxed);

    session.begin_wan_recovery();
    redial_public_dht_bootnodes(swarm, session, public_dht);
    retry_stalled_relay_reservations(swarm, session, true);
}

/// When coord is set, WAN DM needs a relay circuit. Reservations can stall; retry on connected bootstraps.
fn retry_stalled_relay_reservations(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    force: bool,
) {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return;
    }
    if !force && listen_ready_for_node(session, true, swarm) {
        return;
    }
    session.refresh_bootstrap_connected_flag(swarm);
    if !session.any_bootstrap_connected.load(Ordering::Relaxed) {
        return;
    }
    let addrs: Vec<(PeerId, Multiaddr)> = session
        .bootstrap_relay_addr
        .read()
        .ok()
        .map(|m| {
            m.iter()
                .filter(|(p, _)| swarm.is_connected(p))
                .map(|(p, a)| (*p, a.clone()))
                .collect()
        })
        .unwrap_or_default();
    if addrs.is_empty() {
        return;
    }
    native_log::info(
        "relay",
        format!(
            "retry relay reservation on {} bootstrap(s) (no dialable listen addr yet)",
            addrs.len()
        ),
    );
    for (peer, addr) in addrs {
        try_relay_reservation(swarm, session, peer, &addr, force);
    }
}

fn dial_bootstrap_peers(
    swarm: &mut Swarm<ChatBehaviour>,
    peers: &[Multiaddr],
    emit: &mut dyn FnMut(GossipChatEvent),
) {
    let mut tcp_first: Vec<Multiaddr> = Vec::new();
    let mut other: Vec<Multiaddr> = Vec::new();
    for ma in peers {
        if ma.is_empty() {
            continue;
        }
        if is_tcp_multiaddr(ma) {
            tcp_first.push(ma.clone());
        } else if !is_quic_multiaddr(ma) {
            other.push(ma.clone());
        }
    }
    for ma in tcp_first.iter().chain(other.iter()) {
        native_log::debug("dial", format!("bootstrap dial {ma}"));
        if let Err(e) = swarm.dial(ma.clone()) {
            native_log::warn("dial", format!("bootstrap dial failed {ma}: {e}"));
            emit(GossipChatEvent::DialFailed {
                peer: dial_opts_peer_hint(ma),
                error: format!("{e}"),
            });
        }
    }
}

fn listen_ephemeral(swarm: &mut Swarm<ChatBehaviour>, ma: &str) -> Result<(), ChatServerError> {
    let parsed = parse_ma(ma)?;
    match swarm.listen_on(parsed) {
        Ok(_) => Ok(()),
        Err(e) => {
            native_log::warn("listen", format!("listen skipped ({ma}): {e}"));
            Ok(())
        }
    }
}

fn new_msg_id() -> String {
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

/// Exposed for FFI so Dart can correlate delivery acks with outbound bubbles.
pub fn new_msg_id_for_ffi() -> String {
    new_msg_id()
}

async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("read length: {e}"))?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 4 * 1024 * 1024 {
        return Err("frame too large".to_string());
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("read body: {e}"))?;
    let mut frame = Vec::with_capacity(4 + len);
    frame.extend_from_slice(&len_buf);
    frame.extend_from_slice(&body);
    Ok(frame)
}

async fn write_frame<W: AsyncWriteExt + Unpin>(writer: &mut W, frame: &[u8]) -> Result<(), String> {
    if frame.len() < 4 {
        return Err("frame too short".to_string());
    }
    writer
        .write_all(frame)
        .await
        .map_err(|e| format!("write frame: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

async fn send_frame_to_peer(
    peer: PeerId,
    frame: Vec<u8>,
    writers: StreamWriters,
) -> Result<(), String> {
    send_frame_on_open_stream(peer, frame, &writers)
}

fn stream_read_is_terminal(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("unexpected eof")
        || e.contains("early eof")
        || e.contains("connection reset")
        || e.contains("broken pipe")
        || e.contains("stream closed")
        || e.contains("closed by remote")
}

/// One long-lived `/ghal-bol/msg/1.0.0` per libp2p peer (read/write until reset).
async fn handle_inbound_stream(
    peer: PeerId,
    stream: libp2p::Stream,
    session: Arc<SessionState>,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    _control: stream::Control,
) {
    if session.is_bootstrap_peer(peer) {
        return;
    }
    let (mut reader, writer) = stream.split();
    let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let owns_writer = {
        let mut owns = false;
        if let Ok(mut g) = writers.lock() {
            if !g.contains_key(&peer) {
                g.insert(peer, tx);
                owns = true;
            }
        }
        owns
    };
    let write_task = if owns_writer {
        emit_chat_ready_if_can_send(Arc::clone(&session), peer, Arc::clone(&writers), events_tx.clone());
        session.ensure_dm_peer_from_libp2p(peer);
        if let Some(pk) = session
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
        {
            session.try_emit_peer_identified(peer, pk, &events_tx);
        }
        let mut writer = writer;
        let writers_w = Arc::clone(&writers);
        Some(tokio::spawn(async move {
            let mut rx = rx;
            while let Some(frame) = rx.recv().await {
                if write_frame(&mut writer, &frame).await.is_err() {
                    break;
                }
            }
            if let Ok(mut g) = writers_w.lock() {
                g.remove(&peer);
            }
        }))
    } else {
        drop(writer);
        None
    };

    let my_public = session.my_public_key_hex.clone();
    let my_secret = session.identity.secp256k1_secret();

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(f) => f,
            Err(e) => {
                if stream_read_is_terminal(&e) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
                continue;
            }
        };
        let share = match frame_wire_share(&frame) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if share == CALL_SHARE {
            let env = match call_envelope_from_frame(&frame) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let parsed = match parse_call_envelope(&env, &my_public, &my_secret) {
                Ok(p) => p,
                Err(e) => {
                    native_log::warn("call", format!("drop call frame from {peer}: {e}"));
                    continue;
                }
            };
            if !secp256k1_public_hex_matches_peer_id(&parsed.sender_public_key_hex, &peer) {
                native_log::warn(
                    "call",
                    format!("drop call from {peer}: signing key mismatch"),
                );
                continue;
            }
            if let Err(e) =
                call_state::apply_inbound(&parsed.sender_public_key_hex, &parsed.call_id, parsed.kind)
            {
                native_log::debug(
                    "call",
                    format!(
                        "ignore inbound {} call_id={} from {peer}: {e}",
                        parsed.kind.wire_name(),
                        parsed.call_id
                    ),
                );
                continue;
            }
            session.ensure_dm_peer(&parsed.sender_public_key_hex, peer);
            session.try_emit_peer_identified(
                peer,
                parsed.sender_public_key_hex.clone(),
                &events_tx,
            );
            if let Some(tx) = &events_tx {
                let _ = tx.send(GossipChatEvent::CallSignal {
                    from: peer,
                    id: parsed.id.clone(),
                    call_id: parsed.call_id.clone(),
                    signal: parsed.kind.wire_name().to_string(),
                    sender_public_key_hex: parsed.sender_public_key_hex.clone(),
                    created_at_ms: parsed.created_at_ms,
                    payload: parsed.payload.clone(),
                });
            }
            continue;
        }
        if share != MSG_SHARE {
            continue;
        }
        let env = match frame_bytes_to_envelope(&frame) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let parsed = match parse_envelope(&env, &my_public, &my_secret) {
            Ok(p) => p,
            Err(e) => {
                native_log::warn("stream", format!("drop frame from {peer}: {e}"));
                continue;
            }
        };
        match parsed {
            ParsedMsg::Text(t) => {
                if !secp256k1_public_hex_matches_peer_id(&t.sender_public_key_hex, &peer) {
                    native_log::warn(
                        "stream",
                        format!("drop text from {peer}: signing key mismatch"),
                    );
                    continue;
                }
                native_log::debug(
                    "stream",
                    format!(
                        "inbound text from {peer} id={} len={}",
                        t.id,
                        t.text.len()
                    ),
                );
                let is_new = session.remember_inbound_id(&t.id, chrono_now_ms());
                let was_known =
                    session.dm_peer_for_libp2p(peer).is_some_and(|d| d.has_send_keys());
                session.ensure_dm_peer(&t.sender_public_key_hex, peer);
                if is_new && !was_known {
                    session.try_emit_peer_identified(
                        peer,
                        t.sender_public_key_hex.clone(),
                        &events_tx,
                    );
                    emit_chat_ready_if_can_send(
                        Arc::clone(&session),
                        peer,
                        Arc::clone(&writers),
                        events_tx.clone(),
                    );
                }
                if is_new {
                    if !crate::dm_event_handler::persist_inbound_text_on_wire(
                        &peer.to_string(),
                        &t.id,
                        &t.text,
                        &t.sender_public_key_hex,
                        t.created_at_ms,
                    ) {
                        native_log::warn(
                            "DM/store",
                            format!(
                                "inbound text not persisted on wire id={} from {peer} (handler context?)",
                                t.id
                            ),
                        );
                    }
                    if let Some(tx) = &events_tx {
                        let _ = tx.send(GossipChatEvent::DmMessage {
                            from: peer,
                            id: t.id.clone(),
                            msg_kind: "text".to_string(),
                            text: Some(t.text.clone()),
                            ref_id: None,
                            sender_public_key_hex: t.sender_public_key_hex.clone(),
                            created_at_ms: t.created_at_ms,
                        });
                    }
                } else {
                    native_log::debug(
                        "stream",
                        format!("duplicate text id={} from {peer} — ack retry only", t.id),
                    );
                }
                // `:p2p` background must always send `ack_received` (UI may be dead; foreground
                // peer can be stale). In-room UI additionally sends `ack_read` after delivery.
                send_inbound_delivery_ack(
                    peer,
                    &t.id,
                    &t.sender_public_key_hex,
                    session.as_ref(),
                    &writers,
                )
                .await;
                let in_room = app_ack_read_enabled()
                    && session.is_foreground_peer(peer)
                    && !session.is_read_ack_confirmed(&t.id);
                if in_room {
                    send_inbound_read_ack_if_possible(
                        peer,
                        &t.id,
                        &t.sender_public_key_hex,
                        session.as_ref(),
                        &writers,
                    )
                    .await;
                }
            }
            // Asymmetric ack routing (see `dm_delivery_sync.dart` / DESIGN.md):
            // - Off-room → `ack_received` only; in-room → `ack_received` then `ack_read`.
            // - Our outbound outbox clears on peer `ack_received` or `ack_read` for our message id.
            // - `ack_read` on our outbound id → peer read; `ack_received` on inbound id → read-receipt confirm.
            ParsedMsg::Ack(a) => {
                if !secp256k1_public_hex_matches_peer_id(&a.sender_public_key_hex, &peer) {
                    native_log::warn(
                        "stream",
                        format!("drop ack from {peer}: signing key mismatch"),
                    );
                    continue;
                }
                if a.kind == MsgKind::AckRequest {
                    // Deprecated wire kind — we never send this. Recipient drives delivery via
                    // `ack_received` / `ack_read` only (see `docs/GOTIGIN_DM_MSG_V1.md`).
                    native_log::debug(
                        "stream",
                        format!("ignore ack_request ref={} from {peer}", a.ref_id),
                    );
                    continue;
                }
                if a.kind == MsgKind::AckReceived {
                    session.complete_outbound(&a.ref_id);
                    // Peer confirms they got our `ack_read` for their text (ref_id = their message id).
                    if session.has_pending_read_ack(&a.ref_id)
                        || session.has_seen_inbound_id(&a.ref_id)
                    {
                        session.mark_read_ack_confirmed(&a.ref_id);
                    }
                }
                if a.kind == MsgKind::AckRead {
                    // Read implies delivery — stop outbox retry without a separate `ack_received`.
                    session.complete_outbound(&a.ref_id);
                    session.ensure_dm_peer(&a.sender_public_key_hex, peer);
                    let _ = send_ack_frame(
                        peer,
                        &a.sender_public_key_hex,
                        &a.ref_id,
                        MsgKind::AckReceived,
                        session.as_ref(),
                        &writers,
                    )
                    .await;
                }
                let kind = match a.kind {
                    MsgKind::AckReceived => "ack_received",
                    MsgKind::AckRead => "ack_read",
                    MsgKind::Text | MsgKind::AckRequest => continue,
                };
                if let Some(tx) = &events_tx {
                    let _ = tx.send(GossipChatEvent::DmMessage {
                        from: peer,
                        id: a.id.clone(),
                        msg_kind: kind.to_string(),
                        text: None,
                        ref_id: Some(a.ref_id.clone()),
                        sender_public_key_hex: a.sender_public_key_hex.clone(),
                        created_at_ms: a.created_at_ms,
                    });
                }
            }
        }
    }

    if owns_writer {
        if let Ok(mut g) = writers.lock() {
            g.remove(&peer);
        }
        if let Some(task) = write_task {
            let _ = task.await;
        }
    }
}

/// Open `/ghal-bol/msg/1.0.0` if missing (persistent stream).
async fn ensure_chat_stream(
    peer: PeerId,
    mut control: stream::Control,
    writers: StreamWriters,
    session: Arc<SessionState>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) -> Result<(), String> {
    if writer_open_for_peer(&writers, peer) {
        emit_chat_ready_if_can_send(Arc::clone(&session), peer, Arc::clone(&writers), events_tx.clone());
        return Ok(());
    }
    if session.log_stream_open_once(peer) {
        native_log::debug("stream", format!("open outbound chat stream to {peer}"));
    }
    let mut last_err = String::new();
    let mut stream = None;
    for attempt in 0..3u8 {
        match control
            .open_stream(peer, StreamProtocol::new(STREAM_PROTOCOL))
            .await
        {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => {
                last_err = format!("open_stream {peer}: {e}");
                let transient = last_err.contains("oneshot canceled")
                    || last_err.contains("canceled")
                    || last_err.contains("Connection refused");
                if attempt < 2 && transient {
                    tokio::time::sleep(Duration::from_millis(40 * (attempt as u64 + 1))).await;
                    continue;
                }
                return Err(last_err);
            }
        }
    }
    let stream = stream.ok_or_else(|| last_err)?;
    tokio::spawn(handle_inbound_stream(
        peer,
        stream,
        Arc::clone(&session),
        writers.clone(),
        events_tx.clone(),
        control,
    ));
    // Wait for handle_inbound_stream to install the mux writer (Android relay dials need >24 yields).
    let deadline = time::Instant::now() + Duration::from_secs(4);
    while time::Instant::now() < deadline {
        if writer_open_for_peer(&writers, peer) {
            emit_chat_ready_if_can_send(Arc::clone(&session), peer, Arc::clone(&writers), events_tx.clone());
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err("chat stream opening — try send again shortly".to_string())
}

async fn open_outbound_stream_if_needed(
    peer: PeerId,
    control: stream::Control,
    writers: StreamWriters,
    session: Arc<SessionState>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if writer_open_for_peer(&writers, peer) {
        return;
    }
    if !session.try_begin_stream_open(peer) {
        // Another task is already trying; avoid creating a self-cancel storm.
        return;
    }
    if let Err(e) = ensure_chat_stream(
        peer,
        control,
        writers,
        Arc::clone(&session),
        events_tx.clone(),
    )
    .await
    {
        if let Some(tx) = events_tx {
            let _ = tx.send(GossipChatEvent::DialFailed {
                peer: Some(peer),
                error: format!("open chat stream: {e}"),
            });
        }
    }
    // Always release, success or fail.
    session.end_stream_open(peer);
}

async fn on_dm_peer_connected(
    session: Arc<SessionState>,
    control: stream::Control,
    writers: StreamWriters,
    connected: PeerId,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if !session.is_dm_contact(connected) {
        return;
    }
    session.ensure_dm_peer_from_libp2p(connected);
    if let Some(pk) = session
        .dm_peer_for_libp2p(connected)
        .and_then(|d| d.public_key_hex.clone())
    {
        session.try_emit_peer_identified(connected, pk, &events_tx);
    }
    open_outbound_stream_if_needed(
        connected,
        control,
        writers,
        Arc::clone(&session),
        events_tx,
    )
    .await;
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Room open/close and peer register must not sit behind a long `send_text_dm` queue.
fn outbound_cmd_priority(cmd: &OutboundCmd) -> u8 {
    match cmd {
        OutboundCmd::SetForegroundPeer { .. } | OutboundCmd::RunReadAckCatchup { .. } => 0,
        OutboundCmd::RegisterDmPeer { .. } => 1,
        OutboundCmd::DialBootstrapPeers { .. } => 2,
        OutboundCmd::FlushKadPublish => 3,
        OutboundCmd::SendAck { .. } => 4,
        OutboundCmd::SendCallSignal { .. } => 5,
        OutboundCmd::SendText { .. } => 6,
    }
}

async fn drain_outbound_queue(
    outbound_rx: &std::sync::mpsc::Receiver<OutboundCmd>,
    swarm: &mut Swarm<ChatBehaviour>,
    session: Arc<SessionState>,
    writers: StreamWriters,
    control: stream::Control,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    max_cmds: usize,
) {
    let mut batch = Vec::with_capacity(max_cmds.min(64));
    for _ in 0..max_cmds {
        let Ok(cmd) = outbound_rx.try_recv() else {
            break;
        };
        batch.push(cmd);
    }
    if batch.is_empty() {
        return;
    }
    batch.sort_by_key(|c| outbound_cmd_priority(c));
    for cmd in batch {
        let send_msg_id = match &cmd {
            OutboundCmd::SendText { message_id, .. } => Some(message_id.clone()),
            _ => None,
        };
        if let Err(e) = process_outbound_cmd(
            cmd,
            swarm,
            Arc::clone(&session),
            Arc::clone(&writers),
            control.clone(),
            events_tx.clone(),
        )
        .await
        {
            if let Some(tx) = &events_tx {
                if let Some(message_id) = send_msg_id {
                    // Outbox resends until `ack_received`; do not surface terminal send_failed.
                    if !session.outbox_contains(&message_id) && !is_transient_outbound_error(&e) {
                        let _ = tx.send(GossipChatEvent::SendFailed {
                            message_id,
                            error: e,
                        });
                    }
                } else {
                    let _ = tx.send(GossipChatEvent::DialFailed {
                        peer: None,
                        error: e,
                    });
                }
            }
        }
    }
}

async fn process_outbound_cmd(
    cmd: OutboundCmd,
    swarm: &mut Swarm<ChatBehaviour>,
    session: Arc<SessionState>,
    writers: StreamWriters,
    control: stream::Control,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) -> Result<(), String> {
    if matches!(cmd, OutboundCmd::FlushKadPublish) {
        let local = *swarm.local_peer_id();
        if let Ok(mut v) = session.published_listen.write() {
            v.retain(|ma| super::dht_bootstrap::is_kad_publish_tcp_multiaddr(ma));
        }
        let n = super::dht_bootstrap::tcp_dm_publish_addrs(session.published_listen_snapshot()).len();
        native_log::info("kad", format!("republish nudged for {local} ({n} tcp listen addr(s) cached)"));
        session.flush_kad_publish(&mut swarm.behaviour_mut().kademlia, &local, true);
        return Ok(());
    }
    if let OutboundCmd::RegisterDmPeer {
        peer_id,
        public_key_hex,
    } = &cmd
    {
        session.register_dm_peer_key(*peer_id, public_key_hex);
        if let (Some(path), Some(ns)) = (&session.transcript_path, &session.app_namespace) {
            transcript_sync_outbound_tick(session.as_ref(), Path::new(path), ns.trim());
        }
        let pk = public_key_hex.trim();
        if pk.len() == 66 {
            if crate::coord_runtime::wan_discovery_via_coord_only() {
                let now = chrono_now_ms();
                if session.should_coord_lookup_pk(pk, now, 1_000) {
                    native_log::info("dial", "register_dm_peer: coord lookup");
                    coord_lookup_dm_peer(swarm, session.as_ref(), pk).await;
                }
            } else if let Ok(derived) = peer_id_from_secp256k1_public_key_hex(pk) {
                if let Ok(target) = derived.parse::<PeerId>() {
                    native_log::info("dial", format!("register_dm_peer: kad lookup+dial {target}"));
                    kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, target);
                    try_routed_dial(swarm, session.as_ref(), target);
                }
            }
        } else if let Some(pid) = *peer_id {
            if !crate::coord_runtime::wan_discovery_via_coord_only() {
                kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, pid);
                try_routed_dial(swarm, session.as_ref(), pid);
            }
        }
        return Ok(());
    }
    if let OutboundCmd::DialBootstrapPeers { addrs } = &cmd {
        for ma in addrs {
            if super::dht_bootstrap::is_trusted_bootstrap_dial_addr(ma) {
                let _ = swarm.dial(ma.clone());
            }
        }
        return Ok(());
    }
    if let OutboundCmd::RunReadAckCatchup { peer_id } = &cmd {
        let peer = *peer_id;
        if !session.is_foreground_peer(peer) || !app_ack_read_enabled() {
            return Ok(());
        }
        native_log::info(
            "read_ack",
            format!("read gate opened — catch-up ack_read for foreground {peer}"),
        );
        seed_read_acks_for_peer_from_transcript(session.as_ref(), peer);
        let session2 = Arc::clone(&session);
        let writers2 = Arc::clone(&writers);
        tokio::spawn(async move {
            if !session2.is_foreground_peer(peer) || !app_ack_read_enabled() {
                return;
            }
            read_ack_catchup_for_peer(session2, writers2, peer, true, true).await;
        });
        return Ok(());
    }
    if let OutboundCmd::SetForegroundPeer { peer_id } = &cmd {
        let previous = session.current_foreground_peer().or_else(|| {
            last_room_peer().and_then(|pk| {
                peer_id_from_secp256k1_public_key_hex(&pk)
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
        });
        session.set_foreground_peer(*peer_id);
        if let Some(left) = previous {
            let leaving = match peer_id {
                None => true,
                Some(new) => *new != left,
            };
            if leaving {
                spawn_leave_read_ack_drain(
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    left,
                );
            }
        }
        if peer_id.is_none() {
            if let Ok(mut last) = last_room_peer_mx().write() {
                *last = None;
            }
            return Ok(());
        }
        let peer = peer_id.unwrap();
        if session.current_foreground_peer() != Some(peer) {
            native_log::debug(
                "read_ack",
                format!("chat room enter {peer} skipped — foreground already changed"),
            );
            return Ok(());
        }
        seed_read_acks_for_peer_from_transcript(session.as_ref(), peer);
        if !app_ack_read_enabled() {
            native_log::debug(
                "read_ack",
                format!(
                    "chat room enter {peer} deferred — read gate off; seeded transcript backlog"
                ),
            );
            return Ok(());
        }
        native_log::info(
            "read_ack",
            format!("chat room enter {peer} — send ack_read for all unacked inbound"),
        );
        if let (Some(path), Some(ns)) = (&session.transcript_path, &session.app_namespace) {
            transcript_sync_outbound_tick(session.as_ref(), Path::new(path), ns.trim());
        }
        let session2 = Arc::clone(&session);
        let writers2 = Arc::clone(&writers);
        tokio::spawn(async move {
            if session2.current_foreground_peer() != Some(peer) || !app_ack_read_enabled() {
                return;
            }
            read_ack_catchup_for_peer(session2, writers2, peer, true, true).await;
        });
        return Ok(());
    }

    let mut sent_text_id: Option<String> = None;
    let mut pending_call: Option<PendingCallSignal> = None;
    let (peer, frame, done) = match cmd {
        OutboundCmd::RegisterDmPeer { .. }
        | OutboundCmd::SetForegroundPeer { .. }
        | OutboundCmd::RunReadAckCatchup { .. }
        | OutboundCmd::DialBootstrapPeers { .. }
        | OutboundCmd::FlushKadPublish => unreachable!(),
        OutboundCmd::SendCallSignal {
            recipient_public_key_hex,
            call_id,
            signal_kind,
            payload,
            signal_id,
        } => {
            let pk = recipient_public_key_hex.trim();
            if pk.len() == 66 {
                session.register_dm_peer_key(None, pk);
            }
            let peer = session.resolve_send_peer(pk).ok_or_else(|| {
                "unknown contact — add them via invitation first".to_string()
            })?;
            session.ensure_dm_peer_from_libp2p(peer);
            if let Err(e) = call_state::apply_outbound(pk, &call_id, signal_kind) {
                return Err(e);
            }
            let payload_for_queue = payload.clone();
            let env = build_call_envelope(
                &signal_id,
                &call_id,
                signal_kind,
                session.identity.keypair(),
                pk,
                payload,
                chrono_now_ms(),
            )?;
            let frame = call_envelope_to_frame_bytes(&env)?;
            native_log::info(
                "call",
                format!(
                    "enqueue call signal {} call_id={} peer={peer}",
                    signal_kind.wire_name(),
                    call_id
                ),
            );
            pending_call = Some(PendingCallSignal {
                recipient_public_key_hex: pk.to_string(),
                call_id: call_id.clone(),
                signal_kind,
                payload: payload_for_queue,
                signal_id,
                frame,
                peer_id: peer,
            });
            (peer, Vec::new(), None)
        }
        OutboundCmd::SendText {
            recipient_public_key_hex,
            text,
            message_id,
            done,
        } => {
            sent_text_id = Some(message_id.clone());
            let pk = recipient_public_key_hex.trim();
            if pk.len() == 66 {
                session.register_dm_peer_key(None, pk);
            }
            let peer = session.resolve_send_peer(pk).ok_or_else(|| {
                let err = "unknown contact — add them via invitation first".to_string();
                native_log::warn(
                    "outbound",
                    format!("send_text rejected: {err} pk={pk} msg_id={message_id}"),
                );
                err
            })?;
            session.ensure_dm_peer_from_libp2p(peer);
            // Critical: do not wait for periodic coord_tick to start dialing.
            // If the app just switched networks, minutes-long stalls were caused by missing/late
            // coord lookups and relying on routed-dial paths that are disabled on mobile-data.
            if !swarm.is_connected(&peer) && pk.len() == 66 {
                native_log::info("dial", format!("send queued: coord lookup {peer}"));
                coord_lookup_dm_peer(swarm, session.as_ref(), pk).await;
            }
            let remote = session
                .dm_peer(pk)
                .ok_or_else(|| "unknown dm peer — add contact first".to_string())?;
            let _recipient_pk = remote
                .public_key_hex
                .as_deref()
                .ok_or_else(|| "peer public key not known yet — wait for connect".to_string())?;
            let now = chrono_now_ms();
            let pending = PendingOutbound {
                message_id: message_id.clone(),
                peer_id: peer,
                recipient_public_key_hex: recipient_public_key_hex.clone(),
                text: text.clone(),
                created_at_ms: now,
                last_send_ms: now,
                on_wire: false,
            };
            let frame_bytes = build_pending_outbound_frame(session.as_ref(), &pending)?;
            session.track_outbound(pending);
            native_log::info(
                "outbound",
                format!(
                    "enqueue send_text peer={peer} msg_id={message_id} text_len={}",
                    text.len()
                ),
            );
            (peer, frame_bytes, done)
        }
        OutboundCmd::SendAck {
            recipient_public_key_hex,
            ref_id,
            ack_kind,
        } => {
            let peer_id = session
                .resolve_send_peer(&recipient_public_key_hex)
                .ok_or_else(|| "unknown dm peer".to_string())?;
            let env = build_ack_envelope(
                &new_msg_id(),
                &ref_id,
                ack_kind,
                session.identity.keypair(),
                &recipient_public_key_hex,
                chrono_now_ms(),
            )?;
            (peer_id, envelope_to_frame_bytes(&env)?, None)
        }
    };
    if let Some(call) = pending_call {
        if !swarm.is_connected(&peer) {
            native_log::info("dial", format!("call signal queued: lookup+dial {peer} (not connected yet)"));
            if let Some(pk) = session
                .dm_peer_for_libp2p(peer)
                .and_then(|d| d.public_key_hex.clone())
                .filter(|pk| pk.len() == 66)
            {
                if crate::coord_runtime::wan_discovery_via_coord_only() {
                    coord_lookup_dm_peer(swarm, session.as_ref(), &pk).await;
                } else {
                    kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, peer);
                    try_routed_dial(swarm, session.as_ref(), peer);
                }
            } else {
                kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, peer);
                try_routed_dial(swarm, session.as_ref(), peer);
            }
            session.enqueue_pending_call_signal(call);
            return Ok(());
        }
        if !writer_open_for_peer(&writers, peer) {
            let _ = ensure_chat_stream(
                peer,
                control,
                Arc::clone(&writers),
                Arc::clone(&session),
                events_tx.clone(),
            )
            .await;
        }
        if !writer_open_for_peer(&writers, peer) {
            native_log::info("call", format!("call signal queued: stream opening (peer={peer})"));
            session.enqueue_pending_call_signal(call);
            return Ok(());
        }
        let r = send_frame_to_peer(peer, call.frame, writers).await;
        if r.is_ok() {
            native_log::info(
                "call",
                format!(
                    "call frame on wire peer={peer} {} call_id={}",
                    call.signal_kind.wire_name(),
                    call.call_id
                ),
            );
        }
        return r;
    }
    if !swarm.is_connected(&peer) {
        if crate::coord_runtime::wan_discovery_via_coord_only() {
            if let Some(pk) = session
                .dm_peer_for_libp2p(peer)
                .and_then(|p| p.public_key_hex.clone())
            {
                native_log::info("dial", format!("send queued: coord lookup {peer}"));
                coord_lookup_dm_peer(swarm, session.as_ref(), &pk).await;
            }
        } else {
            native_log::info("dial", format!("send queued: lookup+dial {peer} (not connected yet)"));
            kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, peer);
            try_routed_dial(swarm, session.as_ref(), peer);
        }
        let err = "connecting to peer — try send again in a moment".to_string();
        if let Some(done) = done {
            let _ = done.send(Err(err.clone()));
        }
        return Err(err);
    }
    if !writer_open_for_peer(&writers, peer) {
        let _ = ensure_chat_stream(
            peer,
            control.clone(),
            Arc::clone(&writers),
            Arc::clone(&session),
            events_tx.clone(),
        )
        .await;
    }
    if !writer_open_for_peer(&writers, peer) {
        let err = "chat stream opening — try send again shortly".to_string();
        native_log::info("stream", format!("{err} (peer={peer})"));
        if let Some(done) = done {
            let _ = done.send(Err(err.clone()));
        }
        return Err(err);
    }
    let r = send_frame_to_peer(peer, frame, Arc::clone(&writers)).await;
    if r.is_ok() {
        if let Some(id) = sent_text_id {
            notify_outbound_on_wire(&session, &id, chrono_now_ms(), &events_tx);
            native_log::info("outbound", format!("frame on wire peer={peer} msg_id={id}"));
        }
    } else if let (Some(id), Err(e)) = (&sent_text_id, &r) {
        session.mark_outbox_send_failed(id, chrono_now_ms());
        native_log::warn(
            "outbound",
            format!("send_frame failed peer={peer} msg_id={id}: {e} — outbox will retry"),
        );
        let err_s = e.to_string();
        if err_s.contains("chat stream closed") || err_s.contains("no chat stream") {
            if let Ok(mut g) = writers.lock() {
                g.remove(&peer);
            }
            let _ = ensure_chat_stream(
                peer,
                control.clone(),
                Arc::clone(&writers),
                Arc::clone(&session),
                events_tx.clone(),
            )
            .await;
        }
    }
    if let Some(done) = done {
        let _ = done.send(r.clone());
    }
    r
}

fn pair_from_dm(session: &SessionState, dm: &DmPeer) -> Option<(PeerId, String)> {
    let pk = dm.public_key_hex.as_deref()?.trim().to_string();
    if pk.len() != 66 {
        return None;
    }
    let peer_id = session.resolve_send_peer(&pk)?;
    Some((peer_id, pk))
}

fn resolve_send_pair_for_row(
    session: &SessionState,
    row: &crate::dm_transcript_v1::PendingOutboundRow,
) -> Option<(PeerId, String)> {
    if let Some(dm) = session.dm_peer_for_conversation_key(&row.conversation_key) {
        return pair_from_dm(session, &dm);
    }
    let ck = row.conversation_key.trim();
    for peer_id in session.dm_peer_ids() {
        let Some(dm) = session.dm_peer_for_libp2p(peer_id) else {
            continue;
        };
        let pk = dm.public_key_hex.as_deref().unwrap_or("").trim();
        if ck == peer_id.to_string() || (pk.len() == 66 && ck == pk) {
            return pair_from_dm(session, &dm);
        }
    }
    let ids = session.dm_peer_ids();
    if ids.len() == 1 {
        return session
            .dm_peer_for_libp2p(ids[0])
            .and_then(|dm| pair_from_dm(session, &dm));
    }
    None
}

fn build_pending_outbound_frame(
    session: &SessionState,
    p: &PendingOutbound,
) -> Result<Vec<u8>, String> {
    let remote = session
        .dm_peer(&p.recipient_public_key_hex)
        .ok_or_else(|| "unknown dm peer".to_string())?;
    let recipient_pk = remote
        .public_key_hex
        .as_deref()
        .ok_or_else(|| "peer public key missing".to_string())?;
    let ts = if p.created_at_ms > 0 {
        p.created_at_ms
    } else {
        chrono_now_ms()
    };
    let env = build_text_envelope(
        &p.message_id,
        session.identity.keypair(),
        recipient_pk,
        &p.text,
        ts,
    )?;
    envelope_to_frame_bytes(&env)
}

fn merge_outbound_row_into_outbox(session: &SessionState, row: &crate::dm_transcript_v1::PendingOutboundRow) -> bool {
    if session.outbox_contains(&row.message_id) {
        return false;
    }
    let Some((peer_id, pk)) = resolve_send_pair_for_row(session, row) else {
        return false;
    };
    let now = chrono_now_ms();
    session.track_outbound(PendingOutbound {
        message_id: row.message_id.clone(),
        peer_id,
        recipient_public_key_hex: pk,
        text: row.text.clone(),
        created_at_ms: if row.created_at_ms > 0 {
            row.created_at_ms
        } else {
            now
        },
        last_send_ms: now,
        on_wire: false,
    });
    true
}

fn sync_outbox_from_transcript(session: &SessionState, path: &Path, app_namespace: &str) -> usize {
    let Ok(rows) = crate::dm_transcript_v1::pending_outbound_rows(path, app_namespace) else {
        return 0;
    };
    let mut merged = 0usize;
    for row in rows {
        if merge_outbound_row_into_outbox(session, &row) {
            merged += 1;
        }
    }
    merged
}

/// Drop in-memory outbox rows when the transcript no longer lists them as pending.
fn purge_outbox_delivered_from_transcript(
    session: &SessionState,
    path: &Path,
    app_namespace: &str,
) {
    let Ok(rows) = crate::dm_transcript_v1::pending_outbound_rows(path, app_namespace) else {
        return;
    };
    let still_pending: HashSet<String> = rows.into_iter().map(|r| r.message_id).collect();
    let Ok(mut g) = session.outbox.write() else {
        return;
    };
    let before = g.len();
    g.retain(|id, _| still_pending.contains(id));
    let removed = before.saturating_sub(g.len());
    if removed > 0 {
        native_log::debug(
            "outbox",
            format!("purged {removed} delivered row(s) from in-memory outbox (transcript authoritative)"),
        );
    }
}

/// Upkeep tick: sync in-memory outbox from transcript, then purge delivered rows.
fn transcript_sync_outbound_tick(
    session: &SessionState,
    path: &Path,
    app_namespace: &str,
) {
    sync_outbox_from_transcript(session, path, app_namespace);
    purge_outbox_delivered_from_transcript(session, path, app_namespace);
}

fn bootstrap_outbox_from_transcript(session: &SessionState, path: &Path, app_namespace: &str) {
    let restored = sync_outbox_from_transcript(session, path, app_namespace);
    if restored > 0 {
        native_log::info(
            "outbox",
            format!("restored {restored} pending outbound row(s) from transcript"),
        );
    }
}

fn seed_read_acks_for_peer_from_transcript(session: &SessionState, peer: PeerId) {
    let (path, ns) = match (&session.transcript_path, &session.app_namespace) {
        (Some(p), Some(n)) if !p.trim().is_empty() && !n.trim().is_empty() => {
            (p.as_str(), n.trim())
        }
        _ => return,
    };
    let Some(dm) = session.dm_peer_for_libp2p(peer) else {
        return;
    };
    let Some(signing) = dm.public_key_hex.as_deref() else {
        return;
    };
    let Ok(rows) = crate::dm_transcript_v1::pending_inbound_read_ack_rows(Path::new(path), ns) else {
        return;
    };
    let mut keys = std::collections::HashSet::new();
    keys.insert(peer.to_string());
    keys.insert(signing.to_string());
    let mut seeded = 0usize;
    for row in rows {
        if !keys.contains(row.conversation_key.as_str()) {
            continue;
        }
        if session.is_read_ack_confirmed(&row.message_id) {
            continue;
        }
        session.enqueue_read_ack(peer, &row.message_id, signing);
        seeded += 1;
    }
    if seeded > 0 {
        native_log::debug(
            "read_ack",
            format!("seeded {seeded} pending read ack(s) for {peer} from transcript"),
        );
    }
}

/// Delivery ack for inbound text — sent even when the user is outside the chat room.
async fn send_inbound_delivery_ack(
    peer: PeerId,
    inbound_id: &str,
    sender_signing: &str,
    session: &SessionState,
    writers: &StreamWriters,
) {
    if session.is_delivery_ack_sent(inbound_id) {
        return;
    }
    if send_ack_frame(
        peer,
        sender_signing,
        inbound_id,
        MsgKind::AckReceived,
        session,
        writers,
    )
    .await
    {
        session.mark_delivery_ack_sent(inbound_id);
        session.dequeue_delivery_ack(inbound_id);
        native_log::info(
            "delivery_ack",
            format!("ack_received sent for inbound {inbound_id} to {peer}"),
        );
        return;
    }
    session.enqueue_delivery_ack(peer, inbound_id, sender_signing);
    native_log::warn(
        "delivery_ack",
        format!("ack_received queued for inbound {inbound_id} to {peer} (stream not ready)"),
    );
}

/// Enqueue + send `ack_read` (caller gates: only for in-room arrivals).
async fn send_inbound_read_ack_if_possible(
    peer: PeerId,
    inbound_id: &str,
    sender_signing: &str,
    session: &SessionState,
    writers: &StreamWriters,
) {
    session.enqueue_read_ack(peer, inbound_id, sender_signing);
    if send_ack_frame(peer, sender_signing, inbound_id, MsgKind::AckRead, session, writers).await
    {
        return;
    }
    native_log::warn(
        "read_ack",
        format!("ack_read queued for {inbound_id} to {peer} (stream not ready)"),
    );
}

async fn send_ack_frame(
    peer: PeerId,
    recipient_signing: &str,
    ref_id: &str,
    kind: MsgKind,
    session: &SessionState,
    writers: &StreamWriters,
) -> bool {
    let Ok(env) = build_ack_envelope(
        &new_msg_id(),
        ref_id,
        kind,
        session.identity.keypair(),
        recipient_signing,
        chrono_now_ms(),
    ) else {
        return false;
    };
    let Ok(frame) = envelope_to_frame_bytes(&env) else {
        return false;
    };
    send_frame_to_peer(peer, frame, Arc::clone(writers))
        .await
        .is_ok()
}

/// Retry queued delivery + read acks. Delivery always; queued `ack_read` always (UI background ok).
async fn run_ack_upkeep(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: &[PeerId],
) {
    if connected_peers.is_empty() {
        return;
    }
    run_ack_upkeep_limited(
        session,
        writers,
        connected_peers,
        READ_ACK_UPKEEP_MAX_OPS_PER_TICK,
    )
    .await;
}

async fn run_ack_upkeep_limited(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: &[PeerId],
    read_limit: usize,
) -> usize {
    if connected_peers.is_empty() || read_limit == 0 {
        return 0;
    }
    let connected: HashSet<PeerId> = connected_peers.iter().copied().collect();
    let mut done = 0usize;
    // Delivery acks always retry (background, UI lock, read gate must not block these).
    let delivery_batch = session.delivery_acks_due_for_upkeep(read_limit);
    for item in delivery_batch {
        if done >= read_limit {
            break;
        }
        if !connected.contains(&item.peer_id) || !writer_open_for_peer(&writers, item.peer_id) {
            continue;
        }
        if send_ack_frame(
            item.peer_id,
            &item.recipient_public_key_hex,
            &item.inbound_id,
            MsgKind::AckReceived,
            session.as_ref(),
            &writers,
        )
        .await
        {
            session.mark_delivery_ack_sent(&item.inbound_id);
            session.dequeue_delivery_ack(&item.inbound_id);
            done += 1;
        }
    }
    // Queued `ack_read` retries run in :p2p even when the UI is backgrounded; only *new* in-room
    // arrivals are gated by `app_ack_read_enabled` + foreground peer (see inbound text handler).
    let read_batch = session.read_acks_due_for_upkeep(read_limit.saturating_sub(done));
    for item in read_batch {
        if session.is_read_ack_confirmed(&item.inbound_id) {
            continue;
        }
        if !connected.contains(&item.peer_id) || !writer_open_for_peer(&writers, item.peer_id) {
            continue;
        }
        if send_ack_frame(
            item.peer_id,
            &item.recipient_public_key_hex,
            &item.inbound_id,
            MsgKind::AckRead,
            session.as_ref(),
            &writers,
        )
        .await
        {
            done += 1;
        }
    }
    done
}

/// Burst-send queued `ack_read`. When [seed_transcript] is true (enter room), seed transcript
/// then drain. When false (leave room), drain only — caller seeds transcript synchronously first.
async fn read_ack_catchup_for_peer(
    session: Arc<SessionState>,
    writers: StreamWriters,
    peer: PeerId,
    wait_for_writer: bool,
    seed_transcript: bool,
) {
    if seed_transcript && !app_ack_read_enabled() {
        return;
    }
    if seed_transcript {
        seed_read_acks_for_peer_from_transcript(session.as_ref(), peer);
    }
    if wait_for_writer {
        for _ in 0..80 {
            if writer_open_for_peer(&writers, peer) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    if writer_open_for_peer(&writers, peer) {
        run_ack_upkeep_burst(session, writers, peer).await;
    }
}

/// After [chat_ready] / foreground peer: drain pending `ack_read` quickly (bounded).
async fn run_ack_upkeep_burst(session: Arc<SessionState>, writers: StreamWriters, peer: PeerId) {
    let connected = [peer];
    for round in 0..ACK_BURST_MAX_ROUNDS {
        let pending_before = session.pending_read_ack_len();
        let n = run_ack_upkeep_limited(
            Arc::clone(&session),
            Arc::clone(&writers),
            &connected,
            ACK_BURST_MAX_OPS_PER_PASS,
        )
        .await;
        if n == 0 && session.pending_read_ack_len() == pending_before {
            break;
        }
        if round + 1 >= ACK_BURST_MAX_ROUNDS {
            break;
        }
    }
}

/// Flush queued call signaling once the DM stream to a peer is up.
async fn flush_pending_call_signals(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: Vec<PeerId>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    let batch = session.drain_pending_call_signals(32);
    if batch.is_empty() {
        return;
    }
    for call in batch {
        if !connected_peers.iter().any(|id| *id == call.peer_id) {
            session.requeue_pending_call_signal_front(call);
            continue;
        }
        if !writer_open_for_peer(&writers, call.peer_id) {
            session.requeue_pending_call_signal_front(call);
            continue;
        }
        let peer_id = call.peer_id;
        let signal_kind = call.signal_kind;
        let call_id_log = call.call_id.clone();
        match send_frame_to_peer(peer_id, call.frame.clone(), Arc::clone(&writers)).await {
            Ok(()) => {
                native_log::info(
                    "call",
                    format!(
                        "call frame on wire peer={peer_id} {} call_id={call_id_log}",
                        signal_kind.wire_name(),
                    ),
                );
            }
            Err(e) => {
                if is_transient_outbound_error(&e) {
                    session.requeue_pending_call_signal_front(call);
                } else {
                    native_log::warn(
                        "call",
                        format!(
                            "call send failed peer={} {} call_id={}: {e}",
                            call.peer_id,
                            call.signal_kind.wire_name(),
                            call.call_id
                        ),
                    );
                    if let Some(tx) = &events_tx {
                        let _ = tx.send(GossipChatEvent::DialFailed {
                            peer: Some(call.peer_id),
                            error: e,
                        });
                    }
                }
            }
        }
    }
}

/// Resend outbound texts until `ack_received` or `ack_read` (~1s cadence).
///
/// The **transcript** is authoritative; call [`transcript_sync_outbound_tick`] before this on every tick.
async fn resync_pending_outbox(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: Vec<PeerId>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    control: Option<stream::Control>,
) {
    let now = chrono_now_ms();
    let due = session.outbox_due_for_resend(now);
    if !due.is_empty() && !connected_peers.is_empty() {
        native_log::debug(
            "outbox",
            format!("resync {} pending message(s)", due.len()),
        );
    }
    for p in due {
        if !connected_peers.iter().any(|id| *id == p.peer_id) {
            continue;
        }
        if !writer_open_for_peer(&writers, p.peer_id) {
            if let Some(ctrl) = control.as_ref() {
                open_outbound_stream_if_needed(
                    p.peer_id,
                    ctrl.clone(),
                    Arc::clone(&writers),
                    Arc::clone(&session),
                    events_tx.clone(),
                )
                .await;
            }
            if !writer_open_for_peer(&writers, p.peer_id) {
                continue;
            }
        }
        let frame = match build_pending_outbound_frame(session.as_ref(), &p) {
            Ok(f) => f,
            Err(e) => {
                native_log::warn(
                    "outbox",
                    format!("resync skip msg_id={}: {e}", p.message_id),
                );
                continue;
            }
        };
        match send_frame_to_peer(p.peer_id, frame, Arc::clone(&writers)).await {
            Ok(()) => {
                session.mark_outbox_sent(&p.message_id, now);
                notify_outbound_on_wire(&session, &p.message_id, now, &events_tx);
            }
            Err(e) => {
                session.mark_outbox_send_failed(&p.message_id, now);
                native_log::debug(
                    "outbox",
                    format!("resync send failed msg_id={}: {e}", p.message_id),
                );
            }
        }
    }
}

/// DM upkeep: open stream only when missing for a connected contact; else dial.
fn upkeep_dm_peers(
    swarm: &mut Swarm<ChatBehaviour>,
    session: Arc<SessionState>,
    control: stream::Control,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    for peer in session.dm_peer_ids() {
        if swarm.is_connected(&peer) {
            // Connected: open `/ghal-bol/msg/1.0.0` if needed.
            if !writer_open_for_peer(&writers, peer) {
                let session2 = Arc::clone(&session);
                let writers2 = Arc::clone(&writers);
                let events_tx2 = events_tx.clone();
                let control2 = control.clone();
                tokio::spawn(async move {
                    open_outbound_stream_if_needed(peer, control2, writers2, session2, events_tx2)
                        .await;
                });
            }
            continue;
        }
        // Not connected: discover + dial; do not drop stream map here.
        kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, peer);
        try_routed_dial(swarm, session.as_ref(), peer);
    }
}

/// Routed dial: kad/mDNS supply addresses via `handle_pending_outbound_connection`.
fn try_routed_dial(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, peer: PeerId) {
    // On mobile-data/CGNAT with coord configured, avoid blind peer-id dials from stale
    // peerstore entries; explicit coord/KAD/mDNS address dials are safer.
    if crate::coord_runtime::coord_is_configured() && session.prefers_mobile_coord_strategy() {
        return;
    }
    try_routed_dial_impl(swarm, session, peer);
}

fn sort_dm_dial_addrs_for_profile(
    session: &SessionState,
    addrs: Vec<Multiaddr>,
    for_coord_path: bool,
) -> Vec<Multiaddr> {
    if for_coord_path
        && crate::coord_runtime::coord_is_configured()
        && session.prefers_mobile_coord_strategy()
    {
        return super::dht_bootstrap::wan_kad_record_dial_addrs(addrs);
    }
    super::dht_bootstrap::sort_dm_dial_addrs(addrs)
}

/// Same as [try_routed_dial] but allowed after coord lookup miss (LAN/mDNS when coord has no record).
fn try_routed_dial_impl(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, peer: PeerId) {
    // Defensive: never attempt peer-id dials on mobile-data/CGNAT when coord is configured.
    // Those peerstore address sets often include stale CGNAT ports and even unsupported `/p2p/<id>`
    // entries, leading to long timeouts and "lucky" intermittent reachability.
    if crate::coord_runtime::coord_is_configured() && session.prefers_mobile_coord_strategy() {
        let now = chrono_now_ms();
        if session.should_log_dial_skip(peer, now, 8_000) {
            native_log::info(
                "dial",
                format!("skip routed dial {peer}: mobile coord strategy (relay-only)"),
            );
        }
        return;
    }
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if swarm.is_connected(&peer) {
        return;
    }
    let now = chrono_now_ms();
    if !session.should_routed_dial(peer, now, 2_000) {
        return;
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .condition(PeerCondition::DisconnectedAndNotDialing)
            .build(),
    ) {
        Ok(()) => native_log::debug("dial", format!("routed dial {peer}")),
        Err(DialError::NoAddresses) => {
            if session.log_kad_not_found_once(peer) {
                native_log::warn(
                    "dial",
                    format!("no dial addresses for {peer} yet (DHT/mDNS lookup in progress)"),
                );
            }
        }
        Err(DialError::DialPeerConditionFalse(_)) => {}
        Err(e) => native_log::debug("dial", format!("routed dial {peer}: {e}")),
    }
}

fn kad_lookup_dm_peers(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    let targets: Vec<PeerId> = session
        .dm_peer_ids()
        .into_iter()
        .filter(|p| !swarm.is_connected(p) && session.should_dial_libp2p_peer(*p))
        .collect();
    if targets.is_empty() {
        return;
    }
    native_log::info(
        "kad",
        format!("lookup {} dm peer(s) in DHT: {targets:?}", targets.len()),
    );
    for peer in targets {
        kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, peer);
    }
}

const MAX_IDENTIFY_DM_ADDRS_PER_PEER: usize = 4;

/// Merge dialable TCP listen addresses from identify (or signed peer record) into kad + peerstore.
fn ingest_identify_listen_addrs(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addrs: &[Multiaddr],
    tag: &str,
) {
    if session.is_bootstrap_peer(peer) {
        return;
    }
    // On mobile-data/CGNAT with coord configured, direct listen addrs are often stale/unreachable.
    // Do not pollute kad/peerstore with them; rely on coord relay-circuit dials instead.
    if crate::coord_runtime::coord_is_configured() && session.prefers_mobile_coord_strategy() {
        return;
    }
    let ranked = sort_dm_dial_addrs_for_profile(
        session,
        addrs
            .iter()
            .filter(|a| super::dht_bootstrap::is_dm_dial_multiaddr(a))
            .cloned()
            .collect(),
        true,
    );
    let mut added = 0usize;
    for addr in ranked.into_iter().take(MAX_IDENTIFY_DM_ADDRS_PER_PEER) {
        swarm.behaviour_mut().kademlia.add_address(&peer, addr);
        added += 1;
    }
    if added > 0 && session.should_dial_libp2p_peer(peer) && !swarm.is_connected(&peer) {
        native_log::info(
            tag,
            format!("identify {peer}: {added} tcp listen addr(s) ingested"),
        );
        try_routed_dial(swarm, session, peer);
    }
}

fn is_bare_peer_multiaddr(addr: &Multiaddr) -> bool {
    let mut it = addr.iter();
    matches!(it.next(), Some(Protocol::P2p(_))) && it.next().is_none()
}

fn dial_dm_peer_addr(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
    tag: &str,
) {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if session.is_bootstrap_peer(peer) {
        return;
    }
    // Multiple call-sites can attempt to dial the same DM peer concurrently:
    // - upkeep_dm_peers (kad lookup)
    // - coord_lookup_dm_peer (explicit relay dial)
    // - SendText fast-path ("send queued: coord lookup")
    // - UI bursts of RegisterDmPeer / foreground changes
    //
    // Without a shared throttle, libp2p cancels earlier dials (seen as "oneshot canceled"),
    // and relays can rate-limit / reject ("resource limit exceeded"), causing multi-minute stalls.
    let now = chrono_now_ms();
    let min_interval_ms = if tag == "coord" { 2_000 } else { 1_000 };
    if !session.should_routed_dial(peer, now, min_interval_ms) {
        return;
    }
    let is_relay = super::dht_bootstrap::is_relay_circuit_multiaddr(&addr);
    // On mobile-data with coord configured, only relay-circuit dials are reliable.
    // Direct TCP dials to transient CGNAT ports cause long timeouts and "queued forever" sends.
    if crate::coord_runtime::coord_is_configured() && session.prefers_mobile_coord_strategy() {
        if !is_relay {
            let now = chrono_now_ms();
            if session.should_log_dial_skip(peer, now, 8_000) {
                native_log::info(
                    "dial",
                    format!("skip direct dial {peer}: mobile coord strategy (addr not relay)"),
                );
            }
            return;
        }
    }
    #[cfg(target_os = "android")]
    {
        // Android swarm is TCP+noise only. Some relays advertise circuit addrs over QUIC/WebTransport;
        // dialing those cannot succeed and wastes the critical handover window.
        if is_relay && !is_tcp_multiaddr(&addr) {
            return;
        }
    }
    let loopback_coord = tag == "coord"
        && is_tcp_multiaddr(&addr)
        && super::dht_bootstrap::ipv4_from_ma_str(&addr.to_string())
            .is_some_and(|ip| ip.is_loopback());
    // Never dial "bare" `/p2p/<peer>` multiaddrs (invalid / guaranteed to fail).
    if is_bare_peer_multiaddr(&addr) {
        let now = chrono_now_ms();
        if session.should_log_dial_skip(peer, now, 8_000) {
            native_log::info("dial", format!("skip invalid dial addr for {peer}: {addr}"));
        }
        return;
    }
    if !loopback_coord && !super::dht_bootstrap::is_dm_dial_multiaddr(&addr) {
        return;
    }
    if !is_relay && !is_tcp_multiaddr(&addr) {
        return;
    }
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    swarm
        .behaviour_mut()
        .kademlia
        .add_address(&peer, dial_ma.clone());
    match swarm.dial(dial_ma.clone()) {
        Ok(()) => native_log::info(tag, format!("dialing {peer} via {dial_ma}")),
        Err(e) => native_log::debug(tag, format!("dial {peer} {dial_ma}: {e}")),
    }
}

fn log_closest_peers_result(session: &SessionState, peers: &[libp2p::kad::PeerInfo]) {
    if peers.is_empty() {
        let now = chrono_now_ms();
        if !session.should_log_kad_empty_closest(now) {
            return;
        }
        if session.any_bootstrap_connected.load(Ordering::Relaxed) {
            native_log::info("kad", "get_closest_peers: (empty)");
        } else {
            native_log::warn(
                "kad",
                "get_closest_peers: (empty) — public DHT bootstrap not connected yet. \
                 Same Wi‑Fi: mDNS should find peers without the DHT.",
            );
        }
        return;
    }
    let dm: HashSet<PeerId> = session.dm_peer_ids().into_iter().collect();
    let has_dm = peers.iter().any(|p| dm.contains(&p.peer_id) && !p.addrs.is_empty());
    if !has_dm {
        return;
    }
    let summary: Vec<String> = peers
        .iter()
        .filter(|p| dm.contains(&p.peer_id))
        .map(|p| format!("dm:{}({}addrs)", p.peer_id, p.addrs.len()))
        .collect();
    native_log::info("kad", format!("get_closest_peers: {}", summary.join(", ")));
}

/// DHT lookup + dial for configured DM peers (`get_closest_peers`).
fn dial_closest_peers_for_dm(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peers: Vec<libp2p::kad::PeerInfo>,
    emit: &mut dyn FnMut(GossipChatEvent),
) {
    let dm: HashSet<PeerId> = session.dm_peer_ids().into_iter().collect();
    let mut matched = false;
    for info in &peers {
        if !dm.contains(&info.peer_id) || info.addrs.is_empty() {
            continue;
        }
        matched = true;
        native_log::info(
            "kad",
            format!(
                "FindPeer {}: {} dial address(es)",
                info.peer_id,
                info.addrs.len()
            ),
        );
        dial_kad_discovered_peer(swarm, session, info.peer_id, info.addrs.clone(), emit);
    }
    if !matched && !peers.is_empty() {
        native_log::debug(
            "kad",
            format!(
                "get_closest_peers returned {} peer(s), none are configured DM peers with addresses",
                peers.len()
            ),
        );
    }
}

fn dial_kad_discovered_peer(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addrs: Vec<Multiaddr>,
    _emit: &mut dyn FnMut(GossipChatEvent),
) {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if swarm.is_connected(&peer) {
        return;
    }
    for addr in sort_dm_dial_addrs_for_profile(session, addrs, false) {
        if addr.is_empty() {
            continue;
        }
        dial_dm_peer_addr(swarm, session, peer, addr, "kad");
        break;
    }
}

fn dial_mdns_peer(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
    _emit: &mut dyn FnMut(GossipChatEvent),
) {
    if swarm.is_connected(&peer) {
        return;
    }
    dial_dm_peer_addr(swarm, session, peer, addr, "mdns");
}

/// Periodic connectivity summary (grep `Native/flow` or `connectivity` in App log export).
fn log_connectivity_snapshot(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    writers: &StreamWriters,
) {
    let local = *swarm.local_peer_id();
    let listens: Vec<String> = session
        .published_listen_snapshot()
        .iter()
        .take(6)
        .map(|a| a.to_string())
        .collect();
    let mut dm_lines = Vec::new();
    for peer in session.dm_peer_ids() {
        let connected = swarm.is_connected(&peer);
        let stream = writer_open_for_peer(writers, peer);
        let label = secp256k1_public_key_hex_from_peer_id(&peer)
            .map(|pk| crate::flow_log::short_hex(&pk))
            .unwrap_or_else(|| peer.to_string());
        dm_lines.push(format!("{label}:conn={connected},stream={stream}"));
    }
    let coord_cfg = crate::coord_runtime::coord_is_configured();
    let coord_reg = crate::coord_runtime::coord_is_registered();
    let dht_boot = session.any_bootstrap_connected.load(Ordering::Relaxed);
    let links = session.connected_peers().len();
    let net = session.network_profile_snapshot();
    let profile = net.mode_label();
    let hint = net.dial_hint();
    native_log::info(
        "flow",
        format!(
            "connectivity local={local} profile={profile} hint={hint} \
             listen_addrs={} [{}] dm=[{}] active_links={links} \
             dht_bootstrap_connected={dht_boot} coord_configured={coord_cfg} coord_registered={coord_reg}",
            listens.len(),
            listens.join(" | "),
            dm_lines.join(" "),
        ),
    );
}

/// Routine lookup/dial noise — do not surface to Flutter as link-down.
fn is_transient_swarm_dial_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("no addresses")
        || e.contains("disconnectedandnotdialing")
        || e.contains("dialpeerconditionfalse")
        || e.contains("connection refused")
        || e.contains("connection reset")
        || e.contains("timed out")
        || e.contains("transport error")
}

fn handle_swarm_event(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    event: SwarmEvent<ChatBehaviourEvent>,
    emit: &mut dyn FnMut(GossipChatEvent),
) {
    match event {
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(
            libp2p::kad::Event::OutboundQueryProgressed {
                result:
                    libp2p::kad::QueryResult::GetClosestPeers(Ok(libp2p::kad::GetClosestPeersOk {
                        peers,
                        ..
                    })),
                ..
            },
        )) => {
            log_closest_peers_result(session, &peers);
            dial_closest_peers_for_dm(swarm, session, peers, emit);
            for dm in session.dm_peer_ids() {
                try_routed_dial(swarm, session, dm);
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(
            libp2p::kad::Event::OutboundQueryProgressed {
                result:
                    libp2p::kad::QueryResult::GetClosestPeers(Err(
                        libp2p::kad::GetClosestPeersError::Timeout { peers, .. },
                    )),
                ..
            },
        )) => {
            log_closest_peers_result(session, &peers);
            dial_closest_peers_for_dm(swarm, session, peers, emit);
            for dm in session.dm_peer_ids() {
                try_routed_dial(swarm, session, dm);
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(
            libp2p::kad::Event::OutboundQueryProgressed {
                result: libp2p::kad::QueryResult::GetProviders(Ok(
                    libp2p::kad::GetProvidersOk::FoundProviders { providers, .. },
                )),
                ..
            },
        )) => {
            for provider in providers {
                if session.should_dial_libp2p_peer(provider) {
                    native_log::debug("kad", format!("DHT provider {provider}"));
                }
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(
            libp2p::kad::Event::OutboundQueryProgressed {
                result: libp2p::kad::QueryResult::GetRecord(Err(
                    libp2p::kad::GetRecordError::NotFound { key, .. },
                )),
                ..
            },
        )) => {
            if let Some(peer) = peer_id_from_record_key(&key) {
                if session.should_dial_libp2p_peer(peer) && session.log_kad_not_found_once(peer) {
                    native_log::warn("kad", format!("DHT record not found for {peer}"));
                }
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(
            libp2p::kad::Event::OutboundQueryProgressed {
                result: libp2p::kad::QueryResult::GetRecord(Ok(
                    libp2p::kad::GetRecordOk::FoundRecord(peer_record),
                )),
                ..
            },
        )) => {
            let mut addrs = decode_addr_record(&peer_record.record.value);
            if crate::coord_runtime::wan_discovery_via_coord_only() {
                addrs = super::dht_bootstrap::wan_kad_record_dial_addrs(addrs);
            }
            if addrs.is_empty() {
                return;
            }
            let target = peer_id_from_record_key(&peer_record.record.key).or(peer_record.peer);
            let Some(peer) = target else {
                return;
            };
            if !session.should_dial_libp2p_peer(peer) {
                return;
            }
            if swarm.is_connected(&peer) {
                return;
            }
            native_log::info(
                "kad",
                format!("DHT record for {peer}: {} address(es)", addrs.len()),
            );
            let ranked = sort_dm_dial_addrs_for_profile(session, addrs, true);
            for addr in ranked.into_iter().take(MAX_IDENTIFY_DM_ADDRS_PER_PEER) {
                dial_dm_peer_addr(swarm, session, peer, addr, "kad-record");
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(
            libp2p::kad::Event::OutboundQueryProgressed {
                result: libp2p::kad::QueryResult::Bootstrap(Ok(ok)),
                step,
                ..
            },
        )) if step.last => {
            if ok.num_remaining == 0 {
                native_log::info(
                    "kad",
                    format!("DHT bootstrap finished (last hop {})", ok.peer),
                );
            } else {
                native_log::info(
                    "kad",
                    format!(
                        "DHT bootstrap progress via {} ({} hop(s) remaining)",
                        ok.peer, ok.num_remaining
                    ),
                );
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(
            libp2p::kad::Event::OutboundQueryProgressed {
                result: libp2p::kad::QueryResult::Bootstrap(Err(e)),
                step,
                ..
            },
        )) if step.last => {
            native_log::warn("kad", format!("DHT bootstrap failed: {e}"));
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(
            libp2p::kad::Event::RoutingUpdated {
                peer,
                is_new_peer,
                ..
            },
        )) => {
            if session.is_dm_contact(peer) || session.is_bootstrap_peer(peer) {
                native_log::debug(
                    "kad",
                    format!(
                        "routing table {} peer {peer}",
                        if is_new_peer { "added" } else { "updated" }
                    ),
                );
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Kademlia(_)) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Identify(
            libp2p::identify::Event::Received { peer_id, info, .. },
        )) => {
            ingest_identify_listen_addrs(swarm, session, peer_id, &info.listen_addrs, "identify");
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Identify(
            libp2p::identify::Event::Pushed { .. },
        )) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Identify(_)) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Dcutr(
            libp2p::dcutr::Event {
                remote_peer_id,
                result: Ok(connection_id),
                ..
            },
        )) => {
            if session.is_dm_contact(remote_peer_id) {
                native_log::info(
                    "dcutr",
                    format!("direct connection upgrade to {remote_peer_id} (conn {connection_id:?})"),
                );
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Dcutr(
            libp2p::dcutr::Event {
                remote_peer_id,
                result: Err(e),
                ..
            },
        )) => {
            if session.is_dm_contact(remote_peer_id) {
                native_log::warn(
                    "dcutr",
                    format!("hole punch to DM peer {remote_peer_id} failed: {e}"),
                );
            } else if session.should_dial_libp2p_peer(remote_peer_id) {
                native_log::debug(
                    "dcutr",
                    format!("hole punch to {remote_peer_id} failed: {e}"),
                );
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Autonat(
            libp2p::autonat::Event::StatusChanged { new, .. },
        )) => match new {
            libp2p::autonat::NatStatus::Public(addr) => {
                native_log::info("autonat", format!("public reachability via {addr}"));
                let local = *swarm.local_peer_id();
                if session.merge_published_listen(vec![addr.clone()]) {
                    session.flush_kad_publish(&mut swarm.behaviour_mut().kademlia, &local, true);
                }
                crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                    &session.published_listen_snapshot(),
                );
            }
            libp2p::autonat::NatStatus::Private => {
                native_log::info("autonat", "behind NAT — relay+dcutr path");
            }
            libp2p::autonat::NatStatus::Unknown => {}
        },
        SwarmEvent::Behaviour(ChatBehaviourEvent::Autonat(_)) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Upnp(
            libp2p::upnp::Event::NewExternalAddr(external_addr),
        )) => {
            native_log::info("upnp", format!("external addr {external_addr}"));
            let local = *swarm.local_peer_id();
            if session.merge_published_listen(vec![external_addr.clone()]) {
                session.flush_kad_publish(&mut swarm.behaviour_mut().kademlia, &local, true);
                crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                    &session.published_listen_snapshot(),
                );
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Upnp(_)) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Mdns(libp2p::mdns::Event::Discovered(list))) => {
            for (peer, addr) in list {
                native_log::info("mdns", format!("discovered {peer} at {addr}"));
                dial_mdns_peer(swarm, session, peer, addr, emit);
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Mdns(libp2p::mdns::Event::Expired(_))) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Stream(_)) => {}
        SwarmEvent::NewListenAddr { address, .. } => {
            native_log::info("listen", format!("listening on {address}"));
            let local = *swarm.local_peer_id();
            let is_relay = super::dht_bootstrap::is_relay_circuit_multiaddr(&address);
            let expanded = if is_relay {
                vec![address.clone()]
            } else {
                expand_listen_addresses(&address)
            };
            if is_relay {
                let _ = session.merge_published_listen(vec![address.clone()]);
                session.flush_kad_publish(&mut swarm.behaviour_mut().kademlia, &local, true);
                native_log::info("relay", format!("relay listen addr {address}"));
            } else if session.merge_published_listen(expanded.clone()) {
                session.flush_kad_publish(&mut swarm.behaviour_mut().kademlia, &local, false);
            }
            crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                &session.published_listen_snapshot(),
            );
            if should_emit_listening_event(&address) {
                emit(GossipChatEvent::Listening(address.clone()));
            }
            if super::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr(&address) {
                crate::coord_runtime::schedule_register_presence_force();
                finish_wan_recovery_if_ready(session, swarm);
            }
        }
        SwarmEvent::ConnectionClosed { peer_id, .. } => {
            if session.consume_incidental_reject(peer_id) {
                return;
            }
            if session.is_dm_contact(peer_id) {
                native_log::info("swarm", format!("dm connection closed {peer_id}"));
                emit(GossipChatEvent::PeerDisconnected(peer_id));
            } else if session.is_bootstrap_peer(peer_id) {
                native_log::debug("swarm", format!("bootstrap connection closed {peer_id}"));
                session.refresh_bootstrap_connected_flag(swarm);
            }
            session.note_disconnected(&peer_id);
        }
        SwarmEvent::ConnectionEstablished {
            peer_id,
            endpoint,
            ..
        } => {
            session.register_inbound_dialer_if_needed(peer_id, &endpoint);
            if !session.is_kept_peer(peer_id) {
                session.mark_incidental_reject(peer_id);
                let _ = swarm.disconnect_peer_id(peer_id);
                return;
            }
            if session.is_dm_contact(peer_id) {
                native_log::info(
                    "swarm",
                    format!(
                        "dm connection established {peer_id} via {}",
                        endpoint.get_remote_address()
                    ),
                );
            } else {
                native_log::debug(
                    "swarm",
                    format!(
                        "bootstrap connection {peer_id} via {}",
                        endpoint.get_remote_address()
                    ),
                );
            }
            if session.is_bootstrap_peer(peer_id) {
                session.note_bootstrap_connected();
                let remote = endpoint.get_remote_address().clone();
                let reservation_addr =
                    tcp_relay_reservation_addr(peer_id, &remote).unwrap_or(remote.clone());
                if let Ok(mut m) = session.bootstrap_relay_addr.write() {
                    m.insert(peer_id, reservation_addr.clone());
                }
                let force = session.wan_recovery_active.load(Ordering::Relaxed);
                try_relay_reservation(swarm, session, peer_id, &reservation_addr, force);
            }
        }
        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            let Some(peer) = peer_id else {
                return;
            };
            if session.is_bootstrap_peer(peer) {
                if session.should_log_bootstrap_dial_err(peer, chrono_now_ms()) {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue=bootstrap_dial_error | peer={peer} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                }
            } else if session.should_dial_libp2p_peer(peer) {
                let err_s = format!("{error}");
                if err_s.contains("Relay has no reservation") {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue=relay_peer_not_listening | peer={peer} | check=wait for relay reservation + coord register | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else if err_s.contains("p2p-circuit") {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue=relay_circuit_dial_failed | peer={peer} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else if is_transient_swarm_dial_error(&err_s) {
                    native_log::debug("dial", format!("transient dial {peer}: {error}"));
                } else {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue=outgoing_connection_error | peer={peer_id:?} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                }
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Relay(
            libp2p::relay::client::Event::ReservationReqAccepted {
                relay_peer_id,
                ..
            },
        )) => {
            native_log::info("relay", format!("reservation accepted on {relay_peer_id}"));
            crate::coord_runtime::coord_note_relay_reservation(relay_peer_id);
            let local = *swarm.local_peer_id();
            let relay_addrs: Vec<Multiaddr> = swarm
                .listeners()
                .filter(|ma| super::dht_bootstrap::is_relay_circuit_multiaddr(ma))
                .cloned()
                .collect();
            if !relay_addrs.is_empty() {
                let _ = session.merge_published_listen(relay_addrs);
            }
            session.flush_kad_publish(&mut swarm.behaviour_mut().kademlia, &local, true);
            crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                &session.published_listen_snapshot(),
            );
            if crate::coord_runtime::has_coord_endpoints() {
                crate::coord_runtime::schedule_register_presence_force();
            }
            finish_wan_recovery_if_ready(session, swarm);
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Relay(_)) => {}
        _ => {}
    }
}

/// Collect TCP listen addrs and publish to DHT before `node_ready` (peers often dial immediately).
async fn bootstrap_publishable_listen(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    timeout: Duration,
) {
    let coord_mode = crate::coord_runtime::wan_discovery_via_coord_only();
    let deadline = time::Instant::now() + timeout;
    let local = *swarm.local_peer_id();
    while time::Instant::now() < deadline {
        if listen_ready_for_node(session, coord_mode, swarm) {
            session.flush_kad_publish(&mut swarm.behaviour_mut().kademlia, &local, true);
            return;
        }
        tokio::select! {
            ev = swarm.select_next_some() => {
                if let SwarmEvent::NewListenAddr { address, .. } = ev {
                    let is_relay = super::dht_bootstrap::is_relay_circuit_multiaddr(&address);
                    let expanded = if is_relay {
                        vec![address.clone()]
                    } else {
                        expand_listen_addresses(&address)
                    };
                    if session.merge_published_listen(expanded.clone()) {
                        session.flush_kad_publish(
                            &mut swarm.behaviour_mut().kademlia,
                            &local,
                            is_relay,
                        );
                        crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                            &session.published_listen_snapshot(),
                        );
                    }
                }
            }
            _ = time::sleep(Duration::from_millis(40)) => {}
        }
    }
    if !listen_ready_for_node(session, coord_mode, swarm) {
        if coord_mode {
            native_log::warn(
                "listen",
                "no relay circuit before node_ready — WAN peers cannot dial this device until \
                 reservation accepted (check public DHT bootstrap connectivity)",
            );
        } else {
            native_log::warn(
                "listen",
                "no publishable TCP listen addr before node_ready — peers may not find this device yet",
            );
        }
    }
}

const COORD_LOOKUP_INTERVAL_SECS: u64 = 5;
const NETWORK_PROFILE_POLL_SECS: u64 = 1;
#[cfg(target_os = "android")]
const BOOTSTRAP_REDIAL_INTERVAL_SECS: u64 = 12;
#[cfg(not(target_os = "android"))]
const BOOTSTRAP_REDIAL_INTERVAL_SECS: u64 = 30;
/// Bootstrap TCP can look "connected" on a dead Wi‑Fi route; force redial after this.
const WAN_RECOVERY_BOOTSTRAP_STALE_MS: i64 = 10_000;

async fn coord_lookup_dm_peer(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    public_key_hex: &str,
) {
    let pk = public_key_hex.trim();
    if pk.len() != 66 {
        return;
    }
    let now_ms = chrono_now_ms();
    // Hard gate: when coord says "peer_not_on_server" we must not spam lookups from multiple
    // call-sites (register/send/upkeep). Backoff is per-recipient public key.
    if let Ok(m) = session.coord_lookup_backoff.read() {
        if let Some(b) = m.get(pk) {
            if now_ms < b.next_allowed_ms {
                // Keep this log lightweight; it is only emitted when some code path insists on
                // calling coord_lookup_dm_peer too frequently.
                native_log::debug(
                    "coord",
                    format!(
                        "lookup {pk} skipped (peer_not_on_server backoff; retry_in_ms={})",
                        b.next_allowed_ms.saturating_sub(now_ms)
                    ),
                );
                return;
            }
        }
    }
    let target = peer_id_from_secp256k1_public_key_hex(pk)
        .ok()
        .and_then(|s| s.parse::<PeerId>().ok());
    let Some(target) = target else {
        return;
    };
    if swarm.is_connected(&target) {
        return;
    }
    // On mobile-data, do not spam coord lookups/dials before we have registered at least one
    // relay/public endpoint. Until then, the peer will often be "not on server" and even if
    // coord returns an addr, the remote may not be listening yet.
    if crate::coord_runtime::coord_is_configured()
        && session.prefers_mobile_coord_strategy()
        && !crate::coord_runtime::coord_is_registered()
    {
        // Still allow DHT/mDNS lookup below; it can succeed on same LAN.
        native_log::debug(
            "coord",
            format!("lookup {pk} skipped (self not registered yet; waiting for relay listen)"),
        );
        if !swarm.is_connected(&target) {
            native_log::info(
                "kad",
                format!("coord miss or dial pending for {pk} — DHT lookup for {target}"),
            );
            kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, target);
            try_routed_dial(swarm, session, target);
        }
        return;
    }
    match crate::coord_runtime::lookup_dial_multiaddrs_for_public_key_async(pk).await {
        Ok(addrs) => {
            session.clear_coord_lookup_backoff(pk);
            // On mobile-data with coord configured, direct TCP listen addrs from coord presence
            // are frequently stale/unreachable (CGNAT port churn). Prefer relay-circuit addrs only.
            let addrs = if crate::coord_runtime::coord_is_configured()
                && session.prefers_mobile_coord_strategy()
            {
                super::dht_bootstrap::wan_kad_record_dial_addrs(addrs)
            } else {
                addrs
            };
            if addrs.is_empty() {
                native_log::warn(
                    "coord",
                    format!(
                        "lookup {pk} returned no dialable addrs (check presence endpoints / relay)"
                    ),
                );
            } else {
                // Important: do not spam-dial every returned addr. Overlapping dials cancel each
                // other ("oneshot canceled") and can trigger relay rate limits.
                let ranked = sort_dm_dial_addrs_for_profile(session, addrs, true);
                native_log::info(
                    "coord",
                    format!("coord_lookup_peer ok — dialing {} addr(s)", ranked.len().min(1)),
                );
                if let Some(ma) = ranked.into_iter().next() {
                    dial_dm_peer_addr(swarm, session, target, ma, "coord");
                }
            }
        }
        Err(e) => {
            let es = e.to_string();
            if es.contains("404") || es.contains("peer_not_on_server") {
                session.note_coord_lookup_not_found(pk, now_ms);
            }
            native_log::info(
                "coord",
                format!("lookup {pk} failed ({e}) — falling back to DHT/mDNS"),
            );
        }
    }
    if !swarm.is_connected(&target) {
        native_log::info(
            "kad",
            format!("coord miss or dial pending for {pk} — DHT lookup for {target}"),
        );
        kad_lookup_peer(&mut swarm.behaviour_mut().kademlia, target);
        try_routed_dial(swarm, session, target);
    }
}

async fn coord_lookup_dm_peers(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    for pk in session.dm_public_keys() {
        coord_lookup_dm_peer(swarm, session, &pk).await;
    }
}

/// Blocking-IO friendly run used by **`p2p_ffi`**: poll outbound commands and emit events on std channels.
pub async fn run_gossip_chat_node_with_std_io(
    config: GossipChatConfig,
    identity: crate::DecryptedIdentity,
    outbound_rx: std::sync::mpsc::Receiver<OutboundCmd>,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    stop: Arc<AtomicBool>,
) -> Result<(), ChatServerError> {
    let bootstrap = config.bootstrap_peers.clone();
    native_log::info("p2p", "building swarm");
    let mut swarm = build_swarm(&config)?;
    native_log::info("p2p", "swarm built, opening chat stream accept");
    let mut control = swarm.behaviour().stream.new_control();
    let protocol = StreamProtocol::new(STREAM_PROTOCOL);
    let mut incoming = control
        .accept(protocol.clone())
        .map_err(|e| ChatServerError::Transport(format!("accept stream: {e}")))?;
    let writers: StreamWriters = Arc::new(Mutex::new(HashMap::new()));

    let coord_only = crate::coord_runtime::wan_discovery_via_coord_only();
    let public_dht = resolve_public_dht_bootnodes().await;
    if coord_only {
        native_log::info(
            "coord",
            format!(
                "coord URL set — peer discovery via server; dialing {} public DHT bootstrap(s) for relay",
                public_dht.len()
            ),
        );
    }
    let net = super::dht_bootstrap::detect_local_network_profile();
    let bootstrap_peer_ids: HashSet<PeerId> = public_dht.iter().map(|(p, _)| *p).collect();
    let session = Arc::new(SessionState::new(
        identity,
        &config.dm_peers,
        bootstrap_peer_ids,
        config.transcript_path.clone(),
        config.app_namespace.clone(),
        net.clone(),
    )?);
    native_log::info(
        "p2p",
        format!(
            "swarm up: dm_peers={} invite_bootstrap={} public_dht_addrs={} coord_only={coord_only}",
            session.dm_peer_ids().len(),
            bootstrap.len(),
            public_dht.len()
        ),
    );
    native_log::info(
        "net",
        format!(
            "profile={} hint={} wifi={} cellular={} tether={} usb={} lan={} cgnat={} public4={} global6={}",
            net.mode_label(),
            net.dial_hint(),
            net.has_wifi_iface,
            net.has_cellular_iface,
            net.has_tether_iface,
            net.has_usb_iface,
            net.has_rfc1918_ipv4,
            net.has_cgnat_ipv4,
            net.has_public_ipv4,
            net.has_global_ipv6
        ),
    );
    seed_kad_routing_table(
        &mut swarm.behaviour_mut().kademlia,
        &bootstrap,
        &public_dht,
    );
    bootstrap_kad(&mut swarm.behaviour_mut().kademlia);

    listen_swarm_transports(&mut swarm)?;
    // Do not block the swarm loop for relay reservation; coord_register_tick retries register.
    let listen_wait = if coord_only {
        Duration::from_secs(3)
    } else {
        Duration::from_millis(800)
    };
    bootstrap_publishable_listen(&mut swarm, &session, listen_wait).await;
    crate::coord_runtime::rebuild_coord_endpoints_from_listen(
        &session.published_listen_snapshot(),
    );
    // Listen first, then connect public DHT bootnodes (relay reservation) and invite/bootstrap peers.
    dial_public_dht_bootnodes(&mut swarm, &session, &public_dht);
    dial_bootstrap_peers(&mut swarm, &bootstrap, &mut |ev| {
        let _ = events_tx.send(ev);
    });
    if let (Some(path), Some(ns)) = (&config.transcript_path, &config.app_namespace) {
        let path = Path::new(path);
        let ns = ns.trim();
        if !path.as_os_str().is_empty() && !ns.is_empty() {
            bootstrap_outbox_from_transcript(session.as_ref(), path, ns);
        }
    }
    // Bootstrap consumes NewListenAddr without emitting; replay snapshot so poll/tests see TCP listen.
    for addr in session.published_listen_snapshot() {
        if should_emit_listening_event(&addr) {
            let _ = events_tx.send(GossipChatEvent::Listening(addr));
        }
    }
    let _ = events_tx.send(GossipChatEvent::NodeReady);
    native_log::info("p2p", "node ready");
    coord_lookup_dm_peers(&mut swarm, session.as_ref()).await;
    let session_for_swarm = Arc::clone(&session);

    let mut poll_tick = time::interval(Duration::from_millis(25));
    // If bootstrap peers drop (idle timeout / network quirks), relay reservations and coord
    // registration can silently stall until we re-dial. This must be frequent enough to keep
    // background connectivity from "taking minutes" to recover.
    let mut redial_tick = time::interval(Duration::from_secs(BOOTSTRAP_REDIAL_INTERVAL_SECS));
    redial_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut kad_lookup_tick = time::interval(Duration::from_secs(15));
    kad_lookup_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut coord_tick = time::interval(Duration::from_secs(COORD_LOOKUP_INTERVAL_SECS));
    coord_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut dm_upkeep_tick = time::interval(Duration::from_secs(1));
    dm_upkeep_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stream_upkeep_tick = time::interval(Duration::from_secs(1));
    stream_upkeep_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut flow_snapshot_tick = time::interval(Duration::from_secs(30));
    flow_snapshot_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut network_tick = time::interval(Duration::from_secs(NETWORK_PROFILE_POLL_SECS));
    network_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // First snapshot soon after node_ready so exports show initial listen/coord state.
    flow_snapshot_tick.tick().await;
    network_tick.tick().await;

    loop {
        if stop.load(Ordering::SeqCst) {
            break Ok(());
        }
        drain_outbound_queue(
            &outbound_rx,
            &mut swarm,
            Arc::clone(&session),
            Arc::clone(&writers),
            control.clone(),
            Some(events_tx.clone()),
            MAX_OUTBOUND_CMDS_PER_TICK,
        )
        .await;
        select! {
            biased;
            _ = stream_upkeep_tick.tick() => {
                for pid in session.connected_peers() {
                    if !session.is_dm_contact(pid)
                        || !swarm.is_connected(&pid)
                        || writer_open_for_peer(&writers, pid)
                    {
                        continue;
                    }
                    let session2 = Arc::clone(&session);
                    let writers2 = Arc::clone(&writers);
                    let events_tx2 = events_tx.clone();
                    let control2 = control.clone();
                    tokio::spawn(async move {
                        open_outbound_stream_if_needed(
                            pid,
                            control2,
                            writers2,
                            session2,
                            Some(events_tx2),
                        )
                        .await;
                    });
                }
                if session.pending_read_ack_len() > 0 {
                    let connected_now: Vec<PeerId> = session
                        .connected_peers()
                        .into_iter()
                        .filter(|p| session.is_dm_contact(*p) && swarm.is_connected(p))
                        .collect();
                    if !connected_now.is_empty() {
                        let session3 = Arc::clone(&session);
                        let writers3 = Arc::clone(&writers);
                        tokio::spawn(async move {
                            run_ack_upkeep(session3, writers3, &connected_now).await;
                        });
                    }
                }
            }
            _ = dm_upkeep_tick.tick() => {
                upkeep_dm_peers(
                    &mut swarm,
                    Arc::clone(&session),
                    control.clone(),
                    Arc::clone(&writers),
                    Some(events_tx.clone()),
                );
                let connected_now: Vec<PeerId> = session
                    .connected_peers()
                    .into_iter()
                    .filter(|p| session.is_dm_contact(*p) && swarm.is_connected(p))
                    .collect();
                if let (Some(path), Some(ns)) = (
                    &session.transcript_path,
                    &session.app_namespace,
                ) {
                    transcript_sync_outbound_tick(
                        session.as_ref(),
                        Path::new(path),
                        ns.trim(),
                    );
                }
                resync_pending_outbox(
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    connected_now.clone(),
                    Some(events_tx.clone()),
                    Some(control.clone()),
                )
                .await;
                flush_pending_call_signals(
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    connected_now.clone(),
                    Some(events_tx.clone()),
                )
                .await;
                if session.pending_read_ack_len() > 0 && !connected_now.is_empty() {
                    run_ack_upkeep(
                        Arc::clone(&session),
                        Arc::clone(&writers),
                        &connected_now,
                    )
                    .await;
                }
            }
            _ = kad_lookup_tick.tick() => {
                kad_lookup_dm_peers(&mut swarm, &session);
            }
            _ = network_tick.tick() => {
                let recovering_before = session.wan_recovery_active.load(Ordering::Relaxed);
                let mut handover = false;
                let forced = take_network_change_notify();
                if forced {
                    let net = super::dht_bootstrap::detect_local_network_profile();
                    let (old_mode, new_mode, changed) = if let Ok(mut cur) = session.network_profile.write() {
                        let old_key = super::dht_bootstrap::network_handover_key(&*cur);
                        let old_mode = cur.mode_label().to_string();
                        let new_key = super::dht_bootstrap::network_handover_key(&net);
                        *cur = net;
                        let new_mode = cur.mode_label().to_string();
                        (old_mode, new_mode, old_key != new_key)
                    } else {
                        continue;
                    };
                    if changed {
                        native_log::info(
                            "net",
                            format!("connectivity callback — handover ({old_mode} -> {new_mode})"),
                        );
                        handle_network_path_change(
                            &mut swarm,
                            session.as_ref(),
                            &public_dht,
                            &old_mode,
                            &new_mode,
                        );
                        handover = true;
                    }
                } else if let Some((old_mode, new_mode)) =
                    session.refresh_network_path_if_changed()
                {
                    handle_network_path_change(
                        &mut swarm,
                        session.as_ref(),
                        &public_dht,
                        &old_mode,
                        &new_mode,
                    );
                    handover = true;
                } else if !recovering_before {
                    try_wan_relay_recovery(&mut swarm, session.as_ref());
                }
                if session.wan_recovery_active.load(Ordering::Relaxed) {
                    run_wan_recovery_pass(&mut swarm, session.as_ref(), &public_dht);
                }
                let recovering_after = session.wan_recovery_active.load(Ordering::Relaxed);
                if handover || (recovering_before && !recovering_after) {
                    coord_lookup_dm_peers(&mut swarm, session.as_ref()).await;
                }
            }
            _ = coord_tick.tick() => {
                // If relay/coord readiness dropped while the UI was closed (background only),
                // proactively re-enter WAN recovery so we don't wait a minute for redial_tick.
                if crate::coord_runtime::wan_discovery_via_coord_only()
                    && !wan_recovery_satisfied(session.as_ref(), &swarm)
                    && !session.wan_recovery_active.load(Ordering::Relaxed)
                {
                    native_log::info("net", "WAN not ready — begin recovery pass");
                    session.begin_wan_recovery();
                }
                if session.wan_recovery_active.load(Ordering::Relaxed) {
                    run_wan_recovery_pass(&mut swarm, session.as_ref(), &public_dht);
                } else {
                    let listen = coord_register_listen_snapshot(&swarm, session.as_ref());
                    crate::coord_runtime::coord_register_tick(&listen);
                    try_wan_relay_recovery(&mut swarm, session.as_ref());
                }
                coord_lookup_dm_peers(&mut swarm, session.as_ref()).await;
            }
            _ = flow_snapshot_tick.tick() => {
                log_connectivity_snapshot(&swarm, session.as_ref(), &writers);
            }
            _ = redial_tick.tick() => {
                seed_kad_routing_table(
                    &mut swarm.behaviour_mut().kademlia,
                    &bootstrap,
                    &public_dht,
                );
                if !session.any_bootstrap_connected.load(Ordering::Relaxed) {
                    bootstrap_kad(&mut swarm.behaviour_mut().kademlia);
                    dial_public_dht_bootnodes(&mut swarm, &session, &public_dht);
                    let mut tcp_first: Vec<Multiaddr> = Vec::new();
                    let mut other: Vec<Multiaddr> = Vec::new();
                    for ma in &bootstrap {
                        if ma.is_empty() {
                            continue;
                        }
                        if is_tcp_multiaddr(ma) {
                            tcp_first.push(ma.clone());
                        } else if !is_quic_multiaddr(ma) {
                            other.push(ma.clone());
                        }
                    }
                    for ma in tcp_first.iter().chain(other.iter()) {
                        if !super::dht_bootstrap::is_trusted_bootstrap_dial_addr(ma) {
                            continue;
                        }
                        let skip = dial_opts_peer_hint(ma)
                            .is_some_and(|pid| swarm.is_connected(&pid));
                        if skip {
                            continue;
                        }
                        if let Err(e) = swarm.dial(ma.clone()) {
                            native_log::debug("dial", format!("bootstrap redial {ma}: {e}"));
                        }
                    }
                }
                let force = session.wan_recovery_active.load(Ordering::Relaxed);
                retry_stalled_relay_reservations(&mut swarm, session.as_ref(), force);
            }
            _ = poll_tick.tick() => {
                if session.pending_read_ack_len() > 0 || session.pending_delivery_ack_len() > 0 {
                    let connected_now: Vec<PeerId> = session
                        .connected_peers()
                        .into_iter()
                        .filter(|p| session.is_dm_contact(*p) && swarm.is_connected(p))
                        .collect();
                    if !connected_now.is_empty() {
                        run_ack_upkeep(
                            Arc::clone(&session),
                            Arc::clone(&writers),
                            &connected_now,
                        )
                        .await;
                    }
                }
            }
            incoming_pair = incoming.next() => {
                if let Some((peer, stream)) = incoming_pair {
                    native_log::info("stream", format!("inbound chat stream from {peer}"));
                    let session2 = Arc::clone(&session);
                    let writers2 = Arc::clone(&writers);
                    let events_tx2 = events_tx.clone();
                    let control2 = control.clone();
                    tokio::spawn(handle_inbound_stream(
                        peer,
                        stream,
                        session2,
                        writers2,
                        Some(events_tx2),
                        control2,
                    ));
                }
            }
            ev = swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } = &ev {
                    if let Some(pk) = session.register_inbound_dialer_if_needed(*peer_id, endpoint)
                    {
                        let _ = events_tx.send(GossipChatEvent::PeerIdentified {
                            peer_id: *peer_id,
                            public_key_hex: pk,
                        });
                    }
                    if session.is_bootstrap_peer(*peer_id) {
                        // DHT bootstrap only — not a chat contact.
                    } else if session.is_dm_contact(*peer_id) {
                        let pid = *peer_id;
                        native_log::info("swarm", format!("dm peer connected {pid}"));
                        session.note_connected(pid);
                        let _ = events_tx.send(GossipChatEvent::PeerConnected(pid));
                        let session2 = Arc::clone(&session);
                        let writers2 = Arc::clone(&writers);
                        let events_tx2 = events_tx.clone();
                        let control2 = control.clone();
                        tokio::spawn(async move {
                            on_dm_peer_connected(
                                session2,
                                control2,
                                writers2,
                                pid,
                                Some(events_tx2),
                            )
                            .await;
                        });
                    }
                }
                if let SwarmEvent::ConnectionClosed { peer_id, .. } = &ev {
                    if !session.consume_incidental_reject(*peer_id) {
                        if session.is_dm_contact(*peer_id) {
                            native_log::info("swarm", format!("dm peer disconnected {peer_id}"));
                            let _ = events_tx.send(GossipChatEvent::PeerDisconnected(*peer_id));
                        }
                        session.note_disconnected(peer_id);
                        if let Ok(mut g) = writers.lock() {
                            g.remove(peer_id);
                        }
                    }
                }
                handle_swarm_event(&mut swarm, &session_for_swarm, ev, &mut |ev| {
                    let _ = events_tx.send(ev);
                });
                drain_outbound_queue(
                    &outbound_rx,
                    &mut swarm,
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    control.clone(),
                    Some(events_tx.clone()),
                    MAX_OUTBOUND_CMDS_PER_TICK,
                )
                .await;
            }
        }
    }
}

