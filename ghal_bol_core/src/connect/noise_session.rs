//! Noise XX handshake with identity proof payloads (`docs/GHAL_BOL_CONNECT_V1.md`).

use std::io;

use snow::{Builder, HandshakeState, Keypair, TransportState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use x25519_dalek::{PublicKey, StaticSecret};

pub const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const PROLOGUE: &[u8] = b"ghal_bol_connect_v1";
const PROOF_DOMAIN: &[u8] = b"ghal_bol_connect_v1/proof";
const MAX_NOISE_MSG: usize = 65535;

pub struct ConnectNoiseSession {
    transport: TransportState,
}

impl ConnectNoiseSession {
    /// Initiator: outbound TCP already connected.
    pub async fn initiator<R, W>(
        ident: &crate::DecryptedIdentity,
        static_key: &StaticSecret,
        mut read: R,
        mut write: W,
    ) -> Result<(Self, String, R, W), String>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut hs = build_handshake(static_key, true)?;
        let mut msg_buf = vec![0u8; MAX_NOISE_MSG];
        let n1 = hs
            .write_message(&[], &mut msg_buf)
            .map_err(|e| e.to_string())?;
        write_message(&mut write, &msg_buf[..n1])
            .await
            .map_err(|e| e.to_string())?;

        let msg2 = read_message(&mut read).await?;
        // Hash / transcript before msg2 — must match what the responder signed.
        let hash_before_msg2 = hs.get_handshake_hash().to_vec();
        let mut payload2 = vec![0u8; msg2.len()];
        let n2 = hs
            .read_message(&msg2, &mut payload2)
            .map_err(|e| e.to_string())?;
        payload2.truncate(n2);
        let remote_static = hs
            .get_remote_static()
            .ok_or("missing remote static after msg2")?
            .to_vec();
        let peer_wire = verify_remote_proof(&payload2, &hash_before_msg2, &remote_static)?;

        let proof = build_identity_proof(ident, &hs)?;
        let n3 = hs
            .write_message(proof.as_bytes(), &mut msg_buf)
            .map_err(|e| e.to_string())?;
        write_message(&mut write, &msg_buf[..n3])
            .await
            .map_err(|e| e.to_string())?;

        let transport = hs.into_transport_mode().map_err(|e| e.to_string())?;
        Ok((Self { transport }, peer_wire, read, write))
    }

    /// Responder: accepted inbound TCP.
    pub async fn responder<R, W>(
        ident: &crate::DecryptedIdentity,
        static_key: &StaticSecret,
        mut read: R,
        mut write: W,
    ) -> Result<(Self, String, R, W), String>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut hs = build_handshake(static_key, false)?;
        let msg1 = read_message(&mut read).await?;
        hs.read_message(&msg1, &mut [])
            .map_err(|e| e.to_string())?;

        let proof = build_identity_proof(ident, &hs)?;
        let mut msg_buf = vec![0u8; MAX_NOISE_MSG];
        let n2 = hs
            .write_message(proof.as_bytes(), &mut msg_buf)
            .map_err(|e| e.to_string())?;
        write_message(&mut write, &msg_buf[..n2])
            .await
            .map_err(|e| e.to_string())?;

        let msg3 = read_message(&mut read).await?;
        let hash_before_msg3 = hs.get_handshake_hash().to_vec();
        let mut payload3 = vec![0u8; msg3.len()];
        let n3 = hs
            .read_message(&msg3, &mut payload3)
            .map_err(|e| e.to_string())?;
        payload3.truncate(n3);
        let remote_static = hs
            .get_remote_static()
            .ok_or("missing remote static after msg3")?
            .to_vec();
        let peer_wire = verify_remote_proof(&payload3, &hash_before_msg3, &remote_static)?;

        let transport = hs.into_transport_mode().map_err(|e| e.to_string())?;
        Ok((Self { transport }, peer_wire, read, write))
    }

    /// Encrypt one Noise transport message (no IO) — safe under a short mutex hold.
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; plaintext.len() + 64];
        let n = self
            .transport
            .write_message(plaintext, &mut buf)
            .map_err(|e| e.to_string())?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Decrypt one Noise transport message (no IO).
    pub fn open(&mut self, wire: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; wire.len()];
        let n = self
            .transport
            .read_message(wire, &mut out)
            .map_err(|e| e.to_string())?;
        out.truncate(n);
        Ok(out)
    }
}

