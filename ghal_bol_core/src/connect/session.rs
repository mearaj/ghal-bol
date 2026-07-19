//! Slim session state for native connect (identity-wire keyed).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, RwLock};

use super::types::{
    DmPeer, PendingCallSignal, PendingDeliveryAck, PendingOutbound, PendingReadAck, SessionPeer,
    SEEN_INBOUND_MAX, session_peer_from_identity_wire,
};

/// Same KDF as [`SessionState`] local DM transport secret — peer pk is deterministic
/// from their identity wire (hello still exchanged for confirmation / future ephemerals).
pub(crate) fn dm_transport_sk_for_identity_wire(identity_wire: &str) -> x25519_dalek::StaticSecret {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"ghal_bol_connect_v1/dm_transport_sk");
    h.update(identity_wire.as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    x25519_dalek::StaticSecret::from(digest)
}

pub(crate) fn dm_transport_pk_for_identity_wire(identity_wire: &str) -> [u8; 32] {
    let sk = dm_transport_sk_for_identity_wire(identity_wire);
    *x25519_dalek::PublicKey::from(&sk).as_bytes()
}

pub(crate) struct SessionState {
    pub(crate) identity: crate::DecryptedIdentity,
    peers: RwLock<HashMap<SessionPeer, DmPeer>>,
    connected: RwLock<HashSet<SessionPeer>>,
    stream_ready: RwLock<HashSet<SessionPeer>>,
    chat_ready_emitted: RwLock<HashSet<SessionPeer>>,
    identified_emitted: RwLock<HashSet<SessionPeer>>,
    outbox: RwLock<HashMap<String, PendingOutbound>>,
    outbound_ack_pending_poll: RwLock<HashSet<String>>,
    seen_inbound_ids: RwLock<HashMap<String, i64>>,
    pending_read_acks: RwLock<VecDeque<PendingReadAck>>,
    pending_delivery_acks: RwLock<VecDeque<PendingDeliveryAck>>,
    pending_call_signals: RwLock<VecDeque<PendingCallSignal>>,
    read_ack_confirmed: RwLock<HashSet<String>>,
    delivery_ack_sent: RwLock<HashSet<String>>,
    foreground_peer: RwLock<Option<SessionPeer>>,
    pub(crate) app_namespace: Option<String>,
    dm_transport_local_sk: x25519_dalek::StaticSecret,
    dm_peer_transport_pks: RwLock<HashMap<String, [u8; 32]>>,
    dm_transport_hello_sent: RwLock<HashSet<String>>,
    call_media: Mutex<HashMap<String, CallMediaEntry>>,
    call_video: Mutex<HashMap<String, CallVideoEntry>>,
    peers_on_local_lan: RwLock<HashSet<SessionPeer>>,
}

struct CallMediaEntry {
    peer: SessionPeer,
    controls: crate::call_media::MediaControls,
    wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

struct CallVideoEntry {
    peer: SessionPeer,
    controls: crate::call_video::VideoControls,
    wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

impl SessionState {
    pub fn new(
        identity: crate::DecryptedIdentity,
        dm_peers: &[DmPeer],
        app_namespace: Option<String>,
    ) -> Result<Self, String> {
        let sk = dm_transport_sk_for_identity_wire(&identity.identity_wire());
        let mut peers = HashMap::new();
        for p in dm_peers {
            if let Ok(w) = session_peer_from_identity_wire(&p.identity_wire) {
                peers.insert(w.clone(), DmPeer { identity_wire: w });
            }
        }
        Ok(Self {
            identity,
            peers: RwLock::new(peers),
            connected: RwLock::new(HashSet::new()),
            stream_ready: RwLock::new(HashSet::new()),
            chat_ready_emitted: RwLock::new(HashSet::new()),
            identified_emitted: RwLock::new(HashSet::new()),
            outbox: RwLock::new(HashMap::new()),
            outbound_ack_pending_poll: RwLock::new(HashSet::new()),
            seen_inbound_ids: RwLock::new(HashMap::new()),
            pending_read_acks: RwLock::new(VecDeque::new()),
            pending_delivery_acks: RwLock::new(VecDeque::new()),
            pending_call_signals: RwLock::new(VecDeque::new()),
            read_ack_confirmed: RwLock::new(HashSet::new()),
            delivery_ack_sent: RwLock::new(HashSet::new()),
            foreground_peer: RwLock::new(None),
            app_namespace,
            dm_transport_local_sk: sk,
            dm_peer_transport_pks: RwLock::new(HashMap::new()),
            dm_transport_hello_sent: RwLock::new(HashSet::new()),
            call_media: Mutex::new(HashMap::new()),
            call_video: Mutex::new(HashMap::new()),
            peers_on_local_lan: RwLock::new(HashSet::new()),
        })
    }

