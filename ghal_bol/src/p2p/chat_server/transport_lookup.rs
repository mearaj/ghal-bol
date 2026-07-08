// Active P2P session transport KEM lookup (FFI + in-process helpers).

use std::sync::{Arc, OnceLock, RwLock, Weak};
use x25519_dalek::StaticSecret;

static ACTIVE_SESSION: OnceLock<RwLock<Weak<SessionState>>> = OnceLock::new();

fn active_mx() -> &'static RwLock<Weak<SessionState>> {
    ACTIVE_SESSION.get_or_init(|| RwLock::new(Weak::new()))
}

pub(crate) fn register_active_session(session: &Arc<SessionState>) {
    if let Ok(mut g) = active_mx().write() {
        *g = Arc::downgrade(session);
    }
}

pub(crate) fn unregister_active_session() {
    if let Ok(mut g) = active_mx().write() {
        *g = Weak::new();
    }
}

pub(crate) struct ActiveSessionGuard;

impl Drop for ActiveSessionGuard {
    fn drop(&mut self) {
        unregister_active_session();
    }
}

pub(crate) fn active_session_guard() -> ActiveSessionGuard {
    ActiveSessionGuard
}

/// Local transport secret + peer transport public key for a contact identity wire.
pub fn transport_kem_for_peer(
    peer_identity_wire: &str,
) -> Option<(StaticSecret, [u8; 32])> {
    let wire =
        crate::public_key_util::normalize_contact_identity_wire(peer_identity_wire).ok()?;
    let session = active_mx().read().ok()?.upgrade()?;
    let peer_pk = session.peer_transport_pk(&wire)?;
    let seed = session.dm_transport_local_seed_bytes();
    Some((StaticSecret::from(seed), peer_pk))
}

/// Resolve libp2p `PeerId` for a contact identity wire (embeddable algos, or registered ml-dsa row).
pub fn libp2p_peer_for_contact_identity(identity_wire: &str) -> Option<PeerId> {
    let wire = crate::public_key_util::normalize_contact_identity_wire(identity_wire).ok()?;
    if let Ok(peer) = crate::peer_id_util::peer_id_from_identity_wire(&wire) {
        return Some(peer);
    }
    active_mx()
        .read()
        .ok()?
        .upgrade()?
        .libp2p_peer_for_identity_wire(&wire)
}

/// Contact identity wire for a libp2p peer: inline decode, then active session roster.
pub fn identity_wire_for_libp2p_peer(peer: &PeerId) -> Option<String> {
    if let Some(wire) = crate::peer_id_util::identity_wire_from_peer_id(peer) {
        return Some(wire);
    }
    active_mx()
        .read()
        .ok()?
        .upgrade()?
        .signing_pk_for_libp2p_peer(*peer)
}
