//! In-process native DM worker (shared by FFI and the Unix-socket daemon).

pub mod call_active;
pub mod connectivity_diag;
pub mod native_log;
pub mod network_transport;

pub use crate::connect::{
    bump_foreground_peer_cmd_gen, chat_room_session_at_ms, contact_has_lan_p2p_text_path,
    freeze_open_chat_room_session, last_room_peer, libp2p_peer_for_contact_identity,
    live_foreground_peer_for_catchup, live_foreground_peer_pk, may_send_read_ack_for_contact_pk,
    notify_dm_presence_wake, notify_network_change, notify_relay_refresh,
    queue_read_ack_catchup, run_gossip_chat_node_with_std_io, set_app_ack_read_enabled,
    set_app_ui_visible, set_drop_pending_call_invite_hook, sync_foreground_peer_now,
    ChatServerError, ConnectConfig, ConnectError, DEFAULT_GOSSIP_TOPIC, DmPeer,
    GossipChatConfig, GossipChatEvent, OutboundCmd, SessionPeer, new_msg_id_for_ffi,
};
