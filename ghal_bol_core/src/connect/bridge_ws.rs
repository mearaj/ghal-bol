//! WAN call bridge — outbound WSS client (coord pairs opaque bytes).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::bridge_client::BridgeRequestResult;
use super::peer_session::{PeerSessionRegistry, start_session_io};
use super::session::SessionState;
use super::types::{GossipChatEvent, SessionPeer};
use crate::coord::CoordHttpClient;
use crate::p2p::native_log;

fn bridge_coord_client() -> Result<CoordHttpClient, String> {
    let base = crate::coord_runtime::coord_base_urls()
        .into_iter()
        .next()
        .or_else(|| {
            std::env::var("GHAL_BOL_COORD_URL")
                .or_else(|_| std::env::var("GHAL_BOL_COORD_BASE"))
                .ok()
        })
        .ok_or_else(|| "coord URL not configured".to_string())?;
    let insecure = crate::coord_runtime::coord_insecure_tls()
        || std::env::var("GHAL_BOL_COORD_INSECURE_TLS")
            .ok()
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    CoordHttpClient::new(&base, insecure)
}

/// Coord returns `https://…/v1/bridge/connect`; tungstenite needs `wss://`.
fn bridge_ws_url(connect_url: &str, bridge_id: &str, token: &str) -> String {
    let base = connect_url.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}?bridge_id={bridge_id}&token={token}")
}

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
///
/// Both peers dial **outbound** WSS to coord (docs), but Noise XX still needs asymmetric
/// roles: **caller = initiator**, **callee = responder**.
pub async fn connect_bridge_session(
    registry: Arc<PeerSessionRegistry>,
    session: Arc<SessionState>,
    identity: crate::DecryptedIdentity,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    peer_wire: SessionPeer,
    bridge: BridgeRequestResult,
    noise_initiator: bool,
) -> Result<(), String> {
    let url = bridge_ws_url(&bridge.connect_url, &bridge.bridge_id, &bridge.token);
    native_log::info(
        "bridge",
        format!(
            "wss connecting peer={} bridge_id={} noise={}",
            peer_wire,
            bridge.bridge_id,
            if noise_initiator {
                "initiator"
            } else {
                "responder"
            }
        ),
    );
    let (ws, _) = tokio::time::timeout(WS_CONNECT_TIMEOUT, connect_async(&url))
        .await
        .map_err(|_| "bridge ws connect timeout".to_string())?
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("400") {
                format!(
                    "bridge ws connect: {msg} (nginx must proxy Upgrade/Connection for /v1/bridge/connect — run enable_coord1_https.sh)"
                )
            } else {
                format!("bridge ws connect: {msg}")
            }
        })?;
    let (mut ws_write, mut ws_read) = ws.split();

    let (noise_io, bridge_io) = tokio::io::duplex(2 * 1024 * 1024);
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
        noise_initiator,
        read,
        write,
    )
    .await;
    Ok(())
}

/// Poll coord for inbound bridge pairing (callee role). Blocking HTTP — call from `spawn_blocking`.
pub fn poll_bridge_pending_blocking(identity_wire: &str) -> Result<Vec<BridgePendingItem>, String> {
    let client = bridge_coord_client()?;
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
                bridge_id: item["bridge_id"].as_str().unwrap_or_default().to_string(),
                call_id: item["call_id"].as_str().unwrap_or_default().to_string(),
                caller_identity_wire: item["caller_identity_wire"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                token: item["token"].as_str().unwrap_or_default().to_string(),
                connect_url: item["connect_url"].as_str().unwrap_or_default().to_string(),
            });
        }
    }
    Ok(out)
}

/// Accept a pending bridge as callee (opens outbound WSS + Noise **responder**).
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
    connect_bridge_session(
        registry,
        session,
        identity,
        events_tx,
        caller_wire,
        bridge,
        false, // callee = Noise responder
    )
    .await
}

/// Request coord bridge when LAN path is unavailable for WAN calls.
pub fn bridge_request_for_call(
    peer_identity_wire: &str,
    call_id: &str,
) -> Result<BridgeRequestResult, String> {
    let ident = crate::session_runtime::unlocked_identity_clone()?;
    let client = bridge_coord_client()?;
    super::bridge_client::bridge_request(&client, &ident, peer_identity_wire, call_id)
}

#[cfg(test)]
mod tests {
    use super::bridge_ws_url;

    #[test]
    fn bridge_ws_url_converts_https_to_wss() {
        let u = bridge_ws_url(
            "https://coord1.ghalbol.com:8443/v1/bridge/connect",
            "bid",
            "tok",
        );
        assert_eq!(
            u,
            "wss://coord1.ghalbol.com:8443/v1/bridge/connect?bridge_id=bid&token=tok"
        );
    }

    #[test]
    fn bridge_ws_url_keeps_wss() {
        let u = bridge_ws_url(
            "wss://coord1.ghalbol.com:8443/v1/bridge/connect",
            "bid",
            "tok",
        );
        assert_eq!(
            u,
            "wss://coord1.ghalbol.com:8443/v1/bridge/connect?bridge_id=bid&token=tok"
        );
    }
}
