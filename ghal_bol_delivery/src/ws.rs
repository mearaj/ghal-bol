//! WebSocket session handler.

use crate::auth::{
    ChallengeStore, extend_challenge_bytes, parse_nonce_hex, parse_signature_hex,
    session_challenge_bytes, upload_challenge_bytes, verify_signature,
};
use crate::envelope::validate_envelope;
use crate::error::DeliveryError;
use crate::policy::PolicyLimits;
use crate::store::MailboxStore;
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

type WsSender = mpsc::UnboundedSender<String>;

#[derive(Clone, Default)]
pub struct SessionRegistry {
    inner: Arc<RwLock<HashMap<String, Vec<WsSender>>>>,
    connected_count: Arc<RwLock<usize>>,
}

impl SessionRegistry {
    pub async fn register(&self, identity_wire: &str, tx: WsSender) {
        let mut g = self.inner.write().await;
        g.entry(identity_wire.to_string()).or_default().push(tx);
        *self.connected_count.write().await = g.values().map(|v| v.len()).sum();
    }

    pub async fn unregister(&self, identity_wire: &str, tx: &WsSender) {
        let mut g = self.inner.write().await;
        if let Some(v) = g.get_mut(identity_wire) {
            v.retain(|s| !s.same_channel(tx));
            if v.is_empty() {
                g.remove(identity_wire);
            }
        }
        *self.connected_count.write().await = g.values().map(|v| v.len()).sum();
    }

    pub async fn connected_count(&self) -> usize {
        *self.connected_count.read().await
    }

    pub async fn push(&self, identity_wire: &str, frame: Value) {
        let msg = frame.to_string();
        let g = self.inner.read().await;
        if let Some(senders) = g.get(identity_wire) {
            for tx in senders {
                let _ = tx.send(msg.clone());
            }
        }
    }
}

pub struct WsState {
    pub store: Arc<MailboxStore>,
    pub policy: PolicyLimits,
    pub registry: SessionRegistry,
    pub challenges: Arc<std::sync::Mutex<ChallengeStore>>,
}

