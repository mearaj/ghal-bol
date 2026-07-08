use crate::AppState;
use crate::auth::{ChallengeStore, verify_registration_signature};
use crate::identity::{normalize_identity_wire, percent_decode_uri_component};
use crate::error::{ApiResult, ServerError};
use crate::presence::{PeerEndpoint, PeerRecord};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
        .route("/v1/relay", get(get_relay))
        .route("/v1/register/challenge", post(register_challenge))
        .route("/v1/register", post(register))
        .route("/v1/heartbeat", post(heartbeat))
        .route("/v1/peers/{identity_wire}", get(get_peer))
        .route("/v1/peers", get(list_peers))
        .with_state(state)
}

#[derive(Serialize)]
struct RelayResponse {
    /// Whether this coordinator runs a Circuit Relay v2 node.
    enabled: bool,
    /// Relay libp2p PeerId (stable across restarts).
    peer_id: Option<String>,
    /// Dialable base multiaddrs (without `/p2p/<id>`); clients append `/p2p/<peer_id>/p2p-circuit`.
    addrs: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RelayQuery {
    /// When true, client bootstrap TCP failed — request throttled UPnP remap (event-driven).
    #[serde(default)]
    remap: bool,
}

/// Advertise the co-located relay so clients can reserve a circuit and register it in presence.
async fn get_relay(
    State(state): State<Arc<RouteState>>,
    Query(query): Query<RelayQuery>,
) -> Json<RelayResponse> {
    // Home UPnP: only `?remap=true` after bootstrap failure — not every relay poll (would rotate ports).
    if query.remap {
        state.app.request_upnp_remap();
    }
    let info = state.app.relay_info.lock().ok().and_then(|g| g.clone());
    match info {
        Some(i) if !i.addrs.is_empty() => Json(RelayResponse {
            enabled: true,
            peer_id: Some(i.peer_id),
            addrs: i.addrs,
        }),
        _ => Json(RelayResponse {
            enabled: false,
            peer_id: None,
            addrs: Vec::new(),
        }),
    }
}

#[derive(Serialize)]
struct HealthRelayStatus {
    /// libp2p relay task started and coordinates are published.
    running: bool,
    /// Non-empty `GET /v1/relay` addrs — required for WAN chat.
    wan_ready: bool,
    advertised_addrs: Vec<String>,
}

#[derive(Serialize)]
struct HealthResponse {
    /// `database` + relay `wan_ready`. Do not treat HTTP 200 alone as “chat works”.
    ok: bool,
    service: &'static str,
    database: bool,
    relay: HealthRelayStatus,
}

async fn health(State(state): State<Arc<RouteState>>) -> Json<HealthResponse> {
    let db = Arc::clone(&state.app.presence);
    let database = tokio::task::spawn_blocking(move || db.ping())
        .await
        .ok()
        .and_then(|r| r.ok())
        .is_some();
    let relay_info = state.app.relay_info.lock().ok().and_then(|g| g.clone());
    let relay = match relay_info {
        Some(info) => HealthRelayStatus {
            running: true,
            wan_ready: !info.addrs.is_empty(),
            advertised_addrs: info.addrs,
        },
        None => HealthRelayStatus {
            running: false,
            wan_ready: false,
            advertised_addrs: Vec::new(),
        },
    };
    Json(HealthResponse {
        ok: database && relay.wan_ready,
        service: "ghal_bol_server",
        database,
        relay,
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

    validate_endpoints(&req.endpoints, &state.app.presence.relay_bootstrap_tcp_snapshot())?;
    let endpoints = crate::endpoint_expand::expand_libp2p_circuit_endpoints(req.endpoints);

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

fn validate_endpoints(
    endpoints: &[PeerEndpoint],
    relay_bootstraps: &HashSet<String>,
) -> ApiResult<()> {
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
                         LAN uses mDNS; CGNAT/mobile WAN uses relay circuit on reservation"
                            .into(),
                    ));
                }
                if ep.scheme == "tcp"
                    && relay_bootstraps.contains(&format!("{}:{}", ep.host.trim(), ep.port))
                {
                    return Err(ServerError::BadRequest(
                        "coord register rejects relay bootstrap TCP — \
                         POST only your own inbound DM listen; relay server registers /p2p-circuit"
                            .into(),
                    ));
                }
            }
            "libp2p" => {
                if ep.host.contains("/p2p-circuit") {
                    return Err(ServerError::BadRequest(
                        "relay circuit endpoints are registered by the relay server on reservation, not POST /v1/register".into(),
                    ));
                }
                if ep.host.len() < 12 || ep.host.len() > 512 {
                    return Err(ServerError::BadRequest(
                        "libp2p multiaddr length invalid".into(),
                    ));
                }
                if !ep.host.starts_with('/') {
                    return Err(ServerError::BadRequest(
                        "libp2p endpoint host must be a multiaddr".into(),
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
