//! WAN call bridge — outbound WSS client (coord pairs opaque bytes).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::bridge_client::BridgeRequestResult;
use super::peer_session::{start_session_io, PeerSessionRegistry};
use super::session::SessionState;
use super::types::{GossipChatEvent, SessionPeer};
use crate::coord::CoordHttpClient;
use crate::p2p::native_log;

/// Callee-side bridge notification from `GET /v1/bridge/pending`.
#[derive(Clone, Debug)]
pub struct BridgePendingItem {
    pub bridge_id: String,
    pub call_id: String,
    pub caller_identity_wire: String,
    pub token: String,
    pub connect_url: String,
}

const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(12);

/// Open bridge WSS and run Noise+mux (same as LAN) over relayed binary frames.
pub async fn connect_bridge_session(
    registry: Arc<PeerSessionRegistry>,
    session: Arc<SessionState>,
    identity: crate::DecryptedIdentity,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    peer_wire: SessionPeer,
    bridge: BridgeRequestResult,
) -> Result<(), String> {
    let url = format!(
        "{}?bridge_id={}&token={}",
        bridge.connect_url.trim_end_matches('/'),
        bridge.bridge_id,
        bridge.token
    );
    native_log::info("bridge", format!("wss connecting peer={peer_wire}"));
    let (ws, _) = tokio::time::timeout(WS_CONNECT_TIMEOUT, connect_async(&url))
        .await
        .map_err(|_| "bridge ws connect timeout".to_string())?
        .map_err(|e| format!("bridge ws connect: {e}"))?;
    let (mut ws_write, mut ws_read) = ws.split();

    let (noise_io, bridge_io) = tokio::io::duplex(256 * 1024);
    let (mut bridge_read, mut bridge_write) = tokio::io::split(bridge_io);

    tokio::spawn(async move {
        while let Some(msg) = ws_read.next().await {
            match msg {
                Ok(Message::Binary(b)) => {
                    if bridge_write.write_all(&b).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        loop {
            match bridge_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if ws_write
                        .send(Message::Binary(buf[..n].to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let (read, write) = tokio::io::split(noise_io);
    start_session_io(
        registry,
        session,
        identity,
        events_tx,
        peer_wire,
        true,
        read,
        write,
    )
    .await;
    Ok(())
}

/// Poll coord for inbound bridge pairing (callee role). Blocking HTTP — call from `spawn_blocking`.
pub fn poll_bridge_pending_blocking(identity_wire: &str) -> Result<Vec<BridgePendingItem>, String> {
    let base = std::env::var("GHAL_BOL_COORD_URL")
        .or_else(|_| std::env::var("GHAL_BOL_COORD_BASE"))
        .map_err(|_| "GHAL_BOL_COORD_URL not set".to_string())?;
    let insecure = std::env::var("GHAL_BOL_COORD_INSECURE_TLS")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let client = CoordHttpClient::new(&base, insecure)?;
    let wire = crate::public_key_util::normalize_contact_identity_wire(identity_wire)?;
    let encoded = crate::identity::percent_encode_uri_component(&wire);
    let url = format!(
        "{}/v1/bridge/pending?identity_wire={}",
        client.base_url(),
        encoded
    );
    let v = client.get_json(&url)?;
    let mut out = Vec::new();
    if let Some(items) = v.get("pending").and_then(|x| x.as_array()) {
        for item in items {
            out.push(BridgePendingItem {
                bridge_id: item["bridge_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                call_id: item["call_id"].as_str().unwrap_or_default().to_string(),
                caller_identity_wire: item["caller_identity_wire"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                token: item["token"].as_str().unwrap_or_default().to_string(),
                connect_url: item["connect_url"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            });
        }
    }
    Ok(out)
}

/// Accept a pending bridge as callee (opens outbound WSS + Noise+mux).
pub async fn accept_bridge_pending(
    registry: Arc<PeerSessionRegistry>,
    session: Arc<SessionState>,
    identity: crate::DecryptedIdentity,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    caller_wire: SessionPeer,
    pending: BridgePendingItem,
) -> Result<(), String> {
    let bridge = BridgeRequestResult {
        bridge_id: pending.bridge_id,
        token: pending.token,
        connect_url: pending.connect_url,
    };
    connect_bridge_session(registry, session, identity, events_tx, caller_wire, bridge).await
}

/// Request coord bridge when LAN path is unavailable for WAN calls.
pub fn bridge_request_for_call(
    peer_identity_wire: &str,
    call_id: &str,
) -> Result<BridgeRequestResult, String> {
    let ident = crate::session_runtime::unlocked_identity_clone()?;
    let base = std::env::var("GHAL_BOL_COORD_URL")
        .or_else(|_| std::env::var("GHAL_BOL_COORD_BASE"))
        .map_err(|_| "GHAL_BOL_COORD_URL not set".to_string())?;
    let insecure = std::env::var("GHAL_BOL_COORD_INSECURE_TLS")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let client = crate::coord::CoordHttpClient::new(&base, insecure)?;
    super::bridge_client::bridge_request(&client, &ident, peer_identity_wire, call_id)
}