pub async fn handle_socket(socket: WebSocket, state: Arc<WsState>) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (push_tx, mut push_rx) = mpsc::unbounded_channel::<String>();
    let mut authenticated: Option<String> = None;
    let mut op_nonce_hex: Option<String> = None;

    loop {
        tokio::select! {
            Some(push) = push_rx.recv() => {
                if ws_tx.send(Message::Text(push.into())).await.is_err() {
                    break;
                }
            }
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let response = process_frame(
                            &text,
                            &state,
                            &mut authenticated,
                            &mut op_nonce_hex,
                            &push_tx,
                        )
                        .await;
                        if let Some(resp) = response {
                            if ws_tx.send(Message::Text(resp.to_string().into())).await.is_err() {
                                break;
                            }
                            if resp.get("type").and_then(|v| v.as_str()) == Some("session.ready") {
                                if let Some(id) = authenticated.as_ref() {
                                    let norm = id.clone();
                                    let state_inbound = state.clone();
                                    tokio::spawn(async move {
                                        deliver_pending_inbound(&norm, &state_inbound).await;
                                    });
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }

    if let Some(id) = authenticated.as_ref() {
        state.registry.unregister(id, &push_tx).await;
    }
}

async fn process_frame(
    text: &str,
    state: &Arc<WsState>,
    authenticated: &mut Option<String>,
    op_nonce_hex: &mut Option<String>,
    push_tx: &WsSender,
) -> Option<Value> {
    let frame: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return Some(error_frame("bad_request", &e.to_string(), None)),
    };
    let frame_type = frame
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let request_id = frame.get("request_id").cloned();

    let result = match frame_type.as_str() {
        "session.open" => {
            handle_session_open(frame, state, authenticated, push_tx).await
        }
        "session.auth" => {
            handle_session_auth(frame, state, authenticated, op_nonce_hex, push_tx).await
        }
        "message.upload" => handle_upload(frame, state, authenticated, op_nonce_hex).await,
        "inbox.ack" => handle_inbox_ack(frame, state, authenticated).await,
        "inbox.read" => handle_inbox_read(frame, state, authenticated).await,
        "mailbox.outbox.list" => handle_outbox_list(frame, state, authenticated).await,
        "mailbox.ttl.extend" => handle_extend(frame, state, authenticated, op_nonce_hex).await,
        "quota.status" => handle_quota(state, authenticated).await,
        "ping" => Ok(Some(json!({ "type": "pong" }))),
        _ => Err(DeliveryError::BadRequest(format!("unknown type: {frame_type}"))),
    };

    match result {
        Ok(opt) => opt,
        Err(e) => {
            tracing::warn!(
                frame_type = %frame_type,
                code = e.ws_code(),
                reject_reason = %e,
                "ws frame rejected"
            );
            Some(error_frame(e.ws_code(), &e.to_string(), request_id))
        }
    }
}

async fn handle_session_open(
    frame: Value,
    state: &Arc<WsState>,
    authenticated: &mut Option<String>,
    push_tx: &WsSender,
) -> Result<Option<Value>, DeliveryError> {
    let identity_wire = frame
        .get("identity_wire")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::BadRequest("missing identity_wire".into()))?;
    let mut ch = state.challenges.lock().map_err(|_| {
        DeliveryError::Internal("challenge mutex poisoned".into())
    })?;
    ch.purge_expired();
    let challenge = ch.issue_session(identity_wire, Duration::from_secs(120))?;
    *authenticated = None;
    let _ = push_tx;
    Ok(Some(json!({
        "type": "session.challenge",
        "nonce_hex": hex::encode(challenge.nonce),
    })))
}

async fn handle_session_auth(
    frame: Value,
    state: &Arc<WsState>,
    authenticated: &mut Option<String>,
    op_nonce_hex: &mut Option<String>,
    push_tx: &WsSender,
) -> Result<Option<Value>, DeliveryError> {
    let identity_wire = frame
        .get("identity_wire")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::BadRequest("missing identity_wire".into()))?;
    let nonce = parse_nonce_hex(
        frame
            .get("nonce_hex")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DeliveryError::BadRequest("missing nonce_hex".into()))?,
    )?;
    let sig = parse_signature_hex(
        frame
            .get("signature_hex")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DeliveryError::BadRequest("missing signature_hex".into()))?,
    )?;
    {
        let mut ch = state.challenges.lock().map_err(|_| {
            DeliveryError::Internal("challenge mutex poisoned".into())
        })?;
        ch.take_session_valid(identity_wire, &nonce)?;
    }
    let msg = session_challenge_bytes(&nonce, identity_wire);
    verify_signature(identity_wire, &msg, &sig)?;

    let norm = crate::identity::normalize_identity_wire(identity_wire)?;
    *authenticated = Some(norm.clone());
    state.registry.register(&norm, push_tx.clone()).await;

    let op_nonce_hex_val = {
        let mut ch = state.challenges.lock().map_err(|_| {
            DeliveryError::Internal("challenge mutex poisoned".into())
        })?;
        let op_nonce = ch.issue_op_nonce(&norm, Duration::from_secs(60))?;
        hex::encode(op_nonce)
    };
    *op_nonce_hex = Some(op_nonce_hex_val.clone());

    let quota = state.store.quota_status(&norm)?;

    Ok(Some(json!({
        "type": "session.ready",
        "identity_wire": norm,
        "op_nonce_hex": op_nonce_hex,
        "policy": {
            "type": "policy.limits",
            "min_ttl_secs": state.policy.min_ttl_secs,
            "max_ttl_secs": state.policy.max_ttl_secs,
            "default_ttl_secs": state.policy.default_ttl_secs,
        },
        "quota": {
            "type": "quota.status",
            "allocated_bytes": quota.allocated_bytes,
            "used_bytes": quota.used_bytes,
            "pending_count": quota.pending_count,
        }
    })))
}

async fn deliver_pending_inbound(recipient_wire: &str, state: &Arc<WsState>) {
    if let Ok(rows) = state.store.pending_for_recipient(recipient_wire) {
        for (blob, message_id, expires_at_ms) in rows {
            if let Ok(envelope) = serde_json::from_str::<Value>(&blob) {
                state
                    .registry
                    .push(
                        recipient_wire,
                        json!({
                            "type": "message.inbound",
                            "message_id": message_id,
                            "envelope": envelope,
                            "expires_at_ms": expires_at_ms,
                        }),
                    )
                    .await;
            }
        }
    }
}

