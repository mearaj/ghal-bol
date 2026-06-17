//! libp2p networking (**native targets only** — see crate root `cfg(not(wasm))`).
//!
//! **Direct messages** on **`/ghal-bol/msg/1.0.0`** streams (signed **`ghal_bol_msg_v1`** envelopes).
//! Transport: **QUIC/TCP**, **relay**, **mDNS**, plus coord lookup and invite bootstrap addrs.

pub mod call_active;
pub mod chat_server;
pub mod connectivity_diag;
pub mod native_log;
pub mod network_transport;

pub use chat_server::{
    ChatServerError, DEFAULT_GOSSIP_TOPIC, DmPeer, GossipChatConfig, GossipChatEvent, OutboundCmd,
    last_room_peer, live_foreground_peer_for_catchup, notify_dm_presence_wake,
    notify_network_change, notify_relay_refresh, queue_read_ack_catchup,
    run_gossip_chat_node_with_std_io, set_app_ack_read_enabled, set_app_ui_visible,
    set_drop_pending_call_invite_hook, sync_foreground_peer_now,
};
