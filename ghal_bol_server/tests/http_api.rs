//! In-process HTTP surface checks only — not P2P/WAN/LAN connectivity.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ghal_bol_server::{AppState, RelayInfo, ServerConfig, router};
use std::sync::Arc;
use tower::ServiceExt;

fn test_app() -> axum::Router {
    let config = ServerConfig::default();
    let state = Arc::new(AppState::open_in_memory(config).expect("in-memory db"));
    router(state)
}

async fn json_request(
    app: &axum::Router,
    method: &str,
    uri: &str,
) -> axum::response::Response {
    let req = Request::builder().method(method).uri(uri);
    app.clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_ok_when_relay_advertised() {
    let config = ServerConfig::default();
    let state = Arc::new(AppState::open_in_memory(config).expect("in-memory db"));
    state.set_relay_info(RelayInfo {
        peer_id: "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X".to_string(),
        addrs: vec!["/dns4/coord.ghalbol.com/tcp/4002".to_string()],
    });
    let app = router(state);
    let resp = json_request(&app, "GET", "/health").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_body(resp).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["database"], true);
    assert_eq!(v["relay"]["running"], true);
    assert_eq!(v["relay"]["wan_ready"], true);
}

#[tokio::test]
async fn health_not_wan_ready_without_relay() {
    let app = test_app();
    let resp = json_request(&app, "GET", "/health").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_body(resp).await;
    assert_eq!(v["database"], true);
    assert_eq!(v["ok"], false);
    assert_eq!(v["relay"]["wan_ready"], false);
}

#[tokio::test]
async fn relay_disabled_by_default_then_advertised() {
    let config = ServerConfig::default();
    let state = Arc::new(AppState::open_in_memory(config).expect("db"));
    let app = router(Arc::clone(&state));

    let resp = json_request(&app, "GET", "/v1/relay").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_body(resp).await;
    assert_eq!(v["enabled"], false);

    state.set_relay_info(RelayInfo {
        peer_id: "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X".to_string(),
        addrs: vec!["/dns4/coord.ghalbol.com/tcp/4002".to_string()],
    });
    let resp = json_request(&app, "GET", "/v1/relay").await;
    let v = json_body(resp).await;
    assert_eq!(v["enabled"], true);
    assert_eq!(
        v["peer_id"],
        "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X"
    );
    assert_eq!(v["addrs"][0], "/dns4/coord.ghalbol.com/tcp/4002");
}

async fn json_post(app: &axum::Router, uri: &str, body: serde_json::Value) -> axum::response::Response {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    app.clone()
        .oneshot(
            req.body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn bare_secp256k1_implicit_register_and_lookup() {
    use ghal_bol_server::registration_message_digest;
    use secp256k1::{Secp256k1, SecretKey};

    let app = test_app();
    let sk = SecretKey::from_byte_array([7u8; 32]).expect("sk");
    let secp = Secp256k1::new();
    let wire = hex::encode(sk.public_key(&secp).serialize());

    let ch_resp = json_post(
        &app,
        "/v1/register/challenge",
        serde_json::json!({ "public_key_hex": wire }),
    )
    .await;
    assert_eq!(ch_resp.status(), StatusCode::OK);
    let ch = json_body(ch_resp).await;
    assert_eq!(ch["public_key_hex"], wire);
    let nonce_hex = ch["nonce_hex"].as_str().unwrap();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).unwrap());
    let msg = registration_message_digest(&nonce, &wire);
    let sig = hex::encode(secp.sign_ecdsa(msg, &sk).serialize_der());

    let reg_resp = json_post(
        &app,
        "/v1/register",
        serde_json::json!({
            "public_key_hex": wire,
            "nonce_hex": nonce_hex,
            "signature_hex": sig,
            "endpoints": [{ "scheme": "tcp", "host": "203.0.113.12", "port": 4435 }],
            "transport_capabilities": ["tcp", "sync-v1"]
        }),
    )
    .await;
    assert_eq!(reg_resp.status(), StatusCode::OK);

    let lookup_resp = json_request(&app, "GET", &format!("/v1/peers/{wire}")).await;
    assert_eq!(lookup_resp.status(), StatusCode::OK);
    let peer = json_body(lookup_resp).await;
    assert_eq!(peer["public_key_hex"], wire);
    assert_eq!(peer["endpoints"][0]["host"], "203.0.113.12");
}

