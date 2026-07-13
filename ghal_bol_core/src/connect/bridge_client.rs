//! Coord bridge client for WAN call byte relay.

use crate::coord::CoordHttpClient;
use crate::coord_register_auth::sign_coord_registration;
use crate::p2p::native_log;

#[derive(Clone, Debug)]
pub struct BridgeRequestResult {
    pub bridge_id: String,
    pub token: String,
    pub connect_url: String,
}

pub fn bridge_request(
    client: &CoordHttpClient,
    ident: &crate::DecryptedIdentity,
    peer_identity_wire: &str,
    call_id: &str,
) -> Result<BridgeRequestResult, String> {
    let caller = ident.identity_wire();
    let ch_url = format!("{}/v1/bridge/challenge", client.base_url());
    let ch: serde_json::Value = client
        .post_json(&ch_url, &serde_json::json!({ "caller_identity_wire": caller }))?;
    let nonce_hex = ch["nonce_hex"].as_str().ok_or("missing nonce_hex")?;
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).map_err(|e| e.to_string())?);
    let msg = bridge_request_bytes(&nonce, &caller, peer_identity_wire, call_id);
    let sig = sign_bridge_request(ident, &msg)?;
    let url = format!("{}/v1/bridge/request", client.base_url());
    let resp: serde_json::Value = client.post_json(
        &url,
        &serde_json::json!({
            "caller_identity_wire": caller,
            "peer_identity_wire": peer_identity_wire,
            "call_id": call_id,
            "nonce_hex": nonce_hex,
            "signature_hex": hex::encode(sig),
        }),
    )?;
    native_log::info(
        "bridge",
        format!(
            "bridge request ok bridge_id={}",
            resp["bridge_id"].as_str().unwrap_or("?")
        ),
    );
    Ok(BridgeRequestResult {
        bridge_id: resp["bridge_id"]
            .as_str()
            .ok_or("missing bridge_id")?
            .to_string(),
        token: resp["token"].as_str().ok_or("missing token")?.to_string(),
        connect_url: resp["connect_url"]
            .as_str()
            .ok_or("missing connect_url")?
            .to_string(),
    })
}

fn bridge_request_bytes(
    nonce: &[u8; 32],
    caller: &str,
    peer: &str,
    call_id: &str,
) -> Vec<u8> {
    format!(
        "ghal_bol:bridge:request:v1\n{}\n{}\n{}\n{}",
        hex::encode(nonce),
        caller.trim().to_ascii_lowercase(),
        peer.trim().to_ascii_lowercase(),
        call_id.trim()
    )
    .into_bytes()
}

fn sign_bridge_request(ident: &crate::DecryptedIdentity, msg: &[u8]) -> Result<Vec<u8>, String> {
    use crate::identity::IdentityAlgorithm;
    use sha2::{Digest, Sha256};
    match ident.algorithm() {
        IdentityAlgorithm::Secp256k1 => {
            let hash = Sha256::digest(msg);
            let digest = secp256k1::Message::from_digest(hash.into());
            let sig = secp256k1::Secp256k1::new().sign_ecdsa(digest, ident.secp256k1_secret());
            Ok(sig.serialize_der().to_vec())
        }
        IdentityAlgorithm::Ed25519 | IdentityAlgorithm::EcdsaP256 => ident.sign_message(msg),
    }
}
