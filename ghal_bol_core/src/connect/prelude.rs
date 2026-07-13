//! Connect-layer prelude (no libp2p).

pub use std::collections::{HashMap, HashSet, VecDeque};
pub use std::path::Path;
pub use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
pub use std::sync::{Arc, Mutex, OnceLock, RwLock};
pub use std::time::Duration;

pub use thiserror::Error;
pub use tokio::select;
pub use tokio::sync::mpsc;
pub use tokio::time::{self, MissedTickBehavior};

pub use crate::call_sig_v1::{
    CALL_SHARE, CallSigKind, call_envelope_from_frame, frame_wire_share,
    parse_call_envelope_with_transport,
};
pub(crate) use crate::call_state;
pub(crate) use crate::call_state::call_invite_is_live;
pub(crate) use crate::contacts_v1::is_valid_public_key_hex;
pub use crate::msg_v1::{
    MSG_SHARE, MsgKind, ParsedMsg, build_ack_envelope, build_text_envelope,
    build_transport_kem_hello_envelope, envelope_to_frame_bytes, frame_bytes_to_envelope,
    parse_envelope_with_transport, DmOpenTransportCtx, DmSealTransportCtx,
};
pub use crate::p2p::native_log;

pub use super::types::SessionPeer;
