//! libp2p networking (**native targets only** — see crate root `cfg(not(wasm))`).
//!
//! **LAN text** and **voice/video calls** (WAN + LAN). WAN text uses the delivery server
//! ([`text_transport`](../text_transport.rs)); see [`docs/GHAL_BOL_DELIVERY.md`](../../docs/GHAL_BOL_DELIVERY.md).

pub mod call_active;
pub mod chat_server;
pub mod connectivity_diag;
pub mod native_log;
pub mod network_transport;

pub use chat_server::{
    ChatServerError, DEFAULT_GOSSIP_TOPIC, DmPeer, GossipChatConfig, GossipChatEvent, OutboundCmd,
    last_room_peer, libp2p_peer_for_contact_identity, contact_has_lan_p2p_text_path,
    live_foreground_peer_for_catchup,
    live_foreground_peer_pk, chat_room_session_at_ms, may_send_read_ack_for_contact_pk,
    notify_dm_presence_wake, notify_network_change, notify_relay_refresh,
    queue_read_ack_catchup, run_gossip_chat_node_with_std_io, set_app_ack_read_enabled,
    set_app_ui_visible, set_drop_pending_call_invite_hook, sync_foreground_peer_now,
};
