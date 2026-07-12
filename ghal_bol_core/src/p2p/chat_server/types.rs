/// Outbound DM frame; optional oneshot fires `true` only after bytes hit the libp2p stream.
pub(crate) enum StreamWireItem {
    Frame {
        bytes: Vec<u8>,
        written: Option<tokio::sync::oneshot::Sender<bool>>,
    },
}

pub(crate) type StreamWriters =
    Arc<Mutex<HashMap<PeerId, mpsc::UnboundedSender<StreamWireItem>>>>;

pub const DEFAULT_GOSSIP_TOPIC: &str = "ghal-bol-chat";

#[derive(Clone, Debug)]
pub struct DmPeer {
    pub peer_id: PeerId,
    pub public_key_hex: Option<String>,
}

impl DmPeer {
    pub fn from_public_key_hex(public_key_hex: String) -> Result<Self, String> {
        let wire =
            crate::public_key_util::normalize_contact_identity_wire(public_key_hex.trim())?;
        let peer_id = crate::peer_id_util::peer_id_from_identity_wire(&wire)?;
        Ok(Self {
            peer_id,
            public_key_hex: Some(wire),
        })
    }

    /// Bind identity wire to a known libp2p peer (coord-resolved peers when needed).
    pub fn from_identity_wire_with_peer(identity_wire: String, peer_id: PeerId) -> Self {
        Self {
            peer_id,
            public_key_hex: Some(identity_wire),
        }
    }

    pub fn peer_id_only(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            public_key_hex: None,
        }
    }

    pub fn has_send_keys(&self) -> bool {
        self.public_key_hex
            .as_deref()
            .is_some_and(crate::contacts_v1::is_valid_public_key_hex)
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
        received_at_ms: Option<i64>,
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
        /// Contact identity wire — used to resolve transport PeerId.
        identity_wire: Option<String>,
        /// Monotonic id — drop stale close/open when Flutter/Daemon RPCs reorder on the outbound queue.
        generation: u64,
    },
    /// Hub enabled read receipts after foreground was already set.
    RunReadAckCatchup { identity_wire: String },
    /// Dial invite/coord bootstrap addrs on a running node.
    DialBootstrapPeers { addrs: Vec<Multiaddr> },
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
    CallMediaStop { call_id: String },
    /// Mute/unmute the local microphone for an active call (keeps the clock running).
    CallMediaSetMicMuted { call_id: String, muted: bool },
    /// Route playout to speakerphone vs earpiece (Android `:p2p`; `setSpeakerphoneOn` only).
    CallMediaSetSpeaker { call_id: String, speaker_on: bool },
    /// Start native video for an active call (opens the `/ghal-bol/call-video/1.0.0` substream).
    CallVideoStart {
        call_id: String,
        peer_public_key_hex: String,
        /// When true, captured frames are encoded and sent (camera on at start).
        camera_enabled: bool,
    },
    /// Tear down native video for a call.
    CallVideoStop { call_id: String },
    /// Turn the local camera on/off mid-call (keeps the session/transport up).
    CallVideoSetCameraEnabled { call_id: String, enabled: bool },
}

/// Outbound text kept until the peer’s **`ack_received`** or **`ack_read`** (read implies delivery).
#[derive(Clone)]
pub(crate) struct PendingOutbound {
    message_id: String,
    peer_id: PeerId,
    recipient_public_key_hex: String,
    text: String,
    created_at_ms: i64,
    last_send_ms: i64,
    /// First time this row hit the wire this session; resync must not reset stuck detection.
    first_on_wire_ms: i64,
    /// False until the frame is actually written to the chat stream.
    on_wire: bool,
}

/// DM upkeep ticker: resend unacked outbound about once per second.
pub(crate) const OUTBOX_RESEND_INTERVAL_MS: i64 = 1_000;
/// Linux `p2p_nudge_read_catchup` / keepalive — must not stack on ~1s upkeep retries.
pub(crate) const READ_ACK_CATCHUP_THROTTLE_MS: i64 = 8_000;
/// Routed-dial throttle when outbox is waiting on this peer (must stay below circuit in-flight).
pub(crate) const LAN_DIAL_THROTTLE_URGENT_MS: i64 = 8_000;
/// Urgent coord relay-circuit dials — matches ~1s upkeep + 2s coord lookup cadence (TRANSPORT.md § urgent reconnect).
pub(crate) const CIRCUIT_COORD_DIAL_URGENT_MS: i64 = 2_000;
/// Do not replace an outbound relay-circuit dial until this window elapses (libp2p oneshot cancel).
pub(crate) const CIRCUIT_DIAL_IN_FLIGHT_MS: i64 = 45_000;
/// Urgent/outbox peers — shorter guard so a hung relay hop does not block WAN for 45s after churn.
pub(crate) const CIRCUIT_DIAL_IN_FLIGHT_URGENT_MS: i64 = 12_000;
/// Guardrail: do not stack parallel LAN TCP dials to the same peer (mDNS event coalescing).
pub(crate) const LAN_DIAL_IN_FLIGHT_MS: i64 = 45_000;
/// libp2p may not mark a peer "dialing" for a few ms after `swarm.dial(Ok)` — hold LAN in-flight briefly.
pub(crate) const LAN_DIAL_PENDING_GRACE_MS: i64 = 2_000;
/// Read-receipt retries per upkeep tick (queued until peer confirms).
pub(crate) const READ_ACK_UPKEEP_MAX_OPS_PER_TICK: usize = 64;
pub(crate) const ACK_BURST_MAX_OPS_PER_PASS: usize = 64;
pub(crate) const ACK_BURST_MAX_ROUNDS: usize = 1;
pub(crate) const MAX_PENDING_READ_ACKS: usize = 16_384;
pub(crate) const HISTORY_REPLAY_SPACING_MS: u64 = 8;

#[derive(Clone)]
pub(crate) struct PendingReadAck {
    peer_id: PeerId,
    inbound_id: String,
    recipient_public_key_hex: String,
    /// 0 = send immediately; after wire success, upkeep waits `OUTBOX_RESEND_INTERVAL_MS`.
    last_send_ms: i64,
}

#[derive(Clone)]
pub(crate) struct PendingDeliveryAck {
    peer_id: PeerId,
    inbound_id: String,
    recipient_public_key_hex: String,
    /// When the recipient first accepted the inbound text (`ack_received.received_at_ms`).
    received_at_ms: i64,
    /// When this ack was first queued — used to tell a transient in-flight ack (healthy mux)
    /// from a sustained-stuck ack (dead direct mux after LAN→WAN handover).
    queued_at_ms: i64,
}

#[derive(Clone)]
pub(crate) struct PendingCallSignal {
    call_id: String,
    signal_id: String,
    signal_kind: CallSigKind,
    payload: serde_json::Value,
    peer_id: PeerId,
    recipient_public_key_hex: String,
    created_at_ms: i64,
}

/// Cap outbound work per swarm/poll tick (avoid unbounded drain in one select turn).
pub(crate) const MAX_OUTBOUND_CMDS_PER_TICK: usize = 64;
pub(crate) const SEEN_INBOUND_MAX: usize = 2_048;
