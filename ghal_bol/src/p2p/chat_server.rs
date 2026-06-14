//! libp2p **direct-message** node: **QUIC/TCP**, **relay**, **mDNS**, and **`/ghal-bol/msg/1.0.0`** streams.
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
static RELAY_REFRESH_NOTIFY: AtomicBool = AtomicBool::new(false);
/// DM peer left LAN / link dropped — run coord lookup on the swarm loop without waiting for coord_tick.
static COORD_LOOKUP_NOTIFY: AtomicBool = AtomicBool::new(false);
/// Network handover or chat stream death — reopen `/ghal-bol/msg/1.0.0` without waiting for upkeep tick.
static STREAM_REOPEN_NOTIFY: AtomicBool = AtomicBool::new(false);

/// Optional hook (set by `p2p_runtime`) to drop buffered invite poll events on remote hangup.
static DROP_PENDING_CALL_INVITE: OnceLock<fn(&str)> = OnceLock::new();

pub fn set_drop_pending_call_invite_hook(f: fn(&str)) {
    let _ = DROP_PENDING_CALL_INVITE.set(f);
}

fn drop_pending_call_invite(call_id: &str) {
    if let Some(f) = DROP_PENDING_CALL_INVITE.get() {
        f(call_id);
    }
}

/// Match Flutter `CallController._maxLiveInviteAgeMs` — stale invites must not ring or notify.
use crate::call_state::call_invite_is_live;

