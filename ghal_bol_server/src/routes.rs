use crate::AppState;
use crate::auth::{ChallengeStore, verify_registration_signature};
use crate::error::{ApiResult, ServerError};
use crate::presence::{PeerEndpoint, PeerRecord};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use std::net::ToSocketAddrs;
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
        .route("/v1/peers/{public_key_hex}", get(get_peer))
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

/// Advertise the co-located relay so clients can reserve a circuit and register it in presence.
async fn get_relay(State(state): State<Arc<RouteState>>) -> Json<RelayResponse> {
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
struct HealthResponse {
    ok: bool,
    service: &'static str,
    database: bool,
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
        service: "ghal_bol_server",
        database,
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
    let pk = req.public_key_hex.trim().to_ascii_lowercase();
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
    let pk = req.public_key_hex.trim().to_ascii_lowercase();
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
    let endpoints = expand_libp2p_dns4_circuit_endpoints(req.endpoints);

    let store = Arc::clone(&state.app.presence);
    let peer =
        tokio::task::spawn_blocking(move || store.upsert(pk, endpoints, caps, req.ipv6, req.ipv4))
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
    let pk = req.public_key_hex.clone();
    let peer = tokio::task::spawn_blocking(move || store.heartbeat(&pk))
        .await
        .map_err(|e| ServerError::Internal(format!("task join: {e}")))??;
    tracing::debug!(public_key = %peer.public_key_hex, "heartbeat");
    Ok(Json(HeartbeatResponse { ok: true, peer }))
}

async fn get_peer(
    State(state): State<Arc<RouteState>>,
    Path(public_key_hex): Path<String>,
) -> ApiResult<Json<PeerRecord>> {
    let store = Arc::clone(&state.app.presence);
    let ttl = state.app.config.presence_ttl;
    let pk = public_key_hex.clone();
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

/// Duplicate `/dns4|/dns6|/dns/…/p2p-circuit` libp2p endpoints with resolved `/ip6/…` **and**
/// `/ip4/…` aliases so TCP-only clients (Android has no libp2p DNS transport) can dial the relay
/// circuit by concrete IP. Both families are emitted — IPv6 first (preferred when reachable) — so a
/// dual-stack or IPv6-only dialer can use the IPv6 alias and an IPv4 dialer the IPv4 one.
fn expand_libp2p_dns4_circuit_endpoints(endpoints: Vec<PeerEndpoint>) -> Vec<PeerEndpoint> {
    let mut out = endpoints;
    let mut extra: Vec<PeerEndpoint> = Vec::new();
    for ep in &out {
        if ep.scheme != "libp2p" || !ep.host.contains("/p2p-circuit") {
            continue;
        }
        for host in resolve_libp2p_circuit_dns_to_ip(&ep.host) {
            if !out.iter().any(|e| e.scheme == "libp2p" && e.host == host)
                && !extra.iter().any(|e| e.host == host)
            {
                extra.push(PeerEndpoint {
                    scheme: "libp2p".into(),
                    host,
                    port: 0,
                });
            }
        }
    }
    out.extend(extra);
    out
}

/// Resolve the `/dns*` relay hop of a circuit multiaddr into concrete `/ip6/…` and `/ip4/…`
/// circuit multiaddrs (IPv6 first). Empty when the host is already a literal IP or unresolvable.
fn resolve_libp2p_circuit_dns_to_ip(host: &str) -> Vec<String> {
    if host.contains("/ip4/") || host.contains("/ip6/") {
        return Vec::new();
    }
    let segs: Vec<&str> = host.split('/').filter(|s| !s.is_empty()).collect();
    let mut dns_host: Option<&str> = None;
    let mut port: Option<u16> = None;
    let mut p2p_idx: Option<usize> = None;
    let mut i = 0;
    while i < segs.len() {
        match segs[i] {
            "dns4" | "dns6" | "dns" | "dnsaddr" => {
                if i + 1 < segs.len() {
                    dns_host = Some(segs[i + 1]);
                    i += 2;
                    continue;
                }
            }
            "tcp" => {
                if i + 1 < segs.len() {
                    port = segs[i + 1].parse().ok();
                    i += 2;
                    continue;
                }
            }
            "p2p" => {
                p2p_idx = Some(i);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let (Some(dns), Some(p), Some(p2p_start)) = (dns_host, port, p2p_idx) else {
        return Vec::new();
    };
    let suffix = format!("/{}", segs[p2p_start..].join("/"));
    let Ok(resolved) = format!("{dns}:{p}").to_socket_addrs() else {
        return Vec::new();
    };
    let mut v6 = Vec::new();
    let mut v4 = Vec::new();
    for sa in resolved {
        match sa.ip() {
            std::net::IpAddr::V6(ip) => {
                let a = format!("/ip6/{ip}/tcp/{p}{suffix}");
                if !v6.contains(&a) {
                    v6.push(a);
                }
            }
            std::net::IpAddr::V4(ip) if !ip.is_private() && !ip.is_loopback() => {
                let a = format!("/ip4/{ip}/tcp/{p}{suffix}");
                if !v4.contains(&a) {
                    v4.push(a);
                }
            }
            std::net::IpAddr::V4(_) => {}
        }
    }
    // IPv6 first (preferred), then IPv4.
    v6.extend(v4);
    v6
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
