//! Dial address helpers for coord / invites (libp2p uses `Multiaddr` on the wire).

pub mod addr;
pub mod contact_pk;

pub use addr::{DmDialAddr, coord_endpoints_to_dial_addrs};
pub use contact_pk::{ContactPk, normalize_contact_pk};
