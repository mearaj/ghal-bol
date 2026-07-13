//! Connect-layer event and command types (identity-wire keyed).

use crate::call_sig_v1::CallSigKind;
use crate::msg_v1::MsgKind;
use serde_json::Value;
use thiserror::Error;

pub const CONNECT_MDNS_SERVICE: &str = "_ghalbol._tcp.local.";
pub const CONNECT_PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_GOSSIP_TOPIC: &str = "ghal-bol-chat";

pub type ConnectEvent = GossipChatEvent;

/// Remote peer key — normalized contact identity wire (no PeerId).
pub type SessionPeer = String;

pub fn session_peer_from_identity_wire(wire: &str) -> Result<SessionPeer, String> {
    crate::public_key_util::normalize_contact_identity_wire(wire)
}

#[derive(Clone, Debug)]
pub struct DmPeer {
    pub identity_wire: SessionPeer,
}

impl DmPeer {
    pub fn from_public_key_hex(public_key_hex: String) -> Result<Self, String> {
        Ok(Self {
            identity_wire: session_peer_from_identity_wire(public_key_hex.trim())?,
        })
    }

    pub fn has_send_keys(&self) -> bool {
        crate::contacts_v1::is_valid_public_key_hex(&self.identity_wire)
    }
}

#[derive(Clone, Debug)]
pub struct ConnectConfig {
    pub topic: String,
    pub dm_peers: Vec<DmPeer>,
    pub transcript_path: Option<String>,
    pub app_namespace: Option<String>,
}

pub type GossipChatConfig = ConnectConfig;

impl ConnectConfig {
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            dm_peers: Vec::new(),
            transcript_path: None,
            app_namespace: None,
        }
    }

    pub fn from_unlocked_identity(
        topic: impl Into<String>,
        _id: &crate::DecryptedIdentity,
    ) -> Result<Self, String> {
        Ok(Self::new(topic))
    }
}

#[derive(Clone, Debug)]
pub enum GossipChatEvent {
    Listening(String),
    PeerConnected(SessionPeer),
    PeerDisconnected(SessionPeer),
    DialFailed {
        peer: Option<SessionPeer>,
        error: String,
    },
    DmMessage {
        from: SessionPeer,
        id: String,
        msg_kind: String,
        text: Option<String>,
        ref_id: Option<String>,
        sender_public_key_hex: String,
        created_at_ms: i64,
        received_at_ms: Option<i64>,
    },
    PeerIdentified {
        peer_id: SessionPeer,
        public_key_hex: String,
    },
    ChatReady {
        peer_id: SessionPeer,
    },
    SendFailed {
        message_id: String,
        error: String,
    },
    OutboundSent {
        message_id: String,
    },
    NativeLog {
        level: String,
        tag: String,
        message: String,
    },
    CallSignal {
        from: SessionPeer,
        id: String,
        call_id: String,
        signal: String,
        sender_public_key_hex: String,
        created_at_ms: i64,
        payload: Value,
    },
    CallSignalSent {
        call_id: String,
        signal: String,
        recipient_public_key_hex: String,
    },
    CallMedia {
        call_id: String,
        peer_public_key_hex: String,
        state: String,
        camera_on: bool,
        remote_video_on: bool,
        reason: Option<String>,
    },
    NodeReady,
    NodeStopped {
        error: Option<String>,
    },
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type ChatServerError = ConnectError;

#[derive(Clone)]
pub enum OutboundCmd {
    SendText {
        recipient_public_key_hex: String,
        text: String,
        message_id: String,
        created_at_ms: i64,
        done: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    },
    SendAck {
        recipient_public_key_hex: String,
        ref_id: String,
        ack_kind: MsgKind,
    },
    RegisterDmPeer {
        public_key_hex: String,
    },
    SetForegroundPeer {
        identity_wire: Option<String>,
        generation: u64,
    },
    RunReadAckCatchup {
        identity_wire: String,
    },
    SendCallSignal {
        recipient_public_key_hex: String,
        call_id: String,
        signal_kind: CallSigKind,
        payload: Value,
        signal_id: String,
    },
    CallMediaStart {
        call_id: String,
        peer_public_key_hex: String,
    },
    CallMediaStop {
        call_id: String,
    },
    CallMediaSetMicMuted {
        call_id: String,
        muted: bool,
    },
    CallMediaSetSpeaker {
        call_id: String,
        speaker_on: bool,
    },
    CallVideoStart {
        call_id: String,
        peer_public_key_hex: String,
        camera_enabled: bool,
    },
    CallVideoStop {
        call_id: String,
    },
    CallVideoSetCameraEnabled {
        call_id: String,
        enabled: bool,
    },
}

pub(crate) const OUTBOX_RESEND_INTERVAL_MS: i64 = 1_000;
pub(crate) const READ_ACK_CATCHUP_THROTTLE_MS: i64 = 8_000;
pub(crate) const MAX_OUTBOUND_CMDS_PER_TICK: usize = 64;
pub(crate) const SEEN_INBOUND_MAX: usize = 2_048;

#[derive(Clone)]
pub(crate) struct PendingOutbound {
    pub(crate) message_id: String,
    pub(crate) peer: SessionPeer,
    pub(crate) recipient_public_key_hex: String,
    pub(crate) text: String,
    pub(crate) created_at_ms: i64,
    pub(crate) last_send_ms: i64,
    pub(crate) first_on_wire_ms: i64,
    pub(crate) on_wire: bool,
}

#[derive(Clone)]
pub(crate) struct PendingReadAck {
    pub(crate) peer: SessionPeer,
    pub(crate) inbound_id: String,
    pub(crate) recipient_public_key_hex: String,
    pub(crate) last_send_ms: i64,
}

#[derive(Clone)]
pub(crate) struct PendingDeliveryAck {
    pub(crate) peer: SessionPeer,
    pub(crate) inbound_id: String,
    pub(crate) recipient_public_key_hex: String,
    pub(crate) received_at_ms: i64,
    pub(crate) queued_at_ms: i64,
}

#[derive(Clone)]
pub(crate) struct PendingCallSignal {
    pub(crate) call_id: String,
    pub(crate) signal_id: String,
    pub(crate) signal_kind: CallSigKind,
    pub(crate) payload: Value,
    pub(crate) peer: SessionPeer,
    pub(crate) recipient_public_key_hex: String,
    pub(crate) created_at_ms: i64,
}

pub fn new_msg_id_for_ffi() -> String {
    format!("msg-{}", uuid_simple())
}

fn uuid_simple() -> String {
    use rand_core::{OsRng, RngCore};
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

pub fn contact_has_lan_connect_path(identity_wire: &str) -> bool {
    super::runtime::contact_has_lan_connect_path(identity_wire)
}

pub fn libp2p_peer_for_contact_identity(pk: &str) -> Option<SessionPeer> {
    session_peer_from_identity_wire(pk).ok()
}

pub fn identity_wire_for_session_peer(peer: &SessionPeer) -> Option<String> {
    Some(peer.clone())
}

/// Sidecar runtime commands (mDNS/TCP listener thread).
#[derive(Clone, Debug)]
pub enum ConnectOutboundCmd {
    RegisterContact { identity_wire: String },
    DialContact { identity_wire: String },
    Stop,
}
