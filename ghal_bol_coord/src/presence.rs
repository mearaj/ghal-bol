//! Peer presence and reachable endpoints — persisted in SQLite.

use crate::db;
use crate::error::{ApiResult, ServerError};
use crate::endpoint_expand;
use crate::identity::normalize_identity_wire;
use crate::relay_live::RelayLiveRegistry;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One transport endpoint the peer can be reached on (QUIC, TCP, etc.).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerEndpoint {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

/// Registered peer record returned to callers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerRecord {
    pub public_key_hex: String,
    pub endpoints: Vec<PeerEndpoint>,
    pub transport_capabilities: Vec<String>,
    pub ipv6: Option<String>,
    pub ipv4: Option<String>,
    pub last_heartbeat_unix_ms: u64,
}

/// SQLite-backed presence store (`Send` + `Sync` via mutex).
#[derive(Clone)]
pub struct PresenceStore {
    conn: Arc<Mutex<Connection>>,
    /// `host:port` from `GET /v1/relay` — never dialable or POSTable as peer TCP.
    relay_bootstrap_tcp: Arc<Mutex<HashSet<String>>>,
    /// Live relay reservations — gates `/p2p-circuit` on coord lookup.
    relay_live: RelayLiveRegistry,
}

impl PresenceStore {
    pub fn open(path: &Path) -> Result<Self, ServerError> {
        let conn = db::open_and_migrate(path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            relay_bootstrap_tcp: Arc::new(Mutex::new(HashSet::new())),
            relay_live: RelayLiveRegistry::default(),
        })
    }

    pub fn open_in_memory() -> Result<Self, ServerError> {
        let conn = db::open_memory()?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            relay_bootstrap_tcp: Arc::new(Mutex::new(HashSet::new())),
            relay_live: RelayLiveRegistry::default(),
        })
    }

    pub fn relay_live(&self) -> &RelayLiveRegistry {
        &self.relay_live
    }

    /// Remember relay bootstrap TCP bases (reject on client POST and coord lookup).
    pub fn set_relay_bootstrap_addrs(&self, addrs: &[String]) {
        if let Ok(mut g) = self.relay_bootstrap_tcp.lock() {
            *g = relay_bootstrap_tcp_keys(addrs);
        }
    }

    pub fn relay_bootstrap_tcp_snapshot(&self) -> HashSet<String> {
        self.relay_bootstrap_tcp
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// WAN-dialable endpoints only: relay `/p2p-circuit` or public routable IPv4 TCP.
    /// LAN RFC1918, loopback, CGNAT, and relay bootstrap TCP must never appear in coord lookup.
    fn filter_wan_presence_endpoints(&self, endpoints: Vec<PeerEndpoint>) -> Vec<PeerEndpoint> {
        let bootstraps = self.relay_bootstrap_tcp_snapshot();
        endpoints
            .into_iter()
            .filter(|e| is_wan_dialable_endpoint(e, &bootstraps))
            .collect()
    }

    /// Client POST /v1/register — merge TCP endpoints but never drop server relay circuits.
    pub fn merge_client_register(
        &self,
        public_key_hex: String,
        client_endpoints: Vec<PeerEndpoint>,
        transport_capabilities: Vec<String>,
        ipv6: Option<String>,
        ipv4: Option<String>,
    ) -> Result<PeerRecord, ServerError> {
        let key = normalize_identity_wire(&public_key_hex)?;
        let relay_keep: Vec<PeerEndpoint> = self
            .get_stored(&key)
            .ok()
            .map(|r| {
                r.endpoints
                    .into_iter()
                    .filter(|e| e.scheme == "libp2p" && e.host.contains("/p2p-circuit"))
                    .collect()
            })
            .unwrap_or_default();
        let bootstraps = self.relay_bootstrap_tcp_snapshot();
        let mut endpoints: Vec<PeerEndpoint> = client_endpoints
            .into_iter()
            .filter(|e| !(e.scheme == "libp2p" && e.host.contains("/p2p-circuit")))
            .filter(|e| !is_relay_bootstrap_tcp_endpoint(e, &bootstraps))
            .collect();
        for relay_ep in relay_keep {
            if !endpoints
                .iter()
                .any(|e| e.scheme == "libp2p" && e.host == relay_ep.host)
            {
                endpoints.push(relay_ep);
            }
        }
        let endpoints = self.filter_wan_presence_endpoints(endpoints);
        self.upsert(key, endpoints, transport_capabilities, ipv6, ipv4)
    }

    pub fn upsert(
        &self,
        public_key_hex: String,
        endpoints: Vec<PeerEndpoint>,
        transport_capabilities: Vec<String>,
        ipv6: Option<String>,
        ipv4: Option<String>,
    ) -> Result<PeerRecord, ServerError> {
        let key = normalize_identity_wire(&public_key_hex)?;
        let endpoints = self.filter_wan_presence_endpoints(endpoints);
        let now_ms = unix_ms_now();
        let endpoints_json = serde_json::to_string(&endpoints)
            .map_err(|e| ServerError::Internal(format!("endpoints json: {e}")))?;
        let caps_json = serde_json::to_string(&transport_capabilities)
            .map_err(|e| ServerError::Internal(format!("capabilities json: {e}")))?;
        let conn = self.conn.lock().map_err(lock_err)?;
        conn.execute(
            "INSERT INTO peers (
                public_key_hex, endpoints_json, transport_capabilities_json,
                ipv6, ipv4, last_heartbeat_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(public_key_hex) DO UPDATE SET
                endpoints_json = excluded.endpoints_json,
                transport_capabilities_json = excluded.transport_capabilities_json,
                ipv6 = excluded.ipv6,
                ipv4 = excluded.ipv4,
                last_heartbeat_unix_ms = excluded.last_heartbeat_unix_ms",
            params![key, endpoints_json, caps_json, ipv6, ipv4, now_ms as i64,],
        )?;
        Ok(PeerRecord {
            public_key_hex: key,
            endpoints,
            transport_capabilities,
            ipv6,
            ipv4,
            last_heartbeat_unix_ms: now_ms,
        })
    }

    pub fn heartbeat(&self, public_key_hex: &str) -> ApiResult<PeerRecord> {
        let key = normalize_identity_wire(&public_key_hex)?;
        let now_ms = unix_ms_now();
        let conn = self.conn.lock().map_err(lock_err)?;
        let n = conn.execute(
            "UPDATE peers SET last_heartbeat_unix_ms = ?1 WHERE public_key_hex = ?2",
            params![now_ms as i64, key],
        )?;
        if n == 0 {
            return Err(ServerError::NotFound("peer not registered".into()));
        }
        match self.fetch_one(&conn, &key, 0) {
            Ok(r) => self.prepare_peer_lookup(r),
            Err(e) => Err(e),
        }
    }

    pub fn get(&self, public_key_hex: &str, ttl: Duration) -> ApiResult<PeerRecord> {
        let key = normalize_identity_wire(&public_key_hex)?;
        let cutoff = heartbeat_cutoff_ms(ttl);
        let conn = self.conn.lock().map_err(lock_err)?;
        match self.fetch_one(&conn, &key, cutoff) {
            Ok(r) => self.prepare_peer_lookup(r),
            Err(ServerError::NotFound(_)) => Err(ServerError::NotFound(
                "peer not registered or presence expired".into(),
            )),
            Err(e) => Err(e),
        }
    }

    /// Fetch a peer record without TTL filtering (relay-driven presence updates).
    pub fn get_stored(&self, public_key_hex: &str) -> ApiResult<PeerRecord> {
        let key = normalize_identity_wire(&public_key_hex)?;
        let conn = self.conn.lock().map_err(lock_err)?;
        self.fetch_one(&conn, &key, 0)
    }

    /// Register or refresh a relay `/p2p-circuit` endpoint (server-authoritative WAN presence).
    pub fn upsert_relay_circuit(
        &self,
        public_key_hex: &str,
        circuit_ma: String,
    ) -> Result<PeerRecord, ServerError> {
        let key = normalize_identity_wire(&public_key_hex)?;
        let mut endpoints = self
            .get_stored(&key)
            .map(|r| r.endpoints)
            .unwrap_or_default();
        endpoints.retain(|e| !(e.scheme == "libp2p" && e.host.contains("/p2p-circuit")));
        endpoints.push(PeerEndpoint {
            scheme: "libp2p".into(),
            host: circuit_ma,
            port: 0,
        });
        let endpoints = endpoint_expand::expand_libp2p_circuit_endpoints(endpoints);
        let caps = vec!["tcp".into(), "sync-v1".into()];
        self.upsert(key, endpoints, caps, None, None)
    }

    /// Remove relay circuit endpoints; delete the row when nothing remains.
    pub fn remove_relay_circuit(&self, public_key_hex: &str) -> Result<bool, ServerError> {
        let key = normalize_identity_wire(&public_key_hex)?;
        let record = match self.get_stored(&key) {
            Ok(r) => r,
            Err(ServerError::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        };
        let had_circuit = record.endpoints.iter().any(|e| {
            e.scheme == "libp2p" && e.host.contains("/p2p-circuit")
        });
        if !had_circuit {
            return Ok(false);
        }
        let endpoints = self.filter_wan_presence_endpoints(
            record
                .endpoints
                .into_iter()
                .filter(|e| !(e.scheme == "libp2p" && e.host.contains("/p2p-circuit")))
                .collect(),
        );
        if endpoints.is_empty() {
            let conn = self.conn.lock().map_err(lock_err)?;
            conn.execute("DELETE FROM peers WHERE public_key_hex = ?1", params![key])?;
            return Ok(true);
        }
        self.upsert(
            key,
            endpoints,
            record.transport_capabilities,
            record.ipv6,
            record.ipv4,
        )?;
        Ok(true)
    }

    pub fn list_online(&self, ttl: Duration) -> Result<Vec<PeerRecord>, ServerError> {
        let cutoff = heartbeat_cutoff_ms(ttl);
        let conn = self.conn.lock().map_err(lock_err)?;
        let mut stmt = conn.prepare(
            "SELECT public_key_hex, endpoints_json, transport_capabilities_json,
                    ipv6, ipv4, last_heartbeat_unix_ms
             FROM peers
             WHERE last_heartbeat_unix_ms >= ?1
             ORDER BY last_heartbeat_unix_ms DESC",
        )?;
        let rows = stmt.query_map([cutoff as i64], row_to_peer_record)?;
        let mut out = Vec::new();
        for row in rows {
            let r = row?;
            if let Ok(r) = self.prepare_peer_lookup(r) {
                out.push(r);
            }
        }
        Ok(out)
    }

    /// WAN-filter, expand DNS circuits for dialers, apply live relay gate.
    fn prepare_peer_lookup(&self, mut r: PeerRecord) -> ApiResult<PeerRecord> {
        r.endpoints = self.filter_wan_presence_endpoints(r.endpoints);
        r.endpoints = endpoint_expand::expand_libp2p_circuit_endpoints(r.endpoints);
        self.relay_live.apply_live_relay_gate(&mut r);
        if r.endpoints.is_empty() {
            return Err(ServerError::NotFound(
                "peer not registered or presence expired".into(),
            ));
        }
        Ok(r)
    }

    pub fn purge_expired(&self, ttl: Duration) -> Result<u64, ServerError> {
        let cutoff = heartbeat_cutoff_ms(ttl);
        let conn = self.conn.lock().map_err(lock_err)?;
        let n = conn.execute(
            "DELETE FROM peers WHERE last_heartbeat_unix_ms < ?1",
            [cutoff as i64],
        )?;
        Ok(n as u64)
    }

    pub fn ping(&self) -> Result<(), ServerError> {
        let conn = self.conn.lock().map_err(lock_err)?;
        conn.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    fn fetch_one(
        &self,
        conn: &Connection,
        public_key_hex: &str,
        cutoff_ms: u64,
    ) -> ApiResult<PeerRecord> {
        let mut stmt = conn.prepare(
            "SELECT public_key_hex, endpoints_json, transport_capabilities_json,
                    ipv6, ipv4, last_heartbeat_unix_ms
             FROM peers
             WHERE public_key_hex = ?1 AND last_heartbeat_unix_ms >= ?2",
        )?;
        let mut rows = stmt.query(params![public_key_hex, cutoff_ms as i64])?;
        if let Some(row) = rows.next()? {
            return Ok(row_to_peer_record(row)?);
        }
        Err(ServerError::NotFound("peer not registered".into()))
    }
}

fn row_to_peer_record(row: &rusqlite::Row<'_>) -> Result<PeerRecord, rusqlite::Error> {
    let endpoints_json: String = row.get(1)?;
    let caps_json: String = row.get(2)?;
    let endpoints: Vec<PeerEndpoint> = serde_json::from_str(&endpoints_json).map_err(|e| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("endpoints json: {e}"),
        )))
    })?;
    let transport_capabilities: Vec<String> = serde_json::from_str(&caps_json).map_err(|e| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("capabilities json: {e}"),
        )))
    })?;
    Ok(PeerRecord {
        public_key_hex: row.get(0)?,
        endpoints,
        transport_capabilities,
        ipv6: row.get(3)?,
        ipv4: row.get(4)?,
        last_heartbeat_unix_ms: row.get::<_, i64>(5)? as u64,
    })
}

