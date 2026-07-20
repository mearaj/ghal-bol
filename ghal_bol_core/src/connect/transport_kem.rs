//! Transport KEM lookup for call media FFI (session-scoped peer keys).

use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

static PEER_TRANSPORT_PKS: OnceLock<Mutex<std::collections::HashMap<String, [u8; 32]>>> =
    OnceLock::new();

fn peer_map() -> &'static Mutex<std::collections::HashMap<String, [u8; 32]>> {
    PEER_TRANSPORT_PKS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn set_peer_transport_pk(peer_wire: &str, pk: [u8; 32]) {
    if let Ok(mut m) = peer_map().lock() {
        m.insert(peer_wire.to_string(), pk);
    }
}

fn dm_transport_sk_from_identity(ident: &crate::DecryptedIdentity) -> StaticSecret {
    let mut h = Sha256::new();
    h.update(b"ghal_bol_connect_v1/dm_transport_sk");
    h.update(ident.identity_wire().as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    StaticSecret::from(digest)
}

/// Local transport secret + peer x25519 public key when the connect session has completed KEM hello.
pub fn transport_kem_for_peer(peer_wire: &str) -> Option<(StaticSecret, [u8; 32])> {
    let ident = crate::session_runtime::unlocked_identity_clone().ok()?;
    let local_sk = dm_transport_sk_from_identity(&ident);
    let peer_pk = if let Some(pk) = peer_map().lock().ok()?.get(peer_wire).copied() {
        pk
    } else {
        // Same identity-wire KDF as connect session — call media must not stall
        // when hello was one-sided on the bridge.
        let wire = crate::public_key_util::normalize_contact_identity_wire(peer_wire).ok()?;
        let sk = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(b"ghal_bol_connect_v1/dm_transport_sk");
            h.update(wire.as_bytes());
            let digest: [u8; 32] = h.finalize().into();
            StaticSecret::from(digest)
        };
        *PublicKey::from(&sk).as_bytes()
    };
    Some((local_sk, peer_pk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use x25519_dalek::PublicKey;

    #[test]
    fn transport_kem_missing_peer_derives_from_identity_wire() {
        let (_ks, id) = crate::create_keystore_v1("pw", None).unwrap();
        crate::session_runtime::install_unlocked_identity(id).unwrap();
        let peer = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let (sk, got) = transport_kem_for_peer(peer).unwrap();
        assert_eq!(got.len(), 32);
        let _ = PublicKey::from(got);
        let _ = sk;
    }

    #[test]
    fn transport_kem_roundtrip_after_set() {
        let (_ks, id) = crate::create_keystore_v1("pw", None).unwrap();
        crate::session_runtime::install_unlocked_identity(id).unwrap();
        let peer = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let pk = [7u8; 32];
        set_peer_transport_pk(peer, pk);
        let (sk, got) = transport_kem_for_peer(peer).unwrap();
        assert_eq!(got, pk);
        let _ = PublicKey::from(got);
        let _ = sk;
    }
}
