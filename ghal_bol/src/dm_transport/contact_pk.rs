//! DM contact identity — secp256k1 public key hex (66 chars). No libp2p PeerId.

/// Normalized lowercase compressed secp256k1 public key (hex).
pub type ContactPk = String;

pub fn normalize_contact_pk(s: &str) -> Result<ContactPk, String> {
    let s = s.trim().to_ascii_lowercase();
    if s.len() != 66 {
        return Err("public_key_hex: expected 66 hex chars (compressed secp256k1)".into());
    }
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("public_key_hex: invalid hex".into());
    }
    Ok(s)
}