    pub fn register_dm_peer_key(&self, public_key_hex: &str) {
        if let Ok(w) = session_peer_from_identity_wire(public_key_hex) {
            if let Ok(mut g) = self.peers.write() {
                g.entry(w.clone()).or_insert(DmPeer { identity_wire: w });
            }
        }
    }

    pub fn resolve_send_peer(&self, pk: &str) -> Option<SessionPeer> {
        session_peer_from_identity_wire(pk).ok()
    }


    pub fn set_peer_connected(&self, peer: &SessionPeer, up: bool) {
        if let Ok(mut g) = self.connected.write() {
            if up {
                g.insert(peer.clone());
            } else {
                g.remove(peer);
            }
        }
    }

    pub fn set_stream_ready(&self, peer: &SessionPeer, ready: bool) {
        if let Ok(mut g) = self.stream_ready.write() {
            if ready {
                g.insert(peer.clone());
            } else {
                g.remove(peer);
            }
        }
    }



    pub fn set_peer_on_local_lan(&self, peer: &SessionPeer, on: bool) {
        if let Ok(mut g) = self.peers_on_local_lan.write() {
            if on {
                g.insert(peer.clone());
            } else {
                g.remove(peer);
            }
        }
    }


    pub fn set_foreground_peer(&self, peer: Option<SessionPeer>) {
        if let Ok(mut g) = self.foreground_peer.write() {
            *g = peer;
        }
    }


    pub fn track_outbound(&self, row: PendingOutbound) {
        if let Ok(mut g) = self.outbox.write() {
            g.insert(row.message_id.clone(), row);
        }
    }