fn heartbeat_cutoff_ms(ttl: Duration) -> u64 {
    let now = unix_ms_now();
    now.saturating_sub(ttl.as_millis() as u64)
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn lock_err<T>(_: std::sync::PoisonError<T>) -> ServerError {
    ServerError::Internal("database lock poisoned".into())
}

fn is_wan_dialable_endpoint(ep: &PeerEndpoint, relay_bootstraps: &HashSet<String>) -> bool {
    match ep.scheme.as_str() {
        "libp2p" => ep.host.contains("/p2p-circuit"),
        "tcp" => {
            is_public_routable_tcp_host(&ep.host)
                && ep.port != 0
                && !is_relay_bootstrap_tcp_endpoint(ep, relay_bootstraps)
        }
        _ => false,
    }
}

fn is_relay_bootstrap_tcp_endpoint(ep: &PeerEndpoint, relay_bootstraps: &HashSet<String>) -> bool {
    ep.scheme == "tcp" && relay_bootstraps.contains(&format!("{}:{}", ep.host.trim(), ep.port))
}

fn relay_bootstrap_tcp_keys(addrs: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for addr in addrs {
        let s = addr.trim();
        if let Some(host) = s.split("/ip4/").nth(1).and_then(|r| r.split('/').next()) {
            if let Some(port) = s
                .split("/tcp/")
                .nth(1)
                .and_then(|r| r.split('/').next())
                .and_then(|p| p.parse::<u16>().ok())
            {
                out.insert(format!("{host}:{port}"));
            }
        }
    }
    out
}

fn is_public_routable_tcp_host(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() || host.contains(':') {
        return false;
    }
    let Ok(ip) = host.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    !ip.is_private()
        && !ip.is_loopback()
        && !ip.is_unspecified()
        && !ip.is_link_local()
        && !(ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 0x40)
}

impl From<rusqlite::Error> for ServerError {
    fn from(e: rusqlite::Error) -> Self {
        ServerError::Internal(format!("database: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wan_dialable_rejects_relay_bootstrap_tcp() {
        let bootstraps = relay_bootstrap_tcp_keys(&["/ip4/159.223.110.159/tcp/28048".to_string()]);
        let ep = PeerEndpoint {
            scheme: "tcp".into(),
            host: "159.223.110.159".into(),
            port: 28048,
        };
        assert!(!is_wan_dialable_endpoint(&ep, &bootstraps));
        let peer = PeerEndpoint {
            scheme: "tcp".into(),
            host: "203.0.113.50".into(),
            port: 41234,
        };
        assert!(is_wan_dialable_endpoint(&peer, &bootstraps));
    }

    fn test_secp_pk_hex(seed: u8) -> String {
        let sk = secp256k1::SecretKey::from_byte_array([seed; 32]).expect("test key");
        let secp = secp256k1::Secp256k1::new();
        hex::encode(sk.public_key(&secp).serialize())
    }

    #[test]
    fn merge_register_strips_client_relay_bootstrap_tcp() {
        let store = PresenceStore::open_in_memory().expect("db");
        store.set_relay_bootstrap_addrs(&["/ip4/159.223.110.159/tcp/28048".to_string()]);
        let pk = test_secp_pk_hex(11);
        let circuit = PeerEndpoint {
            scheme: "libp2p".into(),
            host: "/ip4/159.223.110.159/tcp/28048/p2p/12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X/p2p-circuit/p2p/16Uiu2HAm5zdGNzac9hYfCNQZTnANbxWytcMty9twy7u942fT7MCk"
                .into(),
            port: 0,
        };
        store
            .upsert_relay_circuit(&pk, circuit.host.clone())
            .expect("relay upsert");
        let client_tcp = vec![PeerEndpoint {
            scheme: "tcp".into(),
            host: "159.223.110.159".into(),
            port: 28048,
        }];
        let rec = store
            .merge_client_register(pk.clone(), client_tcp, vec!["tcp".into()], None, None)
            .expect("merge");
        assert!(
            rec.endpoints.iter().any(|e| e.scheme == "libp2p"),
            "circuit must remain"
        );
        assert!(
            !rec
                .endpoints
                .iter()
                .any(|e| e.scheme == "tcp" && e.host == "159.223.110.159"),
            "bootstrap tcp must not be stored"
        );
    }

    #[test]
    fn relay_circuit_upsert_expands_dns6() {
        let store = PresenceStore::open_in_memory().expect("db");
        let pk = test_secp_pk_hex(12);
        let dns6 = "/dns6/coord.ghalbol.com/tcp/4002/p2p/12D3KooW/p2p-circuit/p2p/16Uiu2HAm5zdGNzac9hYfCNQZTnANbxWytcMty9twy7u942fT7MCk";
        store
            .upsert_relay_circuit(&pk, dns6.into())
            .expect("upsert");
        let stored = store.get_stored(&pk).expect("stored");
        assert!(
            stored
                .endpoints
                .iter()
                .any(|e| e.host.contains("/dns4/coord.ghalbol.com/tcp/4002")),
            "stored endpoints: {:?}",
            stored.endpoints.iter().map(|e| &e.host).collect::<Vec<_>>()
        );
    }
}
