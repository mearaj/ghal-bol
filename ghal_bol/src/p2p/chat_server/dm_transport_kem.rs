// DM transport KEM state on SessionState + hello issuance on stream open.

use libp2p::identity::PeerId;
use sha2::{Digest, Sha256};

impl SessionState {
    pub(crate) fn dm_local_transport_sk(&self) -> &x25519_dalek::StaticSecret {
        &self.dm_transport_local_sk
    }

    pub(crate) fn dm_transport_local_seed_bytes(&self) -> [u8; 32] {
        self.dm_transport_local_sk.to_bytes()
    }

    pub(crate) fn peer_transport_pk(&self, peer_identity_wire: &str) -> Option<[u8; 32]> {
        let wire =
            crate::public_key_util::normalize_contact_identity_wire(peer_identity_wire).ok()?;
        self.dm_peer_transport_pks
            .read()
            .ok()?
            .get(&wire)
            .copied()
    }

    pub(crate) fn store_peer_transport_pk(&self, peer_identity_wire: &str, pk: [u8; 32]) {
        let Ok(wire) = crate::public_key_util::normalize_contact_identity_wire(peer_identity_wire)
        else {
            return;
        };
        if let Ok(mut g) = self.dm_peer_transport_pks.write() {
            g.insert(wire, pk);
        }
    }

    fn transport_hello_already_sent(&self, peer_identity_wire: &str) -> bool {
        let Ok(wire) = crate::public_key_util::normalize_contact_identity_wire(peer_identity_wire)
        else {
            return true;
        };
        self.dm_transport_hello_sent
            .read()
            .ok()
            .is_some_and(|g| g.contains(&wire))
    }

    fn mark_transport_hello_sent(&self, peer_identity_wire: &str) {
        let Ok(wire) = crate::public_key_util::normalize_contact_identity_wire(peer_identity_wire)
        else {
            return;
        };
        if let Ok(mut g) = self.dm_transport_hello_sent.write() {
            g.insert(wire);
        }
    }
}

/// Best-effort `TransportKemHello` once per contact when the DM stream is ready.
pub(crate) fn maybe_send_transport_kem_hello(
    session: &SessionState,
    peer: PeerId,
    writers: &StreamWriters,
) {
    let Some(recipient_pk) = session.signing_pk_for_libp2p_peer(peer) else {
        return;
    };
    if session.transport_hello_already_sent(&recipient_pk) {
        return;
    }
    let digest = Sha256::digest(recipient_pk.as_bytes());
    let id = format!("tkem-{}", hex::encode(&digest[..8]));
    let env = match build_transport_kem_hello_envelope(
        &id,
        &session.identity,
        &recipient_pk,
        session.dm_local_transport_sk(),
        chrono_now_ms(),
    ) {
        Ok(e) => e,
        Err(e) => {
            native_log::debug("stream", format!("transport kem hello build failed: {e}"));
            return;
        }
    };
    let frame = match envelope_to_frame_bytes(&env) {
        Ok(f) => f,
        Err(e) => {
            native_log::debug("stream", format!("transport kem hello frame: {e}"));
            return;
        }
    };
    match send_frame_on_open_stream(peer, frame, writers) {
        Ok(()) => {
            session.mark_transport_hello_sent(&recipient_pk);
            native_log::debug("stream", format!("transport kem hello sent to {peer}"));
        }
        Err(e) => {
            native_log::debug("stream", format!("transport kem hello deferred: {e}"));
        }
    }
}
