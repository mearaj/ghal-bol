//! WAN call byte bridge — pairs two outbound WebSocket connections (`docs/GHAL_BOL_CONNECT_V1.md`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::Json;
use futures::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};

use crate::bridge_auth::verify_bridge_request_signature;
use crate::error::{ApiResult, ServerError};
use crate::identity::normalize_identity_wire;
use crate::routes::RouteState;

#[derive(Clone)]
pub struct BridgeRegistry {
    inner: Arc<Mutex<BridgeInner>>,
    max_secs: u64,
    max_bytes: u64,
    max_per_peer: usize,
    idle_secs: u64,
}

struct BridgeInner {
    pending: HashMap<String, PendingBridge>,
    active: HashMap<String, ActiveBridge>,
}

#[derive(Clone)]
struct PendingBridge {
    bridge_id: String,
    caller_wire: String,
    callee_wire: String,
    call_id: String,
    caller_token: String,
    callee_token: String,
    expires: Instant,
}

struct ActiveBridge {
    caller_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
    callee_tx: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
    bytes_relayed: u64,
    notify: Arc<Notify>,
}

impl BridgeRegistry {
    pub fn from_env() -> Self {
        let max_secs = std::env::var("GHAL_BOL_BRIDGE_MAX_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4 * 3600);
        let max_bytes = std::env::var("GHAL_BOL_BRIDGE_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let max_per_peer = std::env::var("GHAL_BOL_BRIDGE_MAX_PER_PEER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);
        let idle_secs = std::env::var("GHAL_BOL_BRIDGE_IDLE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);
        Self {
            inner: Arc::new(Mutex::new(BridgeInner {
                pending: HashMap::new(),
                active: HashMap::new(),
            })),
            max_secs,
            max_bytes,
            max_per_peer,
            idle_secs,
        }
    }

    fn ttl(&self) -> Duration {
        Duration::from_secs(self.max_secs.min(24 * 3600))
    }

    pub async fn purge_expired(&self) {
        let now = Instant::now();
        let mut g = self.inner.lock().await;
        g.pending.retain(|_, b| b.expires > now);
    }

    async fn count_for_peer(&self, wire: &str) -> usize {
        let g = self.inner.lock().await;
        let w = wire.to_ascii_lowercase();
        g.pending
            .values()
            .filter(|b| b.caller_wire.to_ascii_lowercase() == w || b.callee_wire.to_ascii_lowercase() == w)
            .count()
            + g.active
                .values()
                .filter(|_| false)
                .count()
    }
}

fn random_token() -> String {
    let mut buf = [0u8; 32];
    rand::rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

fn random_bridge_id() -> String {
    let mut buf = [0u8; 16];
    rand::rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

#[derive(Deserialize)]
pub struct BridgeRequestBody {
    pub peer_identity_wire: String,
    pub call_id: String,
    pub nonce_hex: String,
    pub signature_hex: String,
    #[serde(default)]
    pub caller_identity_wire: Option<String>,
}

#[derive(Serialize)]
pub struct BridgeRequestResponse {
    pub bridge_id: String,
    pub token: String,
    pub connect_url: String,
}

#[derive(Deserialize)]
pub struct BridgeConnectQuery {
    pub bridge_id: String,
    pub token: String,
}

#[derive(Serialize)]
pub struct BridgePendingItem {
    pub bridge_id: String,
    pub call_id: String,
    pub caller_identity_wire: String,
    pub token: String,
    pub connect_url: String,
}

#[derive(Serialize)]
pub struct BridgePendingResponse {
    pub pending: Vec<BridgePendingItem>,
}

pub async fn post_bridge_request(
    State(state): State<Arc<RouteState>>,
    Json(req): Json<BridgeRequestBody>,
) -> ApiResult<Json<BridgeRequestResponse>> {
    let caller = req
        .caller_identity_wire
        .as_deref()
        .map(normalize_identity_wire)
        .transpose()?
        .ok_or_else(|| ServerError::BadRequest("caller_identity_wire required".into()))?;
    let callee = normalize_identity_wire(&req.peer_identity_wire)?;
    let call_id = req.call_id.trim();
    if call_id.is_empty() {
        return Err(ServerError::BadRequest("call_id empty".into()));
    }
    let mut nonce = [0u8; 32];
    let nonce_bytes = hex::decode(req.nonce_hex.trim())
        .map_err(|e| ServerError::BadRequest(format!("nonce_hex: {e}")))?;
    if nonce_bytes.len() != 32 {
        return Err(ServerError::BadRequest("nonce must be 32 bytes".into()));
    }
    nonce.copy_from_slice(&nonce_bytes);
    {
        let mut ch = state.challenges.lock().await;
        ch.take_valid(&caller, &nonce)?;
    }
    let sig = hex::decode(req.signature_hex.trim())
        .map_err(|e| ServerError::BadRequest(format!("signature_hex: {e}")))?;
    verify_bridge_request_signature(&caller, &nonce, &callee, call_id, &sig)?;

    let registry = state.app.bridge.clone();
    registry.purge_expired().await;
    if registry.count_for_peer(&caller).await >= registry.max_per_peer {
        return Err(ServerError::BadRequest("bridge limit per identity".into()));
    }

    let bridge_id = random_bridge_id();
    let caller_token = random_token();
    let callee_token = random_token();
    let now = Instant::now();
    let pending = PendingBridge {
        bridge_id: bridge_id.clone(),
        caller_wire: caller.clone(),
        callee_wire: callee.clone(),
        call_id: call_id.to_string(),
        caller_token: caller_token.clone(),
        callee_token: callee_token.clone(),
        expires: now + registry.ttl(),
    };
    tracing::info!(
        bridge_id = %bridge_id,
        caller = %caller,
        callee = %callee,
        call_id = %call_id,
        "bridge requested"
    );
    {
        let mut g = registry.inner.lock().await;
        g.pending.insert(bridge_id.clone(), pending);
    }

    let connect_url = format!(
        "{}/v1/bridge/connect",
        state.app.config.public_base_url.trim_end_matches('/')
    );
    Ok(Json(BridgeRequestResponse {
        bridge_id,
        token: caller_token,
        connect_url,
    }))
}

pub async fn get_bridge_pending(
    State(state): State<Arc<RouteState>>,
    Query(q): Query<BridgePendingQuery>,
) -> ApiResult<Json<BridgePendingResponse>> {
    let wire = normalize_identity_wire(&q.identity_wire)?;
    let registry = state.app.bridge.clone();
    registry.purge_expired().await;
    let connect_url = format!(
        "{}/v1/bridge/connect",
        state.app.config.public_base_url.trim_end_matches('/')
    );
    let g = registry.inner.lock().await;
    let w = wire.to_ascii_lowercase();
    let pending: Vec<BridgePendingItem> = g
        .pending
        .values()
        .filter(|b| b.callee_wire.to_ascii_lowercase() == w)
        .map(|b| BridgePendingItem {
            bridge_id: b.bridge_id.clone(),
            call_id: b.call_id.clone(),
            caller_identity_wire: b.caller_wire.clone(),
            token: b.callee_token.clone(),
            connect_url: connect_url.clone(),
        })
        .collect();
    Ok(Json(BridgePendingResponse { pending }))
}

#[derive(Deserialize)]
pub struct BridgePendingQuery {
    pub identity_wire: String,
}

pub async fn ws_bridge_connect(
    ws: WebSocketUpgrade,
    State(state): State<Arc<RouteState>>,
    Query(q): Query<BridgeConnectQuery>,
) -> impl IntoResponse {
    let registry = state.app.bridge.clone();
    ws.on_upgrade(move |socket| handle_bridge_socket(socket, registry, q))
}

async fn handle_bridge_socket(
    socket: WebSocket,
    registry: BridgeRegistry,
    q: BridgeConnectQuery,
) {
    let bridge_id = q.bridge_id.trim().to_string();
    let token = q.token.trim().to_string();
    if bridge_id.is_empty() || token.is_empty() {
        return;
    }

    let role = {
        let g = registry.inner.lock().await;
        if let Some(p) = g.pending.get(&bridge_id) {
            if p.caller_token == token {
                Some(("caller", p.clone()))
            } else if p.callee_token == token {
                Some(("callee", p.clone()))
            } else {
                None
            }
        } else {
            None
        }
    };
    let Some((role, _pending)) = role else {
        tracing::warn!(bridge_id = %bridge_id, "bridge connect rejected — bad token");
        return;
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let notify = Arc::new(Notify::new());
    let paired = {
        let mut g = registry.inner.lock().await;
        if let Some(active) = g.active.get_mut(&bridge_id) {
            if role == "caller" {
                active.caller_tx = Some(tx.clone());
            } else {
                active.callee_tx = Some(tx.clone());
            }
            active.caller_tx.is_some() && active.callee_tx.is_some()
        } else {
            let mut active = ActiveBridge {
                caller_tx: None,
                callee_tx: None,
                bytes_relayed: 0,
                notify: Arc::clone(&notify),
            };
            if role == "caller" {
                active.caller_tx = Some(tx.clone());
            } else {
                active.callee_tx = Some(tx.clone());
            }
            let paired = active.caller_tx.is_some() && active.callee_tx.is_some();
            g.active.insert(bridge_id.clone(), active);
            if paired {
                g.pending.remove(&bridge_id);
            }
            paired
        }
    };

    if paired {
        tracing::info!(bridge_id = %bridge_id, "bridge paired");
        if let Some(n) = {
            let g = registry.inner.lock().await;
            g.active.get(&bridge_id).map(|a| Arc::clone(&a.notify))
        } {
            n.notify_waiters();
        }
    }

    let (mut ws_tx, mut ws_rx) = socket.split();
    let bridge_id_c = bridge_id.clone();
    let registry_c = registry.clone();
    let relay_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if ws_tx.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
        let _ = bridge_id_c;
        let _ = registry_c;
    });

    let started = Instant::now();
    let idle = Duration::from_secs(registry.idle_secs);
    let max_duration = Duration::from_secs(registry.max_secs);
    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let peer_tx = {
                            let g = registry.inner.lock().await;
                            g.active.get(&bridge_id).and_then(|a| {
                                if role == "caller" {
                                    a.callee_tx.clone()
                                } else {
                                    a.caller_tx.clone()
                                }
                            })
                        };
                        if let Some(peer_tx) = peer_tx {
                            let len = data.len() as u64;
                            let over = {
                                let mut g = registry.inner.lock().await;
                                if let Some(a) = g.active.get_mut(&bridge_id) {
                                    a.bytes_relayed = a.bytes_relayed.saturating_add(len);
                                    registry.max_bytes > 0 && a.bytes_relayed > registry.max_bytes
                                } else {
                                    true
                                }
                            };
                            if over {
                                break;
                            }
                            let _ = peer_tx.send(data.to_vec());
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            _ = tokio::time::sleep(idle) => {
                break;
            }
        }
        if started.elapsed() > max_duration {
            break;
        }
    }

    relay_task.abort();
    let mut g = registry.inner.lock().await;
    g.active.remove(&bridge_id);
    tracing::info!(
        bridge_id = %bridge_id,
        role = %role,
        duration_ms = started.elapsed().as_millis(),
        "bridge closed"
    );
}

/// Issue a bridge challenge nonce (reuse registration challenge store pattern).
pub async fn bridge_challenge(
    State(state): State<Arc<RouteState>>,
    Json(req): Json<BridgeChallengeRequest>,
) -> ApiResult<Json<BridgeChallengeResponse>> {
    let mut ch = state.challenges.lock().await;
    ch.purge_expired();
    let pending = ch.issue(&req.caller_identity_wire, state.app.config.challenge_ttl)?;
    let pk = normalize_identity_wire(&req.caller_identity_wire)?;
    Ok(Json(BridgeChallengeResponse {
        caller_identity_wire: pk,
        nonce_hex: hex::encode(pending.nonce),
        expires_in_secs: state.app.config.challenge_ttl.as_secs(),
        message_domain: "ghal_bol:bridge:request:v1",
    }))
}

#[derive(Deserialize)]
pub struct BridgeChallengeRequest {
    pub caller_identity_wire: String,
}

#[derive(Serialize)]
pub struct BridgeChallengeResponse {
    pub caller_identity_wire: String,
    pub nonce_hex: String,
    pub expires_in_secs: u64,
    pub     message_domain: &'static str,
}
