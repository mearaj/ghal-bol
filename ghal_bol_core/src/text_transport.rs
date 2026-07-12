//! Text chat transport policy.
//!
//! - **WAN text** — [`ghal_bol_delivery`] when `GHAL_BOL_DELIVERY_URL` is set (E2E ciphertext only).
//! - **LAN text** — libp2p mDNS/direct TCP (`/ghal-bol/msg/1.0.0`) when both peers are on LAN.
//! - **Voice/video calls** — libp2p on LAN and WAN (coord + relay); unchanged.

/// WAN / offline-capable text uses the delivery mailbox (not libp2p relay outbox).
pub fn wan_text_via_delivery_server() -> bool {
    crate::delivery_runtime::delivery_mode_enabled()
}

/// Legacy full P2P text on coord/relay WAN paths (removed when delivery URL is set).
pub fn p2p_wan_text_enabled() -> bool {
    !wan_text_via_delivery_server()
}

/// LAN direct libp2p text remains available (mDNS discovery + DM stream).
pub fn p2p_lan_text_enabled() -> bool {
    true
}
