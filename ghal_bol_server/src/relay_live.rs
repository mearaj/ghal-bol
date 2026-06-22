//! Live relay reservation registry — authoritative gate for coord lookup.
//!
//! SQLite may retain a `/p2p-circuit` row after a reservation closes (happy-eyeballs hop
//! churn). `GET /v1/peers` must only return circuits while the relay still holds an
//! **accepted** reservation for that peer **now**.
//!
//! Bootstrap happy-eyeballs may emit `ReservationClosed` for a spare TCP hop while the
//! reservation remains live on another link — use a per-peer refcount so the live gate
//! does not flap to 404 during hop churn (TRANSPORT.md § "WAN coordination").

use crate::presence::{PeerEndpoint, PeerRecord};
use libp2p::PeerId;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct RelayLiveRegistry {
    inner: Arc<Mutex<RelayLiveInner>>,
}

#[derive(Default)]
struct RelayLiveInner {
    peer_keys: HashMap<PeerId, String>,
    pk_peers: HashMap<String, PeerId>,
    /// Live reservation slots — `accepted` mirrors `reservation_refs > 0`.
    reservation_refs: HashMap<PeerId, u32>,
    accepted: HashSet<PeerId>,
}

impl RelayLiveRegistry {
    pub fn note_peer_pk(&self, peer_id: PeerId, pk: String) {
        let pk = pk.to_ascii_lowercase();
        if let Ok(mut g) = self.inner.lock() {
            g.peer_keys.insert(peer_id, pk.clone());
            g.pk_peers.insert(pk, peer_id);
        }
    }

    pub fn pk_for_peer(&self, peer_id: PeerId) -> Option<String> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.peer_keys.get(&peer_id).cloned())
    }

    pub fn on_reservation_accepted(&self, peer_id: PeerId, renewed: bool) {
        if let Ok(mut g) = self.inner.lock() {
            if renewed {
                g.accepted.insert(peer_id);
            } else {
                let count = g.reservation_refs.entry(peer_id).or_insert(0);
                *count = count.saturating_add(1);
                g.accepted.insert(peer_id);
            }
        }
    }

    /// Bootstrap happy-eyeballs closed a spare TCP hop — drop live gate only when refcount hits zero.
    pub fn on_reservation_closed(&self, peer_id: PeerId) {
        if let Ok(mut g) = self.inner.lock() {
            if let Some(count) = g.reservation_refs.get_mut(&peer_id) {
                if *count > 0 {
                    *count -= 1;
                }
                if *count == 0 {
                    g.reservation_refs.remove(&peer_id);
                    g.accepted.remove(&peer_id);
                }
            } else {
                g.accepted.remove(&peer_id);
            }
        }
    }

    /// Reservation timed out or relay explicitly ended presence for this peer.
    pub fn on_reservation_end(&self, peer_id: PeerId) {
        if let Ok(mut g) = self.inner.lock() {
            g.reservation_refs.remove(&peer_id);
            g.accepted.remove(&peer_id);
            if let Some(pk) = g.peer_keys.remove(&peer_id) {
                g.pk_peers.remove(&pk);
            }
        }
    }

    pub fn is_peer_live(&self, peer_id: PeerId) -> bool {
        self.inner
            .lock()
            .ok()
            .is_some_and(|g| g.accepted.contains(&peer_id))
    }

    pub fn reservation_refcount(&self, peer_id: PeerId) -> u32 {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.reservation_refs.get(&peer_id).copied())
            .unwrap_or(0)
    }

    /// Snapshot of peers with a live reservation **and** a known public key, for the periodic
    /// presence keepalive. The relay grants hour-long reservations but coord presence rows expire
    /// after `presence_ttl` (90 s by default), so without re-touching these rows a relay-only
    /// (NAT'd) peer becomes a 404 on coord ~90 s after reserving even though it stays reachable for
    /// the full hour. See `RelayLoopCtx::refresh_live_presence` and TRANSPORT.md
    /// § "Relay presence keepalive".
    pub fn live_peers_with_pk(&self) -> Vec<(PeerId, String)> {
        let Ok(g) = self.inner.lock() else {
            return Vec::new();
        };
        g.accepted
            .iter()
            .filter_map(|peer| g.peer_keys.get(peer).map(|pk| (*peer, pk.clone())))
            .collect()
    }

    pub fn pk_has_live_relay_circuit(&self, pk: &str) -> bool {
        let pk = pk.to_ascii_lowercase();
        let Ok(g) = self.inner.lock() else {
            return false;
        };
        g.pk_peers
            .get(&pk)
            .is_some_and(|peer| g.accepted.contains(peer))
    }

    /// Strip stale relay circuits from a coord lookup response.
    pub fn apply_live_relay_gate(&self, record: &mut PeerRecord) {
        if self.pk_has_live_relay_circuit(&record.public_key_hex) {
            return;
        }
        record.endpoints.retain(|e| !is_relay_circuit_endpoint(e));
    }
}

