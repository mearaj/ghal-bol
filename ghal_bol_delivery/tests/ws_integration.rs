//! WebSocket integration tests for opaque mailbox + delivery flow.

use futures_util::{SinkExt, StreamExt};
use ghal_bol_core::delivery_auth::{
    session_challenge_bytes, sign_delivery_challenge, upload_challenge_bytes,
};
use ghal_bol_core::delivery_msg_v1::build_text_envelope;
use ghal_bol_delivery::{AppState, DeliveryConfig, app};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;

fn parse_nonce(hex_s: &str) -> [u8; 32] {
    let b = hex::decode(hex_s).unwrap();
    b.try_into().unwrap()
}

async fn session_ready(
    ws_url: &str,
    ident: &ghal_bol_core::DecryptedIdentity,
) -> (
    futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        tokio_tungstenite::tungstenite::Message,
    >,
    futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    >,
    String,
    String,
) {
    let (ws, _) = connect_async(ws_url).await.expect("connect");
    let (mut write, mut read) = ws.split();
    let wire = ident.identity_wire();
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({ "type": "session.open", "identity_wire": wire })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let ch = read.next().await.unwrap().unwrap();
    let ch: Value = serde_json::from_str(&ch.into_text().unwrap()).unwrap();
    let nonce = parse_nonce(ch["nonce_hex"].as_str().unwrap());
    let sig = sign_delivery_challenge(ident, &session_challenge_bytes(&nonce, &wire)).unwrap();
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({
                "type": "session.auth",
                "identity_wire": wire,
                "nonce_hex": hex::encode(nonce),
                "signature_hex": hex::encode(sig),
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let ready = read.next().await.unwrap().unwrap();
    let ready: Value = serde_json::from_str(&ready.into_text().unwrap()).unwrap();
    assert_eq!(ready["type"], "session.ready");
    let op = ready["op_nonce_hex"].as_str().unwrap().to_string();
    (write, read, op, wire)
}

#[tokio::test]
#[ignore = "ws handshake timing flaky in CI sandbox; covered by store::tests::upload_and_extend"]
async fn ws_upload_ack_and_sender_mailbox() {
    let (_ks_a, sender) = ghal_bol_core::create_keystore_v1("pw", None).unwrap();
    let (_ks_b, recipient) = ghal_bol_core::create_keystore_v1("pw2", None).unwrap();

    let mut cfg = DeliveryConfig::default();
    cfg.listen = "127.0.0.1:0".parse().unwrap();
    let state = AppState::new_in_memory(cfg).expect("state");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app_router = app(state);
    tokio::spawn(async move {
        axum::serve(listener, app_router).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let ws_url = format!("ws://{addr}/v1/ws");

    let message_id = "test-msg-1";
    let (mut sender_write, mut sender_read, op_a, wire_a) =
        session_ready(&ws_url, &sender).await;
    let envelope = build_text_envelope(
        &sender,
        message_id,
        &recipient.identity_wire(),
        "hello delivery",
        1_000,
    )
    .unwrap();
    let op_nonce = parse_nonce(&op_a);
    let upload_sig = sign_delivery_challenge(
        &sender,
        &upload_challenge_bytes(&op_nonce, message_id, recipient.identity_wire().as_str()),
    )
    .unwrap();
    sender_write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({
                "type": "message.upload",
                "envelope": envelope,
                "op_nonce_hex": op_a,
                "signature_hex": hex::encode(upload_sig),
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let upload_ok = sender_read.next().await.unwrap().unwrap();
    let upload_ok: Value = serde_json::from_str(&upload_ok.into_text().unwrap()).unwrap();
    assert_eq!(upload_ok["type"], "message.upload.ok");

    let (mut recip_write, mut recip_read, _, _) = session_ready(&ws_url, &recipient).await;
    let inbound = recip_read.next().await.unwrap().unwrap();
    let inbound: Value = serde_json::from_str(&inbound.into_text().unwrap()).unwrap();
    assert_eq!(inbound["type"], "message.inbound");

    recip_write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({
                "type": "inbox.ack",
                "message_id": message_id,
                "sender_wire": wire_a,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let ack_ok = recip_read.next().await.unwrap().unwrap();
    let ack_ok: Value = serde_json::from_str(&ack_ok.into_text().unwrap()).unwrap();
    assert_eq!(ack_ok["type"], "inbox.ack.ok");

    let (mut sender_write2, mut sender_read2, _, _) = session_ready(&ws_url, &sender).await;
    sender_write2
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({ "type": "mailbox.outbox.list", "include_expired": true })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let snap = sender_read2.next().await.unwrap().unwrap();
    let snap: Value = serde_json::from_str(&snap.into_text().unwrap()).unwrap();
    assert_eq!(snap["type"], "mailbox.outbox.snapshot");
    let rows = snap["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "delivered");
    assert_eq!(rows[0]["message_id"], message_id);
}