#[tokio::test]
async fn explicit_secp256k1_prefix_normalizes_to_bare_on_store() {
    use ghal_bol_server::registration_message_digest;
    use secp256k1::{Secp256k1, SecretKey};

    let app = test_app();
    let sk = SecretKey::from_byte_array([8u8; 32]).expect("sk");
    let secp = Secp256k1::new();
    let bare = hex::encode(sk.public_key(&secp).serialize());
    let wire = format!("secp256k1:{bare}");

    let ch_resp = json_post(
        &app,
        "/v1/register/challenge",
        serde_json::json!({ "public_key_hex": wire }),
    )
    .await;
    assert_eq!(ch_resp.status(), StatusCode::OK);
    let ch = json_body(ch_resp).await;
    assert_eq!(ch["public_key_hex"], bare);
    let nonce_hex = ch["nonce_hex"].as_str().unwrap();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).unwrap());
    let msg = registration_message_digest(&nonce, &bare);
    let sig = hex::encode(secp.sign_ecdsa(msg, &sk).serialize_der());

    let reg_resp = json_post(
        &app,
        "/v1/register",
        serde_json::json!({
            "public_key_hex": wire,
            "nonce_hex": nonce_hex,
            "signature_hex": sig,
            "endpoints": [{ "scheme": "tcp", "host": "203.0.113.13", "port": 4436 }],
            "transport_capabilities": ["tcp", "sync-v1"]
        }),
    )
    .await;
    assert_eq!(reg_resp.status(), StatusCode::OK);
    let reg = json_body(reg_resp).await;
    assert_eq!(reg["peer"]["public_key_hex"], bare);

    let lookup_resp = json_request(&app, "GET", &format!("/v1/peers/{bare}")).await;
    assert_eq!(lookup_resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn ed25519_identity_register_and_lookup() {
    use ed25519_dalek::Signer;
    use ghal_bol_server::registration_challenge_bytes;

    let app = test_app();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
    let wire = format!("ed25519:{}", hex::encode(signing.verifying_key().to_bytes()));

    let ch_resp = json_post(
        &app,
        "/v1/register/challenge",
        serde_json::json!({ "public_key_hex": wire }),
    )
    .await;
    assert_eq!(ch_resp.status(), StatusCode::OK);
    let ch = json_body(ch_resp).await;
    let nonce_hex = ch["nonce_hex"].as_str().unwrap();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).unwrap());
    let challenge = registration_challenge_bytes(&nonce, &wire);
    let sig = hex::encode(signing.sign(&challenge).to_bytes());

    let reg_resp = json_post(
        &app,
        "/v1/register",
        serde_json::json!({
            "public_key_hex": wire,
            "nonce_hex": nonce_hex,
            "signature_hex": sig,
            "endpoints": [{ "scheme": "tcp", "host": "203.0.113.10", "port": 4433 }],
            "transport_capabilities": ["tcp", "sync-v1"]
        }),
    )
    .await;
    assert_eq!(reg_resp.status(), StatusCode::OK);

    let encoded = wire.replace(':', "%3A");
    let lookup_resp = json_request(&app, "GET", &format!("/v1/peers/{encoded}")).await;
    assert_eq!(lookup_resp.status(), StatusCode::OK);
    let peer = json_body(lookup_resp).await;
    assert_eq!(peer["public_key_hex"], wire);
}

#[tokio::test]
async fn ecdsa_p256_identity_register_and_lookup() {
    use ghal_bol_server::registration_challenge_bytes;
    use p256::ecdsa::{signature::Signer, SigningKey};

    let app = test_app();
    let signing = SigningKey::from_slice(&[11u8; 32]).expect("test ecdsa-p256 key");
    let verifying = signing.verifying_key();
    let wire = format!(
        "ecdsa-p256:{}",
        hex::encode(verifying.to_encoded_point(false).as_bytes())
    );

    let ch_resp = json_post(
        &app,
        "/v1/register/challenge",
        serde_json::json!({ "public_key_hex": wire }),
    )
    .await;
    assert_eq!(ch_resp.status(), StatusCode::OK);
    let ch = json_body(ch_resp).await;
    let nonce_hex = ch["nonce_hex"].as_str().unwrap();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).unwrap());
    let challenge = registration_challenge_bytes(&nonce, &wire);
    let sig: p256::ecdsa::Signature = signing.sign(&challenge);
    let sig = hex::encode(sig.to_der());

    let reg_resp = json_post(
        &app,
        "/v1/register",
        serde_json::json!({
            "public_key_hex": wire,
            "nonce_hex": nonce_hex,
            "signature_hex": sig,
            "endpoints": [{ "scheme": "tcp", "host": "203.0.113.11", "port": 4434 }],
            "transport_capabilities": ["tcp", "sync-v1"]
        }),
    )
    .await;
    assert_eq!(reg_resp.status(), StatusCode::OK);

    let encoded = wire.replace(':', "%3A");
    let lookup_resp = json_request(&app, "GET", &format!("/v1/peers/{encoded}")).await;
    assert_eq!(lookup_resp.status(), StatusCode::OK);
    let peer = json_body(lookup_resp).await;
    assert_eq!(peer["public_key_hex"], wire);
}

#[tokio::test]
async fn ml_dsa65_identity_register_and_lookup() {
    use ghal_bol_server::ml_dsa_identity;
    use ghal_bol_server::registration_challenge_bytes;

    let app = test_app();
    let seed = ml_dsa_identity::generate_secret_seed();
    let sk = ml_dsa_identity::signing_key_from_seed_bytes(&seed).unwrap();
    let pk = ml_dsa_identity::public_key_bytes_from_seed(&seed).unwrap();
    let wire = format!("ml-dsa-65:{}", hex::encode(&pk));

    let ch_resp = json_post(
        &app,
        "/v1/register/challenge",
        serde_json::json!({ "public_key_hex": wire }),
    )
    .await;
    assert_eq!(ch_resp.status(), StatusCode::OK);
    let ch = json_body(ch_resp).await;
    let nonce_hex = ch["nonce_hex"].as_str().unwrap();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).unwrap());
    let challenge = registration_challenge_bytes(&nonce, &wire);
    let sig = hex::encode(ml_dsa_identity::sign_message(&sk, &challenge).unwrap());

    let reg_resp = json_post(
        &app,
        "/v1/register",
        serde_json::json!({
            "public_key_hex": wire,
            "nonce_hex": nonce_hex,
            "signature_hex": sig,
            "endpoints": [{ "scheme": "tcp", "host": "203.0.113.11", "port": 4434 }],
            "transport_capabilities": ["tcp", "sync-v1"]
        }),
    )
    .await;
    assert_eq!(reg_resp.status(), StatusCode::OK);

    let encoded = wire.replace(':', "%3A");
    let lookup_resp = json_request(&app, "GET", &format!("/v1/peers/{encoded}")).await;
    assert_eq!(lookup_resp.status(), StatusCode::OK);
    let peer = json_body(lookup_resp).await;
    assert_eq!(peer["public_key_hex"], wire);
}