#[inline]
fn platform_incoming_call_show(peer_pk: &str, call_id: &str) {
    if app_ui_visible() {
        return;
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::incoming_call_notify::show_incoming_call(peer_pk, call_id);
}

#[inline]
fn platform_incoming_call_dismiss() {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    crate::incoming_call_notify::dismiss_incoming_call();
}

fn on_local_call_signal_sent(call_id: &str, kind: crate::call_sig_v1::CallSigKind) {
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

/// Android connectivity / default-network change — swarm loop re-runs handover recovery.
pub fn notify_network_change() {
    NETWORK_CHANGE_NOTIFY.store(true, Ordering::SeqCst);
}

/// Re-fetch `/v1/relay` and re-dial the co-located relay (e.g. `p2p_start` `already_running`
/// while bore/ngrok relay came up after the swarm started).
pub fn notify_relay_refresh() {
    RELAY_REFRESH_NOTIFY.store(true, Ordering::SeqCst);
}

pub(crate) fn take_network_change_notify() -> bool {
    NETWORK_CHANGE_NOTIFY.swap(false, Ordering::SeqCst)
}

pub(crate) fn take_relay_refresh_notify() -> bool {
    RELAY_REFRESH_NOTIFY.swap(false, Ordering::SeqCst)
}

pub(crate) fn notify_coord_lookup() {
    COORD_LOOKUP_NOTIFY.store(true, Ordering::SeqCst);
}

pub(crate) fn take_coord_lookup_notify() -> bool {
    COORD_LOOKUP_NOTIFY.swap(false, Ordering::SeqCst)
}

pub(crate) fn notify_stream_reopen() {
    STREAM_REOPEN_NOTIFY.store(true, Ordering::SeqCst);
}

pub(crate) fn take_stream_reopen_notify() -> bool {
    STREAM_REOPEN_NOTIFY.swap(false, Ordering::SeqCst)
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

static FOREGROUND_PEER_CMD_GEN: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Bump on each `p2p_set_foreground_peer` / `sync_ui_session` room change.
pub fn bump_foreground_peer_cmd_gen() -> u64 {
    FOREGROUND_PEER_CMD_GEN.fetch_add(1, Ordering::SeqCst) + 1
}

fn foreground_peer_cmd_gen_latest() -> u64 {
    FOREGROUND_PEER_CMD_GEN.load(Ordering::SeqCst)
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

fn emit_call_media(
    tx: &Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    call_id: &str,
    peer_public_key_hex: &str,
    state: &str,
    reason: Option<&str>,
) {
    let Some(tx) = tx else {
        return;
    };
    let snap = super::call_active::snapshot();
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

/// UI visibility gate (protonet: read state only while chatroom is active / app visible).
/// When false: inbound text gets `ack_received` only; no `ack_read` enqueue, seed, or upkeep.
static APP_ACK_READ_ENABLED: OnceLock<AtomicBool> = OnceLock::new();

/// When true: Flutter UI is visible — skip OS incoming-call notification (in-app ring only).
static APP_UI_VISIBLE: OnceLock<AtomicBool> = OnceLock::new();

fn app_ui_visible_mx() -> &'static AtomicBool {
    APP_UI_VISIBLE.get_or_init(|| AtomicBool::new(false))
}

/// Called from FFI when the app foreground/background changes.
pub fn set_app_ui_visible(visible: bool) {
    app_ui_visible_mx().store(visible, Ordering::SeqCst);
    if visible {
        platform_incoming_call_dismiss();
    }
}

pub fn app_ui_visible() -> bool {
    app_ui_visible_mx().load(Ordering::SeqCst)
}

fn app_ack_read_enabled_mx() -> &'static AtomicBool {
    APP_ACK_READ_ENABLED.get_or_init(|| AtomicBool::new(false))
}

/// Single gate for **new** in-room read receipts (inbound text, enter-room catch-up).
/// Leave backlog retries use `pending_read_acks` upkeep and are not gated on UI visibility.
fn may_send_in_room_read_ack(session: &SessionState, peer: PeerId) -> bool {
    app_ui_visible()
        && app_ack_read_enabled()
        && session.is_foreground_peer(peer)
}

/// Called from FFI when the app backgrounds or UI is torn down.
pub fn set_app_ack_read_enabled(enabled: bool) {
    app_ack_read_enabled_mx().store(enabled, Ordering::SeqCst);
}

pub fn queue_read_ack_catchup(out_tx: &std::sync::mpsc::Sender<OutboundCmd>, peer: ContactPk) {
    if !app_ui_visible() || !app_ack_read_enabled() || !live_foreground_peer().is_some_and(|f| f == peer) {
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
use libp2p::swarm::{DialError, NetworkBehaviour, Swarm, SwarmEvent, ConnectionId};
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

use super::network_transport::{expand_listen_addresses, peer_id_from_multiaddr};
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
    /// Outbound call signal written to the DM stream (UI must not show "ringing" before this).
    CallSignalSent {
        call_id: String,
        signal: String,
        recipient_public_key_hex: String,
    },
    /// Native voice/video lifecycle — Flutter updates UI only (Phase E).
    CallMedia {
        call_id: String,
        peer_public_key_hex: String,
        state: String,
        camera_on: bool,
        remote_video_on: bool,
        reason: Option<String>,
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
        /// Monotonic id — drop stale close/open when Flutter/Daemon RPCs reorder on the outbound queue.
        generation: u64,
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
    /// Start native voice media for an active call (opens the `/ghal-bol/call/1.0.0` substream).
    CallMediaStart {
        call_id: String,
        peer_public_key_hex: String,
    },
    /// Tear down native voice media for a call (hangup / decline / failure).
    CallMediaStop {
        call_id: String,
    },
    /// Mute/unmute the local microphone for an active call (keeps the clock running).
    CallMediaSetMicMuted {
        call_id: String,
        muted: bool,
    },
    /// Route playout to speakerphone vs earpiece (Android `:p2p`; `setSpeakerphoneOn` only).
    CallMediaSetSpeaker {
        call_id: String,
        speaker_on: bool,
    },
    /// Start native video for an active call (opens the `/ghal-bol/call-video/1.0.0` substream).
    CallVideoStart {
        call_id: String,
        peer_public_key_hex: String,
        /// When true, captured frames are encoded and sent (camera on at start).
        camera_enabled: bool,
    },
    /// Tear down native video for a call.
    CallVideoStop {
        call_id: String,
    },
    /// Turn the local camera on/off mid-call (keeps the session/transport up).
    CallVideoSetCameraEnabled {
        call_id: String,
        enabled: bool,
    },
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
    /// 0 = send immediately; after wire success, upkeep waits `OUTBOX_RESEND_INTERVAL_MS`.
    last_send_ms: i64,
}

#[derive(Clone)]
struct PendingDeliveryAck {
    peer_id: PeerId,
    inbound_id: String,
    recipient_public_key_hex: String,
}

#[derive(Clone)]
struct PendingCallSignal {
    call_id: String,
    signal_kind: CallSigKind,
    frame: Vec<u8>,
    peer_id: PeerId,
    recipient_public_key_hex: String,
    created_at_ms: i64,
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
    /// Dialable listen addresses accumulated across listeners (coord/mDNS).
    published_listen: RwLock<Vec<Multiaddr>>,
    /// Throttle routed dials / stream-open attempts (ms since epoch).
    routed_dial_attempt_ms: RwLock<HashMap<PeerId, i64>>,
    /// Relay-circuit coord dials — separate from peer-id routed dials so LAN upkeep NoAddresses
    /// spam does not block WAN `/p2p-circuit` handshakes (different throttle semantics).
    circuit_coord_dial_last_ms: RwLock<HashMap<PeerId, i64>>,
    stream_open_log_emitted: RwLock<HashSet<PeerId>>,
    /// Prevent concurrent open_stream storms per peer (causes "receiver is gone"/oneshot canceled).
    stream_open_inflight: RwLock<HashSet<PeerId>>,
    /// Coord relay peer ids (for logging + relay reservation).
    bootstrap_peer_ids: RwLock<HashSet<PeerId>>,
    relay_reserve_requested: RwLock<HashSet<PeerId>>,
    /// Throttle `listen_on(/p2p-circuit)` attempts per relay peer.
    /// Repeated listen attempts create large listen/behaviour churn and can delay WAN readiness.
    relay_reserve_last_attempt_ms: RwLock<HashMap<PeerId, i64>>,
    /// One in-flight `listen_on(/p2p-circuit)` per relay until accepted or timeout.
    relay_reserve_in_flight_ms: RwLock<HashMap<PeerId, i64>>,
    /// Live bootstrap TCP links per coord relay (for HOP anchor + prune).
    bootstrap_tcp_conns: RwLock<HashMap<PeerId, HashMap<ConnectionId, Multiaddr>>>,
    /// Identify completed on bootstrap HOP (libp2p relay client expects this before reserve).
    bootstrap_identified: RwLock<HashSet<PeerId>>,
    /// First bootstrap TCP connect time per relay (fallback when Identify was drained at startup).
    bootstrap_tcp_since_ms: RwLock<HashMap<PeerId, i64>>,
    /// Defer circuit reservation until happy-eyeballs links settle (`RELAY_BOOTSTRAP_SETTLE_MS`).
    bootstrap_reserve_after_ms: RwLock<HashMap<PeerId, i64>>,
    /// Remote multiaddr per connected coord relay (relay reservation retries).
    bootstrap_relay_addr: RwLock<HashMap<PeerId, Multiaddr>>,
    /// At least one coord relay peer has a live libp2p connection.
    any_bootstrap_connected: AtomicBool,
    /// Throttle repeated coord lookups per contact public key (UI can spam register/send bursts).
    last_coord_lookup_ms: RwLock<HashMap<String, i64>>,
    /// Backoff coord lookups when peer isn't registered yet (HTTP 404 peer_not_on_server).
    /// Key: recipient public_key_hex.
    coord_lookup_backoff: RwLock<HashMap<String, CoordLookupBackoff>>,
    bootstrap_dial_err_log_ms: RwLock<HashMap<PeerId, i64>>,
    /// Throttle redundant `swarm.dial` to the same coord relay (refetch/redial storms).
    bootstrap_dial_last_ms: RwLock<HashMap<PeerId, i64>>,
    /// IPv6 bootstrap dial failed (unreachable) — prefer IPv4 for HOP/prune until expiry.
    bootstrap_ipv6_unreachable_ms: RwLock<HashMap<PeerId, i64>>,
    /// Peers we rejected on connect (relay/bootstrap noise); suppress disconnect logs.
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
    network_profile: RwLock<super::network_transport::LocalNetworkProfile>,
    /// Fast relay/coord/bootstrap loop after Wi‑Fi ↔ mobile (or OS connectivity callback).
    wan_recovery_active: AtomicBool,
    /// Co-located Ghal Bol relay `(peer_id, base_addrs from GET /v1/relay)` for refresh.
    ghalbol_relay_state: RwLock<Option<(PeerId, Vec<String>)>>,
    relay_cache_path: Option<std::path::PathBuf>,
    ghalbol_relay_last_fetch_ms: RwLock<i64>,
    /// Rate-limit diagnostic logs for dial skips (avoid log storms).
    dial_skip_log_ms: RwLock<HashMap<PeerId, i64>>,
    /// mDNS discovered this DM peer on the local LAN (WAN-first dial otherwise).
    peers_on_local_lan: RwLock<HashMap<PeerId, i64>>,
    /// Count of currently-open **direct** (non-relay-circuit) connections per peer.
    /// Lets a peer freshly seen on the LAN decide whether it still needs a direct
    /// LAN link (it is connected only over a relay circuit) — see `handle_mdns_discovered_list`.
    peers_direct_conns: RwLock<HashMap<PeerId, u32>>,
    /// Throttle for mDNS-driven LAN upgrade dials so a re-announce burst does not
    /// re-dial every second (`LAN_UPGRADE_DIAL_THROTTLE_MS`).
    lan_upgrade_dial_ms: RwLock<HashMap<PeerId, i64>>,
    /// Live mDNS LAN TCP candidates per peer (libp2p emits addrs one-by-one — not a dial cache).
    peer_mdns_lan_candidate_addrs: RwLock<HashMap<PeerId, Vec<Multiaddr>>>,
    /// DM contacts whose connection just dropped — reconnect is urgent until this deadline (ms).
    /// While urgent, coord lookup bypasses the `peer_not_on_server` backoff and we retry every
    /// upkeep tick so a transient drop does not turn into a multi-second message delay.
    dm_reconnect_urgent: RwLock<HashMap<String, i64>>,
    /// Last encrypted DM frame in either direction (detect zombie libp2p links).
    dm_wire_activity_ms: RwLock<HashMap<PeerId, i64>>,
    /// When a DM peer is connected but the chat stream writer is missing.
    dm_no_writer_since_ms: RwLock<HashMap<PeerId, i64>>,
    /// Peers with an open `/ghal-bol/msg/1.0.0` writer (mirrors stream map for upkeep).
    dm_stream_has_writer: RwLock<HashSet<PeerId>>,
    /// `open_stream` failed on a dead mux — disconnect and redial on next upkeep tick.
    dm_connection_reset_peers: RwLock<HashSet<PeerId>>,
    /// Periodic relay reservation refresh even when a circuit is already listening.
    relay_keepalive_last_ms: RwLock<i64>,
    /// Back off relay-circuit dials after `ResourceLimitExceeded` from the coord relay.
    relay_circuit_dial_backoff_until: RwLock<HashMap<PeerId, i64>>,
    /// Outbound relay-circuit dial in flight — do not re-dial until done (libp2p cancels prior hop).
    dm_circuit_dial_in_flight_ms: RwLock<HashMap<PeerId, i64>>,
    /// Direct LAN TCP dial in flight — defer relay circuit dials so libp2p does not cancel oneshot.
    dm_lan_dial_in_flight_ms: RwLock<HashMap<PeerId, i64>>,
    /// When a DM peer was hot-registered — wait for mDNS on Wi‑Fi before coord relay dials.
    dm_peer_registered_ms: RwLock<HashMap<PeerId, i64>>,
    /// Last coord lookup outcome per DM public key (for actionable dial-skip logs).
    coord_lookup_last_category: RwLock<HashMap<String, super::connectivity_diag::CoordLookupCategory>>,
    /// Last WAN coord listen fingerprint — detects relay/public path churn without iface change.
    last_wan_listen_fp: RwLock<Vec<String>>,
    /// Active native voice-call media sessions, keyed by `call_id`. Each entry holds the
    /// per-call controls (mute/stop + stats) and the channel into the engine for inbound
    /// (peer → engine) sealed packets. See `docs/GHAL_BOL_CALL_NATIVE_V2.md`.
    call_media: Mutex<HashMap<String, CallMediaEntry>>,
    /// Active native **video**-call sessions, keyed by `call_id`. Parallel to
    /// `call_media` (voice); a call may have both. See `docs/GHAL_BOL_VIDEO_NATIVE_V1.md`.
    call_video: Mutex<HashMap<String, CallVideoEntry>>,
}

/// One active call's transport bridge state held in [`SessionState::call_media`].
struct CallMediaEntry {
    peer_id: PeerId,
    controls: crate::call_media::MediaControls,
    /// Inbound sealed packets (peer → our engine). Cloned by the RX stream handler.
    wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

/// One active video call's transport bridge state held in [`SessionState::call_video`].
struct CallVideoEntry {
    peer_id: PeerId,
    controls: crate::call_video::VideoControls,
    /// Inbound sealed video chunks (peer → our engine). Cloned by the RX stream handler.
    wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
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
        network_profile: super::network_transport::LocalNetworkProfile,
        relay_cache_path: Option<std::path::PathBuf>,
        ghalbol_relay_state: Option<(PeerId, Vec<String>)>,
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
                        "skip dm peer {} at start: not secp256k1 identity (relay nodes are not contacts)",
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
            routed_dial_attempt_ms: RwLock::new(HashMap::new()),
            circuit_coord_dial_last_ms: RwLock::new(HashMap::new()),
            stream_open_log_emitted: RwLock::new(HashSet::new()),
            stream_open_inflight: RwLock::new(HashSet::new()),
            bootstrap_peer_ids: RwLock::new(bootstrap_peer_ids),
            relay_reserve_requested: RwLock::new(HashSet::new()),
            relay_reserve_last_attempt_ms: RwLock::new(HashMap::new()),
            relay_reserve_in_flight_ms: RwLock::new(HashMap::new()),
            bootstrap_tcp_conns: RwLock::new(HashMap::new()),
            bootstrap_identified: RwLock::new(HashSet::new()),
            bootstrap_tcp_since_ms: RwLock::new(HashMap::new()),
            bootstrap_reserve_after_ms: RwLock::new(HashMap::new()),
            bootstrap_relay_addr: RwLock::new(HashMap::new()),
            any_bootstrap_connected: AtomicBool::new(false),
            last_coord_lookup_ms: RwLock::new(HashMap::new()),
            coord_lookup_backoff: RwLock::new(HashMap::new()),
            bootstrap_dial_err_log_ms: RwLock::new(HashMap::new()),
            bootstrap_dial_last_ms: RwLock::new(HashMap::new()),
            bootstrap_ipv6_unreachable_ms: RwLock::new(HashMap::new()),
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
            ghalbol_relay_state: RwLock::new(ghalbol_relay_state),
            relay_cache_path,
            ghalbol_relay_last_fetch_ms: RwLock::new(0),
            dial_skip_log_ms: RwLock::new(HashMap::new()),
            peers_on_local_lan: RwLock::new(HashMap::new()),
            peers_direct_conns: RwLock::new(HashMap::new()),
            lan_upgrade_dial_ms: RwLock::new(HashMap::new()),
            peer_mdns_lan_candidate_addrs: RwLock::new(HashMap::new()),
            dm_reconnect_urgent: RwLock::new(HashMap::new()),
            dm_wire_activity_ms: RwLock::new(HashMap::new()),
            dm_no_writer_since_ms: RwLock::new(HashMap::new()),
            dm_stream_has_writer: RwLock::new(HashSet::new()),
            dm_connection_reset_peers: RwLock::new(HashSet::new()),
            relay_keepalive_last_ms: RwLock::new(0),
            relay_circuit_dial_backoff_until: RwLock::new(HashMap::new()),
            dm_circuit_dial_in_flight_ms: RwLock::new(HashMap::new()),
            dm_lan_dial_in_flight_ms: RwLock::new(HashMap::new()),
            dm_peer_registered_ms: RwLock::new(HashMap::new()),
            coord_lookup_last_category: RwLock::new(HashMap::new()),
            last_wan_listen_fp: RwLock::new(Vec::new()),
            call_media: Mutex::new(HashMap::new()),
            call_video: Mutex::new(HashMap::new()),
        })
    }

    /// True while a native media session for `call_id` is registered.
    fn call_media_active(&self, call_id: &str) -> bool {
        self.call_media
            .lock()
            .map(|m| m.contains_key(call_id))
            .unwrap_or(false)
    }

    fn call_media_register(
        &self,
        call_id: String,
        peer_id: PeerId,
        controls: crate::call_media::MediaControls,
        wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        if let Ok(mut m) = self.call_media.lock() {
            m.insert(
                call_id,
                CallMediaEntry {
                    peer_id,
                    controls,
                    wire_in_tx,
                },
            );
        }
    }

    /// Channel into the engine for inbound packets of `call_id`, but only when the
    /// stream comes from the libp2p peer this call was started with (RX stream handler).
    fn call_media_wire_in_for_peer(
        &self,
        call_id: &str,
        peer: PeerId,
    ) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.call_media.lock().ok().and_then(|m| {
            m.get(call_id).and_then(|e| {
                if e.peer_id == peer {
                    Some(e.wire_in_tx.clone())
                } else {
                    None
                }
            })
        })
    }

    /// Stop and remove one media session; returns whether it existed.
    fn call_media_stop(&self, call_id: &str) -> bool {
        let entry = self.call_media.lock().ok().and_then(|mut m| m.remove(call_id));
        if let Some(e) = entry {
            e.controls.request_stop();
            true
        } else {
            false
        }
    }

    fn call_media_stop_all(&self) {
        if let Ok(mut m) = self.call_media.lock() {
            for (_, e) in m.drain() {
                e.controls.request_stop();
            }
        }
    }

    fn call_media_set_mic_muted(&self, call_id: &str, muted: bool) -> bool {
        self.call_media
            .lock()
            .ok()
            .and_then(|m| m.get(call_id).map(|e| e.controls.set_mic_muted(muted)))
            .is_some()
    }

    fn call_video_active(&self, call_id: &str) -> bool {
        self.call_video
            .lock()
            .map(|m| m.contains_key(call_id))
            .unwrap_or(false)
    }

    fn call_video_register(
        &self,
        call_id: String,
        peer_id: PeerId,
        controls: crate::call_video::VideoControls,
        wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        if let Ok(mut m) = self.call_video.lock() {
            m.insert(call_id, CallVideoEntry { peer_id, controls, wire_in_tx });
        }
    }

    /// Channel into the video engine for inbound chunks of `call_id`, but only when
    /// the stream comes from the libp2p peer this call was started with.
    fn call_video_wire_in_for_peer(
        &self,
        call_id: &str,
        peer: PeerId,
    ) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.call_video.lock().ok().and_then(|m| {
            m.get(call_id).and_then(|e| {
                if e.peer_id == peer {
                    Some(e.wire_in_tx.clone())
                } else {
                    None
                }
            })
        })
    }

    fn call_video_stop(&self, call_id: &str) -> bool {
        let entry = self.call_video.lock().ok().and_then(|mut m| m.remove(call_id));
        crate::call_video::clear_decoded_frames(call_id);
        if let Some(e) = entry {
            e.controls.request_stop();
            true
        } else {
            false
        }
    }

    fn call_video_stop_all(&self) {
        if let Ok(mut m) = self.call_video.lock() {
            for (call_id, e) in m.drain() {
                e.controls.request_stop();
                crate::call_video::clear_decoded_frames(&call_id);
            }
        }
    }

    fn call_video_set_camera_off(&self, call_id: &str, off: bool) -> bool {
        self.call_video
            .lock()
            .ok()
            .and_then(|m| m.get(call_id).map(|e| e.controls.set_camera_off(off)))
            .is_some()
    }

    fn note_peer_on_local_lan(&self, peer: PeerId) {
        let now = chrono_now_ms();
        let Ok(mut m) = self.peers_on_local_lan.write() else {
            return;
        };
        m.insert(peer, now);
        m.retain(|_, t| now.saturating_sub(*t) < PEER_LAN_SEEN_TTL_MS);
    }

    fn peer_on_local_lan(&self, peer: PeerId) -> bool {
        let now = chrono_now_ms();
        self.peers_on_local_lan
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|t| now.saturating_sub(t) < PEER_LAN_SEEN_TTL_MS)
    }

    /// A peer left the LAN (mDNS `Expired`): drop its LAN preference so dial ranking
    /// returns to WAN-first immediately instead of waiting out `PEER_LAN_SEEN_TTL_MS`.
    /// Returns `true` if the peer was actually marked on-LAN.
    fn forget_peer_on_local_lan(&self, peer: PeerId) -> bool {
        let Ok(mut m) = self.peers_on_local_lan.write() else {
            return false;
        };
        let removed = m.remove(&peer).is_some();
        if removed {
            if let Ok(mut lan) = self.peer_mdns_lan_candidate_addrs.write() {
                lan.remove(&peer);
            }
        }
        removed
    }

    fn merge_mdns_lan_candidate(&self, peer: PeerId, addr: &Multiaddr) {
        if !is_direct_lan_tcp_mdns_candidate(addr) {
            return;
        }
        if let Ok(mut m) = self.peer_mdns_lan_candidate_addrs.write() {
            let v = m.entry(peer).or_default();
            if !v.iter().any(|a| a == addr) {
                v.push(addr.clone());
            }
        }
    }

    fn remove_mdns_lan_candidate(&self, peer: PeerId, failed: Option<&Multiaddr>) {
        let Some(failed_ma) = failed else {
            return;
        };
        if let Ok(mut m) = self.peer_mdns_lan_candidate_addrs.write() {
            if let Some(v) = m.get_mut(&peer) {
                v.retain(|a| a != failed_ma);
                if v.is_empty() {
                    m.remove(&peer);
                }
            }
        }
    }

    /// Drop a failed mDNS LAN candidate; pick the next live addr on the following dial.
    fn invalidate_stale_lan_dial(
        &self,
        peer: PeerId,
        failed: Option<&Multiaddr>,
        _now_ms: i64,
    ) {
        self.remove_mdns_lan_candidate(peer, failed);
    }

    /// On Wi‑Fi/LAN, skip coord relay dials while a direct LAN path is viable or in flight.
    /// Applies before the first `ConnectionEstablished` — relay + mDNS LAN dials racing
    /// cancel each other and stall chat for ~15s (see TRANSPORT.md).
    fn should_defer_coord_relay_for_lan(
        &self,
        peer: PeerId,
        now_ms: i64,
        _connected: bool,
    ) -> bool {
        if !self.network_profile_snapshot().has_active_lan() {
            return false;
        }
        if self.peer_has_direct_connection(peer) {
            return true;
        }
        if self.peer_mdns_lan_addr(peer).is_some() {
            return true;
        }
        if self.lan_dial_in_flight_blocks(peer, now_ms) {
            return true;
        }
        const LAN_GRACE_MS: i64 = 20_000;
        self.peers_on_local_lan
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|t| now_ms.saturating_sub(t) < LAN_GRACE_MS)
    }

    /// mDNS `Expired` for one multiaddr — only drop LAN state when the expired addr is still
    /// our cached direct TCP target (libp2p often expires an old port after advertising a new one).
    fn note_peer_mdns_lan_addr_expired(&self, peer: PeerId, expired: &Multiaddr) -> bool {
        if super::network_transport::is_relay_circuit_multiaddr(expired) {
            return false;
        }
        let had = self
            .peer_mdns_lan_candidate_addrs
            .read()
            .ok()
            .and_then(|m| m.get(&peer).cloned())
            .is_some_and(|v| v.iter().any(|a| a == expired));
        if !had {
            return false;
        }
        self.remove_mdns_lan_candidate(peer, Some(expired));
        if self.peer_mdns_lan_addr(peer).is_some() {
            return false;
        }
        self.forget_peer_on_local_lan(peer)
    }

    fn peer_mdns_lan_addr(&self, peer: PeerId) -> Option<Multiaddr> {
        let candidates = self
            .peer_mdns_lan_candidate_addrs
            .read()
            .ok()
            .and_then(|m| m.get(&peer).cloned())?;
        pick_best_mdns_lan_tcp_addr(&candidates)
    }

    fn clear_routed_dial_throttle(&self, peer: PeerId) {
        if let Ok(mut g) = self.routed_dial_attempt_ms.write() {
            g.remove(&peer);
        }
    }

    /// Track a newly-established connection's path so we know whether a peer has a
    /// **direct** (non-relay) link. `is_relay` is derived from the connection's remote
    /// multiaddr (`/p2p-circuit`).
    fn note_connection_path(&self, peer: PeerId, is_relay: bool) {
        if is_relay {
            return;
        }
        if let Ok(mut m) = self.peers_direct_conns.write() {
            *m.entry(peer).or_insert(0) += 1;
        }
    }

    /// A connection closed; if it was a direct (non-relay) link, decrement the count.
    fn drop_connection_path(&self, peer: PeerId, is_relay: bool) {
        if is_relay {
            return;
        }
        if let Ok(mut m) = self.peers_direct_conns.write() {
            if let Some(n) = m.get_mut(&peer) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    m.remove(&peer);
                }
            }
        }
    }

    /// True when at least one direct (non-relay) connection to `peer` is open.
    fn peer_has_direct_connection(&self, peer: PeerId) -> bool {
        self.peers_direct_conns
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|n| n > 0)
    }

    /// Throttle gate for mDNS-driven LAN upgrade dials.
    fn should_lan_upgrade_dial(&self, peer: PeerId, now_ms: i64) -> bool {
        let Ok(mut m) = self.lan_upgrade_dial_ms.write() else {
            return false;
        };
        let last = m.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < LAN_UPGRADE_DIAL_THROTTLE_MS {
            return false;
        }
        m.insert(peer, now_ms);
        true
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
            .any(|ma| super::network_transport::is_relay_circuit_multiaddr(ma));
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

    fn request_dm_connection_reset(&self, peer: PeerId) {
        if let Ok(mut g) = self.dm_connection_reset_peers.write() {
            g.insert(peer);
        }
    }

    fn drain_dm_connection_resets(&self) -> Vec<PeerId> {
        let Ok(mut g) = self.dm_connection_reset_peers.write() else {
            return Vec::new();
        };
        g.drain().collect()
    }

    fn begin_wan_recovery(&self) {
        self.wan_recovery_active.store(true, Ordering::Relaxed);
    }

    fn refresh_bootstrap_connected_flag(&self, swarm: &Swarm<ChatBehaviour>) {
        let any = self
            .bootstrap_peer_ids
            .read()
            .ok()
            .is_some_and(|g| g.iter().any(|p| swarm.is_connected(p)));
        self.any_bootstrap_connected
            .store(any, Ordering::Relaxed);
    }

    fn network_profile_snapshot(&self) -> super::network_transport::LocalNetworkProfile {
        self.network_profile
            .read()
            .ok()
            .map(|p| *p)
            .unwrap_or_default()
    }

    /// Re-detect interfaces; returns `(old_mode, new_mode)` when dial/coord strategy should change.
    fn refresh_network_path_if_changed(&self) -> Option<(String, String)> {
        let new = super::network_transport::detect_local_network_profile();
        let Ok(mut cur) = self.network_profile.write() else {
            return None;
        };
        let old_key = super::network_transport::network_handover_key(&*cur);
        let new_key = super::network_transport::network_handover_key(&new);
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
        if self.is_pk_reconnect_urgent(pk, now_ms) {
            let Ok(mut m) = self.last_coord_lookup_ms.write() else {
                return true;
            };
            let last = m.get(pk).copied().unwrap_or(0);
            if now_ms.saturating_sub(last) < 800 {
                return false;
            }
            m.insert(pk.to_string(), now_ms);
            return true;
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
        let mut next_step = match prev {
            None => 1_000,
            Some(p) => (p.step_ms.saturating_mul(2)).clamp(1_000, 30_000),
        };
        // Peer may still be reserving relay + registering on coord — do not back off to 30s.
        if self.dm_peer(pk).is_some() {
            next_step = next_step.min(3_000);
        }
        m.insert(
            pk.to_string(),
            CoordLookupBackoff {
                next_allowed_ms: now_ms.saturating_add(next_step),
                step_ms: next_step,
            },
        );
        crate::coord_runtime::sync_coord_lookup_peer_not_found(pk, next_step as u64, now_ms);
    }

    /// A DM connection just closed — mark its key urgent so reconnect is attempted immediately
    /// (bypassing the coord 404 backoff) for a bounded window. See AGENTS.md override rules.
    /// Does not extend an active window (avoids perpetual urgent from repeated upkeep ticks).
    fn mark_dm_reconnect_urgent(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if pk.len() != 66 {
            return;
        }
        // A fresh drop invalidates any prior "peer_not_on_server" backoff: the peer was just
        // here, so try coord again right away instead of waiting out the exponential gap.
        self.clear_coord_lookup_backoff(pk);
        let now = chrono_now_ms();
        let deadline = now.saturating_add(DM_RECONNECT_URGENT_WINDOW_MS);
        if let Ok(mut m) = self.dm_reconnect_urgent.write() {
            if m.get(pk).is_some_and(|d| now < *d) {
                return;
            }
            m.insert(pk.to_string(), deadline);
        }
    }

    /// Force-refresh urgent window (disconnect, outbox restore at node_ready) even if still active.
    fn refresh_dm_reconnect_urgent(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if pk.len() != 66 {
            return;
        }
        self.clear_coord_lookup_backoff(pk);
        if let Ok(mut m) = self.dm_reconnect_urgent.write() {
            m.insert(
                pk.to_string(),
                chrono_now_ms().saturating_add(DM_RECONNECT_URGENT_WINDOW_MS),
            );
        }
    }

    fn is_pk_reconnect_urgent(&self, pk_hex: &str, now_ms: i64) -> bool {
        let pk = pk_hex.trim();
        self.dm_reconnect_urgent
            .read()
            .ok()
            .and_then(|m| m.get(pk).copied())
            .is_some_and(|deadline| now_ms < deadline)
    }

    fn is_peer_reconnect_urgent(&self, peer: PeerId, now_ms: i64) -> bool {
        self.dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex)
            .is_some_and(|pk| self.is_pk_reconnect_urgent(&pk, now_ms))
    }

    /// DM keys still inside their urgent-reconnect window (expired entries are dropped).
    fn urgent_reconnect_pks(&self, now_ms: i64) -> Vec<String> {
        let Ok(mut m) = self.dm_reconnect_urgent.write() else {
            return Vec::new();
        };
        m.retain(|_, deadline| now_ms < *deadline);
        m.keys().cloned().collect()
    }

    fn clear_dm_reconnect_urgent(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if pk.is_empty() {
            return;
        }
        if let Ok(mut m) = self.dm_reconnect_urgent.write() {
            m.remove(pk);
        }
    }

    fn clear_coord_lookup_backoff(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if pk.len() != 66 {
            return;
        }
        if let Ok(mut m) = self.coord_lookup_backoff.write() {
            m.remove(pk);
        }
        crate::coord_runtime::clear_coord_lookup_backoff_for_pk(pk);
    }

    fn note_dm_wire_activity(&self, peer: PeerId) {
        if let Ok(mut m) = self.dm_wire_activity_ms.write() {
            m.insert(peer, chrono_now_ms());
        }
    }

    /// Last confirmed **inbound** DM frame (acks, text, call signals from peer).
    fn note_dm_inbound_activity(&self, peer: PeerId) {
        self.note_dm_wire_activity(peer);
    }

    fn set_dm_stream_writer(&self, peer: PeerId, open: bool) {
        if let Ok(mut s) = self.dm_stream_has_writer.write() {
            if open {
                s.insert(peer);
                if let Ok(mut m) = self.dm_no_writer_since_ms.write() {
                    m.remove(&peer);
                }
            } else {
                s.remove(&peer);
            }
        }
    }

    fn dm_has_stream_writer(&self, peer: PeerId) -> bool {
        self.dm_stream_has_writer
            .read()
            .ok()
            .is_some_and(|s| s.contains(&peer))
    }

    /// Connected libp2p link looks healthy on paper but DM cannot send/receive.
    fn dm_link_needs_recovery(&self, peer: PeerId, now_ms: i64) -> bool {
        const STALE_INBOUND_MS: i64 = 45_000;
        const NO_WRITER_MS: i64 = 6_000;
        let has_writer = self.dm_has_stream_writer(peer);
        if !has_writer {
            let Ok(mut m) = self.dm_no_writer_since_ms.write() else {
                return true;
            };
            let first = m.entry(peer).or_insert(now_ms);
            if now_ms.saturating_sub(*first) >= NO_WRITER_MS {
                return true;
            }
        }
        if let Ok(m) = self.dm_wire_activity_ms.read() {
            if let Some(last) = m.get(&peer) {
                return now_ms.saturating_sub(*last) >= STALE_INBOUND_MS;
            }
        }
        false
    }

    fn should_relay_keepalive(&self, now_ms: i64) -> bool {
        const INTERVAL_MS: i64 = 45_000;
        let Ok(mut last) = self.relay_keepalive_last_ms.write() else {
            return false;
        };
        if now_ms.saturating_sub(*last) < INTERVAL_MS {
            return false;
        }
        *last = now_ms;
        true
    }

    fn note_relay_circuit_dial_rate_limited(&self, peer: PeerId, now_ms: i64) {
        const BACKOFF_MS: i64 = 90_000;
        if let Ok(mut m) = self.relay_circuit_dial_backoff_until.write() {
            m.insert(peer, now_ms.saturating_add(BACKOFF_MS));
        }
    }

    fn relay_circuit_dial_backoff_active(&self, peer: PeerId, now_ms: i64) -> bool {
        let Ok(mut m) = self.relay_circuit_dial_backoff_until.write() else {
            return false;
        };
        if let Some(until) = m.get(&peer).copied() {
            if now_ms < until {
                return true;
            }
            m.remove(&peer);
        }
        false
    }

    fn clear_relay_circuit_dial_backoff(&self, peer: PeerId) {
        if let Ok(mut m) = self.relay_circuit_dial_backoff_until.write() {
            m.remove(&peer);
        }
    }

    fn mark_circuit_dial_in_flight(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut m) = self.dm_circuit_dial_in_flight_ms.write() {
            m.insert(peer, now_ms);
        }
    }

    fn clear_circuit_dial_in_flight(&self, peer: PeerId) {
        if let Ok(mut m) = self.dm_circuit_dial_in_flight_ms.write() {
            m.remove(&peer);
        }
    }

    fn mark_lan_dial_in_flight(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut m) = self.dm_lan_dial_in_flight_ms.write() {
            m.insert(peer, now_ms);
        }
    }

    fn clear_lan_dial_in_flight(&self, peer: PeerId) {
        if let Ok(mut m) = self.dm_lan_dial_in_flight_ms.write() {
            m.remove(&peer);
        }
    }

    /// True while a direct LAN TCP dial must finish before opening a relay circuit to the same peer.
    fn lan_dial_in_flight_blocks(&self, peer: PeerId, now_ms: i64) -> bool {
        const IN_FLIGHT_MS: i64 = 15_000;
        let Ok(m) = self.dm_lan_dial_in_flight_ms.read() else {
            return false;
        };
        m.get(&peer)
            .is_some_and(|start| now_ms.saturating_sub(*start) < IN_FLIGHT_MS)
    }

    /// True while a relay-circuit dial to this peer must not be replaced (avoids oneshot cancel).
    fn circuit_dial_in_flight_blocks(&self, peer: PeerId, now_ms: i64) -> bool {
        const IN_FLIGHT_MS: i64 = 45_000;
        let Ok(m) = self.dm_circuit_dial_in_flight_ms.read() else {
            return false;
        };
        m.get(&peer)
            .is_some_and(|start| now_ms.saturating_sub(*start) < IN_FLIGHT_MS)
    }

    /// Drop stale in-flight circuit dials so a hung hop does not block retries forever.
    fn expire_stale_circuit_dials(&self, now_ms: i64) {
        const EXPIRE_MS: i64 = 45_000;
        let Ok(mut m) = self.dm_circuit_dial_in_flight_ms.write() else {
            return;
        };
        m.retain(|peer, start| {
            let keep = now_ms.saturating_sub(*start) < EXPIRE_MS;
            if !keep {
                native_log::warn(
                    "dial",
                    format!("relay-circuit dial to {peer} timed out ({EXPIRE_MS}ms) — retry allowed"),
                );
            }
            keep
        });
    }

    fn expire_stale_lan_dials(&self, now_ms: i64) {
        const EXPIRE_MS: i64 = 20_000;
        let Ok(mut m) = self.dm_lan_dial_in_flight_ms.write() else {
            return;
        };
        m.retain(|_peer, start| now_ms.saturating_sub(*start) < EXPIRE_MS);
    }

    fn set_coord_lookup_category(
        &self,
        pk_hex: &str,
        cat: super::connectivity_diag::CoordLookupCategory,
    ) {
        let pk = pk_hex.trim();
        if pk.len() != 66 {
            return;
        }
        if let Ok(mut m) = self.coord_lookup_last_category.write() {
            m.insert(pk.to_string(), cat);
        }
    }

    fn coord_lookup_category_for_peer(
        &self,
        peer: PeerId,
    ) -> Option<super::connectivity_diag::CoordLookupCategory> {
        let pk = self
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())?;
        self.coord_lookup_last_category
            .read()
            .ok()
            .and_then(|m| m.get(pk.trim()).copied())
    }

    fn pending_outbox_count(&self) -> usize {
        self.outbox.read().ok().map(|g| g.len()).unwrap_or(0)
    }

    /// True when relay/public listen addrs changed since the last non-empty snapshot.
    fn wan_listen_fp_changed(&self, fp: &[String]) -> bool {
        let Ok(mut last) = self.last_wan_listen_fp.write() else {
            return false;
        };
        let changed = !last.is_empty() && *last != fp;
        *last = fp.to_vec();
        changed
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

    /// Returns false when the same bootstrap relay was dialed too recently (pending TCP).
    fn should_issue_bootstrap_dial(&self, peer: PeerId, now_ms: i64, force: bool) -> bool {
        const THROTTLE_MS: i64 = 10_000;
        const FORCE_MIN_MS: i64 = 3_000;
        let gap = if force { FORCE_MIN_MS } else { THROTTLE_MS };
        let Ok(mut m) = self.bootstrap_dial_last_ms.write() else {
            return true;
        };
        let last = m.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < gap {
            return false;
        }
        m.insert(peer, now_ms);
        true
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

    fn bootstrap_ipv6_degraded(&self, peer: PeerId, now_ms: i64) -> bool {
        const DEGRADED_MS: i64 = 600_000;
        self.bootstrap_ipv6_unreachable_ms
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|t| now_ms.saturating_sub(t) < DEGRADED_MS)
    }

    fn note_bootstrap_ipv6_unreachable(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut m) = self.bootstrap_ipv6_unreachable_ms.write() {
            m.insert(peer, now_ms);
        }
    }

    fn bootstrap_family_rank(&self, ma: &Multiaddr, peer: PeerId, now_ms: i64) -> u8 {
        let profile = self.network_profile_snapshot();
        super::network_transport::relay_bootstrap_family_rank(
            ma,
            profile.on_mobile_data_path(),
            self.bootstrap_ipv6_degraded(peer, now_ms),
        )
    }

    fn note_bootstrap_connected(&self) {
        self.any_bootstrap_connected.store(true, Ordering::Relaxed);
    }

    fn is_bootstrap_peer(&self, peer: PeerId) -> bool {
        self.bootstrap_peer_ids
            .read()
            .ok()
            .is_some_and(|g| g.contains(&peer))
    }

    fn note_bootstrap_identified(&self, relay: PeerId) {
        if let Ok(mut g) = self.bootstrap_identified.write() {
            g.insert(relay);
        }
    }

    fn is_bootstrap_identified(&self, relay: PeerId) -> bool {
        self.bootstrap_identified
            .read()
            .ok()
            .is_some_and(|g| g.contains(&relay))
    }

    /// Reset per-relay bootstrap/reserve state when the last HOP TCP link drops.
    fn clear_bootstrap_relay_session(&self, relay: PeerId) {
        if let Ok(mut g) = self.bootstrap_identified.write() {
            g.remove(&relay);
        }
        if let Ok(mut m) = self.bootstrap_tcp_since_ms.write() {
            m.remove(&relay);
        }
        clear_relay_reserve_in_flight(self, relay);
        if let Ok(mut m) = self.bootstrap_reserve_after_ms.write() {
            m.remove(&relay);
        }
        if let Ok(mut g) = self.relay_reserve_requested.write() {
            g.remove(&relay);
        }
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

    /// Registered DM contact (invite or inbound dial) — not an incidental relay peer.
    fn is_dm_contact(&self, peer: PeerId) -> bool {
        self.should_dial_libp2p_peer(peer)
    }

    fn should_routed_dial(&self, peer: PeerId, now_ms: i64, min_interval_ms: i64) -> bool {
        let Ok(g) = self.routed_dial_attempt_ms.read() else {
            return true;
        };
        let last = g.get(&peer).copied().unwrap_or(0);
        now_ms.saturating_sub(last) >= min_interval_ms
    }

    fn note_routed_dial_attempt(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut g) = self.routed_dial_attempt_ms.write() {
            g.insert(peer, now_ms);
        }
    }

    fn try_mdns_lan_failover_dial(
        &self,
        swarm: &mut Swarm<ChatBehaviour>,
        peer: PeerId,
        failed: Option<&Multiaddr>,
    ) {
        self.remove_mdns_lan_candidate(peer, failed);
        self.clear_lan_dial_in_flight(peer);
        self.clear_routed_dial_throttle(peer);
        let Some(next) = self.peer_mdns_lan_addr(peer) else {
            return;
        };
        if failed.is_some_and(|f| f == &next) {
            return;
        }
        dial_mdns_lan_addr(swarm, self, peer, next, true);
    }

    fn should_circuit_coord_dial(&self, peer: PeerId, now_ms: i64, min_interval_ms: i64) -> bool {
        let Ok(g) = self.circuit_coord_dial_last_ms.read() else {
            return true;
        };
        let last = g.get(&peer).copied().unwrap_or(0);
        now_ms.saturating_sub(last) >= min_interval_ms
    }

    fn note_circuit_coord_dial_attempt(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut g) = self.circuit_coord_dial_last_ms.write() {
            g.insert(peer, now_ms);
        }
    }

    fn log_stream_open_once(&self, peer: PeerId) -> bool {
        let Ok(mut g) = self.stream_open_log_emitted.write() else {
            return true;
        };
        g.insert(peer)
    }

    /// Returns true when new dialable addresses were added.
    fn merge_published_listen(&self, addrs: Vec<Multiaddr>) -> bool {
        let Ok(mut v) = self.published_listen.write() else {
            return false;
        };
        v.retain(|ma| super::network_transport::is_dm_listen_tcp_multiaddr(ma));
        let before = v.len();
        for ma in addrs {
            if !super::network_transport::is_dm_listen_tcp_multiaddr(&ma) {
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

    fn has_pending_outbox_for_pk(&self, pk_hex: &str) -> bool {
        let pk = pk_hex.trim().to_lowercase();
        if pk.len() != 66 {
            return false;
        }
        self.outbox.read().ok().is_some_and(|g| {
            g.values()
                .any(|p| p.recipient_public_key_hex.trim().eq_ignore_ascii_case(&pk))
        })
    }

    fn peer_has_pending_outbox(&self, peer: PeerId) -> bool {
        self.dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
            .is_some_and(|pk| self.has_pending_outbox_for_pk(&pk))
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
        self.note_dm_wire_activity(peer);
    }

    fn note_disconnected(&self, peer: &PeerId) {
        if let Ok(mut g) = self.connected.write() {
            g.remove(peer);
        }
        self.clear_chat_ready_emitted(*peer);
        if let Ok(mut g) = self.history_replay_done.write() {
            g.remove(peer);
        }
        self.set_dm_stream_writer(*peer, false);
        if let Ok(mut m) = self.dm_no_writer_since_ms.write() {
            m.remove(peer);
        }
        if let Ok(mut m) = self.dm_wire_activity_ms.write() {
            m.remove(peer);
        }
    }

    fn clear_chat_ready_emitted(&self, peer: PeerId) {
        if let Ok(mut g) = self.chat_ready_emitted.write() {
            g.remove(&peer);
        }
    }

    /// After network handover, drop 404 backoff so the next tick does a live coord lookup.
    fn clear_coord_lookup_backoff_all(&self) {
        if let Ok(mut m) = self.coord_lookup_backoff.write() {
            m.clear();
        }
    }

    fn connected_peers(&self) -> Vec<PeerId> {
        self.connected
            .read()
            .ok()
            .map(|g| g.iter().copied().collect())
            .unwrap_or_default()
    }

    /// libp2p PeerIds for configured DM contacts (for DM dial/upkeep).
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

    /// Only dial mDNS/coord peers we already know from the invite (never random LAN nodes).
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
            // Ed25519 relay peers must never become DM rows (no `/ghal-bol/msg/1.0.0`).
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
        refresh_outbox_peer_ids(self);
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
        let is_new = !tables.by_peer_id.contains_key(&libp2p_peer);
        let entry = tables
            .by_peer_id
            .entry(libp2p_peer)
            .or_insert_with(|| DmPeer::peer_id_only(libp2p_peer));
        entry.public_key_hex = Some(pk.to_string());
        if is_new {
            if let Ok(mut m) = self.dm_peer_registered_ms.write() {
                m.insert(libp2p_peer, chrono_now_ms());
            }
        }
    }

    fn dm_peer_registered_at(&self, peer: PeerId) -> Option<i64> {
        self.dm_peer_registered_ms
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
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
            last_send_ms: 0,
        });
    }

    fn mark_read_ack_wire_sent(&self, inbound_id: &str) {
        let id = inbound_id.trim();
        if id.is_empty() {
            return;
        }
        let now = chrono_now_ms();
        if let Ok(mut q) = self.pending_read_acks.write() {
            for item in q.iter_mut() {
                if item.inbound_id == id {
                    item.last_send_ms = now;
                    break;
                }
            }
        }
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
                let _ = crate::dm_transcript_store::patch_inbound_read_ack_sent_at_path(
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
        let now = chrono_now_ms();
        let mut due: Vec<PendingReadAck> = q
            .iter()
            .filter(|item| {
                if confirmed
                    .as_ref()
                    .is_some_and(|s| s.contains(&item.inbound_id))
                {
                    return false;
                }
                item.last_send_ms == 0
                    || now.saturating_sub(item.last_send_ms) >= OUTBOX_RESEND_INTERVAL_MS
            })
            .cloned()
            .collect();
        due.sort_by_key(|p| p.last_send_ms);
        due.truncate(limit);
        due
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

    /// Drop queued signals for a ended call (prevents ghost rings from stale invites).
    fn purge_pending_call_signals(&self, call_id: &str) {
        let cid = call_id.trim();
        if cid.is_empty() {
            return;
        }
        let Ok(mut q) = self.pending_call_signals.write() else {
            return;
        };
        q.retain(|c| c.call_id != cid);
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
    let has_outbox = peer_has_pending_outbox(session.as_ref(), peer);
    if !first && !has_outbox {
        return;
    }
    if first {
        if let Some(tx) = events_tx.clone() {
            let _ = tx.send(GossipChatEvent::ChatReady { peer_id: peer });
        }
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
        resync_outbox_burst_for_peer(
            session2.clone(),
            writers2.clone(),
            peer,
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
        if may_send_in_room_read_ack(session2.as_ref(), peer) {
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
        if send_frame_to_peer(peer, frame, Arc::clone(&writers), Some(session.as_ref()))
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
    /// Keepalive: periodic pings keep otherwise-idle DM/relay connections active so libp2p's
    /// `idle_connection_timeout` does not silently drop a live chat link (and the next message
    /// pay a full reconnect). Ping failure also detects a dead route faster.
    pub ping: libp2p::ping::Behaviour,
    pub stream: stream::Behaviour,
}

/// TCP-only transport when `GHAL_BOL_MINIMAL_SWARM` is set (local integration runs).
#[cfg(not(feature = "test-minimal-swarm"))]
fn minimal_swarm_mode() -> bool {
    std::env::var_os("GHAL_BOL_MINIMAL_SWARM")
        .is_some_and(|v| !matches!(v.to_str(), Some("0" | "false" | "no")))
}

#[inline(never)]
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
    #[cfg(feature = "test-minimal-swarm")]
    let upnp = Toggle::from(None);
    #[cfg(not(feature = "test-minimal-swarm"))]
    let upnp = Toggle::from(Some(libp2p::upnp::tokio::Behaviour::default()));
    // Keepalive ping: interval must be shorter than `SWARM_IDLE_CONNECTION_TIMEOUT_SECS`
    // (45s on Android) so a healthy-but-idle chat connection is never dropped between messages.
    let ping = libp2p::ping::Behaviour::new(
        libp2p::ping::Config::new()
            .with_interval(Duration::from_secs(PING_INTERVAL_SECS))
            .with_timeout(Duration::from_secs(PING_TIMEOUT_SECS)),
    );
    native_log::info(
        "p2p",
        "behaviours: relay+dcutr+identify+autonat+upnp+mdns+ping",
    );
    ChatBehaviour {
        relay,
        dcutr: libp2p::dcutr::Behaviour::new(local_peer_id),
        identify: libp2p::identify::Behaviour::new(identify_cfg),
        autonat: libp2p::autonat::Behaviour::new(local_peer_id, libp2p::autonat::Config::default()),
        upnp,
        mdns,
        ping,
        stream: stream::Behaviour::new(),
    }
}

/// Keepalive ping cadence. Interval is well under the idle-connection timeout so a live but
/// quiet chat link stays up; timeout bounds detection of a dead route.
const PING_INTERVAL_SECS: u64 = 8;
const PING_TIMEOUT_SECS: u64 = 15;

/// Shorter on Android so dead Wi‑Fi TCP does not block bootstrap redial for minutes.
#[cfg(target_os = "android")]
const SWARM_IDLE_CONNECTION_TIMEOUT_SECS: u64 = 45;

#[cfg(not(target_os = "android"))]
const SWARM_IDLE_CONNECTION_TIMEOUT_SECS: u64 = 300;

/// Phones: TCP+noise only (no QUIC/TLS stack) — avoids common Android libp2p build failures.
#[cfg(target_os = "android")]
#[inline(never)]
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

/// TCP-only swarm for CI integration tests (`test-minimal-swarm` feature).
#[cfg(all(not(target_os = "android"), feature = "test-minimal-swarm"))]
#[inline(never)]
fn build_swarm(config: &GossipChatConfig) -> Result<Swarm<ChatBehaviour>, ChatServerError> {
    native_log::info("p2p", "swarm transport: minimal tcp+noise (integration)");
    let swarm = SwarmBuilder::with_existing_identity(config.keypair.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| ChatServerError::Transport(e.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
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

#[cfg(all(not(target_os = "android"), not(feature = "test-minimal-swarm")))]
#[inline(never)]
fn build_swarm(config: &GossipChatConfig) -> Result<Swarm<ChatBehaviour>, ChatServerError> {
    let keypair = config.keypair.clone();
    let swarm = if minimal_swarm_mode() {
        native_log::info("p2p", "swarm transport: minimal tcp+noise (env)");
        SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|e| ChatServerError::Transport(e.to_string()))?
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .map_err(|e| ChatServerError::Transport(format!("relay client: {e}")))?
            .with_behaviour(|key, relay| Ok(chat_behaviour(key, relay)))
            .map_err(|e| ChatServerError::Transport(e.to_string()))?
            .with_swarm_config(|c| {
                c.with_idle_connection_timeout(Duration::from_secs(
                    SWARM_IDLE_CONNECTION_TIMEOUT_SECS,
                ))
            })
            .build()
    } else {
        // TCP uses noise only (same as Android) so phones and desktop can DM on LAN/coord TCP.
        native_log::info("p2p", "swarm transport: tcp+noise+quic");
        SwarmBuilder::with_existing_identity(keypair)
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
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .map_err(|e| ChatServerError::Transport(format!("relay client: {e}")))?
            .with_behaviour(|key, relay| Ok(chat_behaviour(key, relay)))
            .map_err(|e| ChatServerError::Transport(e.to_string()))?
            .with_swarm_config(|c| {
                c.with_idle_connection_timeout(Duration::from_secs(
                    SWARM_IDLE_CONNECTION_TIMEOUT_SECS,
                ))
            })
            .build()
    };
    Ok(swarm)
}

fn listen_swarm_transports(swarm: &mut Swarm<ChatBehaviour>) -> Result<(), ChatServerError> {
    listen_ephemeral(swarm, "/ip4/0.0.0.0/tcp/0")?;
    #[cfg(all(not(target_os = "android"), not(feature = "test-minimal-swarm")))]
    if !minimal_swarm_mode() {
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

fn issue_bootstrap_dials(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    nodes: &[(PeerId, Multiaddr)],
    log_label: &str,
    force: bool,
) {
    use std::collections::HashMap;
    let now_ms = chrono_now_ms();
    let mut by_peer: HashMap<PeerId, Vec<Multiaddr>> = HashMap::new();
    for (peer, ma) in nodes {
        if !super::network_transport::is_trusted_bootstrap_dial_addr(ma) {
            continue;
        }
        by_peer.entry(*peer).or_default().push(ma.clone());
    }
    for (peer, mut addrs) in by_peer {
        if swarm.is_connected(&peer) {
            continue;
        }
        if !session.should_issue_bootstrap_dial(peer, now_ms, force) {
            continue;
        }
        addrs.sort_by_key(|ma| session.bootstrap_family_rank(ma, peer, now_ms));
        addrs.dedup_by(|a, b| a.to_string() == b.to_string());
        if session.bootstrap_ipv6_degraded(peer, now_ms) {
            addrs.retain(|ma| !ma.to_string().contains("/ip6/"));
        }
        // Wi‑Fi/LAN with a reachable IPv4 relay base: skip IPv6 bootstrap dials that often
        // log `Network is unreachable` and duplicate TCP to the same relay.
        if session.network_profile_snapshot().has_active_lan() {
            let has_v4 = addrs.iter().any(|ma| ma.to_string().contains("/ip4/"));
            if has_v4 {
                addrs.retain(|ma| !ma.to_string().contains("/ip6/"));
            }
        }
        if addrs.is_empty() {
            continue;
        }
        for ma in addrs {
            if let Ok(mut m) = session.bootstrap_relay_addr.write() {
                m.entry(peer).or_insert_with(|| ma.clone());
            }
            native_log::info("dial", format!("{log_label} {peer} via {ma}"));
            if let Err(e) = swarm.dial(ma.clone()) {
                native_log::warn("dial", format!("{log_label} {peer} {ma}: {e}"));
            }
        }
    }
}

/// Dial coord relay bootstrap nodes (base TCP multiaddrs from GET /v1/relay).
fn dial_coord_relays(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    nodes: &[(PeerId, Multiaddr)],
) {
    issue_bootstrap_dials(swarm, session, nodes, "coord relay", false);
}

/// After a network handover: dial coord relay(s) only when not already connected — never tear down an active relay link
/// (STORY.md / TRANSPORT.md: handover must not drop in-flight DM over relay circuits).
fn ensure_coord_relays_connected(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    nodes: &[(PeerId, Multiaddr)],
) {
    session.refresh_bootstrap_connected_flag(swarm);
    issue_bootstrap_dials(swarm, session, nodes, "coord relay dial", true);
}

/// Per-relay throttle for `listen_on(/p2p-circuit)`.
const RELAY_RESERVE_THROTTLE_MS: i64 = 10_000;
/// Faster probe retries on CGNAT while bootstrap TCP is still pending (cellular can be slow).
const CGNAT_PROBE_THROTTLE_MS: i64 = 2_500;
/// Do not re-issue `listen_on` while a reservation is in flight (libp2p cancels the prior attempt).
const RELAY_RESERVE_IN_FLIGHT_TIMEOUT_MS: i64 = 30_000;
/// Wait for happy-eyeballs dual-stack bootstrap links before picking the HOP anchor.
const RELAY_BOOTSTRAP_SETTLE_MS: i64 = 450;
/// After bootstrap TCP is up, issue `listen_on` even if Identify was not observed yet
/// (startup `bootstrap_publishable_listen` may drain Identify before the main loop).
const RELAY_TCP_HOP_FALLBACK_MS: i64 = 800;
/// Max wait for relay circuit before `node_ready` when WAN/coord mode is on.
const BOOTSTRAP_LISTEN_MAX_SECS: u64 = 12;

fn note_bootstrap_tcp_since(session: &SessionState, relay: PeerId, now_ms: i64) {
    if let Ok(mut m) = session.bootstrap_tcp_since_ms.write() {
        m.entry(relay).or_insert(now_ms);
    }
}

/// Bootstrap HOP is ready for a single `listen_on(/p2p-circuit)`.
fn bootstrap_hop_ready_for_listen(session: &SessionState, relay: PeerId, now_ms: i64) -> bool {
    if session.is_bootstrap_identified(relay) {
        return true;
    }
    session
        .bootstrap_tcp_since_ms
        .read()
        .ok()
        .and_then(|m| m.get(&relay).copied())
        .is_some_and(|since| now_ms.saturating_sub(since) >= RELAY_TCP_HOP_FALLBACK_MS)
}

/// Live bootstrap TCP address(es) for a relay — the HOP libp2p pins for `listen_on(/p2p-circuit)`.
fn bootstrap_hop_anchor(session: &SessionState, relay: PeerId) -> Option<Multiaddr> {
    let conns = session
        .bootstrap_tcp_conns
        .read()
        .ok()
        .and_then(|m| m.get(&relay).cloned())?;
    if conns.is_empty() {
        return None;
    }
    if conns.len() == 1 {
        return conns.values().next().cloned();
    }
    let now_ms = chrono_now_ms();
    conns
        .values()
        .min_by_key(|ma| session.bootstrap_family_rank(ma, relay, now_ms))
        .cloned()
}

/// Prune dual-stack HOP links if needed, then return the anchor for one `listen_on`.
fn bootstrap_hop_anchor_for_reserve(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
) -> Option<Multiaddr> {
    if bootstrap_relay_conn_count(session, relay) > 1 {
        return prune_duplicate_relay_bootstrap_connections(swarm, session, relay);
    }
    bootstrap_hop_anchor(session, relay)
}

fn attempt_wan_relay_reserve(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, force: bool) {
    run_bootstrap_relay_reserve_pass(swarm, session);
    if relay_circuit_listening(swarm) {
        return;
    }
    let now_ms = chrono_now_ms();
    if !force && any_relay_reserve_in_flight(session, now_ms) {
        return;
    }
    let _ = try_relay_reservations(swarm, session, force);
}

fn relay_reserve_in_flight_blocks(session: &SessionState, relay: PeerId, now_ms: i64) -> bool {
    let Ok(m) = session.relay_reserve_in_flight_ms.read() else {
        return false;
    };
    let Some(start) = m.get(&relay).copied() else {
        return false;
    };
    now_ms.saturating_sub(start) < RELAY_RESERVE_IN_FLIGHT_TIMEOUT_MS
}

fn mark_relay_reserve_in_flight(session: &SessionState, relay: PeerId, now_ms: i64) {
    if let Ok(mut m) = session.relay_reserve_in_flight_ms.write() {
        m.insert(relay, now_ms);
    }
}

fn clear_relay_reserve_in_flight(session: &SessionState, relay: PeerId) {
    if let Ok(mut m) = session.relay_reserve_in_flight_ms.write() {
        m.remove(&relay);
    }
}

fn any_relay_reserve_in_flight(session: &SessionState, now_ms: i64) -> bool {
    session
        .relay_reserve_in_flight_ms
        .read()
        .ok()
        .map(|m| {
            m.iter().any(|(_, start)| {
                now_ms.saturating_sub(*start) < RELAY_RESERVE_IN_FLIGHT_TIMEOUT_MS
            })
        })
        .unwrap_or(false)
}

fn schedule_bootstrap_relay_reserve(session: &SessionState, relay: PeerId, now_ms: i64) {
    if let Ok(mut m) = session.bootstrap_reserve_after_ms.write() {
        m.insert(relay, now_ms.saturating_add(RELAY_BOOTSTRAP_SETTLE_MS));
    }
}

fn note_bootstrap_tcp_conn(
    session: &SessionState,
    relay: PeerId,
    conn_id: ConnectionId,
    remote: &Multiaddr,
) {
    if remote.to_string().contains("/p2p-circuit") {
        return;
    }
    if let Ok(mut m) = session.bootstrap_tcp_conns.write() {
        m.entry(relay).or_default().insert(conn_id, remote.clone());
    }
}

fn drop_bootstrap_tcp_conn(session: &SessionState, relay: PeerId, conn_id: ConnectionId) {
    if let Ok(mut m) = session.bootstrap_tcp_conns.write() {
        if let Some(inner) = m.get_mut(&relay) {
            inner.remove(&conn_id);
            if inner.is_empty() {
                m.remove(&relay);
            }
        }
    }
}

fn bootstrap_relay_conn_count(session: &SessionState, relay: PeerId) -> usize {
    session
        .bootstrap_tcp_conns
        .read()
        .ok()
        .and_then(|m| m.get(&relay).map(|c| c.len()))
        .unwrap_or(0)
}

/// libp2p relay client pins HOP to one bootstrap TCP link — keep the best family only.
fn prune_duplicate_relay_bootstrap_connections(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
) -> Option<Multiaddr> {
    let conns = session
        .bootstrap_tcp_conns
        .read()
        .ok()
        .and_then(|m| m.get(&relay).cloned())?;
    if conns.is_empty() {
        return None;
    }
    if conns.len() == 1 {
        return conns.values().next().cloned();
    }
    let now_ms = chrono_now_ms();
    let mut ranked: Vec<(ConnectionId, Multiaddr, u8)> = conns
        .iter()
        .map(|(id, ma)| {
            (
                *id,
                ma.clone(),
                session.bootstrap_family_rank(ma, relay, now_ms),
            )
        })
        .collect();
    ranked.sort_by_key(|(_, _, rank)| *rank);
    let (_keep_id, keep_ma, _) = ranked[0].clone();
    for (drop_id, drop_ma, _) in ranked.into_iter().skip(1) {
        native_log::info(
            "relay",
            format!("prune duplicate bootstrap TCP {relay}: drop {drop_ma}, keep {keep_ma}"),
        );
        let _ = swarm.close_connection(drop_id);
        drop_bootstrap_tcp_conn(session, relay, drop_id);
    }
    if let Ok(mut m) = session.bootstrap_relay_addr.write() {
        m.insert(relay, keep_ma.clone());
    }
    Some(keep_ma)
}

/// After happy-eyeballs settle: one `listen_on(/p2p-circuit)` on the live HOP anchor only.
fn run_bootstrap_relay_reserve_pass(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
) {
    if !crate::coord_runtime::wan_discovery_via_coord_only() || relay_circuit_listening(swarm) {
        return;
    }
    let now_ms = chrono_now_ms();
    let relays: Vec<PeerId> = session
        .bootstrap_tcp_conns
        .read()
        .ok()
        .map(|m| m.keys().copied().collect())
        .unwrap_or_default();
    let force = session.wan_recovery_active.load(Ordering::Relaxed);
    for relay in relays {
        if !session.is_bootstrap_peer(relay) || !swarm.is_connected(&relay) {
            continue;
        }
        if !bootstrap_hop_ready_for_listen(session, relay, now_ms) {
            continue;
        }
        let _ = try_relay_reservation(swarm, session, relay, force);
    }
}

fn relay_reserve_throttle_ms(session: &SessionState) -> i64 {
    let profile = session.network_profile_snapshot();
    if (profile.on_mobile_data_path() || profile.needs_relay_for_wan())
        && !session.any_bootstrap_connected.load(Ordering::Relaxed)
    {
        CGNAT_PROBE_THROTTLE_MS
    } else {
        RELAY_RESERVE_THROTTLE_MS
    }
}

/// Bootstrap TCP came up — track link, defer reservation until happy-eyeballs settles.
fn on_bootstrap_tcp_connected(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer_id: PeerId,
    connection_id: ConnectionId,
    endpoint: &ConnectedPoint,
) {
    if !session.is_bootstrap_peer(peer_id) {
        return;
    }
    let remote = endpoint.get_remote_address().clone();
    native_log::info(
        "swarm",
        format!("bootstrap connection {peer_id} via {remote}"),
    );
    session.note_bootstrap_connected();
    note_bootstrap_tcp_conn(session, peer_id, connection_id, &remote);
    note_bootstrap_tcp_since(session, peer_id, chrono_now_ms());
    schedule_bootstrap_relay_reserve(session, peer_id, chrono_now_ms());
    if bootstrap_relay_conn_count(session, peer_id) > 1 {
        let _ = prune_duplicate_relay_bootstrap_connections(swarm, session, peer_id);
    }
    // Reservation runs from Identify + `ensure_wan_relay_circuit` after HOP settle — not here
    // (libp2p relay client needs Identify on the bootstrap link; premature `listen_on` races
    // happy-eyeballs and can pin the wrong HOP).
}

/// True once an IPv4/dns4 relay circuit is listening (stable WAN endpoint; ignore `/dns6/…`).
fn relay_circuit_listening(swarm: &Swarm<ChatBehaviour>) -> bool {
    swarm
        .listeners()
        .any(super::network_transport::is_coord_ipv4_relay_listen)
}

/// Relays eligible for circuit reservation (connected bootstrap HOP, not already circuit-listening).
fn eligible_relays_for_reservation(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
) -> Vec<PeerId> {
    let Some(conns) = session.bootstrap_tcp_conns.read().ok() else {
        return Vec::new();
    };
    conns
        .keys()
        .filter(|p| {
            session.is_bootstrap_peer(**p)
                && swarm.is_connected(p)
                && bootstrap_hop_anchor(session, **p).is_some()
                && !swarm
                    .listeners()
                    .any(|l| super::network_transport::is_coord_ipv4_relay_listen(l)
                        && l.to_string().contains(&format!("/p2p/{p}")))
        })
        .copied()
        .collect()
}

fn ghalbol_relay_peer(session: &SessionState) -> Option<PeerId> {
    session
        .ghalbol_relay_state
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|(p, _)| *p))
}

/// libp2p relay-client WAN circuit — **single entry** for reserve / retry / recovery.
///
/// Encodes rust-libp2p relay-client constraints (TRANSPORT.md § Client, § CGNAT):
/// - HOP pins to **one** bootstrap TCP link → prune before `listen_on`.
/// - Each new `listen_on(/p2p-circuit)` **cancels** the prior in-flight reservation.
/// - Bootstrap path: **Identify** on the HOP link before `listen_on`.
/// - CGNAT probe `listen_on` only when bootstrap TCP is **not** up (never parallel).
fn ensure_wan_relay_circuit(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: Option<&[(PeerId, Multiaddr)]>,
    force: bool,
) -> bool {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return false;
    }
    if listen_ready_for_node(session, true, swarm) {
        return true;
    }
    let now_ms = chrono_now_ms();
    // Always advance the deferred reserve pass; only block issuing a *new* listen_on while one
    // is in flight (libp2p cancels the prior reservation if we re-issue too soon).
    run_bootstrap_relay_reserve_pass(swarm, session);
    if relay_circuit_listening(swarm) {
        return true;
    }
    if !force && any_relay_reserve_in_flight(session, now_ms) {
        return false;
    }

    session.refresh_bootstrap_connected_flag(swarm);
    let bootstrap_up = session.any_bootstrap_connected.load(Ordering::Relaxed);

    if let Some(relays) = coord_relays {
        if !bootstrap_up && !relays.is_empty() {
            if force {
                ensure_coord_relays_connected(swarm, session, relays);
            } else {
                dial_coord_relays(swarm, session, relays);
            }
        }
    }

    run_bootstrap_relay_reserve_pass(swarm, session);
    if relay_circuit_listening(swarm) {
        return true;
    }

    if !bootstrap_up {
        let profile = session.network_profile_snapshot();
        if profile.on_mobile_data_path() || profile.needs_relay_for_wan() {
            if try_ghalbol_probe_style_circuit_listen(swarm, session, force) {
                native_log::info(
                    "relay",
                    "CGNAT probe-style relay reservation initiated (bootstrap TCP pending)",
                );
                return true;
            }
        }
        return false;
    }

    let issued = try_relay_reservations(swarm, session, force);
    if issued > 0 {
        native_log::info(
            "relay",
            format!(
                "reservation listen_on issued for {issued} bootstrap(s) — await ReservationReqAccepted"
            ),
        );
    }
    issued > 0
}

/// Relays eligible for circuit reservation. When coord + ghalbol relay are configured, reserve
/// only on our reliable relay — parallel `listen_on` on extra bootstraps usually refuses
/// (`Failed to get Reservation`) and interferes with the ghalbol handshake (TRANSPORT.md).
/// Cached coord addrs carry WAN when relay is down (STORY.md).
fn relays_to_try_for_reservation(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
) -> Vec<PeerId> {
    let eligible = eligible_relays_for_reservation(swarm, session);
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return eligible;
    }
    let Some(ghalbol) = ghalbol_relay_peer(session) else {
        return eligible;
    };
    if eligible.iter().any(|p| *p == ghalbol) {
        return vec![ghalbol];
    }
    // Not connected yet — caller dials base TCP then retries after ConnectionEstablished/identify.
    Vec::new()
}

/// Issue `listen_on(/p2p-circuit)` on the advertised ghalbol relay — same as `relay_probe`:
/// the relay client dials through the circuit multiaddr; no separate bootstrap dial first.
fn try_ghalbol_probe_style_circuit_listen(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    _force: bool,
) -> bool {
    if !crate::coord_runtime::wan_discovery_via_coord_only() || relay_circuit_listening(swarm) {
        return false;
    }
    let Some(ghalbol) = ghalbol_relay_peer(session) else {
        return false;
    };
    let nodes = session
        .ghalbol_relay_state
        .read()
        .ok()
        .and_then(|g| g.clone())
        .map(|(peer, addrs)| {
            super::network_transport::resolve_relay_bootnodes(&peer.to_string(), &addrs)
        })
        .unwrap_or_default();
    let now_ms = chrono_now_ms();
    let mut candidates: Vec<(PeerId, Multiaddr)> = nodes
        .into_iter()
        .filter(|(p, ma)| {
            *p == ghalbol && super::network_transport::is_trusted_bootstrap_dial_addr(ma)
        })
        .collect();
    candidates.sort_by_key(|(p, ma)| session.bootstrap_family_rank(ma, *p, now_ms));
    let Some((_, base_ma)) = candidates.into_iter().next() else {
        return false;
    };

    let already_listening = swarm.listeners().any(|ma| {
        ma.to_string().contains("/p2p-circuit")
            && ma.to_string().contains(&format!("/p2p/{ghalbol}"))
    });
    if already_listening {
        return false;
    }
    if session.any_bootstrap_connected.load(Ordering::Relaxed) {
        return false;
    }

    if relay_reserve_in_flight_blocks(session, ghalbol, now_ms) {
        return false;
    }

    let throttle_ms = relay_reserve_throttle_ms(session);
    if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
        if let Some(last) = m.get(&ghalbol).copied() {
            if now_ms.saturating_sub(last) < throttle_ms {
                return false;
            }
        }
        m.insert(ghalbol, now_ms);
    } else {
        return false;
    }

    let mut base = base_ma;
    if !base.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        base.push(Protocol::P2p(ghalbol));
    }
    let Some(listen_ma) = super::network_transport::relay_circuit_listen_addr(&base) else {
        return false;
    };
    match swarm.listen_on(listen_ma.clone()) {
        Ok(_) => {
            mark_relay_reserve_in_flight(session, ghalbol, now_ms);
            native_log::info(
                "relay",
                format!("ghalbol circuit listen (probe path) via {listen_ma}"),
            );
            true
        }
        Err(e) => {
            native_log::warn("relay", format!("ghalbol circuit listen {listen_ma}: {e}"));
            clear_relay_reserve_in_flight(session, ghalbol);
            if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
                m.insert(ghalbol, now_ms);
            }
            false
        }
    }
}

/// Issue a relay reservation once identify has completed on a bootstrap link (handshake ready).
fn try_relay_reservation_after_identify(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
) {
    if relay_circuit_listening(swarm) {
        return;
    }
    if !session.is_bootstrap_peer(relay) {
        return;
    }
    schedule_bootstrap_relay_reserve(session, relay, chrono_now_ms());
    run_bootstrap_relay_reserve_pass(swarm, session);
}

/// Reserve a relay circuit on EVERY eligible bootstrap, in parallel.
///
/// Serializing onto one relay at a time (the previous "one-at-a-time" scheme) let a single
/// bootstrap whose reservation is *pending but never accepted* block all the others: WAN
/// reachability then took minutes or never came up. The per-relay throttle inside
/// `try_relay_reservation` (`RELAY_RESERVE_THROTTLE_MS`) already prevents 1s `listen_on` storms,
/// so fanning out is both safe and necessary — a granting relay is found in seconds.
/// Returns the number of relays a fresh `listen_on` was issued for this pass.
fn try_relay_reservations(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    force: bool,
) -> usize {
    if relay_circuit_listening(swarm) {
        return 0;
    }
    let now_ms = chrono_now_ms();
    if any_relay_reserve_in_flight(session, now_ms) {
        return 0;
    }
    run_bootstrap_relay_reserve_pass(swarm, session);
    if relay_circuit_listening(swarm) {
        return 0;
    }
    let mut issued = 0usize;
    for peer in relays_to_try_for_reservation(swarm, session) {
        if try_relay_reservation(swarm, session, peer, force) {
            issued += 1;
        }
    }
    issued
}

/// Request a relay reservation on a connected bootstrap (NAT traversal for phones).
/// Returns `true` only when a fresh `listen_on(/p2p-circuit)` was issued this call.
/// Uses the **live HOP TCP multiaddr**, never the dial-cache addr (dual-stack can differ).
fn try_relay_reservation(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
    _force: bool,
) -> bool {
    if !session.is_bootstrap_peer(relay) {
        return false;
    }
    let now_ms = chrono_now_ms();
    if !bootstrap_hop_ready_for_listen(session, relay, now_ms) {
        return false;
    }
    if relay_reserve_in_flight_blocks(session, relay, now_ms) {
        return false;
    }
    if let Ok(mut m) = session.relay_reserve_in_flight_ms.write() {
        if let Some(start) = m.get(&relay).copied() {
            if now_ms.saturating_sub(start) >= RELAY_RESERVE_IN_FLIGHT_TIMEOUT_MS {
                m.remove(&relay);
                if let Ok(mut g) = session.relay_reserve_requested.write() {
                    g.remove(&relay);
                }
                native_log::warn(
                    "relay",
                    format!("reservation in-flight timeout on {relay} — retry"),
                );
            }
        }
    }
    // If we are already listening on this IPv4 circuit, do not re-issue listens.
    let already_listening = swarm.listeners().any(|ma| {
        super::network_transport::is_coord_ipv4_relay_listen(ma)
            && ma.to_string().contains(&format!("/p2p/{relay}"))
    });
    if already_listening {
        return false;
    }
    // Per-relay time throttle — ALWAYS applies (even under `force`) so reserving on all bootstraps
    // in parallel can never become a 1s `listen_on` storm. A handover clears this map, so the
    // first post-handover attempt is still immediate.
    if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
        if let Some(last) = m.get(&relay).copied() {
            if now_ms.saturating_sub(last) < RELAY_RESERVE_THROTTLE_MS {
                return false;
            }
        }
        m.insert(relay, now_ms);
    } else {
        return false;
    }

    let Some(relay_addr) = bootstrap_hop_anchor_for_reserve(swarm, session, relay) else {
        return false;
    };
    if relay_addr.to_string().contains("/p2p-circuit") {
        return false;
    }

    let Some(listen_ma) = super::network_transport::relay_circuit_listen_addr(&relay_addr) else {
        native_log::warn(
            "relay",
            format!("no circuit listen addr for {relay} from HOP {relay_addr}"),
        );
        return false;
    };
    match swarm.listen_on(listen_ma.clone()) {
        Ok(_) => {
            mark_relay_reserve_in_flight(session, relay, now_ms);
            native_log::info("relay", format!("reserving circuit on {relay} via {listen_ma}"));
            true
        }
        Err(e) => {
            native_log::warn("relay", format!("relay reserve listen {listen_ma}: {e}"));
            clear_relay_reserve_in_flight(session, relay);
            if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
                m.insert(relay, now_ms);
            }
            false
        }
    }
}

