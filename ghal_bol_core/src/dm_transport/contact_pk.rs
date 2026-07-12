//! DM contact identity wire — `[algorithm:]public_key_hex`. No libp2p PeerId.

pub use crate::public_key_util::normalize_contact_identity_wire;

/// Normalized identity wire string stored on contacts / roster.
pub type ContactPk = String;

/// Normalize to canonical identity wire.
pub fn normalize_contact_pk(s: &str) -> Result<ContactPk, String> {
    normalize_contact_identity_wire(s)
}
