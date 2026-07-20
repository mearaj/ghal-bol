use crate::AppState;
use crate::auth::{ChallengeStore, verify_registration_signature};
use crate::bridge::{
    bridge_challenge, get_bridge_pending, post_bridge_request, ws_bridge_connect,
};
use crate::identity::{normalize_identity_wire, percent_decode_uri_component};
use crate::error::{ApiResult, ServerError};
use crate::presence::{PeerEndpoint, PeerRecord};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Challenge store (ephemeral; not persisted).
pub struct RouteState {
    pub app: Arc<AppState>,
    pub challenges: Mutex<ChallengeStore>,
}

pub fn router(app: Arc<AppState>) -> Router {
    let state = Arc::new(RouteState {
        app: Arc::clone(&app),
        challenges: Mutex::new(ChallengeStore::default()),
    });

    Router::new()
        .route("/health", get(health))
        .route("/v1/register/challenge", post(register_challenge))
        .route("/v1/register", post(register))
        .route("/v1/heartbeat", post(heartbeat))
        .route("/v1/peers/{identity_wire}", get(get_peer))
        .route("/v1/peers", get(list_peers))
        .route("/v1/bridge/challenge", post(bridge_challenge))
        .route("/v1/bridge/request", post(post_bridge_request))
        .route("/v1/bridge/pending", get(get_bridge_pending))
        .route("/v1/bridge/connect", get(ws_bridge_connect))
        .with_state(state)
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    database: bool,
    bridge: bool,
}

async fn health(State(state): State<Arc<RouteState>>) -> Json<HealthResponse> {
    let db = Arc::clone(&state.app.presence);
    let database = tokio::task::spawn_blocking(move || db.ping())
        .await
        .ok()
        .and_then(|r| r.ok())
        .is_some();
    Json(HealthResponse {
        ok: database,
        service: "ghal_bol_coord",
        database,
        bridge: true,
    })
}

#[derive(Deserialize)]
struct ChallengeRequest {
    public_key_hex: String,
}

#[derive(Serialize)]
struct ChallengeResponse {
    public_key_hex: String,
    nonce_hex: String,
    expires_in_secs: u64,
    message_domain: &'static str,
}

async fn register_challenge(
    State(state): State<Arc<RouteState>>,
    Json(req): Json<ChallengeRequest>,
) -> ApiResult<Json<ChallengeResponse>> {
    let mut ch = state.challenges.lock().await;
    ch.purge_expired();
    let pending = ch.issue(&req.public_key_hex, state.app.config.challenge_ttl)?;
    let pk = normalize_identity_wire(&req.public_key_hex)?;
    Ok(Json(ChallengeResponse {
        public_key_hex: pk,
        nonce_hex: hex::encode(pending.nonce),
        expires_in_secs: state.app.config.challenge_ttl.as_secs(),
        message_domain: "ghal_bol:register:v1",
    }))
}

#[derive(Deserialize)]
struct RegisterRequest {
    public_key_hex: String,
    nonce_hex: String,
    signature_hex: String,
    endpoints: Vec<PeerEndpoint>,
    #[serde(default)]
    transport_capabilities: Vec<String>,
    ipv6: Option<String>,
    ipv4: Option<String>,
}

#[derive(Serialize)]
struct RegisterResponse {
    ok: bool,
    peer: PeerRecord,
}