    pub fn pending_outbox_for_peer(&self, peer: &SessionPeer) -> Vec<PendingOutbound> {
        self.outbox
            .read()
            .ok()
            .map(|g| {
                g.values()
                    .filter(|r| &r.peer == peer)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn enqueue_pending_call_signal(&self, sig: PendingCallSignal) {
        if let Ok(mut g) = self.pending_call_signals.write() {
            g.push_back(sig);
        }
    }


    pub fn call_media_stop(&self, call_id: &str) -> bool {
        self.call_media.lock().ok().map(|mut g| g.remove(call_id).is_some()).unwrap_or(false)
    }

    pub fn call_video_stop(&self, call_id: &str) -> bool {
        self.call_video.lock().ok().map(|mut g| g.remove(call_id).is_some()).unwrap_or(false)
    }

    pub fn call_media_set_mic_muted(&self, call_id: &str, muted: bool) -> bool {
        self.call_media
            .lock()
            .ok()
            .and_then(|g| {
                g.get(call_id).map(|e| {
                    e.controls.set_mic_muted(muted);
                    true
                })
            })
            .unwrap_or(false)
    }

    pub fn call_video_set_camera_off(&self, call_id: &str, off: bool) -> bool {
        self.call_video
            .lock()
            .ok()
            .and_then(|g| g.get(call_id).map(|e| {
                e.controls.set_camera_off(off);
                true
            }))
            .unwrap_or(false)
    }

    pub fn call_media_active(&self, call_id: &str) -> bool {
        self.call_media.lock().ok().is_some_and(|g| g.contains_key(call_id))
    }

    pub fn dm_local_transport_sk(&self) -> &x25519_dalek::StaticSecret {
        &self.dm_transport_local_sk
    }

    pub fn store_peer_transport_pk(&self, peer_identity_wire: &str, pk: [u8; 32]) {
        if let Ok(wire) = session_peer_from_identity_wire(peer_identity_wire) {
            if let Ok(mut g) = self.dm_peer_transport_pks.write() {
                g.insert(wire, pk);
            }
        }
    }

    /// Peer X25519 transport public key for sealing.
    ///
    /// Prefers a key learned from `TransportKemHello`. Falls back to the same
    /// identity-wire KDF used for the local transport secret — hello can be
    /// one-sided on a fresh bridge (initiator never sees responder hello) and
    /// call invites must not sit deferred forever.
    pub fn peer_transport_pk(&self, peer_identity_wire: &str) -> Option<[u8; 32]> {
        let wire = session_peer_from_identity_wire(peer_identity_wire).ok()?;
        if let Some(pk) = self
            .dm_peer_transport_pks
            .read()
            .ok()
            .and_then(|g| g.get(&wire).copied())
        {
            return Some(pk);
        }
        Some(dm_transport_pk_for_identity_wire(&wire))
    }

    pub(crate) fn clear_transport_kem_for_peer(&self, peer: &SessionPeer) {
        if let Ok(mut g) = self.dm_peer_transport_pks.write() {
            g.remove(peer);
        }
        if let Ok(mut g) = self.dm_transport_hello_sent.write() {
            g.remove(peer);
        }
    }

    pub fn mark_seen_inbound(&self, id: &str, now_ms: i64) -> bool {
        if let Ok(mut g) = self.seen_inbound_ids.write() {
            if g.contains_key(id) {
                return false;
            }
            g.insert(id.to_string(), now_ms);
            if g.len() > SEEN_INBOUND_MAX {
                if let Some((old, _)) = g.iter().min_by_key(|(_, t)| *t).map(|(k, v)| (k.clone(), *v)) {
                    g.remove(&old);
                }
            }
            return true;
        }
        false
    }

    pub fn emit_chat_ready_once(&self, peer: &SessionPeer) -> bool {
        self.chat_ready_emitted
            .write()
            .ok()
            .is_some_and(|mut g| g.insert(peer.clone()))
    }

    pub fn emit_identified_once(&self, peer: &SessionPeer) -> bool {
        self.identified_emitted
            .write()
            .ok()
            .is_some_and(|mut g| g.insert(peer.clone()))
    }

    pub(crate) fn transport_hello_already_sent(&self, peer: &SessionPeer) -> bool {
        self.dm_transport_hello_sent
            .read()
            .ok()
            .is_some_and(|g| g.contains(peer))
    }

    pub(crate) fn mark_transport_hello_sent(&self, peer: &SessionPeer) {
        if let Ok(mut g) = self.dm_transport_hello_sent.write() {
            g.insert(peer.clone());
        }
    }

    pub(crate) fn finalize_outbound_ack(&self, message_id: &str) {
        let id = message_id.trim();
        if id.is_empty() {
            return;
        }
        if let Ok(mut g) = self.outbox.write() {
            g.remove(id);
        }
        if let Ok(mut g) = self.outbound_ack_pending_poll.write() {
            g.insert(id.to_string());
        }
    }

    pub(crate) fn mark_outbox_sent(&self, message_id: &str, now_ms: i64) -> bool {
        let Ok(mut g) = self.outbox.write() else {
            return false;
        };
        if let Some(p) = g.get_mut(message_id) {
            let first = !p.on_wire;
            p.on_wire = true;
            if first {
                p.first_on_wire_ms = now_ms;
            }
            p.last_send_ms = now_ms;
            return first;
        }
        false
    }

    pub(crate) fn mark_outbox_send_failed(&self, message_id: &str, now_ms: i64) {
        if let Ok(mut g) = self.outbox.write() {
            if let Some(p) = g.get_mut(message_id) {
                p.on_wire = false;
                p.first_on_wire_ms = 0;
                p.last_send_ms = now_ms;
            }
        }
    }

    pub(crate) fn outbox_due_for_resend(&self, now_ms: i64) -> Vec<PendingOutbound> {
        self.outbox
            .read()
            .ok()
            .map(|g| {
                g.values()
                    .filter(|p| {
                        !p.on_wire
                            || now_ms.saturating_sub(p.last_send_ms) >= super::types::OUTBOX_RESEND_INTERVAL_MS
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn is_delivery_ack_sent(&self, inbound_id: &str) -> bool {
        self.delivery_ack_sent
            .read()
            .ok()
            .is_some_and(|g| g.contains(inbound_id))
    }

    pub(crate) fn mark_delivery_ack_sent(&self, inbound_id: &str) {
        if let Ok(mut g) = self.delivery_ack_sent.write() {
            g.insert(inbound_id.to_string());
        }
    }

    pub(crate) fn clear_delivery_ack_sent(&self, inbound_id: &str) {
        if let Ok(mut g) = self.delivery_ack_sent.write() {
            g.remove(inbound_id);
        }
    }

    pub(crate) fn enqueue_delivery_ack(
        &self,
        peer: &SessionPeer,
        inbound_id: &str,
        recipient_signing: &str,
        received_at_ms: i64,
    ) {
        let id = inbound_id.trim().to_string();
        if id.is_empty() {
            return;
        }
        if let Ok(mut q) = self.pending_delivery_acks.write() {
            if q.iter().any(|p| p.inbound_id == id) {
                return;
            }
            if q.len() >= 512 {
                q.pop_front();
            }
            q.push_back(PendingDeliveryAck {
                peer: peer.clone(),
                inbound_id: id,
                recipient_public_key_hex: recipient_signing.trim().to_string(),
                received_at_ms,
            });
        }
    }

    pub(crate) fn dequeue_delivery_ack(&self, inbound_id: &str) {
        if let Ok(mut q) = self.pending_delivery_acks.write() {
            q.retain(|p| p.inbound_id != inbound_id);
        }
    }

    pub(crate) fn delivery_acks_due_for_upkeep(&self, limit: usize) -> Vec<PendingDeliveryAck> {
        self.pending_delivery_acks
            .read()
            .ok()
            .map(|q| q.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn has_pending_read_ack(&self, inbound_id: &str) -> bool {
        self.pending_read_acks
            .read()
            .ok()
            .is_some_and(|q| q.iter().any(|p| p.inbound_id == inbound_id))
    }

    pub(crate) fn is_read_ack_confirmed(&self, inbound_id: &str) -> bool {
        self.read_ack_confirmed
            .read()
            .ok()
            .is_some_and(|g| g.contains(inbound_id))
    }

    pub(crate) fn mark_read_ack_confirmed(&self, inbound_id: &str) {
        if let Ok(mut g) = self.read_ack_confirmed.write() {
            g.insert(inbound_id.to_string());
        }
        if let Ok(mut q) = self.pending_read_acks.write() {
            q.retain(|p| p.inbound_id != inbound_id);
        }
    }

    pub(crate) fn try_claim_read_ack_wire_send(&self, peer: &SessionPeer, inbound_id: &str) -> bool {
        let id = inbound_id.trim();
        if id.is_empty() {
            return false;
        }
        if self.is_read_ack_confirmed(id) {
            return false;
        }
        let Ok(mut q) = self.pending_read_acks.write() else {
            return false;
        };
        if q.iter().any(|p| p.inbound_id == id) {
            return true;
        }
        if q.len() >= 512 {
            q.pop_front();
        }
        q.push_back(PendingReadAck {
            peer: peer.clone(),
            inbound_id: id.to_string(),
            recipient_public_key_hex: peer.clone(),
            last_send_ms: 0,
        });
        true
    }

    pub(crate) fn release_read_ack_wire_claim(&self, _inbound_id: &str) {}

    pub(crate) fn mark_read_ack_wire_sent(&self, inbound_id: &str) {
        if let Ok(mut q) = self.pending_read_acks.write() {
            for item in q.iter_mut() {
                if item.inbound_id == inbound_id {
                    item.last_send_ms = chrono_now_ms();
                }
            }
        }
    }

    pub(crate) fn enqueue_read_ack_backlog(&self, peer: &SessionPeer, inbound_id: &str) -> bool {
        let id = inbound_id.trim().to_string();
        if id.is_empty() {
            return false;
        }
        if let Ok(mut s) = self.read_ack_confirmed.write() {
            s.remove(&id);
        }
        let Ok(mut q) = self.pending_read_acks.write() else {
            return false;
        };
        if q.iter().any(|p| p.inbound_id == id) {
            return false;
        }
        if q.len() >= 512 {
            q.pop_front();
        }
        q.push_back(PendingReadAck {
            peer: peer.clone(),
            inbound_id: id,
            recipient_public_key_hex: peer.clone(),
            last_send_ms: 0,
        });
        true
    }

    pub(crate) fn has_pending_read_acks_for(&self, peer: &SessionPeer) -> bool {
        self.pending_read_acks
            .read()
            .ok()
            .is_some_and(|q| q.iter().any(|p| &p.peer == peer))
    }

    pub(crate) fn pending_read_ack_len(&self) -> usize {
        self.pending_read_acks.read().ok().map(|q| q.len()).unwrap_or(0)
    }

    pub(crate) fn read_acks_due_for_upkeep(&self, limit: usize) -> Vec<PendingReadAck> {
        self.pending_read_acks
            .read()
            .ok()
            .map(|q| q.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn drain_pending_call_signals(&self, limit: usize) -> Vec<PendingCallSignal> {
        self.pending_call_signals
            .write()
            .ok()
            .map(|mut q| {
                let n = limit.min(q.len());
                q.drain(0..n).collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn requeue_pending_call_signal_front(&self, item: PendingCallSignal) {
        if let Ok(mut q) = self.pending_call_signals.write() {
            if q.len() < 128 {
                q.push_front(item);
            }
        }
    }

    pub(crate) fn call_media_register(
        &self,
        call_id: String,
        peer: SessionPeer,
        controls: crate::call_media::MediaControls,
        wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        if let Ok(mut m) = self.call_media.lock() {
            m.insert(
                call_id,
                CallMediaEntry {
                    peer,
                    controls,
                    wire_in_tx,
                },
            );
        }
    }

    pub(crate) fn call_media_wire_in_any(
        &self,
        peer: &SessionPeer,
    ) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.call_media.lock().ok().and_then(|g| {
            g.values()
                .find(|e| &e.peer == peer)
                .map(|e| e.wire_in_tx.clone())
        })
    }

    pub(crate) fn call_video_register(
        &self,
        call_id: String,
        peer: SessionPeer,
        controls: crate::call_video::VideoControls,
        wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        if let Ok(mut m) = self.call_video.lock() {
            m.insert(
                call_id,
                CallVideoEntry {
                    peer,
                    controls,
                    wire_in_tx,
                },
            );
        }
    }

    pub(crate) fn call_video_active(&self, call_id: &str) -> bool {
        self.call_video.lock().ok().is_some_and(|g| g.contains_key(call_id))
    }

    pub(crate) fn call_video_wire_in_any(
        &self,
        peer: &SessionPeer,
    ) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.call_video.lock().ok().and_then(|g| {
            g.values()
                .find(|e| &e.peer == peer)
                .map(|e| e.wire_in_tx.clone())
        })
    }
}

pub(crate) fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