async fn handle_upload(
    frame: Value,
    state: &Arc<WsState>,
    authenticated: &Option<String>,
    op_nonce_hex: &mut Option<String>,
) -> Result<Option<Value>, DeliveryError> {
    let session = authenticated
        .as_ref()
        .ok_or_else(|| DeliveryError::Unauthorized("session not ready".into()))?;
    let envelope = frame
        .get("envelope")
        .ok_or_else(|| DeliveryError::BadRequest("missing envelope".into()))?;
    let env = validate_envelope(envelope, session)?;

    let op_nonce = parse_nonce_hex(
        frame
            .get("op_nonce_hex")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DeliveryError::BadRequest("missing op_nonce_hex".into()))?,
    )?;
    let sig = parse_signature_hex(
        frame
            .get("signature_hex")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DeliveryError::BadRequest("missing signature_hex".into()))?,
    )?;
    let upload_msg = upload_challenge_bytes(&op_nonce, &env.message_id, &env.recipient_wire);
    verify_signature(session, &upload_msg, &sig)?;
    {
        let mut ch = state.challenges.lock().map_err(|_| {
            DeliveryError::Internal("challenge mutex poisoned".into())
        })?;
        ch.take_op_valid(session, &op_nonce)?;
        let new_nonce = ch.issue_op_nonce(session, Duration::from_secs(60))?;
        *op_nonce_hex = Some(hex::encode(new_nonce));
    }

    let ttl_secs = state.policy.clamp_ttl(
        frame.get("ttl_secs").and_then(|v| v.as_u64()),
    )?;

    let message_id = env.message_id.clone();
    let recipient_wire = env.recipient_wire.clone();
    let (quota, replaced) = state.store.upload(env, ttl_secs, &state.policy)?;

    state
        .registry
        .push(
            &recipient_wire,
            json!({
                "type": "message.inbound",
                "message_id": message_id,
                "envelope": envelope,
            }),
        )
        .await;

    Ok(Some(json!({
        "type": "message.upload.ok",
        "message_id": message_id,
        "replaced": replaced,
        "op_nonce_hex": op_nonce_hex,
        "quota": {
            "type": "quota.status",
            "allocated_bytes": quota.allocated_bytes,
            "used_bytes": quota.used_bytes,
            "pending_count": quota.pending_count,
        }
    })))
}

async fn handle_inbox_ack(
    frame: Value,
    state: &Arc<WsState>,
    authenticated: &Option<String>,
) -> Result<Option<Value>, DeliveryError> {
    let session = authenticated
        .as_ref()
        .ok_or_else(|| DeliveryError::Unauthorized("session not ready".into()))?;
    let message_id = frame
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::BadRequest("missing message_id".into()))?;
    let sender_wire = frame
        .get("sender_wire")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::BadRequest("missing sender_wire".into()))?;
    let sender_norm = crate::identity::normalize_identity_wire(sender_wire)?;

    state
        .store
        .ack_deliver(session, message_id, &sender_norm)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    state
        .registry
        .push(
            &sender_norm,
            json!({
                "type": "message.ack_to_sender",
                "message_id": message_id,
                "recipient_wire": session,
                "delivered_at_ms": now,
            }),
        )
        .await;

    Ok(Some(json!({
        "type": "inbox.ack.ok",
        "message_id": message_id,
    })))
}

async fn handle_inbox_read(
    frame: Value,
    state: &Arc<WsState>,
    authenticated: &Option<String>,
) -> Result<Option<Value>, DeliveryError> {
    let session = authenticated
        .as_ref()
        .ok_or_else(|| DeliveryError::Unauthorized("session not ready".into()))?;
    let message_id = frame
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::BadRequest("missing message_id".into()))?;
    let sender_wire = frame
        .get("sender_wire")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::BadRequest("missing sender_wire".into()))?;
    let sender_norm = crate::identity::normalize_identity_wire(sender_wire)?;

    state
        .store
        .ack_read(session, message_id, &sender_norm)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    state
        .registry
        .push(
            &sender_norm,
            json!({
                "type": "message.read_to_sender",
                "message_id": message_id,
                "recipient_wire": session,
                "read_at_ms": now,
            }),
        )
        .await;

    Ok(Some(json!({
        "type": "inbox.read.ok",
        "message_id": message_id,
    })))
}

