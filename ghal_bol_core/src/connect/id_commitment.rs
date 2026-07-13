//! LAN privacy: mDNS advertises a commitment hash, not the full identity wire.

use sha2::{Digest, Sha256};

use crate::public_key_util::normalize_contact_identity_wire;

const IDC_DOMAIN: &[u8] = b"ghal_bol_connect_v1/idc";

/// 32-char hex commitment for normalized `identity_wire`.
pub fn identity_commitment_hex(identity_wire: &str) -> Result<String, String> {
    let norm = normalize_contact_identity_wire(identity_wire)?;
    let mut h = Sha256::new();
    h.update(IDC_DOMAIN);
    h.update(norm.as_bytes());
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commitment_stable() {
        let wire = "0220899663decabbb1b9f19c2e7baa610e123badd98cfe6e43484f941c45a36d0c";
        let a = identity_commitment_hex(wire).unwrap();
        let b = identity_commitment_hex(wire).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
