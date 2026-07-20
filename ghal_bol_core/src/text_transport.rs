//! Text chat transport policy.
//!
//! **Parallel LAN + WAN invariant:** when `GHAL_BOL_DELIVERY_URL` is set, every outbound text
//! uploads to the delivery server first (offline guarantee). LAN connect is an **additive fast
//! mirror** only — it never bypasses the server. See `docs/GHAL_BOL_CONNECT_V1.md`.
//!
//! - **WAN text (primary)** — [`ghal_bol_delivery`] E2E mailbox when delivery URL is set.
//! - **LAN text (fast)** — native connect or libp2p mDNS/direct TCP when both peers are on LAN.
//! - **Voice/video calls** — native connect (target) / libp2p (legacy) on LAN and WAN.

/// Delivery server handles WAN / offline-capable text (mandatory when URL is set).
pub fn delivery_primary_text() -> bool {
    crate::delivery_runtime::delivery_mode_enabled()
}

/// Alias kept for existing call sites during migration.
pub fn wan_text_via_delivery_server() -> bool {
    delivery_primary_text()
}


/// LAN fast-path text mirror is enabled (additive; does not disable delivery worker).
pub fn lan_fast_path_enabled() -> bool {
    true
}


/// Whether LAN P2P read/delivery acks may mirror alongside delivery-server acks.
pub fn lan_p2p_ack_mirror_enabled(recipient_wire: &str) -> bool {
    lan_fast_path_enabled()
        && crate::p2p::contact_has_lan_p2p_text_path(recipient_wire)
}
