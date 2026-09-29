//! Text chat transport policy.
//!
//! **Product decision:** when `GHAL_BOL_DELIVERY_URL` is set, **all product text** uses
//! [`ghal_bol_delivery`] (E2E mailbox). Voice/video calls use native connect (LAN or coord
//! bridge). Native connect is **not** the product chat path. See `docs/DESIGN.md`.

/// Delivery server handles product text (mandatory when URL is set).
pub fn delivery_primary_text() -> bool {
    crate::delivery_runtime::delivery_mode_enabled()
}

/// Alias kept for existing call sites.
pub fn wan_text_via_delivery_server() -> bool {
    delivery_primary_text()
}

/// Whether native-connect may still carry non-product mirror frames (implementation detail).
/// Product chat remains delivery when [`delivery_primary_text`] is true.
pub fn lan_fast_path_enabled() -> bool {
    true
}

/// Whether LAN P2P read/delivery acks may mirror alongside delivery-server acks.
pub fn lan_p2p_ack_mirror_enabled(recipient_wire: &str) -> bool {
    lan_fast_path_enabled() && crate::p2p::contact_has_lan_p2p_text_path(recipient_wire)
}
