use axum::body::Body;
use ghal_bol_delivery::{AppState, DeliveryConfig, app};
use tower::ServiceExt;

#[tokio::test]
async fn health_ok() {
    let mut cfg = DeliveryConfig::default();
    cfg.data_dir = std::env::temp_dir().join("ghal_bol_delivery_test");
    let state = AppState::new(cfg).expect("state");
    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["service"], "ghal_bol_delivery");
    assert_eq!(json["ok"], true);
}
