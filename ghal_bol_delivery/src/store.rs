//! Mailbox storage and quota.

use crate::config::DeliveryConfig;
use crate::envelope::ValidatedEnvelope;
use crate::error::{DeliveryError, Result};
use crate::policy::PolicyLimits;
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, serde::Serialize)]
pub struct QuotaStatus {
    pub allocated_bytes: i64,
    pub used_bytes: i64,
    pub pending_count: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct MailboxRow {
    pub message_id: String,
    pub recipient_wire: String,
    pub size_bytes: i64,
    pub uploaded_at_ms: i64,
    pub expires_at_ms: i64,
    pub state: String,
}

#[derive(Clone, Debug)]
pub struct HealthMetrics {
    pub connected_peers: usize,
    pub pending_messages: i64,
    pub pending_bytes: i64,
    pub oldest_pending_age_secs: i64,
}

pub struct MailboxStore {
    conn: Mutex<Connection>,
    quota_per_peer: i64,
}

impl MailboxStore {
    pub fn new(conn: Connection, config: &DeliveryConfig) -> Self {
        Self {
            conn: Mutex::new(conn),
            quota_per_peer: config.quota_bytes_per_peer as i64,
        }
    }

    pub fn shared(conn: Connection, config: &DeliveryConfig) -> Arc<Self> {
        Arc::new(Self::new(conn, config))
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    pub fn quota_status(&self, sender_wire: &str) -> Result<QuotaStatus> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let used: i64 = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM pending_messages
             WHERE sender_wire = ?1 AND state = 'queued'",
            params![sender_wire],
            |r| r.get(0),
        )?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_messages
             WHERE sender_wire = ?1 AND state = 'queued'",
            params![sender_wire],
            |r| r.get(0),
        )?;
        Ok(QuotaStatus {
            allocated_bytes: self.quota_per_peer,
            used_bytes: used,
            pending_count: count,
        })
    }

    pub fn list_outbox(&self, sender_wire: &str, include_expired: bool) -> Result<Vec<MailboxRow>> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let sql = if include_expired {
            "SELECT message_id, recipient_wire, size_bytes, uploaded_at_ms, expires_at_ms, state
             FROM pending_messages WHERE sender_wire = ?1
             ORDER BY uploaded_at_ms DESC"
        } else {
            "SELECT message_id, recipient_wire, size_bytes, uploaded_at_ms, expires_at_ms, state
             FROM pending_messages WHERE sender_wire = ?1 AND state = 'queued'
             ORDER BY uploaded_at_ms DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![sender_wire], |r| {
                Ok(MailboxRow {
                    message_id: r.get(0)?,
                    recipient_wire: r.get(1)?,
                    size_bytes: r.get(2)?,
                    uploaded_at_ms: r.get(3)?,
                    expires_at_ms: r.get(4)?,
                    state: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn upload(
        &self,
        env: ValidatedEnvelope,
        ttl_secs: u64,
        policy: &PolicyLimits,
    ) -> Result<(QuotaStatus, bool)> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let now = Self::now_ms();
        let expires_at = now + (ttl_secs as i64) * 1000;
        let max_exp = policy.max_expires_at_ms(now);
        let expires_at = expires_at.min(max_exp);

        let existing_state: Option<String> = conn
            .query_row(
                "SELECT state FROM pending_messages WHERE sender_wire = ?1 AND message_id = ?2",
                params![env.sender_wire, env.message_id],
                |r| r.get(0),
            )
            .ok();

        let used: i64 = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM pending_messages
             WHERE sender_wire = ?1 AND state = 'queued'",
            params![env.sender_wire],
            |r| r.get(0),
        )?;

        let replace_bytes: i64 = if existing_state.as_deref() == Some("queued") {
            conn.query_row(
                "SELECT size_bytes FROM pending_messages
                 WHERE sender_wire = ?1 AND message_id = ?2",
                params![env.sender_wire, env.message_id],
                |r| r.get(0),
            )
            .unwrap_or(0)
        } else {
            0
        };

        let new_used = used - replace_bytes + env.size_bytes;
        if new_used > self.quota_per_peer {
            return Err(DeliveryError::QuotaExceeded(format!(
                "used {new_used} exceeds allocated {}",
                self.quota_per_peer
            )));
        }

        let replaced = existing_state.as_deref() == Some("queued");
        if let Some(state) = existing_state {
            if state == "queued" {
                conn.execute(
                    "UPDATE pending_messages SET
                        recipient_wire = ?3, envelope_blob = ?4, size_bytes = ?5,
                        uploaded_at_ms = ?6, expires_at_ms = ?7, state = 'queued',
                        delivered_at_ms = NULL, expired_at_ms = NULL
                     WHERE sender_wire = ?1 AND message_id = ?2",
                    params![
                        env.sender_wire,
                        env.message_id,
                        env.recipient_wire,
                        env.envelope_blob,
                        env.size_bytes,
                        now,
                        expires_at,
                    ],
                )?;
            } else {
                conn.execute(
                    "INSERT OR REPLACE INTO pending_messages
                     (sender_wire, message_id, recipient_wire, envelope_blob, size_bytes,
                      uploaded_at_ms, expires_at_ms, state)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued')",
                    params![
                        env.sender_wire,
                        env.message_id,
                        env.recipient_wire,
                        env.envelope_blob,
                        env.size_bytes,
                        now,
                        expires_at,
                    ],
                )?;
            }
        } else {
            conn.execute(
                "INSERT INTO pending_messages
                 (sender_wire, message_id, recipient_wire, envelope_blob, size_bytes,
                  uploaded_at_ms, expires_at_ms, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued')",
                params![
                    env.sender_wire,
                    env.message_id,
                    env.recipient_wire,
                    env.envelope_blob,
                    env.size_bytes,
                    now,
                    expires_at,
                ],
            )?;
        }

        tracing::info!(
            sender_wire = %env.sender_wire,
            message_id = %env.message_id,
            recipient_wire = %env.recipient_wire,
            bytes = env.size_bytes,
            state = "queued",
            replaced,
            "message uploaded"
        );

        let sender_wire = env.sender_wire.clone();
        drop(conn);
        let quota = self.quota_status(&sender_wire)?;
        Ok((quota, replaced))
    }

    pub fn extend_ttl(
        &self,
        sender_wire: &str,
        message_id: &str,
        extend_secs: u64,
        policy: &PolicyLimits,
    ) -> Result<MailboxRow> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let row: MailboxRow = conn
            .query_row(
                "SELECT message_id, recipient_wire, size_bytes, uploaded_at_ms, expires_at_ms, state
                 FROM pending_messages WHERE sender_wire = ?1 AND message_id = ?2",
                params![sender_wire, message_id],
                |r| {
                    Ok(MailboxRow {
                        message_id: r.get(0)?,
                        recipient_wire: r.get(1)?,
                        size_bytes: r.get(2)?,
                        uploaded_at_ms: r.get(3)?,
                        expires_at_ms: r.get(4)?,
                        state: r.get(5)?,
                    })
                },
            )
            .map_err(|_| DeliveryError::NotFound("message not found".into()))?;

        if row.state != "queued" {
            if row.state == "expired" {
                return Err(DeliveryError::Expired(
                    "message expired; resend required".into(),
                ));
            }
            return Err(DeliveryError::BadRequest(format!(
                "cannot extend state={}",
                row.state
            )));
        }

        let now = Self::now_ms();
        let candidate = now + (extend_secs as i64) * 1000;
        let max_exp = policy.max_expires_at_ms(row.uploaded_at_ms);
        let new_expires = candidate.min(max_exp);
        if new_expires <= row.expires_at_ms {
            return Err(DeliveryError::TtlInvalid(
                "extend_secs would not increase expiry".into(),
            ));
        }

        conn.execute(
            "UPDATE pending_messages SET expires_at_ms = ?3
             WHERE sender_wire = ?1 AND message_id = ?2",
            params![sender_wire, message_id, new_expires],
        )?;

        tracing::info!(
            sender_wire = %sender_wire,
            message_id = %message_id,
            new_expires_at_ms = new_expires,
            "ttl extended"
        );

        Ok(MailboxRow {
            expires_at_ms: new_expires,
            ..row
        })
    }

    pub fn ack_deliver(
        &self,
        recipient_wire: &str,
        message_id: &str,
        sender_wire: &str,
    ) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let row_sender: String = conn
            .query_row(
                "SELECT sender_wire FROM pending_messages
                 WHERE recipient_wire = ?1 AND message_id = ?2 AND state = 'queued'",
                params![recipient_wire, message_id],
                |r| r.get(0),
            )
            .map_err(|_| DeliveryError::NotFound("queued message not found".into()))?;

        if row_sender != sender_wire {
            return Err(DeliveryError::Forbidden(
                "sender_wire mismatch for ack".into(),
            ));
        }

        let now = Self::now_ms();
        conn.execute(
            "UPDATE pending_messages SET state = 'delivered', envelope_blob = NULL,
             delivered_at_ms = ?4
             WHERE recipient_wire = ?1 AND message_id = ?2 AND sender_wire = ?3",
            params![recipient_wire, message_id, sender_wire, now],
        )?;

        tracing::info!(
            recipient_wire = %recipient_wire,
            sender_wire = %sender_wire,
            message_id = %message_id,
            state = "delivered",
            "message acked"
        );

        Ok(Some(sender_wire.to_string()))
    }

    pub fn ack_read(
        &self,
        recipient_wire: &str,
        message_id: &str,
        sender_wire: &str,
    ) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let (row_sender, state): (String, String) = conn
            .query_row(
                "SELECT sender_wire, state FROM pending_messages
                 WHERE recipient_wire = ?1 AND message_id = ?2",
                params![recipient_wire, message_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| DeliveryError::NotFound("message metadata not found".into()))?;

        if row_sender != sender_wire {
            return Err(DeliveryError::Forbidden(
                "sender_wire mismatch for read ack".into(),
            ));
        }
        if state == "read" {
            drop(conn);
            return Ok(Some(sender_wire.to_string()));
        }
        if state != "delivered" {
            return Err(DeliveryError::BadRequest(format!(
                "cannot read-ack state={state}"
            )));
        }

        let now = Self::now_ms();
        conn.execute(
            "UPDATE pending_messages SET state = 'read', read_at_ms = ?4
             WHERE recipient_wire = ?1 AND message_id = ?2 AND sender_wire = ?3",
            params![recipient_wire, message_id, sender_wire, now],
        )?;

        tracing::info!(
            recipient_wire = %recipient_wire,
            sender_wire = %sender_wire,
            message_id = %message_id,
            state = "read",
            "message read acked"
        );

        Ok(Some(sender_wire.to_string()))
    }

    pub fn pending_for_recipient(&self, recipient_wire: &str) -> Result<Vec<(String, String, i64)>> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let now = Self::now_ms();
        let mut stmt = conn.prepare(
            "SELECT envelope_blob, message_id, expires_at_ms FROM pending_messages
             WHERE recipient_wire = ?1 AND state = 'queued' AND envelope_blob IS NOT NULL
             AND expires_at_ms > ?2",
        )?;
        let rows = stmt
            .query_map(params![recipient_wire, now], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn sweep_expired(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let now = Self::now_ms();
        let mut stmt = conn.prepare(
            "SELECT sender_wire, message_id, recipient_wire FROM pending_messages
             WHERE state = 'queued' AND expires_at_ms <= ?1",
        )?;
        let expired: Vec<(String, String, String)> = stmt
            .query_map(params![now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        for (sender, message_id, recipient) in &expired {
            conn.execute(
                "UPDATE pending_messages SET state = 'expired', envelope_blob = NULL,
                 expired_at_ms = ?4
                 WHERE sender_wire = ?1 AND message_id = ?2 AND recipient_wire = ?3",
                params![sender, message_id, recipient, now],
            )?;
            tracing::info!(
                sender_wire = %sender,
                message_id = %message_id,
                state = "expired",
                "message expired"
            );
        }
        Ok(expired)
    }

    /// Test helper: force `expires_at_ms` for integration tests.
    #[doc(hidden)]
    pub fn test_force_expires_at(
        &self,
        sender_wire: &str,
        message_id: &str,
        expires_at_ms: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        conn.execute(
            "UPDATE pending_messages SET expires_at_ms = ?3
             WHERE sender_wire = ?1 AND message_id = ?2",
            params![sender_wire, message_id, expires_at_ms],
        )?;
        Ok(())
    }

    pub fn aggregate_health(&self, connected_peers: usize) -> Result<HealthMetrics> {
        let conn = self.conn.lock().map_err(|_| {
            DeliveryError::Internal("store mutex poisoned".into())
        })?;
        let now = Self::now_ms();
        let pending_messages: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_messages WHERE state = 'queued'",
            [],
            |r| r.get(0),
        )?;
        let pending_bytes: i64 = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM pending_messages WHERE state = 'queued'",
            [],
            |r| r.get(0),
        )?;
        let oldest: Option<i64> = conn
            .query_row(
                "SELECT MIN(uploaded_at_ms) FROM pending_messages WHERE state = 'queued'",
                [],
                |r| r.get(0),
            )
            .ok();
        let oldest_pending_age_secs = oldest
            .map(|t| ((now - t).max(0)) / 1000)
            .unwrap_or(0);
        Ok(HealthMetrics {
            connected_peers,
            pending_messages,
            pending_bytes,
            oldest_pending_age_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DeliveryConfig;
    use crate::db;
    use crate::envelope::ValidatedEnvelope;
    use crate::policy::PolicyLimits;

    #[test]
    fn upload_and_extend() {
        let cfg = DeliveryConfig::default();
        let store = MailboxStore::new(db::open_memory().unwrap(), &cfg);
        let policy = PolicyLimits::from_config(&cfg);
        let sender = "02".repeat(33);
        let validated = ValidatedEnvelope {
            message_id: "m1".into(),
            sender_wire: sender.clone(),
            recipient_wire: "03".repeat(33),
            envelope_blob: "{}".into(),
            size_bytes: 64,
        };
        store.upload(validated.clone(), 300, &policy).unwrap();
        let row = store.extend_ttl(&sender, "m1", 3600, &policy).unwrap();
        assert!(row.expires_at_ms > 0);
    }

    #[test]
    fn ack_deliver_then_read() {
        let cfg = DeliveryConfig::default();
        let store = MailboxStore::new(db::open_memory().unwrap(), &cfg);
        let sender = "02".repeat(33);
        let recipient = "03".repeat(33);
        let validated = ValidatedEnvelope {
            message_id: "m-read".into(),
            sender_wire: sender.clone(),
            recipient_wire: recipient.clone(),
            envelope_blob: "{}".into(),
            size_bytes: 32,
        };
        let policy = PolicyLimits::from_config(&cfg);
        store.upload(validated, 300, &policy).unwrap();
        store
            .ack_deliver(&recipient, "m-read", &sender)
            .unwrap();
        store
            .ack_read(&recipient, "m-read", &sender)
            .unwrap();
        let rows = store.list_outbox(&sender, true).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, "read");
    }
}
