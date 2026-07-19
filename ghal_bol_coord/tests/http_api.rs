//! In-process HTTP surface checks only — not P2P/WAN/LAN connectivity.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ghal_bol_coord::{AppState, ServerConfig, router};
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
async fn health_ok_with_database() {
    let app = test_app();
    let resp = json_request(&app, "GET", "/health").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_body(resp).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["database"], true);
    assert_eq!(v["bridge"], true);
}

#[tokio::test]
async fn register_and_lookup_roundtrip() {
    use ghal_bol_coord::registration_message_digest;
    use secp256k1::{Secp256k1, SecretKey};

    let app = test_app();
    let sk = SecretKey::from_byte_array([7u8; 32]).expect("key");
    let secp = Secp256k1::new();
    let pk_hex = hex::encode(sk.public_key(&secp).serialize());

    let ch_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/register/challenge")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "public_key_hex": pk_hex }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ch_resp.status(), StatusCode::OK);
    let ch = json_body(ch_resp).await;
    let nonce_hex = ch["nonce_hex"].as_str().unwrap();
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).unwrap());
    let digest = registration_message_digest(&nonce, &pk_hex);
    let sig = secp.sign_ecdsa(digest, &sk);

    let reg_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/register")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "public_key_hex": pk_hex,
                        "nonce_hex": nonce_hex,
                        "signature_hex": hex::encode(sig.serialize_der()),
                        "endpoints": [{
                            "scheme": "tcp",
                            "host": "203.0.113.50",
                            "port": 41234
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reg_resp.status(), StatusCode::OK);

    let lookup = json_request(&app, "GET", &format!("/v1/peers/{pk_hex}")).await;
    assert_eq!(lookup.status(), StatusCode::OK);
    let v = json_body(lookup).await;
    assert_eq!(v["public_key_hex"], pk_hex);
}
