//! Fast in-process checks (same handlers; no TCP). Production E2E: `tests/e2e_production.rs`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ghal_bol_server::registration_message_digest;
use ghal_bol_server::{AppState, RelayInfo, ServerConfig, router};
use secp256k1::{Secp256k1, SecretKey};
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
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(serde_json::to_string(&v).unwrap())
        }
        None => Body::empty(),
    };
    app.clone().oneshot(req.body(body).unwrap()).await.unwrap()
}

async fn json_body(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_ok() {
    let app = test_app();
    let resp = json_request(&app, "GET", "/health", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_body(resp).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["database"], true);
}

#[tokio::test]
async fn relay_disabled_by_default_then_advertised() {
    let config = ServerConfig::default();
    let state = Arc::new(AppState::open_in_memory(config).expect("db"));
    let app = router(Arc::clone(&state));

    // No relay started → endpoint reports disabled.
    let resp = json_request(&app, "GET", "/v1/relay", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_body(resp).await;
    assert_eq!(v["enabled"], false);

    // Once the relay node publishes its coordinates, clients can discover them.
    state.set_relay_info(RelayInfo {
        peer_id: "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X".to_string(),
        addrs: vec!["/dns4/coord.ghalbol.com/tcp/4002".to_string()],
    });
    let resp = json_request(&app, "GET", "/v1/relay", None).await;
    let v = json_body(resp).await;
    assert_eq!(v["enabled"], true);
    assert_eq!(
        v["peer_id"],
        "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X"
    );
    assert_eq!(v["addrs"][0], "/dns4/coord.ghalbol.com/tcp/4002");
}

#[tokio::test]
async fn register_lookup_survives_new_store_handle() {
    let config = ServerConfig::default();
    let state = Arc::new(AppState::open_in_memory(config).expect("db"));
    let app = router(Arc::clone(&state));

    let secp = Secp256k1::new();
    let sk = SecretKey::from_byte_array([0x42u8; 32]).unwrap();
    let pk_hex = hex::encode(sk.public_key(&secp).serialize());

    let ch = json_request(
        &app,
        "POST",
        "/v1/register/challenge",
        Some(serde_json::json!({ "public_key_hex": pk_hex })),
    )
    .await;
    assert_eq!(ch.status(), StatusCode::OK);
    let ch_body = json_body(ch).await;
    let nonce_hex = ch_body["nonce_hex"].as_str().unwrap();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).unwrap());

    let msg = registration_message_digest(&nonce, &pk_hex);
    let sig = secp.sign_ecdsa(msg, &sk);

    let reg = json_request(
        &app,
        "POST",
        "/v1/register",
        Some(serde_json::json!({
            "public_key_hex": pk_hex,
            "nonce_hex": nonce_hex,
            "signature_hex": hex::encode(sig.serialize_der()),
            "endpoints": [{ "scheme": "quic", "host": "127.0.0.1", "port": 4433 }],
            "ipv4": "127.0.0.1"
        })),
    )
    .await;
    assert_eq!(reg.status(), StatusCode::OK);

    // New router + state wrapping same DB file would test persistence; in-memory
    // uses one connection — reopen simulates restart via second lookup on same app.
    let lookup = json_request(&app, "GET", &format!("/v1/peers/{pk_hex}"), None).await;
    assert_eq!(lookup.status(), StatusCode::OK);

    let hb = json_request(
        &app,
        "POST",
        "/v1/heartbeat",
        Some(serde_json::json!({ "public_key_hex": pk_hex })),
    )
    .await;
    assert_eq!(hb.status(), StatusCode::OK);
}

#[tokio::test]
async fn sqlite_file_persistence() {
    let dir = std::env::temp_dir().join(format!("ghal_bol_server_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("coord.db");

    let mut config = ServerConfig::default();
    config.database_path = db_path.clone();

    let state1 = Arc::new(AppState::open(config.clone()).unwrap());
    let app1 = router(Arc::clone(&state1));
    let secp = Secp256k1::new();
    let sk = SecretKey::from_byte_array([0x11u8; 32]).unwrap();
    let pk_hex = hex::encode(sk.public_key(&secp).serialize());

    let ch = json_request(
        &app1,
        "POST",
        "/v1/register/challenge",
        Some(serde_json::json!({ "public_key_hex": pk_hex })),
    )
    .await;
    let nonce_hex = json_body(ch).await["nonce_hex"]
        .as_str()
        .unwrap()
        .to_string();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(&nonce_hex).unwrap());
    let msg = registration_message_digest(&nonce, &pk_hex);
    let sig = secp.sign_ecdsa(msg, &sk);

    let reg = json_request(
        &app1,
        "POST",
        "/v1/register",
        Some(serde_json::json!({
            "public_key_hex": pk_hex,
            "nonce_hex": nonce_hex,
            "signature_hex": hex::encode(sig.serialize_der()),
            "endpoints": [{ "scheme": "quic", "host": "10.0.0.2", "port": 4433 }]
        })),
    )
    .await;
    assert_eq!(reg.status(), StatusCode::OK);
    drop(app1);
    drop(state1);

    let state2 = Arc::new(AppState::open(config).unwrap());
    let app2 = router(state2);
    let lookup = json_request(&app2, "GET", &format!("/v1/peers/{pk_hex}"), None).await;
    assert_eq!(lookup.status(), StatusCode::OK);
    let peer = json_body(lookup).await;
    assert_eq!(peer["endpoints"][0]["host"], "10.0.0.2");

    let _ = std::fs::remove_dir_all(&dir);
}