/// Poll/UI only needs TCP dialable listen addrs (LAN or relay circuit), not every relay transport variant.
fn should_emit_listening_event(addr: &Multiaddr) -> bool {
    super::network_transport::is_dm_listen_tcp_multiaddr(addr)
        && (!super::network_transport::is_relay_circuit_multiaddr(addr)
            || super::network_transport::is_coord_ipv4_relay_listen(addr))
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
        if super::network_transport::is_relay_circuit_multiaddr(&ma)
            && !super::network_transport::is_coord_ipv4_relay_listen(&ma)
        {
            continue;
        }
        if !out.iter().any(|e| e == ma) {
            out.push(ma.clone());
        }
    }
    out
}

/// Drop relay circuits libp2p has closed; merge live swarm listeners into the publish cache.
fn sync_published_listen_from_swarm(
    session: &SessionState,
    swarm: &Swarm<ChatBehaviour>,
) -> bool {
    let live: std::collections::HashSet<Multiaddr> = swarm.listeners().cloned().collect();
    let Ok(mut v) = session.published_listen.write() else {
        return false;
    };
    let before = v.clone();
    v.retain(|ma| {
        if super::network_transport::is_relay_circuit_multiaddr(ma) {
            return live.contains(ma)
                && super::network_transport::is_coord_ipv4_relay_listen(ma);
        }
        true
    });
    for ma in live {
        if super::network_transport::is_dm_listen_tcp_multiaddr(&ma)
            && !v.iter().any(|x| x == &ma)
        {
            v.push(ma);
        }
    }
    *v != before
}

