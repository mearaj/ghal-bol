//! libp2p **direct-message** node: **QUIC/TCP**, **relay**, **mDNS**, and **`/ghal-bol/msg/1.0.0`** streams.
//!
//! **Text:** WAN uses [`ghal_bol_delivery`](crate::delivery_runtime) (E2E encrypted mailbox).
//! LAN text uses signed **`ghal_bol_msg_v1`** on libp2p streams (mDNS/direct TCP).
//! **Calls:** voice/video signaling + media on libp2p (WAN relay + LAN).
//!
//! Wire detail: [`docs/GHAL_BOL_DM_MSG_V1.md`](../../docs/GHAL_BOL_DM_MSG_V1.md) (LAN P2P text),
//! [`docs/GHAL_BOL_DELIVERY.md`](../../docs/GHAL_BOL_DELIVERY.md) (WAN text).

mod prelude;
mod util;
mod notify;
mod ui_session;
mod chat_room_session;

// Core logic is split across `include!` fragments (single module scope — no circular imports).
// Submodules: prelude, util, notify, ui_session.
pub(crate) use prelude::*;
pub(crate) use util::{
    chrono_now_ms, COORD_PEER_NOT_ON_COORD_LOG_MIN_MS, PRESENCE_WAKE_RUN_DEBOUNCE_MS,
};
pub(crate) use notify::{
    drop_pending_call_invite, notify_coord_lookup, notify_stream_reopen, take_coord_lookup_notify,
    take_dm_presence_wake_notify, take_network_change_notify, take_relay_refresh_notify,
    take_stream_reopen_notify, ANDROID_WIFI_TRANSPORT, LAN_RECOVERY_MIN_MS,
};
pub(crate) use chat_room_session::{
    begin_chat_room_session, clear_chat_room_session, freeze_chat_room_for_peer,
    read_ack_cutoff_ms, tick_chat_room_session_if_active,
};
pub(crate) use ui_session::{
    app_ack_read_enabled, app_ui_visible, emit_call_media, foreground_peer_cmd_gen_latest,
    inbound_suppresses_hub_unread, is_live_foreground_peer, last_room_peer_mx,
    live_foreground_peer, may_send_in_room_read_ack, on_local_call_signal_sent,
    platform_incoming_call_dismiss, platform_incoming_call_show, read_ack_catchup_throttled,
};

include!("types.rs");
include!("session.rs");
include!("dm_transport_kem.rs");
include!("transport_lookup.rs");
include!("outbox_wire.rs");
include!("behaviour.rs");
include!("bootstrap_relay.rs");
include!("frames.rs");
include!("dm_stream.rs");
include!("call_media.rs");
include!("outbound.rs");
include!("outbox_acks.rs");
include!("dm_dial.rs");
include!("swarm_events.rs");
include!("coord_lookup.rs");
include!("run_loop.rs");

pub use notify::{
    notify_dm_presence_wake, notify_network_change, notify_relay_refresh,
    set_android_wifi_transport_available, set_drop_pending_call_invite_hook,
};
pub use chat_room_session::{chat_room_session_at_ms, freeze_open_chat_room_session};
pub use ui_session::{
    bump_foreground_peer_cmd_gen, last_room_peer, live_foreground_peer_for_catchup,
    live_foreground_peer_pk, may_send_read_ack_for_contact_pk, queue_read_ack_catchup,
    set_app_ack_read_enabled, set_app_ui_visible, sync_foreground_peer_now,
};
