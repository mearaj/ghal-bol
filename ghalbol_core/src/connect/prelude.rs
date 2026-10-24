//! Connect-layer prelude (no libp2p).

pub use std::collections::HashSet;
pub use std::time::Duration;

pub use tokio::sync::mpsc;

pub use crate::call_sig_v1::{
    CALL_SHARE, CallSigKind, call_envelope_from_frame, frame_wire_share,
    parse_call_envelope_with_transport,
};
pub(crate) use crate::call_state;
pub(crate) use crate::call_state::call_invite_is_live;
pub use crate::msg_v1::{
    DmOpenTransportCtx, DmSealTransportCtx, MSG_SHARE, MsgKind, ParsedMsg, build_ack_envelope,
    build_attachment_offer_envelope, build_availability_status_envelope, build_text_envelope,
    build_transport_kem_hello_envelope, build_voice_envelope, envelope_to_frame_bytes,
    frame_bytes_to_envelope, parse_envelope_with_transport,
};
pub use crate::p2p::native_log;