/// Re-register on coord, refresh relay reservations, and reconnect DM peers after any network
/// or public-reachability change (interface handover, OS callback, or new relay circuit).
fn refresh_coord_reachability_after_network_change(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
    log_ctx: &str,
    full_handover: bool,
) {
    native_log::info("net", log_ctx.to_string());
    if !full_handover {
        refresh_coord_presence_soft(swarm, session, coord_relays);
        return;
    }
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        let _ = sync_published_listen_from_swarm(session, swarm);
        crate::coord_runtime::rebuild_coord_endpoints_from_listen(
            &coord_register_listen_snapshot(swarm, session),
        );
        return;
    }
    session.clear_coord_lookup_backoff_all();
    clear_wan_listen_state_for_handover(session);
    let _ = sync_published_listen_from_swarm(session, swarm);
    crate::coord_runtime::coord_invalidate_presence_on_network_change();
    crate::coord_runtime::rebuild_coord_endpoints_from_listen(
        &coord_register_listen_snapshot(swarm, session),
    );
    if !wan_recovery_satisfied(session, swarm) {
        session.begin_wan_recovery();
    }
    notify_relay_refresh();
    for pk in session.dm_public_keys() {
        session.mark_dm_reconnect_urgent(&pk);
    }
    notify_coord_lookup();
    notify_stream_reopen();
    crate::coord_runtime::schedule_register_presence_force();
    ensure_coord_relays_connected(swarm, session, coord_relays);
    retry_stalled_relay_reservations(swarm, session, true);
}

/// Coord presence + DM rediscovery without tearing down bootstrap HOP / reservation state.
/// Used when only the published listen set drifted (e.g. libp2p opened a `/dns6/.../p2p-circuit`).
fn refresh_coord_presence_soft(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
) {
    let _ = sync_published_listen_from_swarm(session, swarm);
    let snap = coord_register_listen_snapshot(swarm, session);
    crate::coord_runtime::rebuild_coord_endpoints_from_listen(&snap);
    if !relay_circuit_listening(swarm) {
        session.begin_wan_recovery();
        ensure_wan_relay_circuit(swarm, session, Some(coord_relays), false);
    }
}

/// Drop stale relay listen addrs after a network handover. Keep LAN TCP when still on LAN.
fn clear_wan_listen_state_for_handover(session: &SessionState) {
    let on_lan = session.network_profile_snapshot().has_active_lan();
    if let Ok(mut v) = session.published_listen.write() {
        v.retain(|ma| !super::network_transport::is_relay_circuit_multiaddr(ma));
        if crate::coord_runtime::wan_discovery_via_coord_only() && !on_lan {
            v.retain(|ma| {
                !super::network_transport::ipv4_from_ma_str(&ma.to_string())
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
    if let Ok(mut m) = session.relay_reserve_in_flight_ms.write() {
        m.clear();
    }
    // Keep live bootstrap HOP tracking — clearing here while TCP is still up breaks
    // `bootstrap_hop_anchor` until a new ConnectionEstablished (which never fires).
    if let Ok(mut m) = session.bootstrap_reserve_after_ms.write() {
        m.clear();
    }
    if let Ok(mut m) = session.bootstrap_dial_last_ms.write() {
        m.clear();
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
            .any(super::network_transport::is_coord_ipv4_relay_listen);
    }
    let snap = session.published_listen_snapshot();
    if snap
        .iter()
        .any(super::network_transport::is_coord_ipv4_relay_listen)
    {
        return true;
    }
    !super::network_transport::tcp_dm_publish_addrs(snap).is_empty()
}

fn try_wan_relay_recovery(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    let _ = ensure_wan_relay_circuit(swarm, session, None, false);
}

fn wan_recovery_satisfied(session: &SessionState, swarm: &Swarm<ChatBehaviour>) -> bool {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return true;
    }
    if !listen_ready_for_node(session, true, swarm) {
        return false;
    }
    // When coord HTTP is unreachable, a relay circuit is enough for WAN;
    // keep retrying coord register in the background without blocking recovery completion.
    if crate::coord_runtime::coord_http_degraded() {
        return true;
    }
    crate::coord_runtime::coord_is_registered()
}

fn finish_wan_recovery_if_ready(session: &SessionState, swarm: &Swarm<ChatBehaviour>) {
    if !session.wan_recovery_active.load(Ordering::Relaxed) {
        return;
    }
    if wan_recovery_satisfied(session, swarm) {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        let msg = if crate::coord_runtime::coord_http_degraded() {
            "WAN recovery complete — relay circuit listening (coord HTTP degraded)"
        } else {
            "WAN recovery complete — relay circuit + coord registered"
        };
        native_log::info("net", msg);
    }
}

fn run_wan_recovery_pass(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
) {
    if !session.wan_recovery_active.load(Ordering::Relaxed) {
        return;
    }
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        return;
    }
    // On an active Wi‑Fi/LAN we keep existing bootstrap links and never force redial churn (that
    // disrupts working Wi‑Fi paths). But we STILL pursue a relay circuit + coord registration:
    // off‑LAN contacts (mobile data) can only reach us over WAN, so LAN must never abort recovery.
    if !listen_ready_for_node(session, true, swarm) {
        if coord_relays.is_empty() {
            notify_relay_refresh();
        } else {
            ensure_wan_relay_circuit(swarm, session, Some(coord_relays), true);
        }
    }
    let listen = coord_register_listen_snapshot(swarm, session);
    crate::coord_runtime::coord_register_tick(&listen);
    finish_wan_recovery_if_ready(session, swarm);
}

fn handle_network_path_change(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
    old_mode: &str,
    new_mode: &str,
) {
    refresh_coord_reachability_after_network_change(
        swarm,
        session,
        coord_relays,
        &format!("network path changed {old_mode} -> {new_mode}"),
        true,
    );
}

/// When coord is set, WAN DM needs a relay circuit. Reservations can stall; retry on connected bootstraps.
/// Minimum interval between `GET /v1/relay` refetches when the relay is already connected.
const GHALBOL_RELAY_REFETCH_MS: i64 = 30_000;
/// Aggressive refetch while no relay dial addr is known (coord may have just enabled relay).
const GHALBOL_RELAY_REFETCH_EMPTY_MS: i64 = 5_000;

fn merge_relay_nodes_into_coord_relays(
    coord_relays: &mut Vec<(PeerId, Multiaddr)>,
    nodes: &[(PeerId, Multiaddr)],
) {
    for (peer, ma) in nodes {
        if !coord_relays.iter().any(|(_, a)| a == ma) {
            native_log::info(
                "relay",
                format!("ghalbol relay {peer}: resolved dial addr {ma} (refresh)"),
            );
            coord_relays.push((*peer, ma.clone()));
        }
    }
}

/// Re-fetch `/v1/relay`, merge dial addrs, and dial + reserve on the co-located relay.
async fn maybe_refresh_ghalbol_relay(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &mut Vec<(PeerId, Multiaddr)>,
    force: bool,
) {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return;
    }
    let now = chrono_now_ms();
    if !force {
        let last = session
            .ghalbol_relay_last_fetch_ms
            .read()
            .ok()
            .map(|g| *g)
            .unwrap_or(0);
        let relay_connected = session
            .ghalbol_relay_state
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|(p, _)| *p))
            .is_some_and(|p| swarm.is_connected(&p));
        if relay_connected && crate::coord_runtime::coord_is_registered() {
            return;
        }
        let need_relay = coord_relays.is_empty()
            || !listen_ready_for_node(session, true, swarm)
            || !crate::coord_runtime::coord_is_registered();
        let min_gap = if need_relay {
            GHALBOL_RELAY_REFETCH_EMPTY_MS
        } else {
            GHALBOL_RELAY_REFETCH_MS
        };
        if now.saturating_sub(last) < min_gap {
            return;
        }
    }
    let cache = session.relay_cache_path.clone();
    let all_relays = tokio::task::spawn_blocking(move || {
        crate::coord_runtime::fetch_all_ghalbol_relays(cache)
    })
    .await
    .ok()
    .unwrap_or_default();
    if let Ok(mut g) = session.ghalbol_relay_last_fetch_ms.write() {
        *g = now;
    }
    if all_relays.is_empty() {
        native_log::warn(
            "relay",
            "GET /v1/relay returned no dialable relay — WAN unreachable until coord advertises \
             a relay circuit (coord server must expose GET /v1/relay with dialable addrs)",
        );
        return;
    }
    let mut merged_nodes: Vec<(PeerId, Multiaddr)> = Vec::new();
    for (peer_str, addrs) in &all_relays {
        let Ok(relay_peer) = peer_str.parse::<PeerId>() else {
            continue;
        };
        if let Ok(mut g) = session.bootstrap_peer_ids.write() {
            g.insert(relay_peer);
        }
        let nodes = super::network_transport::resolve_relay_bootnodes(peer_str, addrs);
        if nodes.is_empty() {
            native_log::warn(
                "relay",
                format!("ghalbol relay {relay_peer} refetch: no dialable public addr yet"),
            );
            continue;
        }
        native_log::info(
            "relay",
            format!(
                "ghalbol relay {relay_peer}: {} dial addr(s) after refetch",
                nodes.len()
            ),
        );
        merge_relay_nodes_into_coord_relays(coord_relays, &nodes);
        merge_relay_nodes_into_coord_relays(&mut merged_nodes, &nodes);
    }
    if let Some((peer_str, addrs)) = all_relays.first() {
        if let Ok(relay_peer) = peer_str.parse::<PeerId>() {
            if let Ok(mut g) = session.ghalbol_relay_state.write() {
                *g = Some((relay_peer, addrs.clone()));
            }
        }
    }
    if merged_nodes.is_empty() {
        return;
    }
    ensure_coord_relays_connected(swarm, session, &merged_nodes);
    let force_res = force || session.wan_recovery_active.load(Ordering::Relaxed);
    ensure_wan_relay_circuit(swarm, session, Some(&merged_nodes), force_res);
}

