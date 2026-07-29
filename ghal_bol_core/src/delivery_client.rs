//! Persistent WebSocket client to `ghal_bol_delivery`.
//!
//! URL comes from `GHAL_BOL_DELIVERY_URL` / `p2p_start` `delivery_url` (e.g. `wss://…` or
//! `ws://127.0.0.1:8770` on the delivery host). No DNS/loopback rewriting — set the URL you mean.

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::delivery_auth::{
    extend_challenge_bytes, session_challenge_bytes, sign_delivery_challenge,
    upload_challenge_bytes,
};
use crate::delivery_msg_v1::{
    build_attachment_envelope, build_text_envelope, build_voice_envelope,
};
use crate::p2p::native_log;
use crate::session_runtime::unlocked_identity_clone;

/// Matches coord HTTP `connect_timeout` in [`crate::coord::CoordHttpClient::new`].
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(12);

async fn open_ws(
    url: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
> {
    let (ws, _) = connect_async(url)
        .await
        .map_err(|e| format!("ws connect: {e}"))?;
    Ok(ws)
}

pub fn ws_url_from_base(base: &str) -> String {
    let t = base.trim().trim_end_matches('/');
    if t.ends_with("/v1/ws") {
        t.to_string()
    } else {
        format!("{t}/v1/ws")
    }
}

pub async fn connect_and_auth(
    url: &str,
    ident: &crate::DecryptedIdentity,
) -> Result<DeliverySession, String> {
    let wire = ident.identity_wire();
    native_log::info("delivery", format!("ws connecting url={url}"));
    let ws = tokio::time::timeout(WS_CONNECT_TIMEOUT, open_ws(url))
        .await
        .map_err(|_| {
            format!(
                "ws connect: timed out after {}s (url={url})",
                WS_CONNECT_TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| format!("ws connect: {e}"))?;
    let (mut write, mut read) = ws.split();

    write
        .send(Message::Text(
            json!({ "type": "session.open", "identity_wire": wire })
                .to_string()
                .into(),
        ))
        .await
        .map_err(|e| format!("ws send: {e}"))?;

    let mut prefetched = VecDeque::new();
    let challenge = recv_until_type(&mut read, "session.challenge", &mut prefetched).await?;
    let nonce_hex = challenge
        .get("nonce_hex")
        .and_then(|v| v.as_str())
        .ok_or("missing nonce_hex")?;
    let nonce = parse_nonce32(nonce_hex)?;
    let sig = sign_delivery_challenge(ident, &session_challenge_bytes(&nonce, &wire))?;
    write
        .send(Message::Text(
            json!({
                "type": "session.auth",
                "identity_wire": wire,
                "nonce_hex": nonce_hex,
                "signature_hex": hex::encode(sig),
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|e| format!("ws auth send: {e}"))?;

    let ready = recv_until_type(&mut read, "session.ready", &mut prefetched).await?;
    native_log::info("delivery", "ws session.ready");
    let op_nonce_hex = ready
        .get("op_nonce_hex")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(DeliverySession {
        write,
        read,
        prefetched,
        op_nonce_hex,
        policy: ready.get("policy").cloned().unwrap_or(Value::Null),
        quota: ready.get("quota").cloned().unwrap_or(Value::Null),
    })
}

pub struct DeliverySession {
    write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    /// Server may push `message.inbound` before `session.ready` is read — keep for `recv_push`.
    prefetched: VecDeque<Value>,
    pub op_nonce_hex: Option<String>,
    pub policy: Value,
    pub quota: Value,
}

impl DeliverySession {
    pub async fn upload_text(
        &mut self,
        ident: &crate::DecryptedIdentity,
        message_id: &str,
        recipient_wire: &str,
        text: &str,
        ttl_secs: Option<u64>,
    ) -> Result<Value, String> {
        let op_nonce_hex = self.op_nonce_hex.as_deref().ok_or("missing op_nonce_hex")?;
        let op_nonce = parse_nonce32(op_nonce_hex)?;
        let created_at_ms = now_ms();
        let envelope = build_text_envelope(ident, message_id, recipient_wire, text, created_at_ms)?;
        let recipient_norm = envelope
            .get("recipient_wire")
            .and_then(|v| v.as_str())
            .unwrap_or(recipient_wire);
        let upload_msg = upload_challenge_bytes(&op_nonce, message_id, recipient_norm);
        let sig = sign_delivery_challenge(ident, &upload_msg)?;
        let mut frame = json!({
            "type": "message.upload",
            "envelope": envelope,
            "op_nonce_hex": op_nonce_hex,
            "signature_hex": hex::encode(sig),
        });
        if let Some(ttl) = ttl_secs {
            frame["ttl_secs"] = json!(ttl);
        }
        self.write
            .send(Message::Text(frame.to_string().into()))
            .await
            .map_err(|e| format!("upload send: {e}"))?;
        let resp =
            recv_until_type(&mut self.read, "message.upload.ok", &mut self.prefetched).await?;
        if let Some(n) = resp.get("op_nonce_hex").and_then(|v| v.as_str()) {
            self.op_nonce_hex = Some(n.to_string());
        }
        if let Some(q) = resp.get("quota") {
            self.quota = q.clone();
        }
        Ok(resp)
    }

    pub async fn upload_voice(
        &mut self,
        ident: &crate::DecryptedIdentity,
        message_id: &str,
        recipient_wire: &str,
        duration_ms: u32,
        opus_blob: &[u8],
        ttl_secs: Option<u64>,
    ) -> Result<Value, String> {
        let op_nonce_hex = self.op_nonce_hex.as_deref().ok_or("missing op_nonce_hex")?;
        let op_nonce = parse_nonce32(op_nonce_hex)?;
        let created_at_ms = now_ms();
        let envelope = build_voice_envelope(
            ident,
            message_id,
            recipient_wire,
            duration_ms,
            opus_blob,
            created_at_ms,
        )?;
        let recipient_norm = envelope
            .get("recipient_wire")
            .and_then(|v| v.as_str())
            .unwrap_or(recipient_wire);
        let upload_msg = upload_challenge_bytes(&op_nonce, message_id, recipient_norm);
        let sig = sign_delivery_challenge(ident, &upload_msg)?;
        let mut frame = json!({
            "type": "message.upload",
            "envelope": envelope,
            "op_nonce_hex": op_nonce_hex,
            "signature_hex": hex::encode(sig),
        });
        if let Some(ttl) = ttl_secs {
            frame["ttl_secs"] = json!(ttl);
        }
        self.write
            .send(Message::Text(frame.to_string().into()))
            .await
            .map_err(|e| format!("upload send: {e}"))?;
        let resp =
            recv_until_type(&mut self.read, "message.upload.ok", &mut self.prefetched).await?;
        if let Some(n) = resp.get("op_nonce_hex").and_then(|v| v.as_str()) {
            self.op_nonce_hex = Some(n.to_string());
        }
        if let Some(q) = resp.get("quota") {
            self.quota = q.clone();
        }
        Ok(resp)
    }

    pub async fn upload_attachment(
        &mut self,
        ident: &crate::DecryptedIdentity,
        message_id: &str,
        recipient_wire: &str,
        inner: &crate::attach_v1::AttachmentInner,
        ttl_secs: Option<u64>,
    ) -> Result<Value, String> {
        let op_nonce_hex = self.op_nonce_hex.as_deref().ok_or("missing op_nonce_hex")?;
        let op_nonce = parse_nonce32(op_nonce_hex)?;
        let created_at_ms = now_ms();
        let envelope =
            build_attachment_envelope(ident, message_id, recipient_wire, inner, created_at_ms)?;
        let recipient_norm = envelope
            .get("recipient_wire")
            .and_then(|v| v.as_str())
            .unwrap_or(recipient_wire);
        let upload_msg = upload_challenge_bytes(&op_nonce, message_id, recipient_norm);
        let sig = sign_delivery_challenge(ident, &upload_msg)?;
        let mut frame = json!({
            "type": "message.upload",
            "envelope": envelope,
            "op_nonce_hex": op_nonce_hex,
            "signature_hex": hex::encode(sig),
        });
        if let Some(ttl) = ttl_secs {
            frame["ttl_secs"] = json!(ttl);
        }
        self.write
            .send(Message::Text(frame.to_string().into()))
            .await
            .map_err(|e| format!("upload send: {e}"))?;
        let resp =
            recv_until_type(&mut self.read, "message.upload.ok", &mut self.prefetched).await?;
        if let Some(n) = resp.get("op_nonce_hex").and_then(|v| v.as_str()) {
            self.op_nonce_hex = Some(n.to_string());
        }
        if let Some(q) = resp.get("quota") {
            self.quota = q.clone();
        }
        Ok(resp)
    }

    pub async fn inbox_read(&mut self, message_id: &str, sender_wire: &str) -> Result<(), String> {
        self.write
            .send(Message::Text(
                json!({
                    "type": "inbox.read",
                    "message_id": message_id,
                    "sender_wire": sender_wire,
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|e| format!("read send: {e}"))?;
        let _ = recv_until_type(&mut self.read, "inbox.read.ok", &mut self.prefetched).await?;
        Ok(())
    }

    pub async fn inbox_ack(&mut self, message_id: &str, sender_wire: &str) -> Result<(), String> {
        self.write
            .send(Message::Text(
                json!({
                    "type": "inbox.ack",
                    "message_id": message_id,
                    "sender_wire": sender_wire,
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|e| format!("ack send: {e}"))?;
        let _ = recv_until_type(&mut self.read, "inbox.ack.ok", &mut self.prefetched).await?;
        Ok(())
    }

    pub async fn mailbox_list(&mut self, include_expired: bool) -> Result<Value, String> {
        self.write
            .send(Message::Text(
                json!({
                    "type": "mailbox.outbox.list",
                    "include_expired": include_expired,
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|e| format!("list send: {e}"))?;
        recv_until_type(
            &mut self.read,
            "mailbox.outbox.snapshot",
            &mut self.prefetched,
        )
        .await
    }

    pub async fn extend_ttl(
        &mut self,
        ident: &crate::DecryptedIdentity,
        message_id: &str,
        extend_secs: u64,
    ) -> Result<Value, String> {
        let op_nonce_hex = self.op_nonce_hex.as_deref().ok_or("missing op_nonce_hex")?;
        let op_nonce = parse_nonce32(op_nonce_hex)?;
        let sig = sign_delivery_challenge(ident, &extend_challenge_bytes(&op_nonce, message_id))?;
        self.write
            .send(Message::Text(
                json!({
                    "type": "mailbox.ttl.extend",
                    "message_id": message_id,
                    "extend_secs": extend_secs,
                    "op_nonce_hex": op_nonce_hex,
                    "signature_hex": hex::encode(sig),
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|e| format!("extend send: {e}"))?;
        let resp =
            recv_until_type(&mut self.read, "mailbox.ttl.extended", &mut self.prefetched).await?;
        if let Some(n) = resp.get("op_nonce_hex").and_then(|v| v.as_str()) {
            self.op_nonce_hex = Some(n.to_string());
        }
        Ok(resp)
    }

    pub async fn recv_push(&mut self) -> Result<Option<Value>, String> {
        if let Some(v) = self.prefetched.pop_front() {
            return Ok(Some(v));
        }
        tokio::select! {
            msg = self.read.next() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        let v: Value = serde_json::from_str(&t).map_err(|e| format!("json: {e}"))?;
                        Ok(Some(v))
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = self.write.send(Message::Pong(p)).await;
                        Ok(None)
                    }
                    Some(Ok(Message::Close(_))) => Err("ws closed".to_string()),
                    Some(Err(e)) => Err(format!("ws read: {e}")),
                    None => Err("ws eof".to_string()),
                    _ => Ok(None),
                }
            }
        }
    }
}

async fn recv_until_type(
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    want: &str,
    prefetch: &mut VecDeque<Value>,
) -> Result<Value, String> {
    for _ in 0..128 {
        let msg = match read.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) => return Err(format!("ws read: {e}")),
            None => return Err("ws closed while waiting for response".to_string()),
        };
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Ping(_) => continue,
            Message::Close(_) => return Err("ws closed".to_string()),
            _ => continue,
        };
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("json: {e}"))?;
        let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if ty == "error" {
            return Err(v
                .get("message")
                .and_then(|x| x.as_str())
                .unwrap_or("delivery error")
                .to_string());
        }
        if ty == want {
            return Ok(v);
        }
        prefetch.push_back(v);
    }
    Err(format!("timeout waiting for {want}"))
}

fn parse_nonce32(hex_s: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_s.trim()).map_err(|e| format!("nonce: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| "nonce must be 32 bytes".to_string())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Runs delivery I/O on a dedicated std thread (safe from the delivery worker tokio runtime).
fn on_delivery_io_thread<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name("delivery_io".into())
        .spawn(f)
        .map_err(|e| format!("spawn delivery_io: {e}"))?
        .join()
        .map_err(|_| "delivery_io thread panicked".to_string())?
}

/// Blocking upload for sync RPC paths (spawns short-lived connection).
pub fn blocking_upload_text(
    url: &str,
    recipient_wire: &str,
    text: &str,
    message_id: &str,
    ttl_secs: Option<u64>,
) -> Result<Value, String> {
    let url = url.to_string();
    let recipient_wire = recipient_wire.to_string();
    let text = text.to_string();
    let message_id = message_id.to_string();
    on_delivery_io_thread(move || {
        crate::rustls_init::ensure_rustls_crypto_provider();
        let ws_url = ws_url_from_base(&url);
        let ident = unlocked_identity_clone().map_err(|e| e.to_string())?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("tokio: {e}"))?;
        rt.block_on(async {
            let mut session = connect_and_auth(&ws_url, &ident).await?;
            session
                .upload_text(&ident, &message_id, &recipient_wire, &text, ttl_secs)
                .await
        })
    })
}

pub fn blocking_upload_voice(
    url: &str,
    recipient_wire: &str,
    duration_ms: u32,
    opus_blob: &[u8],
    message_id: &str,
    ttl_secs: Option<u64>,
) -> Result<Value, String> {
    let url = url.to_string();
    let recipient_wire = recipient_wire.to_string();
    let opus_blob = opus_blob.to_vec();
    let message_id = message_id.to_string();
    on_delivery_io_thread(move || {
        crate::rustls_init::ensure_rustls_crypto_provider();
        let ws_url = ws_url_from_base(&url);
        let ident = unlocked_identity_clone().map_err(|e| e.to_string())?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("tokio: {e}"))?;
        rt.block_on(async {
            let mut session = connect_and_auth(&ws_url, &ident).await?;
            session
                .upload_voice(
                    &ident,
                    &message_id,
                    &recipient_wire,
                    duration_ms,
                    &opus_blob,
                    ttl_secs,
                )
                .await
        })
    })
}

pub fn blocking_upload_attachment(
    url: &str,
    recipient_wire: &str,
    inner: &crate::attach_v1::AttachmentInner,
    message_id: &str,
    ttl_secs: Option<u64>,
) -> Result<Value, String> {
    let url = url.to_string();
    let recipient_wire = recipient_wire.to_string();
    let inner = inner.clone();
    let message_id = message_id.to_string();
    on_delivery_io_thread(move || {
        crate::rustls_init::ensure_rustls_crypto_provider();
        let ws_url = ws_url_from_base(&url);
        let ident = unlocked_identity_clone().map_err(|e| e.to_string())?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("tokio: {e}"))?;
        rt.block_on(async {
            let mut session = connect_and_auth(&ws_url, &ident).await?;
            session
                .upload_attachment(&ident, &message_id, &recipient_wire, &inner, ttl_secs)
                .await
        })
    })
}

/// Fire-and-forget inbox read receipt on a fresh connection.
pub fn blocking_inbox_read(url: &str, message_id: &str, sender_wire: &str) -> Result<(), String> {
    let url = url.to_string();
    let message_id = message_id.to_string();
    let sender_wire = sender_wire.to_string();
    on_delivery_io_thread(move || {
        crate::rustls_init::ensure_rustls_crypto_provider();
        let ws_url = ws_url_from_base(&url);
        let ident = unlocked_identity_clone().map_err(|e| e.to_string())?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("tokio: {e}"))?;
        rt.block_on(async {
            let mut session = connect_and_auth(&ws_url, &ident).await?;
            session.inbox_read(&message_id, &sender_wire).await
        })
    })
}

#[cfg(test)]
mod tests {
    use super::ws_url_from_base;

    #[test]
    fn ws_url_from_base_appends_v1_ws() {
        assert_eq!(
            ws_url_from_base("wss://delivery.ghalbol.com:55003"),
            "wss://delivery.ghalbol.com:55003/v1/ws"
        );
        assert_eq!(
            ws_url_from_base("wss://delivery.ghalbol.com:55003/v1/ws"),
            "wss://delivery.ghalbol.com:55003/v1/ws"
        );
    }
}
