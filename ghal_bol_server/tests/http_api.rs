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