fn retry_stalled_relay_reservations(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    force: bool,
) {
    let _ = ensure_wan_relay_circuit(swarm, session, None, force);
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
    _session: Option<&SessionState>,
) -> Result<(), String> {
    send_frame_on_open_stream(peer, frame, &writers)?;
    Ok(())
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
    if owns_writer {
        session.set_dm_stream_writer(peer, true);
        session.note_dm_inbound_activity(peer);
    }
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
        let session_w = Arc::clone(&session);
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
            session_w.set_dm_stream_writer(peer, false);
            session_w.clear_chat_ready_emitted(peer);
            notify_stream_reopen();
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
        session.note_dm_inbound_activity(peer);
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
            if matches!(parsed.kind, crate::call_sig_v1::CallSigKind::Invite) {
                let now_ms = chrono_now_ms();
                if !call_invite_is_live(parsed.created_at_ms, now_ms) {
                    native_log::info(
                        "call",
                        format!(
                            "drop stale invite call_id={} from {peer} (age_ms={})",
                            parsed.call_id,
                            now_ms.saturating_sub(parsed.created_at_ms)
                        ),
                    );
                    drop_pending_call_invite(&parsed.call_id);
                    call_state::clear_peer(&parsed.sender_public_key_hex);
                    platform_incoming_call_dismiss();
                    continue;
                }
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
            match parsed.kind {
                crate::call_sig_v1::CallSigKind::Invite => {
                    let media_up = super::call_active::snapshot().is_some();
                    let phase = call_state::peer_call_phase(&parsed.sender_public_key_hex);
                    // Ring only for a fresh inbound invite — never during live media or outbound ring.
                    if !media_up && phase == call_state::CallPhase::IncomingRinging {
                        platform_incoming_call_show(
                            &parsed.sender_public_key_hex,
                            &parsed.call_id,
                        );
                    }
                }
                crate::call_sig_v1::CallSigKind::Accept => {
                    platform_incoming_call_dismiss();
                }
                crate::call_sig_v1::CallSigKind::VideoOn => {
                    platform_incoming_call_dismiss();
                    super::call_active::set_remote_video_on(&parsed.call_id, true);
                    emit_call_media(
                        &events_tx,
                        &parsed.call_id,
                        &parsed.sender_public_key_hex,
                        "remote_video_on",
                        None,
                    );
                }
                crate::call_sig_v1::CallSigKind::VideoOff => {
                    platform_incoming_call_dismiss();
                    super::call_active::set_remote_video_on(&parsed.call_id, false);
                    emit_call_media(
                        &events_tx,
                        &parsed.call_id,
                        &parsed.sender_public_key_hex,
                        "remote_video_off",
                        None,
                    );
                }
                crate::call_sig_v1::CallSigKind::Hangup | crate::call_sig_v1::CallSigKind::Reject => {
                    platform_incoming_call_dismiss();
                    drop_pending_call_invite(&parsed.call_id);
                    let pk = parsed.sender_public_key_hex.clone();
                    let cid = parsed.call_id.clone();
                    if super::call_active::snapshot().is_some_and(|s| s.call_id == cid) {
                        session.call_media_stop(&cid);
                        session.call_video_stop(&cid);
                        super::call_active::clear();
                        emit_call_media(&events_tx, &cid, &pk, "call_ended", Some("remote_hangup"));
                    }
                }
                _ => {}
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
                let in_room = may_send_in_room_read_ack(session.as_ref(), peer)
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
        session.set_dm_stream_writer(peer, false);
        session.clear_chat_ready_emitted(peer);
        notify_stream_reopen();
        if let Some(task) = write_task {
            let _ = task.await;
        }
    }
}

/// Drop a zombie chat mux writer so the next open_stream can replace it.
fn drop_dm_chat_stream_writer(peer: PeerId, writers: &StreamWriters, session: &SessionState) {
    if let Ok(mut g) = writers.lock() {
        g.remove(&peer);
    }
    session.set_dm_stream_writer(peer, false);
    session.clear_chat_ready_emitted(peer);
    notify_stream_reopen();
}

fn stream_open_needs_connection_reset(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("receiver is gone") || e.contains("oneshot canceled")
}

fn is_direct_lan_tcp_ma(ma: &Multiaddr) -> bool {
    if !is_tcp_multiaddr(ma) || super::network_transport::is_relay_circuit_multiaddr(ma) {
        return false;
    }
    super::network_transport::ipv4_from_ma_str(&ma.to_string()).is_some_and(|ip| ip.is_private())
}

fn tcp_port_from_ma(ma: &Multiaddr) -> Option<u16> {
    ma.to_string()
        .split("/tcp/")
        .nth(1)?
        .split('/')
        .next()?
        .parse()
        .ok()
}

fn is_direct_lan_tcp_mdns_candidate(ma: &Multiaddr) -> bool {
    !is_quic_multiaddr(ma)
        && is_tcp_multiaddr(ma)
        && !super::network_transport::is_relay_circuit_multiaddr(ma)
        && super::network_transport::ipv4_from_ma_str(&ma.to_string())
            .is_some_and(|ip| ip.is_private())
        && super::network_transport::is_dm_dial_multiaddr(ma)
}

/// Rank LAN TCP mDNS candidates for dial order (best first).
///
/// libp2p mDNS often advertises an IPv6-only listener port on RFC1918 `/ip4/` records
/// (e.g. `::1/tcp/41283` mirrored as `192.168.x/tcp/41283`) while the live LAN bind uses
/// a different port (`192.168.x/tcp/38147`). Prefer **lowest** publishable port below 40000
/// when a sub-40000 alternative exists on the same host; otherwise prefer **highest** port
/// so a fresh rebind beats a stale lower port.
fn rank_mdns_lan_tcp_candidates(addrs: &[Multiaddr]) -> Vec<Multiaddr> {
    let mut by_host: HashMap<String, Vec<Multiaddr>> = HashMap::new();
    for ma in addrs.iter().filter(|ma| is_direct_lan_tcp_mdns_candidate(ma)) {
        let Some(ip) = super::network_transport::ipv4_from_ma_str(&ma.to_string()) else {
            continue;
        };
        by_host.entry(ip.to_string()).or_default().push(ma.clone());
    }
    let mut ranked = Vec::new();
    for (_host, host_addrs) in by_host {
        let publishable: Vec<Multiaddr> = host_addrs
            .iter()
            .filter(|ma| super::network_transport::is_publishable_listen_addr(ma))
            .cloned()
            .collect();
        let pool = if publishable.len() >= 2 {
            let has_sub_40k = publishable
                .iter()
                .any(|ma| tcp_port_from_ma(ma).unwrap_or(0) < 40_000);
            if has_sub_40k {
                publishable
                    .into_iter()
                    .filter(|ma| tcp_port_from_ma(ma).unwrap_or(0) < 40_000)
                    .collect()
            } else {
                publishable
            }
        } else if !publishable.is_empty() {
            publishable
        } else {
            host_addrs
        };
        let mut sorted = pool;
        sorted.sort_by_key(|ma| std::cmp::Reverse(tcp_port_from_ma(ma).unwrap_or(0)));
        ranked.extend(sorted);
    }
    ranked.sort_by_key(|ma| std::cmp::Reverse(tcp_port_from_ma(ma).unwrap_or(0)));
    ranked
}

fn pick_best_mdns_lan_tcp_addr(addrs: &[Multiaddr]) -> Option<Multiaddr> {
    rank_mdns_lan_tcp_candidates(addrs).into_iter().next()
}

fn failed_dial_multiaddr_from_error(err_s: &str) -> Option<Multiaddr> {
    let needle = err_s.find("/ip4/").or_else(|| err_s.find("/ip6/"))?;
    let rest = &err_s[needle..];
    let end = rest
        .find(": :")
        .or_else(|| rest.find("): "))
        .unwrap_or(rest.len());
    let candidate = rest[..end].trim_end_matches(')').trim();
    candidate.parse().ok()
}

async fn ensure_dm_stream_for_send(
    peer: PeerId,
    session: Arc<SessionState>,
    control: stream::Control,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if writer_open_for_peer(&writers, peer) {
        return;
    }
    if !session.try_begin_stream_open(peer) {
        return;
    }
    let open_err = ensure_chat_stream(
        peer,
        control,
        writers,
        Arc::clone(&session),
        events_tx,
    )
    .await
    .err();
    session.end_stream_open(peer);
    if let Some(e) = open_err {
        if stream_open_needs_connection_reset(&e) {
            session.request_dm_connection_reset(peer);
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
                if stream_open_needs_connection_reset(&last_err) {
                    return Err(last_err);
                }
                let transient = last_err.contains("Connection refused");
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
        if stream_open_needs_connection_reset(&e) {
            session.request_dm_connection_reset(peer);
        }
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

/// Native voice-call media substream. One per direction per call: each side
/// **opens** one (its TX) and **accepts** one (its RX). First frame is a small
/// plaintext header `{"call_id":"…"}`; all later frames are sealed media packets.
/// See `docs/GHAL_BOL_CALL_NATIVE_V2.md`.
pub(crate) const CALL_STREAM_PROTOCOL: &str = "/ghal-bol/call/1.0.0";

/// Native **video**-call media substream — same framing as [`CALL_STREAM_PROTOCOL`]
/// (plaintext `{"call_id":"…"}` header, then sealed video chunks) but a separate
/// protocol so the swarm routes audio and video to their own engines.
/// See `docs/GHAL_BOL_VIDEO_NATIVE_V1.md`.
pub(crate) const CALL_VIDEO_STREAM_PROTOCOL: &str = "/ghal-bol/call-video/1.0.0";

const CALL_MEDIA_MAX_FRAME: usize = 64 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
struct CallStreamHeader {
    call_id: String,
}

/// Length-prefixed (u32 LE) body write for the media substream.
async fn write_media_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    body: &[u8],
) -> Result<(), String> {
    if body.len() > CALL_MEDIA_MAX_FRAME {
        return Err("call media frame too large".to_string());
    }
    writer
        .write_all(&(body.len() as u32).to_le_bytes())
        .await
        .map_err(|e| format!("write len: {e}"))?;
    writer
        .write_all(body)
        .await
        .map_err(|e| format!("write body: {e}"))?;
    writer.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

/// Length-prefixed body read for the media substream.
async fn read_media_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("read len: {e}"))?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > CALL_MEDIA_MAX_FRAME {
        return Err("call media frame too large".to_string());
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("read body: {e}"))?;
    Ok(body)
}

/// Start native voice media for `call_id`: derive identity media keys, open the
/// audio device, spawn the engine session, and open our TX media substream. The
/// peer's audio arrives on a separate inbound substream (see
/// [`handle_inbound_call_stream`]). Idempotent per `call_id`.
async fn start_call_media(
    session: Arc<SessionState>,
    mut control: stream::Control,
    call_id: String,
    peer_public_key_hex: String,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) -> Result<(), String> {
    let pk = peer_public_key_hex.trim().to_string();
    if pk.len() != 66 {
        return Err("call media: peer public key must be 66 hex chars".to_string());
    }
    if session.call_media_active(&call_id) {
        super::call_active::on_voice_start(&call_id, &pk);
        emit_call_media(&events_tx, &call_id, &pk, "voice_started", None);
        return Ok(());
    }
    let peer = session
        .resolve_send_peer(&pk)
        .ok_or_else(|| "call media: unknown contact".to_string())?;
    let keys =
        crate::call_media_key::derive_call_media_keys_from_identity(&session.identity, &pk, &call_id)?;
    let local_is_a = crate::call_media::local_is_a(&session.my_public_key_hex, &pk);
    let engine = crate::call_media::MediaEngine::new_opus(&keys.frame_key, local_is_a)?;

    #[cfg(target_os = "android")]
    if let Err(e) = crate::call_media::ensure_voice_audio_mode() {
        native_log::warn("call_media", format!("android audio mode: {e}"));
    }

    let mut backend = crate::call_media::default_audio_backend();
    let audio = backend
        .start()
        .map_err(|e| format!("call media: audio start: {e}"))?;
    let controls = crate::call_media::MediaControls::new();

    let (wire_out_tx, mut wire_out_rx) = mpsc::channel::<Vec<u8>>(256);
    let (wire_in_tx, wire_in_rx) = mpsc::channel::<Vec<u8>>(256);

    // Register before opening the TX stream so a racing inbound RX stream can attach.
    session.call_media_register(call_id.clone(), peer, controls.clone(), wire_in_tx);

    native_log::info(
        "call_media",
        format!("start call_id={call_id} peer={peer} local_is_a={local_is_a}"),
    );

    // Engine session task (owns engine + audio backend for the call lifetime).
    let session_ctl = controls.clone();
    tokio::spawn(async move {
        crate::call_media::run_media_session(engine, audio, wire_out_tx, wire_in_rx, session_ctl)
            .await;
        backend.stop();
    });

    // TX task: open our outbound media substream, send the header, then pump
    // sealed frames from the engine until the call stops or the stream breaks.
    let tx_ctl = controls.clone();
    let tx_call_id = call_id.clone();
    tokio::spawn(async move {
        let mut stream = match control
            .open_stream(peer, StreamProtocol::new(CALL_STREAM_PROTOCOL))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                native_log::warn(
                    "call_media",
                    format!("open media stream to {peer} failed: {e}"),
                );
                tx_ctl.request_stop();
                return;
            }
        };
        let header = serde_json::to_vec(&CallStreamHeader {
            call_id: tx_call_id.clone(),
        })
        .unwrap_or_default();
        if let Err(e) = write_media_frame(&mut stream, &header).await {
            native_log::warn("call_media", format!("media header write failed: {e}"));
            tx_ctl.request_stop();
            return;
        }
        loop {
            if tx_ctl.is_stopped() {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(250), wire_out_rx.recv()).await {
                Ok(Some(bytes)) => {
                    if let Err(e) = write_media_frame(&mut stream, &bytes).await {
                        native_log::warn("call_media", format!("media write ended: {e}"));
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {} // idle tick — re-check stop flag
            }
        }
        tx_ctl.request_stop();
        native_log::info("call_media", format!("tx stream closed call_id={tx_call_id}"));
    });

    // Lightweight stats logger so device tests show audio flowing without FFI.
    let stats_session = Arc::clone(&session);
    let stats_call_id = call_id.clone();
    let stats_ctl = controls;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop {
            tick.tick().await;
            if stats_ctl.is_stopped() || !stats_session.call_media_active(&stats_call_id) {
                break;
            }
            native_log::info(
                "call_media",
                format!(
                    "call_id={stats_call_id} sent={} recv={}",
                    stats_ctl.sent(),
                    stats_ctl.received()
                ),
            );
        }
    });

    super::call_active::on_voice_start(&call_id, &pk);
    emit_call_media(&events_tx, &call_id, &pk, "voice_started", None);
    Ok(())
}