fn build_handshake(static_key: &StaticSecret, initiator: bool) -> Result<HandshakeState, String> {
    let kp = Keypair {
        private: static_key.to_bytes().to_vec(),
        public: PublicKey::from(static_key).to_bytes().to_vec(),
    };
    let builder = Builder::new(
        NOISE_PATTERN
            .parse()
            .map_err(|e: snow::Error| e.to_string())?,
    )
    .local_private_key(&kp.private)
    .map_err(|e: snow::Error| e.to_string())?
    .prologue(PROLOGUE)
    .map_err(|e: snow::Error| e.to_string())?;
    if initiator {
        builder.build_initiator().map_err(|e| e.to_string())
    } else {
        builder.build_responder().map_err(|e| e.to_string())
    }
}

fn transport_static_secret(ident: &crate::DecryptedIdentity) -> StaticSecret {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"ghal_bol_connect_v1/transport_sk");
    h.update(ident.identity_wire().as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    StaticSecret::from(digest)
}

pub fn transport_static_secret_for_identity(ident: &crate::DecryptedIdentity) -> StaticSecret {
    transport_static_secret(ident)
}

fn build_identity_proof(
    ident: &crate::DecryptedIdentity,
    hs: &HandshakeState,
) -> Result<String, String> {
    let wire = ident.identity_wire();
    let hash = hs.get_handshake_hash();
    let sk = transport_static_secret(ident);
    let static_pub = PublicKey::from(&sk);
    let mut sign_bytes = Vec::new();
    sign_bytes.extend_from_slice(PROOF_DOMAIN);
    sign_bytes.extend_from_slice(hash);
    sign_bytes.extend_from_slice(static_pub.as_bytes());
    let sig = ident.sign_message(&sign_bytes)?;
    Ok(serde_json::json!({
        "identity_wire": wire,
        "sig_hex": hex::encode(sig),
    })
    .to_string())
}

fn verify_remote_proof(
    payload: &[u8],
    handshake_hash_before_msg: &[u8],
    remote_static_pub: &[u8],
) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_slice(payload).map_err(|e| format!("proof json: {e}"))?;
    let wire = v
        .get("identity_wire")
        .and_then(|x| x.as_str())
        .ok_or("proof missing identity_wire")?;
    let sig_hex = v
        .get("sig_hex")
        .and_then(|x| x.as_str())
        .ok_or("proof missing sig_hex")?;
    let sig = hex::decode(sig_hex.trim()).map_err(|e| format!("sig hex: {e}"))?;
    let mut sign_bytes = Vec::new();
    sign_bytes.extend_from_slice(PROOF_DOMAIN);
    sign_bytes.extend_from_slice(handshake_hash_before_msg);
    sign_bytes.extend_from_slice(remote_static_pub);
    crate::identity_sign::verify_identity_signature(wire, &sign_bytes, &sig)?;
    Ok(wire.to_string())
}

pub(crate) async fn write_sealed_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    sealed: &[u8],
) -> Result<(), String> {
    write_message(w, sealed).await.map_err(|e| e.to_string())
}

pub(crate) async fn read_sealed_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>, String> {
    read_message(r).await
}

async fn write_message<W: AsyncWrite + Unpin>(w: &mut W, msg: &[u8]) -> io::Result<()> {
    let len = msg.len() as u32;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(msg).await?;
    w.flush().await
}

async fn read_message<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .await
        .map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_NOISE_MSG {
        return Err("noise message too large".to_string());
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity(password: &str) -> crate::DecryptedIdentity {
        crate::create_keystore_v1(password, None)
            .expect("keystore")
            .1
    }

    #[tokio::test]
    async fn noise_xx_handshake_roundtrip_over_duplex() {
        let a = test_identity("pw-a");
        let b = test_identity("pw-b");
        let wire_a = a.identity_wire();
        let wire_b = b.identity_wire();
        let sk_a = transport_static_secret_for_identity(&a);
        let sk_b = transport_static_secret_for_identity(&b);

        let (client, server) = tokio::io::duplex(64 * 1024);
        let (c_read, c_write) = tokio::io::split(client);
        let (s_read, s_write) = tokio::io::split(server);

        let init = tokio::spawn(async move {
            ConnectNoiseSession::initiator(&a, &sk_a, c_read, c_write).await
        });
        let resp = tokio::spawn(async move {
            ConnectNoiseSession::responder(&b, &sk_b, s_read, s_write).await
        });

        let (init_r, resp_r) = tokio::join!(init, resp);
        let (_sess_a, peer_b, _, _) = init_r.unwrap().expect("initiator");
        let (_sess_b, peer_a, _, _) = resp_r.unwrap().expect("responder");
        assert_eq!(peer_b, wire_b);
        assert_eq!(peer_a, wire_a);
    }
}