async fn register(
    State(state): State<Arc<RouteState>>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<Json<RegisterResponse>> {
    let pk = normalize_identity_wire(&req.public_key_hex)?;
    let nonce: [u8; 32] = decode_fixed_32(&req.nonce_hex, "nonce_hex")?;
    let sig = hex::decode(req.signature_hex.trim())
        .map_err(|e| ServerError::BadRequest(format!("signature_hex: {e}")))?;

    {
        let mut ch = state.challenges.lock().await;
        ch.take_valid(&pk, &nonce)?;
        verify_registration_signature(&pk, &nonce, &sig)?;
    }

    let caps = if req.transport_capabilities.is_empty() {
        vec!["tcp".into(), "sync-v1".into()]
    } else {
        req.transport_capabilities
    };

    validate_endpoints(&req.endpoints)?;
    let endpoints = req.endpoints;

    let store = Arc::clone(&state.app.presence);
    let peer = tokio::task::spawn_blocking(move || {
        store.merge_client_register(pk, endpoints, caps, req.ipv6, req.ipv4)
    })
    .await
    .map_err(|e| ServerError::Internal(format!("task join: {e}")))??;

    tracing::info!(
        public_key = %peer.public_key_hex,
        endpoints = peer.endpoints.len(),
        "peer registered"
    );
    Ok(Json(RegisterResponse { ok: true, peer }))
}

#[derive(Deserialize)]
struct HeartbeatRequest {
    public_key_hex: String,
}

#[derive(Serialize)]
struct HeartbeatResponse {
    ok: bool,
    peer: PeerRecord,
}

async fn heartbeat(
    State(state): State<Arc<RouteState>>,
    Json(req): Json<HeartbeatRequest>,
) -> ApiResult<Json<HeartbeatResponse>> {
    let store = Arc::clone(&state.app.presence);
    let pk = normalize_identity_wire(&req.public_key_hex)?;
    let peer = tokio::task::spawn_blocking(move || store.heartbeat(&pk))
        .await
        .map_err(|e| ServerError::Internal(format!("task join: {e}")))??;
    tracing::debug!(public_key = %peer.public_key_hex, "heartbeat");
    Ok(Json(HeartbeatResponse { ok: true, peer }))
}

async fn get_peer(
    State(state): State<Arc<RouteState>>,
    Path(identity_wire): Path<String>,
) -> ApiResult<Json<PeerRecord>> {
    let store = Arc::clone(&state.app.presence);
    let ttl = state.app.config.presence_ttl;
    let decoded = percent_decode_uri_component(&identity_wire);
    let pk = normalize_identity_wire(&decoded)?;
    let peer = tokio::task::spawn_blocking(move || store.get(&pk, ttl))
        .await
        .map_err(|e| ServerError::Internal(format!("task join: {e}")))??;
    Ok(Json(peer))
}

#[derive(Serialize)]
struct PeerListResponse {
    peers: Vec<PeerRecord>,
}

async fn list_peers(State(state): State<Arc<RouteState>>) -> ApiResult<Json<PeerListResponse>> {
    let store = Arc::clone(&state.app.presence);
    let ttl = state.app.config.presence_ttl;
    let peers = tokio::task::spawn_blocking(move || store.list_online(ttl))
        .await
        .map_err(|e| ServerError::Internal(format!("task join: {e}")))??;
    Ok(Json(PeerListResponse { peers }))
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

fn validate_endpoints(endpoints: &[PeerEndpoint]) -> ApiResult<()> {
    if endpoints.is_empty() {
        return Err(ServerError::BadRequest("endpoints empty".into()));
    }
    if endpoints.len() > 24 {
        return Err(ServerError::BadRequest("too many endpoints".into()));
    }
    for ep in endpoints {
        match ep.scheme.as_str() {
            "tcp" | "quic" => {
                if ep.host.trim().is_empty() {
                    return Err(ServerError::BadRequest("endpoint host empty".into()));
                }
                if ep.port == 0 {
                    return Err(ServerError::BadRequest("endpoint port required".into()));
                }
                if ep.scheme == "tcp" && !is_public_routable_tcp_host(&ep.host) {
                    return Err(ServerError::BadRequest(
                        "coord register accepts public routable IPv4 TCP only — \
                         LAN uses mDNS; WAN text uses delivery server"
                            .into(),
                    ));
                }
            }
            other => {
                return Err(ServerError::BadRequest(format!(
                    "unsupported endpoint scheme: {other}"
                )));
            }
        }
    }
    Ok(())
}

fn decode_fixed_32(hex_s: &str, field: &str) -> ApiResult<[u8; 32]> {
    let bytes = Vec::from_hex(hex_s.trim())
        .map_err(|e| ServerError::BadRequest(format!("{field}: {e}")))?;
    if bytes.len() != 32 {
        return Err(ServerError::BadRequest(format!(
            "{field}: expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}