async fn handle_outbox_list(
    frame: Value,
    state: &Arc<WsState>,
    authenticated: &Option<String>,
) -> Result<Option<Value>, DeliveryError> {
    let session = authenticated
        .as_ref()
        .ok_or_else(|| DeliveryError::Unauthorized("session not ready".into()))?;
    let include_expired = frame
        .get("include_expired")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let rows = state.store.list_outbox(session, include_expired)?;
    Ok(Some(json!({
        "type": "mailbox.outbox.snapshot",
        "rows": rows,
    })))
}

async fn handle_extend(
    frame: Value,
    state: &Arc<WsState>,
    authenticated: &Option<String>,
    op_nonce_hex: &mut Option<String>,
) -> Result<Option<Value>, DeliveryError> {
    let session = authenticated
        .as_ref()
        .ok_or_else(|| DeliveryError::Unauthorized("session not ready".into()))?;
    let message_id = frame
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::BadRequest("missing message_id".into()))?;
    let extend_secs = frame
        .get("extend_secs")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| DeliveryError::BadRequest("missing extend_secs".into()))?;
    if extend_secs < state.policy.min_ttl_secs || extend_secs > state.policy.max_ttl_secs {
        return Err(DeliveryError::TtlInvalid(format!(
            "extend_secs outside policy bounds"
        )));
    }

    let op_nonce = parse_nonce_hex(
        frame
            .get("op_nonce_hex")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DeliveryError::BadRequest("missing op_nonce_hex".into()))?,
    )?;
    let sig = parse_signature_hex(
        frame
            .get("signature_hex")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DeliveryError::BadRequest("missing signature_hex".into()))?,
    )?;
    let extend_msg = extend_challenge_bytes(&op_nonce, message_id);
    verify_signature(session, &extend_msg, &sig)?;
    {
        let mut ch = state.challenges.lock().map_err(|_| {
            DeliveryError::Internal("challenge mutex poisoned".into())
        })?;
        ch.take_op_valid(session, &op_nonce)?;
        let new_nonce = ch.issue_op_nonce(session, Duration::from_secs(60))?;
        *op_nonce_hex = Some(hex::encode(new_nonce));
    }

    let row = state
        .store
        .extend_ttl(session, message_id, extend_secs, &state.policy)?;

    Ok(Some(json!({
        "type": "mailbox.ttl.extended",
        "row": row,
        "op_nonce_hex": op_nonce_hex,
    })))
}

async fn handle_quota(
    state: &Arc<WsState>,
    authenticated: &Option<String>,
) -> Result<Option<Value>, DeliveryError> {
    let session = authenticated
        .as_ref()
        .ok_or_else(|| DeliveryError::Unauthorized("session not ready".into()))?;
    let quota = state.store.quota_status(session)?;
    Ok(Some(json!({
        "type": "quota.status",
        "allocated_bytes": quota.allocated_bytes,
        "used_bytes": quota.used_bytes,
        "pending_count": quota.pending_count,
    })))
}

fn error_frame(code: &str, message: &str, request_id: Option<Value>) -> Value {
    let mut v = json!({
        "type": "error",
        "code": code,
        "message": message,
    });
    if let Some(rid) = request_id {
        v["request_id"] = rid;
    }
    v
}

pub fn spawn_ttl_sweeper(store: Arc<MailboxStore>, registry: SessionRegistry) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Ok(expired) = store.sweep_expired() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                for (sender, message_id, recipient) in expired {
                    registry
                        .push(
                            &sender,
                            json!({
                                "type": "message.expired",
                                "message_id": message_id,
                                "recipient_wire": recipient,
                                "expired_at_ms": now,
                            }),
                        )
                        .await;
                }
            }
        }
    });
}
