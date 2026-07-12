use axum::{
    Json, Router,
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::ws::{WsState, handle_socket, spawn_ttl_sweeper};
use crate::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/ws", get(ws_upgrade))
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let connected = state.registry.connected_count().await;
    let body = match state.store.aggregate_health(connected) {
        Ok(metrics) => json!({
            "ok": true,
            "service": "ghal_bol_delivery",
            "instance_id": crate::instance::instance_id(),
            "schema_version": crate::db::SCHEMA_VERSION,
            "connected_peers": metrics.connected_peers,
            "pending_messages": metrics.pending_messages,
            "pending_bytes": metrics.pending_bytes,
            "oldest_pending_age_secs": metrics.oldest_pending_age_secs,
        }),
        Err(e) => json!({
            "ok": false,
            "service": "ghal_bol_delivery",
            "error": e.to_string(),
        }),
    };
    Json(body)
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let ws_state = Arc::new(WsState {
        store: state.store.clone(),
        policy: state.policy.clone(),
        registry: state.registry.clone(),
        challenges: state.challenges.clone(),
    });
    ws.on_upgrade(move |socket| handle_socket(socket, ws_state))
}

pub fn spawn_background_tasks(state: Arc<AppState>) {
    spawn_ttl_sweeper(state.store.clone(), state.registry.clone());
}
