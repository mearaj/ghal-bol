//! Native connect transport — mDNS + tokio TCP + Noise + channel mux.

pub mod bridge_client;
pub mod bridge_ws;
pub mod channel_mux;
pub mod chat_room_session;
pub mod frames;
pub mod id_commitment;
pub mod lan_discovery;
pub mod noise_session;
pub mod notify;
pub mod outbound;
pub mod outbox_acks;
pub mod peer_session;
pub mod prelude;
pub mod runtime;
pub mod session;
pub mod transport_kem;
pub mod types;
pub mod ui_session;
pub mod util;
pub mod worker;

pub use bridge_client::bridge_request;
pub use channel_mux::{ChannelMuxReader, ChannelMuxWriter, MUX_HEADER_LEN};
pub use chat_room_session::{
    chat_room_session_at_ms, freeze_open_chat_room_session,
};
pub use id_commitment::identity_commitment_hex;
pub use lan_discovery::{LanDiscovery, LanDiscoveryEvent};
pub use noise_session::{ConnectNoiseSession, NOISE_PATTERN};
pub use notify::{
    notify_dm_presence_wake, notify_network_change, notify_relay_refresh,
    set_android_wifi_transport_available, set_drop_pending_call_invite_hook,
};
pub use runtime::{connect_start, connect_stop, contact_has_lan_connect_path};
pub use types::{
    ConnectConfig, ConnectError, DmPeer, GossipChatConfig, GossipChatEvent, OutboundCmd,
    SessionPeer, ChatServerError, DEFAULT_GOSSIP_TOPIC, new_msg_id_for_ffi,
    contact_has_lan_connect_path as contact_has_lan_p2p_text_path,
    identity_wire_for_session_peer as identity_wire_for_libp2p_peer,
    libp2p_peer_for_contact_identity,
};
pub use ui_session::{
    app_ui_visible, bump_foreground_peer_cmd_gen, last_room_peer,
    live_foreground_peer_for_catchup, live_foreground_peer_pk,
    may_send_read_ack_for_contact_pk, queue_read_ack_catchup,
    set_app_ack_read_enabled, set_app_ui_visible, sync_foreground_peer_now,
};
pub use worker::run_connect_node_with_std_io;

pub use transport_kem::transport_kem_for_peer;
pub use types::ConnectEvent;
pub use types::ConnectOutboundCmd;

/// Alias for legacy callers — async entrypoint.
pub async fn run_gossip_chat_node_with_std_io(
    config: GossipChatConfig,
    identity: crate::DecryptedIdentity,
    outbound_rx: std::sync::mpsc::Receiver<OutboundCmd>,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), ChatServerError> {
    run_connect_node_with_std_io(config, identity, outbound_rx, events_tx, stop)
        .await
        .map_err(|e| ChatServerError::Other(e.to_string()))
}
