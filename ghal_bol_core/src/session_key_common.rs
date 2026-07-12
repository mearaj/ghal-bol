//! Shared HKDF helpers for transport KEM key expansion.

use hkdf::Hkdf;
use sha2::Sha256;

use crate::identity::Identity;

/// Sorted normalized identity wire strings so both peers expand identical HKDF `info`.
pub(crate) fn identity_pair_binding(local_wire: &str, peer_wire: &str) -> Result<Vec<u8>, String> {
    let mut a = Identity::parse(local_wire)?.to_wire();
    let mut b = Identity::parse(peer_wire)?.to_wire();
    if a == b {
        return Err("peer identity must differ from local identity".to_string());
    }
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    out.extend_from_slice(a.as_bytes());
    out.extend_from_slice(b.as_bytes());
    Ok(out)
}

pub(crate) fn hkdf_expand_32(ikm: &[u8], salt: &[u8], info: &[u8]) -> Result<[u8; 32], String> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut out = [0u8; 32];
    hk.expand(info, &mut out)
        .map_err(|_| "hkdf expand failed".to_string())?;
    Ok(out)
}
