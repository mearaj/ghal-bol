//! HTTP client for [`ghal_bol_server`](../../ghal_bol_server) coordination API (Tier 1).

use secp256k1::{Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoordEndpoint {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CoordPeerRecord {
    pub public_key_hex: String,
    pub endpoints: Vec<CoordEndpoint>,
    #[serde(default)]
    pub transport_capabilities: Vec<String>,
    pub ipv6: Option<String>,
    pub ipv4: Option<String>,
    pub last_heartbeat_unix_ms: i64,
}

fn registration_message_digest(nonce: &[u8; 32], public_key_hex: &str) -> secp256k1::Message {
    let body = format!(
        "ghal_bol:register:v1\n{}\n{}",
        hex::encode(nonce),
        public_key_hex.trim().to_ascii_lowercase()
    );
    let hash = Sha256::digest(body.as_bytes());
    secp256k1::Message::from_digest(hash.into())
}

pub struct CoordHttpClient {
    http: reqwest::blocking::Client,
    base: String,
}

impl CoordHttpClient {
    pub fn new(base_url: &str, insecure_tls: bool) -> Result<Self, String> {
        let base = base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err("coord base url empty".into());
        }
        let mut b = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(12))
            .timeout(std::time::Duration::from_secs(25));
        if insecure_tls {
            b = b.danger_accept_invalid_certs(true);
        }
        let http = b.build().map_err(|e| e.to_string())?;
        Ok(Self { http, base })
    }

    /// One immediate retry on transport errors (mobile TLS flake).
    fn send_with_transport_retry(
        &self,
        build: impl Fn(&reqwest::blocking::Client) -> reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response, String> {
        match build(&self.http).send() {
            Ok(resp) => Ok(resp),
            Err(e) if e.is_connect() || e.is_timeout() || e.is_request() => {
                std::thread::sleep(std::time::Duration::from_millis(200));
                build(&self.http).send().map_err(|e2| e2.to_string())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn with_headers(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        builder.header("Accept", "application/json")
    }

    pub fn health(&self) -> Result<bool, String> {
        let url = format!("{}/health", self.base);
        let resp = self.send_with_transport_retry(|http| self.with_headers(http.get(&url)))?;
        if !resp.status().is_success() {
            return Err(format!("health HTTP {}", resp.status()));
        }
        let v: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        Ok(v.get("database") == Some(&serde_json::Value::Bool(true)))
    }

    pub fn register(
        &self,
        secret: &SecretKey,
        public_key_hex: &str,
        endpoints: &[CoordEndpoint],
        ipv4: Option<&str>,
        ipv6: Option<&str>,
    ) -> Result<CoordPeerRecord, String> {
        let pk = public_key_hex.trim().to_ascii_lowercase();
        let ch_url = format!("{}/v1/register/challenge", self.base);
        let ch: serde_json::Value = self
            .send_with_transport_retry(|http| {
                self.with_headers(http.post(&ch_url))
                    .json(&serde_json::json!({ "public_key_hex": pk }))
            })?
            .json()
            .map_err(|e| e.to_string())?;
        let nonce_hex = ch["nonce_hex"].as_str().ok_or("missing nonce_hex")?;
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(&hex::decode(nonce_hex).map_err(|e| e.to_string())?);

        let secp = Secp256k1::new();
        let msg = registration_message_digest(&nonce, &pk);
        let sig = secp.sign_ecdsa(msg, secret);

        let reg_url = format!("{}/v1/register", self.base);
        let resp = self.send_with_transport_retry(|http| {
            self.with_headers(http.post(&reg_url)).json(&serde_json::json!({
                "public_key_hex": pk,
                "nonce_hex": nonce_hex,
                "signature_hex": hex::encode(sig.serialize_der()),
                "endpoints": endpoints,
                "ipv4": ipv4,
                "ipv6": ipv6,
                "transport_capabilities": ["tcp", "sync-v1"]
            }))
        })?;
        if !resp.status().is_success() {
            return Err(format!(
                "register HTTP {}: {}",
                resp.status(),
                resp.text().unwrap_or_default()
            ));
        }
        let v: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        serde_json::from_value(v["peer"].clone()).map_err(|e| e.to_string())
    }

    pub fn heartbeat(&self, public_key_hex: &str) -> Result<CoordPeerRecord, String> {
        let pk = public_key_hex.trim().to_ascii_lowercase();
        let url = format!("{}/v1/heartbeat", self.base);
        let resp = self.send_with_transport_retry(|http| {
            self.with_headers(http.post(&url))
                .json(&serde_json::json!({ "public_key_hex": pk }))
        })?;
        if !resp.status().is_success() {
            return Err(format!("heartbeat HTTP {}", resp.status()));
        }
        let v: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        serde_json::from_value(v["peer"].clone()).map_err(|e| e.to_string())
    }

    /// Fetch the coordinator's co-located Circuit Relay v2 coordinates.
    /// Returns `(peer_id, base_addrs)`; empty when the server runs no relay.
    pub fn get_relay(&self) -> Result<(String, Vec<String>), String> {
        self.get_relay_remap(false)
    }

    /// Like [`get_relay`]; when `remap` is true, home UPnP coord servers renew the WAN port mapping.
    pub fn get_relay_remap(&self, remap: bool) -> Result<(String, Vec<String>), String> {
        let url = if remap {
            format!("{}/v1/relay?remap=true", self.base)
        } else {
            format!("{}/v1/relay", self.base)
        };
        let resp = self.send_with_transport_retry(|http| self.with_headers(http.get(&url)))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            let lower = body.to_ascii_lowercase();
            if lower.contains("<!doctype html") {
                return Err(format!(
                    "coord HTTP transport failure {} (non-JSON body — coord unreachable or proxy error)",
                    status
                ));
            }
            return Err(format!("relay HTTP {}", status));
        }
        let v: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        if v.get("enabled") == Some(&serde_json::Value::Bool(false)) {
            return Ok((String::new(), Vec::new()));
        }
        let peer_id = v["peer_id"].as_str().unwrap_or_default().trim().to_string();
        let addrs = v["addrs"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok((peer_id, addrs))
    }

    pub fn lookup(&self, public_key_hex: &str) -> Result<CoordPeerRecord, String> {
        let pk = public_key_hex.trim().to_ascii_lowercase();
        let url = format!("{}/v1/peers/{}", self.base, pk);
        let resp = self.send_with_transport_retry(|http| self.with_headers(http.get(&url)))?;
        let status = resp.status();
        let body = resp.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("lookup HTTP {}: {}", status, truncate_body(&body)));
        }
        serde_json::from_str(&body).map_err(|e| {
            format!(
                "lookup JSON parse: {e} (body: {})",
                truncate_body(&body)
            )
        })
    }
}

fn truncate_body(body: &str) -> String {
    let t = body.trim();
    if t.len() <= 120 {
        t.to_string()
    } else {
        format!("{}…", &t[..120])
    }
}
