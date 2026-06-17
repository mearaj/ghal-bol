//! Peer presence and reachable endpoints — persisted in SQLite.

use crate::db;
use crate::error::{ApiResult, ServerError};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
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
}

impl PresenceStore {
    pub fn open(path: &Path) -> Result<Self, ServerError> {
        let conn = db::open_and_migrate(path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self, ServerError> {
        let conn = db::open_memory()?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn upsert(
        &self,
        public_key_hex: String,
        endpoints: Vec<PeerEndpoint>,
        transport_capabilities: Vec<String>,
        ipv6: Option<String>,
        ipv4: Option<String>,
    ) -> Result<PeerRecord, ServerError> {
        let key = public_key_hex.to_ascii_lowercase();
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
        let key = public_key_hex.to_ascii_lowercase();
        let now_ms = unix_ms_now();
        let conn = self.conn.lock().map_err(lock_err)?;
        let n = conn.execute(
            "UPDATE peers SET last_heartbeat_unix_ms = ?1 WHERE public_key_hex = ?2",
            params![now_ms as i64, key],
        )?;
        if n == 0 {
            return Err(ServerError::NotFound("peer not registered".into()));
        }
        self.fetch_one(&conn, &key, 0)
    }

    pub fn get(&self, public_key_hex: &str, ttl: Duration) -> ApiResult<PeerRecord> {
        let key = public_key_hex.to_ascii_lowercase();
        let cutoff = heartbeat_cutoff_ms(ttl);
        let conn = self.conn.lock().map_err(lock_err)?;
        match self.fetch_one(&conn, &key, cutoff) {
            Ok(r) => Ok(r),
            Err(ServerError::NotFound(_)) => Err(ServerError::NotFound(
                "peer not registered or presence expired".into(),
            )),
            Err(e) => Err(e),
        }
    }

    /// Fetch a peer record without TTL filtering (relay-driven presence updates).
    pub fn get_stored(&self, public_key_hex: &str) -> ApiResult<PeerRecord> {
        let key = public_key_hex.to_ascii_lowercase();
        let conn = self.conn.lock().map_err(lock_err)?;
        self.fetch_one(&conn, &key, 0)
    }

    /// Register or refresh a relay `/p2p-circuit` endpoint (server-authoritative WAN presence).
    pub fn upsert_relay_circuit(
        &self,
        public_key_hex: &str,
        circuit_ma: String,
    ) -> Result<PeerRecord, ServerError> {
        let key = public_key_hex.to_ascii_lowercase();
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
        let caps = vec!["tcp".into(), "sync-v1".into()];
        self.upsert(key, endpoints, caps, None, None)
    }

    /// Remove relay circuit endpoints; delete the row when nothing remains.
    pub fn remove_relay_circuit(&self, public_key_hex: &str) -> Result<bool, ServerError> {
        let key = public_key_hex.to_ascii_lowercase();
        let record = match self.get_stored(&key) {
            Ok(r) => r,
            Err(ServerError::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        };
        let before = record.endpoints.len();
        let endpoints: Vec<_> = record
            .endpoints
            .into_iter()
            .filter(|e| !(e.scheme == "libp2p" && e.host.contains("/p2p-circuit")))
            .collect();
        if endpoints.len() == before {
            return Ok(false);
        }
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
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

impl From<rusqlite::Error> for ServerError {
    fn from(e: rusqlite::Error) -> Self {
        ServerError::Internal(format!("database: {e}"))
    }
}
