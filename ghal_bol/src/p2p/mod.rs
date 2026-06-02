//! libp2p networking (**native targets only** — see crate root `cfg(not(wasm))`).
//!
//! **Direct messages** on **`/ghal-bol/msg/1.0.0`** streams (signed **`ghal_bol_msg_v1`** envelopes).
//! Transport: **QUIC/TCP**, **relay**, **mDNS**, **Kademlia DHT**, plus coord lookup and bootstrap addrs.

pub mod dht_bootstrap;
pub mod chat_server;
pub mod native_log;

pub use chat_server::{
    last_room_peer, live_foreground_peer_for_catchup, notify_network_change,
    queue_read_ack_catchup, run_gossip_chat_node_with_std_io, set_app_ack_read_enabled,
    sync_foreground_peer_now, ChatServerError, DmPeer, GossipChatConfig, GossipChatEvent,
    OutboundCmd, DEFAULT_GOSSIP_TOPIC,
};