fn is_relay_circuit_endpoint(ep: &PeerEndpoint) -> bool {
    ep.scheme == "libp2p" && ep.host.contains("/p2p-circuit")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_peer() -> PeerId {
        "16Uiu2HAm5zdGNzac9hYfCNQZTnANbxWytcMty9twy7u942fT7MCk"
            .parse()
            .unwrap()
    }

    fn sample_record(pk: &str) -> PeerRecord {
        PeerRecord {
            public_key_hex: pk.to_string(),
            endpoints: vec![PeerEndpoint {
                scheme: "libp2p".into(),
                host: "/ip4/1.2.3.4/tcp/1/p2p/12D3KooW/p2p-circuit/p2p/16Uiu".into(),
                port: 0,
            }],
            transport_capabilities: vec![],
            ipv6: None,
            ipv4: None,
            last_heartbeat_unix_ms: 0,
        }
    }

    #[test]
    fn stale_circuit_stripped_when_reservation_not_live() {
        let reg = RelayLiveRegistry::default();
        let pk = "aa".repeat(32);
        let peer = sample_peer();
        reg.note_peer_pk(peer, pk.clone());
        let mut rec = sample_record(&pk);
        reg.apply_live_relay_gate(&mut rec);
        assert!(rec.endpoints.is_empty());

        reg.on_reservation_accepted(peer, false);
        rec.endpoints.push(PeerEndpoint {
            scheme: "libp2p".into(),
            host: "/ip4/1.2.3.4/tcp/1/p2p/12D3KooW/p2p-circuit/p2p/16Uiu".into(),
            port: 0,
        });
        reg.apply_live_relay_gate(&mut rec);
        assert_eq!(rec.endpoints.len(), 1);
    }

    #[test]
    fn happy_eyeballs_close_spare_hop_keeps_live_gate() {
        let reg = RelayLiveRegistry::default();
        let peer = sample_peer();
        reg.on_reservation_accepted(peer, false);
        reg.on_reservation_accepted(peer, false);
        assert_eq!(reg.reservation_refcount(peer), 2);
        assert!(reg.is_peer_live(peer));

        reg.on_reservation_closed(peer);
        assert_eq!(reg.reservation_refcount(peer), 1);
        assert!(reg.is_peer_live(peer));

        reg.on_reservation_closed(peer);
        assert_eq!(reg.reservation_refcount(peer), 0);
        assert!(!reg.is_peer_live(peer));
    }

    #[test]
    fn renewed_accept_does_not_bump_refcount() {
        let reg = RelayLiveRegistry::default();
        let peer = sample_peer();
        reg.on_reservation_accepted(peer, false);
        reg.on_reservation_accepted(peer, true);
        assert_eq!(reg.reservation_refcount(peer), 1);
        assert!(reg.is_peer_live(peer));
    }

    #[test]
    fn reservation_end_clears_all_refs() {
        let reg = RelayLiveRegistry::default();
        let peer = sample_peer();
        reg.on_reservation_accepted(peer, false);
        reg.on_reservation_accepted(peer, false);
        reg.on_reservation_end(peer);
        assert!(!reg.is_peer_live(peer));
        assert_eq!(reg.reservation_refcount(peer), 0);
    }

    #[test]
    fn live_peers_with_pk_lists_only_live_known_peers() {
        let reg = RelayLiveRegistry::default();
        let peer = sample_peer();
        let pk = "cc".repeat(33);
        // Reserved but pk not yet known via identify → not eligible for presence keepalive.
        reg.on_reservation_accepted(peer, false);
        assert!(reg.live_peers_with_pk().is_empty());
        // identify arrives → keepalive can re-touch this peer's coord presence row.
        reg.note_peer_pk(peer, pk.clone());
        let live = reg.live_peers_with_pk();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].0, peer);
        assert_eq!(live[0].1, pk.to_ascii_lowercase());
        // Reservation ends → drop from keepalive set so a gone peer is not kept artificially fresh.
        reg.on_reservation_end(peer);
        assert!(reg.live_peers_with_pk().is_empty());
    }
}