/// Handle an inbound `/ghal-bol/call/1.0.0` substream: read the header to learn
/// the `call_id`, then forward sealed frames into that call's engine (RX path).
async fn handle_inbound_call_stream(
    peer: PeerId,
    mut stream: libp2p::Stream,
    session: Arc<SessionState>,
) {
    let header = match read_media_frame(&mut stream).await {
        Ok(h) => h,
        Err(e) => {
            native_log::warn("call_media", format!("inbound media header read: {e}"));
            return;
        }
    };
    let call_id = match serde_json::from_slice::<CallStreamHeader>(&header) {
        Ok(h) => h.call_id,
        Err(e) => {
            native_log::warn("call_media", format!("inbound media header parse: {e}"));
            return;
        }
    };

    // The peer's media stream can arrive a touch before our local CallMediaStart;
    // wait briefly for the engine to register.
    let mut wire_in = None;
    for _ in 0..75 {
        if let Some(tx) = session.call_media_wire_in_for_peer(&call_id, peer) {
            wire_in = Some(tx);
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let Some(wire_in) = wire_in else {
        native_log::warn(
            "call_media",
            format!("inbound media for unknown call_id={call_id} from {peer} — dropped"),
        );
        return;
    };
    native_log::info(
        "call_media",
        format!("rx stream attached call_id={call_id} peer={peer}"),
    );
    loop {
        match read_media_frame(&mut stream).await {
            Ok(bytes) => {
                if wire_in.send(bytes).await.is_err() {
                    break; // engine gone
                }
            }
            Err(e) => {
                if !stream_read_is_terminal(&e) {
                    continue;
                }
                break;
            }
        }
    }
    native_log::info("call_media", format!("rx stream closed call_id={call_id}"));
}

/// Start native **video** for `call_id`: derive a distinct video media key, build the
/// H.264 engine, start camera capture (receive-only if no camera), spawn the engine
/// session, and open our TX video substream. Decoded frames land in the per-call
/// frame registry for the FFI/daemon render pull. Idempotent per `call_id`.
async fn start_call_video(
    session: Arc<SessionState>,
    mut control: stream::Control,
    call_id: String,
    peer_public_key_hex: String,
    camera_enabled: bool,
) -> Result<(), String> {
    use crate::call_video::{
        run_video_session, RawVideoFrame, VideoControls, VideoEngine, VideoStreams,
        DEFAULT_REASSEMBLY_PENDING, DEFAULT_VIDEO_JITTER_MAX,
    };

    let pk = peer_public_key_hex.trim().to_string();
    if pk.len() != 66 {
        return Err("call video: peer public key must be 66 hex chars".to_string());
    }
    if session.call_video_active(&call_id) {
        if camera_enabled {
            session.call_video_set_camera_off(&call_id, false);
            super::call_active::set_camera_on(&call_id, true);
        }
        super::call_active::on_video_start(&call_id, &pk, camera_enabled);
        return Ok(());
    }
    crate::call_video::track_call_shm(&call_id);
    let peer = session
        .resolve_send_peer(&pk)
        .ok_or_else(|| "call video: unknown contact".to_string())?;

    // Distinct key from the audio stream (different HKDF salt via a `:video` suffix),
    // so audio and video never share a (key, nonce) space. Both peers derive the same.
    let video_key_id = format!("{call_id}:video");
    let keys = crate::call_media_key::derive_call_media_keys_from_identity(
        &session.identity,
        &pk,
        &video_key_id,
    )?;
    let local_is_a = crate::call_media::local_is_a(&session.my_public_key_hex, &pk);
    // 16 KiB chunks: well under the 64 KiB substream frame cap, fewer writes per frame.
    let engine = VideoEngine::with_params(
        &keys.frame_key,
        local_is_a,
        Box::new(crate::call_video::H264Encoder::new()?),
        Box::new(crate::call_video::H264Decoder::new()?),
        16 * 1024,
        DEFAULT_REASSEMBLY_PENDING,
        DEFAULT_VIDEO_JITTER_MAX,
    );

    let controls = VideoControls::new();
    let (wire_out_tx, mut wire_out_rx) = mpsc::channel::<Vec<u8>>(256);
    let (wire_in_tx, wire_in_rx) = mpsc::channel::<Vec<u8>>(256);

    // Register before opening TX so a racing inbound RX stream can attach.
    session.call_video_register(call_id.clone(), peer, controls.clone(), wire_in_tx);
    if camera_enabled {
        controls.set_camera_off(false);
    }

    native_log::info(
        "call_video",
        format!(
            "start call_id={call_id} peer={peer} local_is_a={local_is_a} camera_enabled={camera_enabled}",
        ),
    );

    // Camera capture (native). Receive-only if no camera is available — the call
    // still shows the peer's video; we just send nothing.
    let capture_rx = match crate::call_video::spawn_camera_capture(controls.clone()) {
        Ok(rx) => rx,
        Err(e) => {
            native_log::warn(
                "call_video",
                format!("camera unavailable ({e}) — receive-only call_id={call_id}"),
            );
            let (keep_tx, rx) = mpsc::channel::<RawVideoFrame>(1);
            let ctl = controls.clone();
            tokio::spawn(async move {
                // Hold the capture sender open so the session stays alive (RX only).
                while !ctl.is_stopped() {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                drop(keep_tx);
            });
            rx
        }
    };

    // Render sink: engine → global frame registry for the FFI render pull.
    let (render_tx, mut render_rx) = mpsc::channel::<RawVideoFrame>(8);
    let render_call_id = call_id.clone();
    tokio::spawn(async move {
        while let Some(frame) = render_rx.recv().await {
            crate::call_video::publish_decoded_frame(&render_call_id, frame);
        }
    });

    let streams = VideoStreams { capture_rx, render_tx };
    let session_ctl = controls.clone();
    let session_call_id = call_id.clone();
    tokio::spawn(async move {
        run_video_session(
            engine,
            streams,
            session_call_id,
            wire_out_tx,
            wire_in_rx,
            session_ctl,
        )
        .await;
    });

    // TX task: open our outbound video substream, send the header, pump sealed chunks.
    let tx_ctl = controls.clone();
    let tx_call_id = call_id.clone();
    tokio::spawn(async move {
        let mut stream = match control
            .open_stream(peer, StreamProtocol::new(CALL_VIDEO_STREAM_PROTOCOL))
            .await
        {
            Ok(s) => s,
            Err(e) => {
                native_log::warn("call_video", format!("open video stream to {peer} failed: {e}"));
                tx_ctl.request_stop();
                return;
            }
        };
        let header = serde_json::to_vec(&CallStreamHeader { call_id: tx_call_id.clone() })
            .unwrap_or_default();
        if let Err(e) = write_media_frame(&mut stream, &header).await {
            native_log::warn("call_video", format!("video header write failed: {e}"));
            tx_ctl.request_stop();
            return;
        }
        loop {
            if tx_ctl.is_stopped() {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(250), wire_out_rx.recv()).await {
                Ok(Some(bytes)) => {
                    if let Err(e) = write_media_frame(&mut stream, &bytes).await {
                        native_log::warn("call_video", format!("video write ended: {e}"));
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {}
            }
        }
        tx_ctl.request_stop();
        native_log::info("call_video", format!("tx stream closed call_id={tx_call_id}"));
    });

    super::call_active::on_video_start(&call_id, &pk, camera_enabled);
    Ok(())
}

/// Handle an inbound `/ghal-bol/call-video/1.0.0` substream: read the header for the
/// `call_id`, then forward sealed chunks into that call's video engine (RX path).
async fn handle_inbound_call_video_stream(
    peer: PeerId,
    mut stream: libp2p::Stream,
    session: Arc<SessionState>,
) {
    let header = match read_media_frame(&mut stream).await {
        Ok(h) => h,
        Err(e) => {
            native_log::warn("call_video", format!("inbound video header read: {e}"));
            return;
        }
    };
    let call_id = match serde_json::from_slice::<CallStreamHeader>(&header) {
        Ok(h) => h.call_id,
        Err(e) => {
            native_log::warn("call_video", format!("inbound video header parse: {e}"));
            return;
        }
    };
    let mut wire_in = None;
    for _ in 0..75 {
        if let Some(tx) = session.call_video_wire_in_for_peer(&call_id, peer) {
            wire_in = Some(tx);
            break;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let Some(wire_in) = wire_in else {
        native_log::warn(
            "call_video",
            format!("inbound video for unknown call_id={call_id} from {peer} — dropped"),
        );
        return;
    };
    native_log::info(
        "call_video",
        format!("rx video stream attached call_id={call_id} peer={peer}"),
    );
    loop {
        match read_media_frame(&mut stream).await {
            Ok(bytes) => {
                if wire_in.send(bytes).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                if !stream_read_is_terminal(&e) {
                    continue;
                }
                break;
            }
        }
    }
    native_log::info("call_video", format!("rx video stream closed call_id={call_id}"));
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
        // Call media control-plane is latency sensitive — never queue behind text.
        OutboundCmd::CallMediaStart { .. }
        | OutboundCmd::CallMediaStop { .. }
        | OutboundCmd::CallMediaSetMicMuted { .. }
        | OutboundCmd::CallMediaSetSpeaker { .. }
        | OutboundCmd::CallVideoStart { .. }
        | OutboundCmd::CallVideoStop { .. }
        | OutboundCmd::CallVideoSetCameraEnabled { .. } => 0,
        OutboundCmd::RegisterDmPeer { .. } => 1,
        OutboundCmd::DialBootstrapPeers { .. } => 2,
        OutboundCmd::SendAck { .. } => 3,
        OutboundCmd::SendCallSignal { .. } => 4,
        OutboundCmd::SendText { .. } => 5,
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
            if let Ok(derived) = peer_id_from_secp256k1_public_key_hex(pk) {
                if let Ok(target) = derived.parse::<PeerId>() {
                    let now = chrono_now_ms();
                    if !dm_peer_chat_link_stable(
                        swarm,
                        session.as_ref(),
                        target,
                        Some(pk),
                        now,
                    ) {
                        kick_dm_peer_discovery(swarm, session.as_ref(), target);
                        if crate::coord_runtime::wan_discovery_via_coord_only()
                            && session.should_coord_lookup_pk(pk, now, 1_000)
                        {
                            native_log::info("dial", "register_dm_peer: coord lookup (additive)");
                            coord_lookup_dm_peer(swarm, session.as_ref(), pk).await;
                        }
                    }
                }
            }
        } else if let Some(pid) = *peer_id {
            let now = chrono_now_ms();
            if !dm_peer_chat_link_stable(swarm, session.as_ref(), pid, None, now) {
                kick_dm_peer_discovery(swarm, session.as_ref(), pid);
            }
        }
        return Ok(());
    }
    if let OutboundCmd::DialBootstrapPeers { addrs } = &cmd {
        for ma in addrs {
            if super::network_transport::is_trusted_bootstrap_dial_addr(ma) {
                let _ = swarm.dial(ma.clone());
            }
        }
        return Ok(());
    }
    if let OutboundCmd::CallMediaStart {
        call_id,
        peer_public_key_hex,
    } = &cmd
    {
        let pk = peer_public_key_hex.trim().to_string();
        if pk.len() == 66 {
            session.register_dm_peer_key(None, &pk);
            // Signaling may have connected already; libp2p/mDNS first, coord additive.
            if let Some(peer) = session.resolve_send_peer(&pk) {
                if !swarm.is_connected(&peer) {
                    kick_dm_peer_discovery(swarm, session.as_ref(), peer);
                    if !swarm.is_connected(&peer)
                        && crate::coord_runtime::wan_discovery_via_coord_only()
                    {
                        let now = chrono_now_ms();
                        if session.should_coord_lookup_pk(&pk, now, 1_000) {
                            native_log::info(
                                "call_media",
                                format!("media start: coord lookup {peer} (additive)"),
                            );
                            coord_lookup_dm_peer(swarm, session.as_ref(), &pk).await;
                        }
                    }
                }
            }
        }
        let session2 = Arc::clone(&session);
        let control2 = control.clone();
        let call_id = call_id.clone();
        let events_tx2 = events_tx.clone();
        tokio::spawn(async move {
            if let Err(e) =
                start_call_media(session2, control2, call_id.clone(), pk, events_tx2).await
            {
                native_log::warn("call_media", format!("start failed call_id={call_id}: {e}"));
            }
        });
        return Ok(());
    }
    if let OutboundCmd::CallMediaStop { call_id } = &cmd {
        let pk = super::call_active::snapshot()
            .filter(|s| s.call_id == *call_id)
            .map(|s| s.peer_public_key_hex.clone());
        let stopped = session.call_media_stop(call_id);
        super::call_active::on_voice_stop(call_id);
        if let Some(pk) = pk {
            emit_call_media(&events_tx, call_id, &pk, "voice_stopped", None);
            if super::call_active::snapshot().is_none() {
                call_state::clear_peer(&pk);
                platform_incoming_call_dismiss();
                emit_call_media(&events_tx, call_id, &pk, "call_ended", Some("media_stopped"));
            }
        }
        #[cfg(target_os = "android")]
        crate::call_media::reset_voice_audio_mode_flag();
        native_log::info(
            "call_media",
            format!("stop call_id={call_id} (was_active={stopped})"),
        );
        return Ok(());
    }
    if let OutboundCmd::CallVideoStart {
        call_id,
        peer_public_key_hex,
        camera_enabled,
    } = &cmd
    {
        let pk = peer_public_key_hex.trim().to_string();
        if pk.len() == 66 {
            session.register_dm_peer_key(None, &pk);
            if let Some(peer) = session.resolve_send_peer(&pk) {
                if !swarm.is_connected(&peer) {
                    kick_dm_peer_discovery(swarm, session.as_ref(), peer);
                }
            }
        }
        // Await setup so follow-up `set_camera_enabled` never races an unregistered session.
        if let Err(e) = start_call_video(
            Arc::clone(&session),
            control.clone(),
            call_id.clone(),
            pk.clone(),
            *camera_enabled,
        )
        .await
        {
            native_log::warn("call_video", format!("start failed call_id={call_id}: {e}"));
            return Err(e);
        }
        emit_call_media(&events_tx, call_id, &pk, "video_started", None);
        return Ok(());
    }
    if let OutboundCmd::CallVideoStop { call_id } = &cmd {
        let pk = super::call_active::snapshot()
            .filter(|s| s.call_id == *call_id)
            .map(|s| s.peer_public_key_hex.clone());
        let stopped = session.call_video_stop(call_id);
        super::call_active::on_video_stop(call_id);
        if let Some(pk) = pk {
            emit_call_media(&events_tx, call_id, &pk, "video_stopped", None);
            if super::call_active::snapshot().is_none() {
                call_state::clear_peer(&pk);
                platform_incoming_call_dismiss();
                emit_call_media(&events_tx, call_id, &pk, "call_ended", Some("video_stopped"));
            }
        }
        native_log::info(
            "call_video",
            format!("stop call_id={call_id} (was_active={stopped})"),
        );
        return Ok(());
    }
    if let OutboundCmd::CallVideoSetCameraEnabled { call_id, enabled } = &cmd {
        let mut ok = session.call_video_set_camera_off(call_id, !*enabled);
        if !ok {
            let session2 = Arc::clone(&session);
            let call_id2 = call_id.clone();
            let enabled2 = *enabled;
            for attempt in 1..=25 {
                tokio::time::sleep(Duration::from_millis(40)).await;
                ok = session2.call_video_set_camera_off(&call_id2, !enabled2);
                if ok {
                    native_log::info(
                        "call_video",
                        format!(
                            "set_camera_enabled call_id={call_id2} enabled={enabled2} applied=true (retry {attempt})",
                        ),
                    );
                    break;
                }
            }
        }
        if ok {
            super::call_active::set_camera_on(call_id, *enabled);
        }
        native_log::info(
            "call_video",
            format!("set_camera_enabled call_id={call_id} enabled={enabled} applied={ok}"),
        );
        return Ok(());
    }
    if let OutboundCmd::CallMediaSetMicMuted { call_id, muted } = &cmd {
        let ok = session.call_media_set_mic_muted(call_id, *muted);
        native_log::debug(
            "call_media",
            format!("set_mic_muted call_id={call_id} muted={muted} applied={ok}"),
        );
        return Ok(());
    }
    if let OutboundCmd::CallMediaSetSpeaker {
        call_id,
        speaker_on,
    } = &cmd
    {
        match crate::call_media::set_speakerphone(*speaker_on) {
            Ok(()) => native_log::info(
                "call_media",
                format!(
                    "set_speaker call_id={call_id} speaker_on={speaker_on} active={}",
                    session.call_media_active(call_id)
                ),
            ),
            Err(e) => native_log::warn(
                "call_media",
                format!("set_speaker call_id={call_id} failed: {e}"),
            ),
        }
        return Ok(());
    }
    if let OutboundCmd::RunReadAckCatchup { peer_id } = &cmd {
        let peer = *peer_id;
        if !may_send_in_room_read_ack(session.as_ref(), peer) {
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
            if !may_send_in_room_read_ack(session2.as_ref(), peer) {
                return;
            }
            read_ack_catchup_for_peer(session2, writers2, peer, true, true).await;
        });
        return Ok(());
    }
    if let OutboundCmd::SetForegroundPeer { peer_id, generation } = &cmd {
        if *generation < foreground_peer_cmd_gen_latest() {
            native_log::debug(
                "read_ack",
                format!(
                    "skip stale SetForegroundPeer gen={generation} latest={}",
                    foreground_peer_cmd_gen_latest()
                ),
            );
            return Ok(());
        }
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
        if !app_ack_read_enabled() || !app_ui_visible() {
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
            format!("chat room enter {peer} — ack_read for in-room backlog only"),
        );
        if let (Some(path), Some(ns)) = (&session.transcript_path, &session.app_namespace) {
            transcript_sync_outbound_tick(session.as_ref(), Path::new(path), ns.trim());
        }
        let session2 = Arc::clone(&session);
        let writers2 = Arc::clone(&writers);
        tokio::spawn(async move {
            if session2.current_foreground_peer() != Some(peer) || !may_send_in_room_read_ack(session2.as_ref(), peer) {
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
        | OutboundCmd::CallMediaStart { .. }
        | OutboundCmd::CallMediaStop { .. }
        | OutboundCmd::CallMediaSetMicMuted { .. }
        | OutboundCmd::CallMediaSetSpeaker { .. }
        | OutboundCmd::CallVideoStart { .. }
        | OutboundCmd::CallVideoStop { .. }
        | OutboundCmd::CallVideoSetCameraEnabled { .. } => unreachable!(),
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
            if matches!(
                signal_kind,
                CallSigKind::Hangup | CallSigKind::Reject
            ) {
                session.purge_pending_call_signals(&call_id);
            }
            on_local_call_signal_sent(&call_id, signal_kind);
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
                call_id: call_id.clone(),
                signal_kind,
                frame,
                peer_id: peer,
                recipient_public_key_hex: pk.to_string(),
                created_at_ms: chrono_now_ms(),
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
            // mDNS/LAN first when live; coord lookup additive (never sole path on Wi‑Fi).
            if !swarm.is_connected(&peer) {
                if pk.len() == 66 {
                    session.mark_dm_reconnect_urgent(pk);
                }
                kick_dm_peer_discovery(swarm, session.as_ref(), peer);
                if !swarm.is_connected(&peer) && pk.len() == 66 {
                    let now = chrono_now_ms();
                    if crate::coord_runtime::wan_discovery_via_coord_only()
                        && session.should_coord_lookup_pk(pk, now, 1_000)
                    {
                        native_log::info("dial", format!("send queued: coord lookup {peer} (additive)"));
                        coord_lookup_dm_peer(swarm, session.as_ref(), pk).await;
                    }
                }
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
            if let Some(ns) = session
                .app_namespace
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let conv_key = recipient_public_key_hex.trim().to_string();
                if conv_key.len() == 66 {
                    let line = crate::dm_transcript_store::StoredChatLine {
                        local_id: message_id.clone(),
                        text: text.clone(),
                        outgoing: true,
                        from: None,
                        message_id: Some(message_id.clone()),
                        delivery: "pending".to_string(),
                        created_at_ms: Some(now),
                        read_ack_sent: false,
                    };
                    match crate::dm_transcript_store::append_if_new(ns, &conv_key, line) {
                        Ok(()) => native_log::info(
                            "outbound",
                            format!(
                                "transcript append outbound msg_id={message_id} conv={conv_key}"
                            ),
                        ),
                        Err(e) => native_log::warn(
                            "outbound",
                            format!(
                                "transcript append outbound failed msg_id={message_id}: {e}"
                            ),
                        ),
                    }
                }
            }
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
                kick_dm_peer_discovery(swarm, session.as_ref(), peer);
                if !swarm.is_connected(&peer)
                    && crate::coord_runtime::wan_discovery_via_coord_only()
                {
                    coord_lookup_dm_peer(swarm, session.as_ref(), &pk).await;
                }
            } else {
                try_routed_dial(swarm, session.as_ref(), peer);
            }
            session.enqueue_pending_call_signal(call);
            return Ok(());
        }
        ensure_dm_stream_for_send(
            peer,
            Arc::clone(&session),
            control.clone(),
            Arc::clone(&writers),
            events_tx.clone(),
        )
        .await;
        if !writer_open_for_peer(&writers, peer) {
            native_log::info("call", format!("call signal queued: stream opening (peer={peer})"));
            session.enqueue_pending_call_signal(call);
            return Ok(());
        }
        let r = send_frame_to_peer(peer, call.frame, writers, Some(session.as_ref())).await;
        if r.is_ok() {
            native_log::info(
                "call",
                format!(
                    "call frame on wire peer={peer} {} call_id={}",
                    call.signal_kind.wire_name(),
                    call.call_id
                ),
            );
            if let Some(tx) = events_tx.as_ref() {
                let _ = tx.send(GossipChatEvent::CallSignalSent {
                    call_id: call.call_id.clone(),
                    signal: call.signal_kind.wire_name().to_string(),
                    recipient_public_key_hex: call.recipient_public_key_hex.clone(),
                });
            }
        }
        return r;
    }
    if !swarm.is_connected(&peer) {
        kick_dm_peer_discovery(swarm, session.as_ref(), peer);
        if !swarm.is_connected(&peer) {
            if let Some(pk) = session
                .dm_peer_for_libp2p(peer)
                .and_then(|p| p.public_key_hex.clone())
                .filter(|pk| pk.len() == 66)
            {
                if crate::coord_runtime::wan_discovery_via_coord_only() {
                    let now = chrono_now_ms();
                    session.mark_dm_reconnect_urgent(&pk);
                    if session.should_coord_lookup_pk(&pk, now, 1_000) {
                        native_log::info(
                            "dial",
                            format!("outbound blocked: coord lookup {peer} (additive)"),
                        );
                        coord_lookup_dm_peer(swarm, session.as_ref(), &pk).await;
                    }
                } else {
                    native_log::info(
                        "dial",
                        format!("outbound blocked: lookup+dial {peer} (not connected yet)"),
                    );
                }
            } else {
                native_log::info(
                    "dial",
                    format!("outbound blocked: lookup+dial {peer} (not connected yet)"),
                );
            }
        }
        let err = "connecting to peer — try send again in a moment".to_string();
        if let Some(done) = done {
            let _ = done.send(Err(err.clone()));
        }
        return Err(err);
    }
    ensure_dm_stream_for_send(
        peer,
        Arc::clone(&session),
        control.clone(),
        Arc::clone(&writers),
        events_tx.clone(),
    )
    .await;
    if !writer_open_for_peer(&writers, peer) {
        let err = "chat stream opening — try send again shortly".to_string();
        native_log::info("stream", format!("{err} (peer={peer})"));
        if let Some(done) = done {
            let _ = done.send(Err(err.clone()));
        }
        return Err(err);
    }
    let r = send_frame_to_peer(peer, frame, Arc::clone(&writers), Some(session.as_ref())).await;
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
            session.request_dm_connection_reset(peer);
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
        // Pending outbox must drive WAN reconnect in `:p2p` — never wait for the user to open a
        // room or send a new message (DESIGN.md: background node owns delivery).
        for pk in session.dm_public_keys() {
            if session.has_pending_outbox_for_pk(&pk) {
                session.refresh_dm_reconnect_urgent(&pk);
            }
        }
        notify_coord_lookup();
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
        session.mark_read_ack_wire_sent(inbound_id);
        native_log::info(
            "read_ack",
            format!("ack_read sent for inbound {inbound_id} to {peer}"),
        );
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
    send_frame_to_peer(peer, frame, Arc::clone(writers), Some(session))
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
        true,
    )
    .await;
}

/// Fast path for delivery acks only (~25 ms poll tick). Read ack retries use ~1 s upkeep.
async fn run_delivery_ack_upkeep(
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
        false,
    )
    .await;
}

async fn run_ack_upkeep_limited(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: &[PeerId],
    read_limit: usize,
    include_read_acks: bool,
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
    if include_read_acks {
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
                session.mark_read_ack_wire_sent(&item.inbound_id);
                done += 1;
            }
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
    if seed_transcript && !may_send_in_room_read_ack(session.as_ref(), peer) {
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
            true,
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
        if call.signal_kind == CallSigKind::Invite {
            if !call_invite_is_live(call.created_at_ms, chrono_now_ms()) {
                native_log::info(
                    "call",
                    format!(
                        "drop stale queued invite call_id={} age_ms={}",
                        call.call_id,
                        chrono_now_ms().saturating_sub(call.created_at_ms)
                    ),
                );
                continue;
            }
            if !call_state::outbound_invite_active(&call.recipient_public_key_hex, &call.call_id) {
                native_log::info(
                    "call",
                    format!("drop queued invite call_id={} — call already ended", call.call_id),
                );
                continue;
            }
        }
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
        let recipient_pk = call.recipient_public_key_hex.clone();
        match send_frame_to_peer(
            peer_id,
            call.frame.clone(),
            Arc::clone(&writers),
            Some(session.as_ref()),
        )
        .await
        {
            Ok(()) => {
                native_log::info(
                    "call",
                    format!(
                        "call frame on wire peer={peer_id} {} call_id={call_id_log}",
                        signal_kind.wire_name(),
                    ),
                );
                if let Some(tx) = events_tx.as_ref() {
                    let _ = tx.send(GossipChatEvent::CallSignalSent {
                        call_id: call_id_log,
                        signal: signal_kind.wire_name().to_string(),
                        recipient_public_key_hex: recipient_pk,
                    });
                }
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

/// Resolve which connected libp2p peer should receive this outbox row (match by secp256k1 pk, not stale peer_id).
fn outbox_target_peer(
    session: &SessionState,
    p: &PendingOutbound,
    connected_peers: &[PeerId],
) -> Option<PeerId> {
    let pk = p.recipient_public_key_hex.trim();
    if pk.len() == 66 {
        for id in connected_peers {
            if session
                .dm_peer_for_libp2p(*id)
                .and_then(|d| d.public_key_hex.clone())
                .is_some_and(|k| k.eq_ignore_ascii_case(pk))
            {
                return Some(*id);
            }
        }
        if let Some(id) = session.resolve_send_peer(pk) {
            if connected_peers.contains(&id) {
                return Some(id);
            }
        }
    }
    if connected_peers.contains(&p.peer_id) {
        Some(p.peer_id)
    } else {
        None
    }
}

fn refresh_outbox_peer_ids(session: &SessionState) {
    let Ok(mut g) = session.outbox.write() else {
        return;
    };
    for p in g.values_mut() {
        let pk = p.recipient_public_key_hex.trim();
        if pk.len() == 66 {
            if let Some(id) = session.resolve_send_peer(pk) {
                p.peer_id = id;
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
    refresh_outbox_peer_ids(session.as_ref());
    let now = chrono_now_ms();
    let due = session.outbox_due_for_resend(now);
    if !due.is_empty() && !connected_peers.is_empty() {
        native_log::debug(
            "outbox",
            format!("resync {} pending message(s)", due.len()),
        );
    }
    for p in due {
        let Some(send_peer) = outbox_target_peer(session.as_ref(), &p, &connected_peers) else {
            continue;
        };
        if !writer_open_for_peer(&writers, send_peer) {
            if let Some(ctrl) = control.as_ref() {
                open_outbound_stream_if_needed(
                    send_peer,
                    ctrl.clone(),
                    Arc::clone(&writers),
                    Arc::clone(&session),
                    events_tx.clone(),
                )
                .await;
            }
            if !writer_open_for_peer(&writers, send_peer) {
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
        match send_frame_to_peer(
            send_peer,
            frame,
            Arc::clone(&writers),
            Some(session.as_ref()),
        )
        .await
        {
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

/// On chat stream open: drain every pending row for this peer immediately (STORY.md — outbox must not wait on timers).
async fn resync_outbox_burst_for_peer(
    session: Arc<SessionState>,
    writers: StreamWriters,
    peer: PeerId,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    control: Option<stream::Control>,
) {
    refresh_outbox_peer_ids(session.as_ref());
    let pk = session
        .dm_peer_for_libp2p(peer)
        .and_then(|d| d.public_key_hex.clone())
        .filter(|pk| pk.len() == 66);
    let Some(pk) = pk else {
        return;
    };
    let rows: Vec<PendingOutbound> = session
        .outbox
        .read()
        .ok()
        .map(|g| {
            g.values()
                .filter(|p| p.recipient_public_key_hex.eq_ignore_ascii_case(&pk))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    native_log::info(
        "outbox",
        format!("burst resync {} pending row(s) to {peer}", rows.len()),
    );
    let now = chrono_now_ms();
    for p in rows {
        if !writer_open_for_peer(&writers, peer) {
            if let Some(ctrl) = control.as_ref() {
                open_outbound_stream_if_needed(
                    peer,
                    ctrl.clone(),
                    Arc::clone(&writers),
                    Arc::clone(&session),
                    events_tx.clone(),
                )
                .await;
            }
            if !writer_open_for_peer(&writers, peer) {
                break;
            }
        }
        let frame = match build_pending_outbound_frame(session.as_ref(), &p) {
            Ok(f) => f,
            Err(e) => {
                native_log::warn(
                    "outbox",
                    format!("burst skip msg_id={}: {e}", p.message_id),
                );
                continue;
            }
        };
        match send_frame_to_peer(
            peer,
            frame,
            Arc::clone(&writers),
            Some(session.as_ref()),
        )
        .await
        {
            Ok(()) => {
                session.mark_outbox_sent(&p.message_id, now);
                notify_outbound_on_wire(session.as_ref(), &p.message_id, now, &events_tx);
            }
            Err(e) => {
                session.mark_outbox_send_failed(&p.message_id, now);
                native_log::warn(
                    "outbox",
                    format!("burst send failed msg_id={}: {e}", p.message_id),
                );
            }
        }
    }
}

/// Connected DM contacts that need a chat stream (acks/outbox cannot send without it).
fn dm_peers_missing_chat_stream(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    writers: &StreamWriters,
) -> Vec<PeerId> {
    session
        .dm_peer_ids()
        .into_iter()
        .filter(|pid| swarm.is_connected(pid) && !writer_open_for_peer(writers, *pid))
        .collect()
}

fn spawn_reopen_dm_chat_streams(
    swarm: &Swarm<ChatBehaviour>,
    session: Arc<SessionState>,
    writers: StreamWriters,
    control: stream::Control,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    for pid in dm_peers_missing_chat_stream(swarm, session.as_ref(), &writers) {
        let session2 = Arc::clone(&session);
        let writers2 = Arc::clone(&writers);
        let events_tx2 = events_tx.clone();
        let control2 = control.clone();
        tokio::spawn(async move {
            open_outbound_stream_if_needed(pid, control2, writers2, session2, events_tx2).await;
        });
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
        kick_dm_peer_discovery(swarm, session.as_ref(), peer);
    }
}

/// Skip coord **relay** dials only when direct LAN + DM stream are already healthy (STORY.md:
/// WAN stays as backup while on LAN — never block WAN/LAN while disconnected or outbox pending).
fn skip_redundant_coord_relay_dial(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    now_ms: i64,
) -> bool {
    if !session.peer_on_local_lan(peer) || !session.peer_has_direct_connection(peer) {
        return false;
    }
    if !swarm.is_connected(&peer) {
        return false;
    }
    if session.dm_link_needs_recovery(peer, now_ms) {
        return false;
    }
    let pk = session
        .dm_peer_for_libp2p(peer)
        .and_then(|d| d.public_key_hex.clone());
    dm_peer_chat_link_stable(swarm, session, peer, pk.as_deref(), now_ms)
}

fn peer_has_pending_outbox(session: &SessionState, peer: PeerId) -> bool {
    session
        .dm_peer_for_libp2p(peer)
        .and_then(|d| d.public_key_hex.clone())
        .is_some_and(|pk| session.has_pending_outbox_for_pk(&pk))
}

/// Immediate mDNS/routed dial. Coord HTTP must never be the only path to a LAN peer.
fn kick_dm_peer_discovery(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    target: PeerId,
) {
    if swarm.is_connected(&target) {
        return;
    }
    if session.peer_mdns_lan_addr(target).is_some() || session.peer_on_local_lan(target) {
        try_routed_dial_lan_mdns(swarm, session, target);
    } else if !crate::coord_runtime::coord_is_configured() {
        try_routed_dial(swarm, session, target);
    }
}

/// Dial live mDNS LAN TCP when present; never reuse coord/stored RFC1918 for LAN.
fn try_routed_dial_lan_mdns(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
) {
    if let Some(addr) = session.peer_mdns_lan_addr(peer) {
        dial_mdns_lan_addr(swarm, session, peer, addr, false);
        return;
    }
    try_routed_dial_impl(swarm, session, peer);
}

/// Routed dial: mDNS/identify supply addresses via peerstore.
fn try_routed_dial(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, peer: PeerId) {
    // On mobile-data/CGNAT with coord configured, avoid blind peer-id dials from stale
    // peerstore entries; explicit coord/mDNS address dials are safer.
    if crate::coord_runtime::coord_is_configured()
        && session.prefers_mobile_coord_strategy()
        && !crate::coord_runtime::coord_http_degraded()
    {
        return;
    }
    try_routed_dial_impl(swarm, session, peer);
}

fn sort_dm_dial_addrs_for_profile(
    session: &SessionState,
    peer: PeerId,
    addrs: Vec<Multiaddr>,
    for_coord_path: bool,
) -> Vec<Multiaddr> {
    // Coord lookup addrs are relay circuits — always IPv4/dns4-first on every platform.
    // LAN desktop must not use LAN-first sort or undifferentiated WAN-first (IPv6 relay
    // often unroutable even when Wi‑Fi is up).
    if for_coord_path && crate::coord_runtime::coord_is_configured() {
        let has_wifi_lan_tcp = session.network_profile_snapshot().has_active_lan()
            && addrs.iter().any(|ma| {
                is_tcp_multiaddr(ma)
                    && !super::network_transport::is_relay_circuit_multiaddr(ma)
                    && super::network_transport::dm_dial_addr_rank(ma) == 0
            });
        if has_wifi_lan_tcp {
            return super::network_transport::sort_dm_dial_addrs(addrs);
        }
        return super::network_transport::wan_coord_dial_addrs(addrs);
    }
    let on_lan = session.peer_on_local_lan(peer);
    if on_lan {
        return super::network_transport::sort_dm_dial_addrs(addrs);
    }
    super::network_transport::rank_dm_dial_addrs_for_peer(addrs, false)
}

/// Same as [try_routed_dial] but allowed after coord lookup miss (LAN/mDNS when coord has no record).
fn try_routed_dial_impl(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, peer: PeerId) {
    // Defensive: never attempt peer-id dials on mobile-data/CGNAT when coord is configured.
    // Those peerstore address sets often include stale CGNAT ports; WAN peers are reached via
    // live coord lookup + relay circuits (STORY.md).
    if crate::coord_runtime::coord_is_configured()
        && session.prefers_mobile_coord_strategy()
        && !crate::coord_runtime::coord_http_degraded()
    {
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
        Ok(()) => {
            session.note_routed_dial_attempt(peer, now);
            native_log::debug("dial", format!("routed dial {peer}"));
        }
        Err(DialError::NoAddresses) => {
            let last = session.coord_lookup_category_for_peer(peer);
            let (reason, action) = super::connectivity_diag::explain_no_dial_addrs(
                last,
                session.peer_on_local_lan(peer),
                crate::coord_runtime::coord_is_registered(),
            );
            if session.should_log_dial_skip(peer, now, 8_000) {
                native_log::warn(
                    "dial",
                    format!(
                        "issue=no_dial_addrs | peer={peer} | reason={reason} | next={action}"
                    ),
                );
            }
        }
        Err(DialError::DialPeerConditionFalse(_)) => {}
        Err(e) => native_log::debug("dial", format!("routed dial {peer}: {e}")),
    }
}

const MAX_IDENTIFY_DM_ADDRS_PER_PEER: usize = 4;

/// Merge dialable TCP listen addresses from identify into peerstore and dial.
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
    // Do not pollute peerstore with them; rely on coord relay-circuit dials instead.
    if crate::coord_runtime::coord_is_configured()
        && session.prefers_mobile_coord_strategy()
        && !crate::coord_runtime::coord_http_degraded()
    {
        return;
    }
    let ranked = sort_dm_dial_addrs_for_profile(
        session,
        peer,
        addrs
            .iter()
            .filter(|a| super::network_transport::is_dm_dial_multiaddr(a))
            .cloned()
            .collect(),
        true,
    );
    if ranked.is_empty() {
        return;
    }
    if session.should_dial_libp2p_peer(peer) && !swarm.is_connected(&peer) {
        native_log::info(
            tag,
            format!(
                "identify {peer}: {} tcp listen addr(s) ingested",
                ranked.len().min(MAX_IDENTIFY_DM_ADDRS_PER_PEER)
            ),
        );
        if let Some(addr) = ranked.into_iter().next() {
            dial_dm_peer_addr(swarm, session, peer, addr, tag);
        }
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
    // - upkeep_dm_peers (coord/mDNS discovery)
    // - coord_lookup_dm_peer (explicit relay dial)
    // - SendText fast-path ("send queued: coord lookup")
    // - UI bursts of RegisterDmPeer / foreground changes
    //
    // Without a shared throttle, libp2p cancels earlier dials (seen as "oneshot canceled"),
    // and relays can rate-limit / reject ("resource limit exceeded"), causing multi-minute stalls.
    let now = chrono_now_ms();
    let is_relay = super::network_transport::is_relay_circuit_multiaddr(&addr);
    let urgent = session.is_peer_reconnect_urgent(peer, now);
    let min_interval_ms = if tag.starts_with("coord") && is_relay {
        if urgent { 8_000 } else { 45_000 }
    } else if tag.starts_with("coord") {
        2_000
    } else {
        2_000
    };
    // On Wi‑Fi/LAN, never race a relay circuit against an in-flight mDNS TCP dial.
    // Broader defer (mDNS candidate, recent LAN sighting, before first connect) is in
    // should_defer_coord_relay_for_lan for coord lookup paths — TRANSPORT.md § “LAN relay vs mDNS race”.
    if is_relay
        && session.network_profile_snapshot().has_active_lan()
        && session.lan_dial_in_flight_blocks(peer, now)
    {
        return;
    }
    if !is_relay {
        if session.lan_dial_in_flight_blocks(peer, now) {
            return;
        }
    }
    if is_relay && session.relay_circuit_dial_backoff_active(peer, now) {
        return;
    }
    if is_relay && !urgent && skip_redundant_coord_relay_dial(swarm, session, peer, now) {
        return;
    }
    if is_relay && session.circuit_dial_in_flight_blocks(peer, now) {
        return;
    }
    if is_relay && tag.starts_with("coord") {
        if !session.should_circuit_coord_dial(peer, now, min_interval_ms) {
            return;
        }
    } else if !session.should_routed_dial(peer, now, min_interval_ms) {
        return;
    }
    // On mobile-data with coord configured, only relay-circuit dials are reliable — except on
    // LAN (mDNS) or RFC1918 while Wi‑Fi is active.
    if crate::coord_runtime::coord_is_configured() && session.prefers_mobile_coord_strategy() {
        let on_lan = session.peer_on_local_lan(peer)
            || (session.network_profile_snapshot().has_active_lan()
                && super::network_transport::ipv4_from_ma_str(&addr.to_string())
                    .is_some_and(|ip| ip.is_private() && !super::network_transport::is_cgnat_ipv4(ip)));
        if !is_relay && !on_lan {
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
        && super::network_transport::ipv4_from_ma_str(&addr.to_string())
            .is_some_and(|ip| ip.is_loopback());
    // Never dial "bare" `/p2p/<peer>` multiaddrs (invalid / guaranteed to fail).
    if is_bare_peer_multiaddr(&addr) {
        let now = chrono_now_ms();
        if session.should_log_dial_skip(peer, now, 8_000) {
            native_log::info("dial", format!("skip invalid dial addr for {peer}: {addr}"));
        }
        return;
    }
    if !loopback_coord && !super::network_transport::is_dm_dial_multiaddr(&addr) {
        return;
    }
    if !is_relay && !is_tcp_multiaddr(&addr) {
        return;
    }
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    match swarm.dial(dial_ma.clone()) {
        Ok(()) => {
            if is_relay {
                session.mark_circuit_dial_in_flight(peer, now);
                if tag.starts_with("coord") {
                    session.note_circuit_coord_dial_attempt(peer, now);
                }
            } else {
                session.note_routed_dial_attempt(peer, now);
                session.mark_lan_dial_in_flight(peer, now);
            }
            native_log::info(tag, format!("dialing {peer} via {dial_ma}"));
        }
        Err(e) => native_log::debug(tag, format!("dial {peer} {dial_ma}: {e}")),
    }
}

fn handle_mdns_discovered_list(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    list: Vec<(PeerId, Multiaddr)>,
    _emit: &mut dyn FnMut(GossipChatEvent),
) {
    let mut by_peer: HashMap<PeerId, Vec<Multiaddr>> = HashMap::new();
    for (peer, addr) in list {
        native_log::info("mdns", format!("discovered {peer} at {addr}"));
        by_peer.entry(peer).or_default().push(addr);
    }
    for (peer, addrs) in by_peer {
        for addr in &addrs {
            if super::network_transport::is_relay_circuit_multiaddr(addr)
                && super::network_transport::ipv4_from_ma_str(&addr.to_string())
                    .is_some_and(|ip| ip.is_private())
            {
                session.note_peer_on_local_lan(peer);
            }
            session.merge_mdns_lan_candidate(peer, addr);
        }
        let ranked = rank_mdns_lan_tcp_candidates(&addrs);
        let Some(best) = ranked.first().cloned() else {
            continue;
        };
        session.note_peer_on_local_lan(peer);
        if swarm.is_connected(&peer) {
            let now = chrono_now_ms();
            let skip_lan_upgrade = session.dm_has_stream_writer(peer)
                && !session.dm_link_needs_recovery(peer, now);
            if !skip_lan_upgrade
                && !session.peer_has_direct_connection(peer)
                && session.should_lan_upgrade_dial(peer, now)
            {
                dial_lan_upgrade(swarm, session, peer, best);
            }
        } else {
            let now = chrono_now_ms();
            let failover = session.lan_dial_in_flight_blocks(peer, now);
            dial_mdns_lan_addr(swarm, session, peer, best, failover);
            // When mDNS lists both a bogus high port and the live LAN port, dial the
            // fallback in parallel so a refused wrong-port dial does not delay LAN ~15s.
            if ranked.len() > 1 {
                dial_mdns_lan_addr(swarm, session, peer, ranked[1].clone(), true);
            }
        }
    }
}

/// Direct LAN TCP from mDNS — additive with in-flight WAN/relay dials (`PeerCondition::Always`).
fn dial_mdns_lan_addr(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
    failover: bool,
) {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if is_bare_peer_multiaddr(&addr)
        || !is_tcp_multiaddr(&addr)
        || !super::network_transport::is_dm_dial_multiaddr(&addr)
    {
        return;
    }
    let now = chrono_now_ms();
    if !failover && session.lan_dial_in_flight_blocks(peer, now) {
        return;
    }
    if !failover && !session.should_routed_dial(peer, now, 2_000) {
        return;
    }
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .addresses(vec![dial_ma.clone()])
            .condition(PeerCondition::Always)
            .build(),
    ) {
        Ok(()) => {
            session.note_routed_dial_attempt(peer, now);
            session.mark_lan_dial_in_flight(peer, now);
            native_log::info("mdns", format!("dialing {peer} via {dial_ma}"));
        }
        Err(e) => native_log::debug("mdns", format!("dial {peer} {dial_ma}: {e}")),
    }
}

/// Dial a peer's **direct LAN** multiaddr even while a relay/circuit connection is already
/// open, so the faster LAN path is established. Uses `PeerCondition::NotDialing` (rather than
/// the default `DisconnectedAndNotDialing`) so the dial is not refused just because a relay
/// link exists. Only direct TCP LAN addrs are dialed here — never relay circuits or bare ids.
/// Additive dial (WAN or LAN) while an existing link is open — `PeerCondition::NotDialing`.
fn dial_additive_dm_addr(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
    tag: &str,
) {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if is_bare_peer_multiaddr(&addr) {
        return;
    }
    let is_relay = super::network_transport::is_relay_circuit_multiaddr(&addr);
    if is_relay {
        let pk_hex = session
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone());
        if dm_peer_chat_link_stable(
            swarm,
            session,
            peer,
            pk_hex.as_deref(),
            chrono_now_ms(),
        ) {
            return;
        }
    }
    if is_relay && session.peer_has_direct_connection(peer) {
        return;
    }
    if !is_relay && !is_tcp_multiaddr(&addr) {
        return;
    }
    #[cfg(target_os = "android")]
    if is_relay && !is_tcp_multiaddr(&addr) {
        return;
    }
    if !is_relay && !super::network_transport::is_dm_dial_multiaddr(&addr) {
        return;
    }
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .addresses(vec![dial_ma.clone()])
            .condition(PeerCondition::NotDialing)
            .build(),
    ) {
        Ok(()) => native_log::info(tag, format!("additive dial {peer} via {dial_ma}")),
        Err(DialError::DialPeerConditionFalse(_)) => {}
        Err(e) => native_log::debug(tag, format!("additive dial {peer} {dial_ma}: {e}")),
    }
}

fn dial_lan_upgrade(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
) {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if super::network_transport::is_relay_circuit_multiaddr(&addr)
        || is_bare_peer_multiaddr(&addr)
        || !is_tcp_multiaddr(&addr)
    {
        return;
    }
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .addresses(vec![dial_ma.clone()])
            .condition(PeerCondition::NotDialing)
            .build(),
    ) {
        Ok(()) => native_log::info("mdns", format!("LAN upgrade dial {peer} via {dial_ma}")),
        Err(DialError::DialPeerConditionFalse(_)) => {}
        Err(e) => native_log::debug("mdns", format!("LAN upgrade dial {peer} {dial_ma}: {e}")),
    }
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
    let coord_degraded = crate::coord_runtime::coord_http_degraded();
    let dht_boot = session.any_bootstrap_connected.load(Ordering::Relaxed);
    let relay_circuit = relay_circuit_listening(swarm);
    let wan_recovery = session.wan_recovery_active.load(Ordering::Relaxed);
    let outbox_pending = session.pending_outbox_count();
    let links = session.connected_peers().len();
    let net = session.network_profile_snapshot();
    let profile = net.mode_label();
    let hint = net.dial_hint();
    native_log::info(
        "flow",
        format!(
            "connectivity local={local} profile={profile} hint={hint} \
             listen_addrs={} [{}] dm=[{}] active_links={links} \
             relay_circuit={relay_circuit} wan_recovery={wan_recovery} outbox_pending={outbox_pending} \
             coord_relay_tcp={dht_boot} coord_configured={coord_cfg} coord_registered={coord_reg} \
             coord_http_degraded={coord_degraded}",
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
        SwarmEvent::Behaviour(ChatBehaviourEvent::Identify(
            libp2p::identify::Event::Received { peer_id, info, .. },
        )) => {
            ingest_identify_listen_addrs(swarm, session, peer_id, &info.listen_addrs, "identify");
            if session.is_bootstrap_peer(peer_id) {
                session.note_bootstrap_identified(peer_id);
                try_relay_reservation_after_identify(swarm, session, peer_id);
            }
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
                let _ = session.merge_published_listen(vec![addr.clone()]);
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
            let _ = session.merge_published_listen(vec![external_addr.clone()]);
            crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                &session.published_listen_snapshot(),
            );
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Upnp(_)) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Mdns(libp2p::mdns::Event::Discovered(list))) => {
            handle_mdns_discovered_list(swarm, session, list, emit);
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Mdns(libp2p::mdns::Event::Expired(list))) => {
            // Drop LAN preference only when the expired addr is still our cached direct TCP
            // target. Phones rebind listen ports often — libp2p emits Expired(old) after
            // Discovered(new); wiping the whole peer on every expire caused stale-port dials.
            let mut peers_left = std::collections::HashSet::new();
            for (peer, addr) in list {
                if session.note_peer_mdns_lan_addr_expired(peer, &addr) {
                    native_log::info("mdns", format!("expired {peer} at {addr} — LAN path dropped"));
                    peers_left.insert(peer);
                }
            }
            if !peers_left.is_empty() && crate::coord_runtime::wan_discovery_via_coord_only() {
                notify_relay_refresh();
                if !wan_recovery_satisfied(session, swarm) {
                    session.begin_wan_recovery();
                }
            }
            let mut dm_left_lan = false;
            for peer in peers_left {
                if !session.is_dm_contact(peer) {
                    continue;
                }
                dm_left_lan = true;
                if let Some(pk) = secp256k1_public_key_hex_from_peer_id(&peer) {
                    session.mark_dm_reconnect_urgent(&pk);
                }
                if !swarm.is_connected(&peer) {
                    kick_dm_peer_discovery(swarm, session, peer);
                }
            }
            if dm_left_lan {
                notify_coord_lookup();
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Stream(_)) => {}
        SwarmEvent::ListenerClosed {
            addresses, reason, ..
        } => {
            // A relay reservation that fails surfaces here as the circuit listener closing with an
            // error reason (e.g. resource limit, connection reset over the tunnel, timeout). Without
            // this log the failure is invisible and "reserving circuit …" appears to loop forever.
            let relay_listener = addresses
                .iter()
                .any(super::network_transport::is_relay_circuit_multiaddr);
            let kind = if relay_listener { "relay circuit" } else { "listener" };
            match &reason {
                Ok(()) => native_log::info(
                    "listen",
                    format!("{kind} closed cleanly addrs={addresses:?}"),
                ),
                Err(e) => native_log::warn(
                    "relay",
                    format!("{kind} closed with error: {e} addrs={addresses:?}"),
                ),
            }
            if relay_listener {
                let closed_ipv4 = addresses
                    .iter()
                    .any(super::network_transport::is_coord_ipv4_relay_listen);
                let still_ipv4 = swarm
                    .listeners()
                    .any(super::network_transport::is_coord_ipv4_relay_listen);
                if !closed_ipv4 {
                    native_log::debug(
                        "relay",
                        format!("ignore non-IPv4 relay listener close addrs={addresses:?}"),
                    );
                } else if still_ipv4 {
                    native_log::debug(
                        "relay",
                        "IPv4 relay circuit still listening — ignore spurious listener close",
                    );
                    let _ = sync_published_listen_from_swarm(session, swarm);
                } else if matches!(reason, Ok(())) {
                    native_log::info(
                        "relay",
                        "relay circuit listener closed cleanly — libp2p renewal; \
                         not re-issuing listen_on (would cancel live reservation)",
                    );
                    let _ = sync_published_listen_from_swarm(session, swarm);
                } else {
                    native_log::warn(
                        "relay",
                        format!(
                            "relay circuit listener closed ({reason:?}) — IPv4 circuit gone; re-reserve"
                        ),
                    );
                    if let Some(ghalbol) = ghalbol_relay_peer(session) {
                        clear_relay_reserve_in_flight(session, ghalbol);
                        if let Ok(mut g) = session.relay_reserve_requested.write() {
                            g.remove(&ghalbol);
                        }
                        if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
                            m.remove(&ghalbol);
                        }
                    }
                    if let Ok(mut v) = session.published_listen.write() {
                        for addr in &addresses {
                            v.retain(|ma| ma != addr);
                        }
                        v.retain(|ma| !super::network_transport::is_relay_circuit_multiaddr(ma));
                    }
                    if crate::coord_runtime::wan_discovery_via_coord_only() {
                        notify_relay_refresh();
                        session.begin_wan_recovery();
                        crate::coord_runtime::schedule_register_presence_force();
                        for pk in session.dm_public_keys() {
                            session.mark_dm_reconnect_urgent(&pk);
                        }
                        notify_coord_lookup();
                    }
                    let _ = ensure_wan_relay_circuit(swarm, session, None, false);
                }
            }
        }
        SwarmEvent::ListenerError { error, .. } => {
            native_log::warn("relay", format!("listener error: {error}"));
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            let is_relay = super::network_transport::is_relay_circuit_multiaddr(&address);
            if is_relay && !super::network_transport::is_coord_ipv4_relay_listen(&address) {
                native_log::debug(
                    "listen",
                    format!("ignore non-IPv4 relay circuit listen {address}"),
                );
            } else {
            native_log::info("listen", format!("listening on {address}"));
            if is_relay {
                let _ = session.merge_published_listen(vec![address.clone()]);
                native_log::info("relay", format!("relay listen addr {address}"));
            } else {
                let expanded = expand_listen_addresses(&address);
                let _ = session.merge_published_listen(expanded);
            }
            let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
            crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                &session.published_listen_snapshot(),
            );
            if should_emit_listening_event(&address) {
                emit(GossipChatEvent::Listening(address.clone()));
            }
            if super::network_transport::is_coord_relay_tcp_circuit_multiaddr(&address) {
                crate::coord_runtime::schedule_register_presence_force();
                finish_wan_recovery_if_ready(session, swarm);
            }
            }
        }
        SwarmEvent::ConnectionClosed {
            peer_id,
            connection_id,
            endpoint,
            ..
        } => {
            if session.consume_incidental_reject(peer_id) {
                return;
            }
            if session.is_bootstrap_peer(peer_id) {
                drop_bootstrap_tcp_conn(session, peer_id, connection_id);
                if bootstrap_relay_conn_count(session, peer_id) == 0 {
                    session.clear_bootstrap_relay_session(peer_id);
                    if ghalbol_relay_peer(session) == Some(peer_id) {
                        native_log::warn(
                            "relay",
                            format!(
                                "coord relay bootstrap TCP lost ({peer_id}) — reservation invalid until reconnected; peers cannot dial our /p2p-circuit"
                            ),
                        );
                        let _ = sync_published_listen_from_swarm(session, swarm);
                        if let Ok(mut v) = session.published_listen.write() {
                            v.retain(|ma| !super::network_transport::is_relay_circuit_multiaddr(ma));
                        }
                        crate::coord_runtime::coord_invalidate_presence_on_network_change();
                        notify_relay_refresh();
                        session.begin_wan_recovery();
                        crate::coord_runtime::schedule_register_presence_force();
                        for pk in session.dm_public_keys() {
                            session.mark_dm_reconnect_urgent(&pk);
                        }
                        notify_coord_lookup();
                    }
                }
                native_log::debug("swarm", format!("bootstrap connection closed {peer_id}"));
                session.refresh_bootstrap_connected_flag(swarm);
            } else if session.is_dm_contact(peer_id) {
                session.clear_circuit_dial_in_flight(peer_id);
                let is_relay =
                    super::network_transport::is_relay_circuit_multiaddr(endpoint.get_remote_address());
                session.drop_connection_path(peer_id, is_relay);
                native_log::info("swarm", format!("dm connection closed {peer_id}"));
                emit(GossipChatEvent::PeerDisconnected(peer_id));
            }
            session.note_disconnected(&peer_id);
        }
        SwarmEvent::ConnectionEstablished {
            peer_id,
            connection_id,
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
                session.clear_circuit_dial_in_flight(peer_id);
                session.clear_lan_dial_in_flight(peer_id);
                session.clear_relay_circuit_dial_backoff(peer_id);
                let is_relay =
                    super::network_transport::is_relay_circuit_multiaddr(endpoint.get_remote_address());
                session.note_connection_path(peer_id, is_relay);
                if !is_relay {
                    if let Some(ip) = super::network_transport::ipv4_from_ma_str(
                        &endpoint.get_remote_address().to_string(),
                    ) {
                        if ip.is_private() {
                            session.note_peer_on_local_lan(peer_id);
                        }
                    }
                }
                native_log::info(
                    "swarm",
                    format!(
                        "dm connection established {peer_id} via {} ({})",
                        endpoint.get_remote_address(),
                        if is_relay { "relay" } else { "direct" }
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
                on_bootstrap_tcp_connected(swarm, session, peer_id, connection_id, &endpoint);
            }
        }
        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            let Some(peer) = peer_id else {
                return;
            };
            if session.is_bootstrap_peer(peer) {
                let err_s = error.to_string();
                let is_ghalbol_relay = ghalbol_relay_peer(session) == Some(peer);
                let now_ms = chrono_now_ms();
                if err_s.contains("Network is unreachable") || err_s.contains("network unreachable")
                {
                    if err_s.contains("/ip6/") {
                        session.note_bootstrap_ipv6_unreachable(peer, now_ms);
                        if is_ghalbol_relay && !swarm.is_connected(&peer) {
                            if let Ok(mut m) = session.bootstrap_dial_last_ms.write() {
                                m.remove(&peer);
                            }
                            let v4_nodes: Vec<(PeerId, Multiaddr)> = session
                                .ghalbol_relay_state
                                .read()
                                .ok()
                                .and_then(|g| g.clone())
                                .map(|(relay_peer, addrs)| {
                                    super::network_transport::resolve_relay_bootnodes(
                                        &relay_peer.to_string(),
                                        &addrs,
                                    )
                                })
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|(p, ma)| {
                                    *p == peer
                                        && ma.to_string().contains("/ip4/")
                                        && super::network_transport::is_trusted_bootstrap_dial_addr(
                                            ma,
                                        )
                                })
                                .collect();
                            if !v4_nodes.is_empty() {
                                issue_bootstrap_dials(
                                    swarm,
                                    session,
                                    &v4_nodes,
                                    "coord relay ipv4 fallback",
                                    true,
                                );
                            }
                        }
                    }
                }
                if is_ghalbol_relay
                    && (err_s.contains("Connection refused")
                        || err_s.contains("connection refused"))
                {
                    let relay_addr = session
                        .bootstrap_relay_addr
                        .read()
                        .ok()
                        .and_then(|m| m.get(&peer).cloned())
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| peer.to_string());
                    crate::coord_runtime::invalidate_cached_ghalbol_relay(
                        session.relay_cache_path.as_deref(),
                    );
                    notify_relay_refresh();
                    native_log::warn(
                        "relay",
                        format!(
                            "Ghal Bol relay TCP unreachable at {relay_addr} — WAN is down until \
                             the relay port is reachable (dev: restart ./ghal_bol_server/deploy/run_server.sh \
                             so bore picks a fresh port; prod: check coord.ghalbol.com:4002). \
                             Cleared stale relay cache; will refetch GET /v1/relay."
                        ),
                    );
                } else if session.should_log_bootstrap_dial_err(peer, chrono_now_ms()) {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue=bootstrap_dial_error | peer={peer} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                }
            } else if session.should_dial_libp2p_peer(peer) {
                session.clear_circuit_dial_in_flight(peer);
                session.clear_lan_dial_in_flight(peer);
                let err_s = format!("{error}");
                let now_ms = chrono_now_ms();
                if crate::coord_runtime::coord_is_configured()
                    && session.peer_on_local_lan(peer)
                    && !err_s.contains("/p2p-circuit")
                {
                    if err_s.contains("Connection refused") {
                        let failed = failed_dial_multiaddr_from_error(&err_s);
                        session.try_mdns_lan_failover_dial(swarm, peer, failed.as_ref());
                        if !swarm.is_connected(&peer) {
                            kick_dm_peer_discovery(swarm, session, peer);
                        }
                    } else if err_s.contains("Timeout") {
                        let failed = failed_dial_multiaddr_from_error(&err_s);
                        session.try_mdns_lan_failover_dial(swarm, peer, failed.as_ref());
                        if !swarm.is_connected(&peer) && session.peer_mdns_lan_addr(peer).is_none() {
                            session.invalidate_stale_lan_dial(peer, failed.as_ref(), now_ms);
                        }
                    }
                }
                let (issue, next) = super::connectivity_diag::explain_outgoing_dial_error(&err_s);
                if err_s.contains("ResourceLimitExceeded") || err_s.contains("resource limit") {
                    session.note_relay_circuit_dial_rate_limited(peer, chrono_now_ms());
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else if err_s.contains("Relay has no reservation") {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else if err_s.contains("p2p-circuit") {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else if is_transient_swarm_dial_error(&err_s) {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | transient dial error: {error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | error={error} | ctx={}",
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
            clear_relay_reserve_in_flight(session, relay_peer_id);
            native_log::info("relay", format!("reservation accepted on {relay_peer_id}"));
            crate::coord_runtime::coord_note_relay_reservation(relay_peer_id);
            let relay_addrs: Vec<Multiaddr> = swarm
                .listeners()
                .filter(|ma| super::network_transport::is_relay_circuit_multiaddr(ma))
                .cloned()
                .collect();
            if !relay_addrs.is_empty() {
                let _ = session.merge_published_listen(relay_addrs);
            }
            let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
            crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                &session.published_listen_snapshot(),
            );
            if crate::coord_runtime::has_coord_endpoints() {
                crate::coord_runtime::schedule_register_presence_force();
            }
            finish_wan_recovery_if_ready(session, swarm);
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Relay(ev)) => {
            native_log::info("relay", format!("relay-client: {ev:?}"));
        }
        _ => {}
    }
}

/// Collect TCP / relay-circuit listen addrs before `node_ready`.
///
/// Must run **all** relay-relevant swarm events through `handle_swarm_event` — dropping
/// Identify/Relay here leaves bootstrap TCP up with no `listen_on` (reservation never starts).
async fn bootstrap_publishable_listen(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    timeout: Duration,
) {
    let coord_mode = crate::coord_runtime::wan_discovery_via_coord_only();
    let deadline = time::Instant::now() + timeout;
    while time::Instant::now() < deadline {
        if listen_ready_for_node(session, coord_mode, swarm) {
            let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
            return;
        }
        tokio::select! {
            ev = swarm.select_next_some() => {
                handle_swarm_event(swarm, session, ev, &mut |_ev| {});
                if coord_mode
                    && session.any_bootstrap_connected.load(Ordering::Relaxed)
                {
                    attempt_wan_relay_reserve(swarm, session, false);
                }
                if listen_ready_for_node(session, coord_mode, swarm) {
                    let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
                    return;
                }
            }
            _ = time::sleep(Duration::from_millis(40)) => {
                if coord_mode
                    && session.any_bootstrap_connected.load(Ordering::Relaxed)
                {
                    attempt_wan_relay_reserve(swarm, session, false);
                }
            }
        }
    }
    if !listen_ready_for_node(session, coord_mode, swarm) {
        if coord_mode {
            native_log::warn(
                "listen",
                "no relay circuit before node_ready — WAN peers cannot dial this device until \
                 reservation accepted (check coord relay connectivity)",
            );
        } else {
            native_log::warn(
                "listen",
                "no publishable TCP listen addr before node_ready — peers may not find this device yet",
            );
        }
    }
}

const COORD_LOOKUP_INTERVAL_SECS: u64 = 2;
/// Min gap between coord HTTP lookups for a disconnected DM peer (dm_upkeep ~1s tick).
const DM_COORD_LOOKUP_MIN_INTERVAL_MS: i64 = 2_000;
const NETWORK_PROFILE_POLL_SECS: u64 = 1;
const PEER_LAN_SEEN_TTL_MS: i64 = 180_000;
/// Per-peer throttle for mDNS-driven LAN upgrade dials. mDNS re-announces frequently; we only
/// need to (re)establish the direct LAN link occasionally while it is missing, not every announce.
const LAN_UPGRADE_DIAL_THROTTLE_MS: i64 = 2_000;
const BOOTSTRAP_REDIAL_INTERVAL_SECS: u64 = 12;
/// After a DM connection drops, treat reconnect as urgent for this long: coord lookups skip the
/// `peer_not_on_server` backoff and run every ~1s upkeep tick. Bounded so a genuinely offline
/// peer eventually falls back to the normal coord cadence + exponential backoff.
const DM_RECONNECT_URGENT_WINDOW_MS: i64 = 30_000;

/// Connected DM with an open chat stream and no urgent recovery — skip coord/relay churn.
fn dm_peer_chat_link_stable(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    pk_hex: Option<&str>,
    now_ms: i64,
) -> bool {
    if !swarm.is_connected(&peer) {
        return false;
    }
    // Healthy open stream — do not churn for urgent/coord/LAN even if a stale addr timed out.
    if session.dm_has_stream_writer(peer) && !session.dm_link_needs_recovery(peer, now_ms) {
        return true;
    }
    if session.dm_link_needs_recovery(peer, now_ms) {
        return false;
    }
    if let Some(pk) = pk_hex {
        if session.is_pk_reconnect_urgent(pk, now_ms) {
            return false;
        }
    }
    session
        .chat_ready_emitted
        .read()
        .ok()
        .is_some_and(|g| g.contains(&peer))
}

fn coord_dial_from_lookup_addrs(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    target: PeerId,
    addrs: Vec<Multiaddr>,
    now_ms: i64,
    wan_additive: bool,
    urgent: bool,
) {
    if addrs.is_empty() {
        return;
    }
    if dm_peer_chat_link_stable(swarm, session, target, None, now_ms) {
        return;
    }
    if swarm.is_connected(&target) && session.peer_has_direct_connection(target) && !urgent {
        return;
    }
    let mut ranked = sort_dm_dial_addrs_for_profile(session, target, addrs, true);
    if let Some(lan_ma) = session.peer_mdns_lan_addr(target) {
        if is_tcp_multiaddr(&lan_ma)
            && !super::network_transport::is_relay_circuit_multiaddr(&lan_ma)
        {
            ranked.retain(|ma| ma != &lan_ma);
            ranked.insert(0, lan_ma);
        }
    }
    // On Wi‑Fi/LAN never dial coord-published RFC1918 — ports go stale; LAN is live mDNS only.
    if session.network_profile_snapshot().has_active_lan() {
        ranked.retain(|ma| !is_direct_lan_tcp_ma(ma));
    }
    // Never dial relay from coord lookup while mDNS LAN is viable — races cancel LAN TCP (TRANSPORT.md).
    if session.should_defer_coord_relay_for_lan(target, now_ms, swarm.is_connected(&target)) {
        ranked.retain(|ma| !super::network_transport::is_relay_circuit_multiaddr(ma));
        if ranked.is_empty() {
            return;
        }
    }
    let ranked: Vec<Multiaddr> = if swarm.is_connected(&target)
        && session.peer_has_direct_connection(target)
        && !urgent
    {
        ranked
            .into_iter()
            .filter(|ma| !super::network_transport::is_relay_circuit_multiaddr(ma))
            .collect()
    } else {
        ranked
    };
    let Some(ma) = ranked.into_iter().next() else {
        return;
    };
    if super::network_transport::is_relay_circuit_multiaddr(&ma)
        && swarm.is_connected(&target)
        && !urgent
    {
        return;
    }
    if wan_additive {
        if urgent || session.should_routed_dial(target, now_ms, 2_000) {
            dial_additive_dm_addr(swarm, session, target, ma, "coord-additive");
        }
    } else {
        let tag = if is_direct_lan_tcp_ma(&ma) {
            "lan"
        } else {
            "coord"
        };
        let before = session
            .circuit_coord_dial_last_ms
            .read()
            .ok()
            .and_then(|m| m.get(&target).copied())
            .unwrap_or(0);
        dial_dm_peer_addr(swarm, session, target, ma, tag);
        let after = session
            .circuit_coord_dial_last_ms
            .read()
            .ok()
            .and_then(|m| m.get(&target).copied())
            .unwrap_or(0);
        if after != before && tag == "coord" {
            native_log::info(
                "coord",
                format!("coord_lookup_peer ok — dialing {target} via relay circuit"),
            );
        }
    }
}

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
    let target = peer_id_from_secp256k1_public_key_hex(pk)
        .ok()
        .and_then(|s| s.parse::<PeerId>().ok());
    let Some(target) = target else {
        return;
    };
    let connected = swarm.is_connected(&target);
    let stale = connected && session.dm_link_needs_recovery(target, now_ms);
    let urgent = session.is_pk_reconnect_urgent(pk, now_ms) || stale;
    // Stable DM stream — stop periodic coord lookups / additive dials that churn the link.
    if dm_peer_chat_link_stable(swarm, session, target, Some(pk), now_ms) {
        return;
    }
    // After LAN loss: keep the existing link and add a WAN path in parallel (STORY.md).
    let wan_additive = connected
        && !session.peer_on_local_lan(target)
        && crate::coord_runtime::coord_is_configured();
    if connected && !wan_additive && !stale {
        return;
    }
    if !connected {
        // WAN first (live coord lookup below); also kick live mDNS LAN when present.
        kick_dm_peer_discovery(swarm, session, target);
        if swarm.is_connected(&target) {
            return;
        }
    }
    let mut skip_coord_http = false;
    if !urgent {
        if let Ok(m) = session.coord_lookup_backoff.read() {
            if let Some(b) = m.get(pk) {
                if now_ms < b.next_allowed_ms {
                    skip_coord_http = true;
                    native_log::debug(
                        "coord",
                        format!(
                            "lookup {pk} HTTP skipped (peer_not_on_server backoff; retry_in_ms={}) — mDNS active",
                            b.next_allowed_ms.saturating_sub(now_ms)
                        ),
                    );
                }
            }
        }
    }
    if !skip_coord_http
        && !urgent
        && crate::coord_runtime::coord_is_configured()
        && session.prefers_mobile_coord_strategy()
        && !crate::coord_runtime::coord_is_registered()
        && !crate::coord_runtime::coord_http_degraded()
    {
        skip_coord_http = true;
        native_log::debug(
            "coord",
            format!("lookup {pk} HTTP skipped (self not registered yet) — mDNS active"),
        );
    }
    if !skip_coord_http {
        match crate::coord_runtime::lookup_dial_multiaddrs_for_public_key_async(pk).await {
            Ok(addrs) => {
                session.clear_coord_lookup_backoff(pk);
                session.set_coord_lookup_category(
                    pk,
                    super::connectivity_diag::CoordLookupCategory::Ok,
                );
                let addrs = if crate::coord_runtime::coord_is_configured()
                    && session.prefers_mobile_coord_strategy()
                {
                    super::network_transport::wan_coord_dial_addrs(addrs)
                } else {
                    addrs
                };
                if addrs.is_empty() {
                    session.set_coord_lookup_category(
                        pk,
                        super::connectivity_diag::CoordLookupCategory::NoDialableAddrs,
                    );
                    let (reason, action) = super::connectivity_diag::explain_coord_lookup_failure(
                        super::connectivity_diag::CoordLookupCategory::NoDialableAddrs,
                        crate::coord_runtime::coord_is_registered(),
                    );
                    native_log::warn(
                        "coord",
                        format!(
                            "lookup {pk} — {reason} | next={action} | raw=no dialable addrs in presence record"
                        ),
                    );
                } else {
                    let dial_now = chrono_now_ms();
                    if dm_peer_chat_link_stable(swarm, session, target, Some(pk), dial_now) {
                        return;
                    }
                    let stale = swarm.is_connected(&target)
                        && session.dm_link_needs_recovery(target, dial_now);
                    let urgent_now =
                        session.is_pk_reconnect_urgent(pk, dial_now) || stale;
                    let wan_additive_now = swarm.is_connected(&target)
                        && !session.peer_on_local_lan(target)
                        && crate::coord_runtime::coord_is_configured();
                    coord_dial_from_lookup_addrs(
                        swarm,
                        session,
                        target,
                        addrs,
                        dial_now,
                        wan_additive_now,
                        urgent_now,
                    );
                }
            }
            Err(e) => {
                let es = e.to_string();
                let cat = super::connectivity_diag::classify_coord_lookup_error(&es);
                session.set_coord_lookup_category(pk, cat);
                if cat == super::connectivity_diag::CoordLookupCategory::PeerNotOnCoord {
                    session.note_coord_lookup_not_found(pk, now_ms);
                } else {
                    crate::coord_runtime::note_coord_transport_failure();
                }
                let (reason, action) = super::connectivity_diag::explain_coord_lookup_failure(
                    cat,
                    crate::coord_runtime::coord_is_registered(),
                );
                native_log::info(
                    "coord",
                    format!(
                        "lookup {pk} — category={} | reason={reason} | next={action} | http={es}",
                        cat.as_str()
                    ),
                );
            }
        }
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
    let mut call_incoming = control
        .accept(StreamProtocol::new(CALL_STREAM_PROTOCOL))
        .map_err(|e| ChatServerError::Transport(format!("accept call stream: {e}")))?;
    let mut call_video_incoming = control
        .accept(StreamProtocol::new(CALL_VIDEO_STREAM_PROTOCOL))
        .map_err(|e| ChatServerError::Transport(format!("accept call video stream: {e}")))?;
    let writers: StreamWriters = Arc::new(Mutex::new(HashMap::new()));

    let coord_only = crate::coord_runtime::wan_discovery_via_coord_only();
    let mut coord_relays: Vec<(PeerId, Multiaddr)> = Vec::new();
    let relay_cache = config
        .transcript_path
        .as_deref()
        .and_then(|tp| Path::new(tp).parent().map(|d| d.join("ghalbol_relay.json")));
    let mut ghalbol_relay_initial: Option<(PeerId, Vec<String>)> = None;
    if coord_only {
        // Fetch co-located relay(s) from every configured coord server.
        let all_relays = tokio::task::spawn_blocking({
            let cache = relay_cache.clone();
            move || crate::coord_runtime::fetch_all_ghalbol_relays(cache)
        })
        .await
        .ok()
        .unwrap_or_default();
        for (relay_peer, relay_addrs) in &all_relays {
            let relay_nodes =
                super::network_transport::resolve_relay_bootnodes(relay_peer, relay_addrs);
            if relay_nodes.is_empty() {
                native_log::warn(
                    "relay",
                    format!(
                        "ghalbol relay {relay_peer} advertised but no dialable public addr yet — will retry via coord_tick"
                    ),
                );
            } else {
                native_log::info(
                    "relay",
                    format!(
                        "ghalbol relay {relay_peer}: {} dial addr(s) — preferred for reservation",
                        relay_nodes.len()
                    ),
                );
                merge_relay_nodes_into_coord_relays(&mut coord_relays, &relay_nodes);
            }
            if let Ok(relay_pid) = relay_peer.parse::<PeerId>() {
                if ghalbol_relay_initial.is_none() {
                    ghalbol_relay_initial = Some((relay_pid, relay_addrs.clone()));
                }
            }
        }
        native_log::info(
            "coord",
            format!(
                "coord URL set — peer discovery via server; dialing {} coord relay node(s) for circuit reservation",
                coord_relays.len()
            ),
        );
        if coord_relays.is_empty() {
            native_log::warn(
                "relay",
                "no coord relay dial addr at startup — will refetch /v1/relay every few seconds; \
                 LAN works via mDNS; WAN needs coord GET /v1/relay with a dialable relay addr",
            );
            notify_relay_refresh();
        }
    }
    let net = super::network_transport::detect_local_network_profile();
    let mut bootstrap_peer_ids: HashSet<PeerId> = coord_relays.iter().map(|(p, _)| *p).collect();
    if let Some((relay_pid, _)) = &ghalbol_relay_initial {
        bootstrap_peer_ids.insert(*relay_pid);
    }
    let session = Arc::new(SessionState::new(
        identity,
        &config.dm_peers,
        bootstrap_peer_ids,
        config.transcript_path.clone(),
        config.app_namespace.clone(),
        net.clone(),
        relay_cache,
        ghalbol_relay_initial,
    )?);
    native_log::info(
        "p2p",
        format!(
            "swarm up: dm_peers={} invite_bootstrap={} coord_relays={} coord_only={coord_only}",
            session.dm_peer_ids().len(),
            bootstrap.len(),
            coord_relays.len()
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
    listen_swarm_transports(&mut swarm)?;
    // Start relay TCP + CGNAT probe before the brief listen wait so cellular bootstrap is not idle.
    if coord_only && !coord_relays.is_empty() {
        dial_coord_relays(&mut swarm, &session, &coord_relays);
        session.begin_wan_recovery();
    }
    // Wait for bootstrap TCP + one HOP-anchored `listen_on` before node_ready (coord WAN).
    let listen_wait = if coord_only {
        Duration::from_secs(BOOTSTRAP_LISTEN_MAX_SECS)
    } else {
        Duration::from_millis(800)
    };
    bootstrap_publishable_listen(&mut swarm, &session, listen_wait).await;
    crate::coord_runtime::rebuild_coord_endpoints_from_listen(
        &session.published_listen_snapshot(),
    );
    if coord_only && !coord_relays.is_empty() {
        dial_coord_relays(&mut swarm, &session, &coord_relays);
    }
    ensure_wan_relay_circuit(&mut swarm, session.as_ref(), Some(&coord_relays), false);
    if coord_only && !relay_circuit_listening(&swarm) {
        if !listen_ready_for_node(session.as_ref(), true, &swarm) {
            native_log::info("net", "WAN not ready at startup — begin recovery pass");
            session.begin_wan_recovery();
        }
    }
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
    let mut coord_tick = time::interval(Duration::from_secs(COORD_LOOKUP_INTERVAL_SECS));
    coord_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut dm_upkeep_tick = time::interval(Duration::from_secs(1));
    dm_upkeep_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stream_upkeep_tick = time::interval(Duration::from_millis(500));
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
            session.call_media_stop_all();
            session.call_video_stop_all();
            super::call_active::clear();
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
                spawn_reopen_dm_chat_streams(
                    &swarm,
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    control.clone(),
                    Some(events_tx.clone()),
                );
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
                let now_ms = chrono_now_ms();
                session.expire_stale_circuit_dials(now_ms);
                session.expire_stale_lan_dials(now_ms);
                for peer in session.drain_dm_connection_resets() {
                    drop_dm_chat_stream_writer(peer, &writers, session.as_ref());
                    if swarm.is_connected(&peer) {
                        native_log::info("dm", format!("reset dead libp2p connection peer={peer}"));
                        let _ = swarm.disconnect_peer_id(peer);
                    }
                    if let Some(pk) = session
                        .dm_peer_for_libp2p(peer)
                        .and_then(|d| d.public_key_hex.clone())
                        .filter(|pk| pk.len() == 66)
                    {
                        session.mark_dm_reconnect_urgent(&pk);
                    }
                    notify_coord_lookup();
                }
                upkeep_dm_peers(
                    &mut swarm,
                    Arc::clone(&session),
                    control.clone(),
                    Arc::clone(&writers),
                    Some(events_tx.clone()),
                );
                for peer in session.dm_peer_ids() {
                    if !swarm.is_connected(&peer) {
                        continue;
                    }
                    if !session.dm_link_needs_recovery(peer, now_ms) {
                        continue;
                    }
                    let Some(pk) = session
                        .dm_peer_for_libp2p(peer)
                        .and_then(|d| d.public_key_hex.clone())
                        .filter(|pk| pk.len() == 66)
                    else {
                        continue;
                    };
                    native_log::info(
                        "dm",
                        format!("stale DM link peer={peer} — urgent reconnect + stream reopen"),
                    );
                    session.mark_dm_reconnect_urgent(&pk);
                    session.clear_chat_ready_emitted(peer);
                    notify_coord_lookup();
                    notify_stream_reopen();
                }
                // Fast reconnect: a contact whose link just dropped is looked up every ~1s
                // (backoff-free) instead of waiting for the coord tick. Bounded by the urgent
                // window so a truly offline peer falls back to the normal cadence.
                for pk in session.urgent_reconnect_pks(now_ms) {
                    coord_lookup_dm_peer(&mut swarm, session.as_ref(), &pk).await;
                }
                if crate::coord_runtime::coord_is_configured() {
                    let listen = coord_register_listen_snapshot(&swarm, session.as_ref());
                    crate::coord_runtime::coord_register_tick(&listen);
                    for pk in session.dm_public_keys() {
                        if session.is_pk_reconnect_urgent(&pk, now_ms) {
                            continue;
                        }
                        let Some(target) = peer_id_from_secp256k1_public_key_hex(&pk)
                            .ok()
                            .and_then(|s| s.parse::<PeerId>().ok())
                        else {
                            continue;
                        };
                        if swarm.is_connected(&target) {
                            continue;
                        }
                        if session.should_coord_lookup_pk(&pk, now_ms, DM_COORD_LOOKUP_MIN_INTERVAL_MS)
                        {
                            coord_lookup_dm_peer(&mut swarm, session.as_ref(), &pk).await;
                        }
                    }
                }
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
            _ = network_tick.tick() => {
                let recovering_before = session.wan_recovery_active.load(Ordering::Relaxed);
                let mut handover = false;
                let forced = take_network_change_notify();
                if forced {
                    let net = super::network_transport::detect_local_network_profile();
                    let (old_mode, new_mode, changed) = if let Ok(mut cur) = session.network_profile.write() {
                        let old_key = super::network_transport::network_handover_key(&*cur);
                        let old_mode = cur.mode_label().to_string();
                        let new_key = super::network_transport::network_handover_key(&net);
                        *cur = net;
                        let new_mode = cur.mode_label().to_string();
                        (old_mode, new_mode, old_key != new_key)
                    } else {
                        continue;
                    };
                    if changed {
                        handle_network_path_change(
                            &mut swarm,
                            session.as_ref(),
                            &coord_relays,
                            &old_mode,
                            &new_mode,
                        );
                    } else {
                        refresh_coord_reachability_after_network_change(
                            &mut swarm,
                            session.as_ref(),
                            &coord_relays,
                            "connectivity callback — public path may have changed (same interface profile)",
                            false,
                        );
                    }
                    handover = true;
                } else if let Some((old_mode, new_mode)) =
                    session.refresh_network_path_if_changed()
                {
                    handle_network_path_change(
                        &mut swarm,
                        session.as_ref(),
                        &coord_relays,
                        &old_mode,
                        &new_mode,
                    );
                    handover = true;
                } else if !recovering_before {
                    try_wan_relay_recovery(&mut swarm, session.as_ref());
                }
                if crate::coord_runtime::coord_is_configured() {
                    let snap = coord_register_listen_snapshot(&swarm, session.as_ref());
                    let fp =
                        super::network_transport::wan_coord_listen_fingerprint(&snap);
                    let fp_changed = session.wan_listen_fp_changed(&fp);
                    let synced =
                        sync_published_listen_from_swarm(session.as_ref(), &swarm);
                    if fp_changed {
                        refresh_coord_reachability_after_network_change(
                            &mut swarm,
                            session.as_ref(),
                            &coord_relays,
                            "WAN listen addrs changed — refresh coord presence",
                            false,
                        );
                    } else if synced {
                        crate::coord_runtime::rebuild_coord_endpoints_from_listen(&snap);
                        crate::coord_runtime::schedule_register_presence_force();
                    }
                }
                if session.wan_recovery_active.load(Ordering::Relaxed) {
                    run_wan_recovery_pass(&mut swarm, session.as_ref(), &coord_relays);
                }
                let recovering_after = session.wan_recovery_active.load(Ordering::Relaxed);
                if handover || (recovering_before && !recovering_after) {
                    coord_lookup_dm_peers(&mut swarm, session.as_ref()).await;
                    spawn_reopen_dm_chat_streams(
                        &swarm,
                        Arc::clone(&session),
                        Arc::clone(&writers),
                        control.clone(),
                        Some(events_tx.clone()),
                    );
                }
            }
            _ = coord_tick.tick() => {
                run_bootstrap_relay_reserve_pass(&mut swarm, session.as_ref());
                let force_relay = take_relay_refresh_notify();
                maybe_refresh_ghalbol_relay(
                    &mut swarm,
                    session.as_ref(),
                    &mut coord_relays,
                    force_relay,
                )
                .await;
                // If WAN reachability is not yet established (no relay circuit / not registered on
                // coord), pursue it — even while on a Wi‑Fi/LAN. Being on a LAN means mDNS covers
                // on‑LAN peers, but contacts on mobile data are OFF‑LAN and can only reach us via a
                // relay circuit + coord registration. Gating WAN recovery on `has_active_lan()` made
                // a Wi‑Fi device permanently invisible to off‑LAN contacts (coord_registered=false,
                // no /p2p-circuit). LAN is additive, never a replacement for WAN.
                if crate::coord_runtime::wan_discovery_via_coord_only()
                    && !wan_recovery_satisfied(session.as_ref(), &swarm)
                    && !session.wan_recovery_active.load(Ordering::Relaxed)
                {
                    native_log::info("net", "WAN not ready — begin recovery pass");
                    session.begin_wan_recovery();
                }
                if session.wan_recovery_active.load(Ordering::Relaxed) {
                    run_wan_recovery_pass(&mut swarm, session.as_ref(), &coord_relays);
                } else {
                    let listen = coord_register_listen_snapshot(&swarm, session.as_ref());
                    crate::coord_runtime::coord_register_tick(&listen);
                    let force_keepalive = session.should_relay_keepalive(chrono_now_ms());
                    ensure_wan_relay_circuit(
                        &mut swarm,
                        session.as_ref(),
                        Some(&coord_relays),
                        force_keepalive,
                    );
                }
                if crate::coord_runtime::coord_http_degraded() {
                    if !session.any_bootstrap_connected.load(Ordering::Relaxed) {
                        dial_coord_relays(&mut swarm, session.as_ref(), &coord_relays);
                    }
                    for pk in session.dm_public_keys() {
                        if let Ok(derived) = peer_id_from_secp256k1_public_key_hex(&pk) {
                            if let Ok(target) = derived.parse::<PeerId>() {
                                if !swarm.is_connected(&target) {
                                    kick_dm_peer_discovery(&mut swarm, session.as_ref(), target);
                                }
                            }
                        }
                    }
                }
                coord_lookup_dm_peers(&mut swarm, session.as_ref()).await;
            }
            _ = flow_snapshot_tick.tick() => {
                log_connectivity_snapshot(&swarm, session.as_ref(), &writers);
            }
            _ = redial_tick.tick() => {
                if !session.any_bootstrap_connected.load(Ordering::Relaxed) {
                    dial_coord_relays(&mut swarm, &session, &coord_relays);
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
                        if !super::network_transport::is_trusted_bootstrap_dial_addr(ma) {
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
                // Delivery acks only on the fast tick; read ack retries are ~1 s (DESIGN.md).
                if session.pending_delivery_ack_len() > 0 {
                    let connected_now: Vec<PeerId> = session
                        .connected_peers()
                        .into_iter()
                        .filter(|p| session.is_dm_contact(*p) && swarm.is_connected(p))
                        .collect();
                    if !connected_now.is_empty() {
                        run_delivery_ack_upkeep(
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
            call_pair = call_incoming.next() => {
                if let Some((peer, stream)) = call_pair {
                    native_log::info("call_media", format!("inbound media stream from {peer}"));
                    tokio::spawn(handle_inbound_call_stream(peer, stream, Arc::clone(&session)));
                }
            }
            call_video_pair = call_video_incoming.next() => {
                if let Some((peer, stream)) = call_video_pair {
                    native_log::info("call_video", format!("inbound video stream from {peer}"));
                    tokio::spawn(handle_inbound_call_video_stream(peer, stream, Arc::clone(&session)));
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
                        // Coord relay bootstrap only — not a chat contact.
                    } else if session.is_dm_contact(*peer_id) {
                        let pid = *peer_id;
                        native_log::info("swarm", format!("dm peer connected {pid}"));
                        session.note_connected(pid);
                        if let Some(pk) = secp256k1_public_key_hex_from_peer_id(&pid) {
                            session.clear_dm_reconnect_urgent(&pk);
                        }
                        session.clear_relay_circuit_dial_backoff(pid);
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
                            // Do not tear down an active call here — brief relay/direct churn
                            // and coord blips recover in seconds; hangup signals end calls.
                            let _ = events_tx.send(GossipChatEvent::PeerDisconnected(*peer_id));
                            // Reconnect is urgent only when no other connection remains (a DM peer
                            // can be reached via relay + a DCUtR direct link at the same time).
                            if !swarm.is_connected(peer_id) {
                                if let Some(pk) = secp256k1_public_key_hex_from_peer_id(peer_id) {
                                    // The peer was just here: next coord lookup skips the
                                    // peer_not_on_server backoff and upkeep retries every ~1s.
                                    session.refresh_dm_reconnect_urgent(&pk);
                                    notify_coord_lookup();
                                }
                            }
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
                if take_coord_lookup_notify() {
                    coord_lookup_dm_peers(&mut swarm, session.as_ref()).await;
                }
                if take_stream_reopen_notify() {
                    spawn_reopen_dm_chat_streams(
                        &swarm,
                        Arc::clone(&session),
                        Arc::clone(&writers),
                        control.clone(),
                        Some(events_tx.clone()),
                    );
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
            }
        }
    }
}
