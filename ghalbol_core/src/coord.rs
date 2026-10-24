//! HTTP client for [`ghalbol_coord`](../../ghalbol_coord) coordination API (Tier 1).

use std::net::SocketAddr;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::coord_dial::{coord_dial_overrides, parse_coord_authority};
use crate::coord_register_auth::sign_coord_registration;
use crate::keystore_v1::DecryptedIdentity;
use crate::public_key_util::normalize_contact_identity_wire;

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

/// Remember which TCP dial worked so we do not burn 12s on a dead public hairpin every poll.
#[derive(Clone, Copy, Debug)]
enum DialChoice {
    Override(SocketAddr),
    SystemDns,
}

pub struct CoordHttpClient {
    base: String,
    host: String,
    port: u16,
    insecure_tls: bool,
    preferred: Mutex<Option<DialChoice>>,
}

/// rustls client config trusting the bundled `webpki-roots` store (no OS verifier).
fn webpki_client_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth()
}

fn build_http_client(
    host: &str,
    insecure_tls: bool,
    override_addr: Option<SocketAddr>,
) -> Result<reqwest::blocking::Client, String> {
    crate::rustls_init::ensure_rustls_crypto_provider();
    let mut b = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(25));
    if let Some(addr) = override_addr {
        b = b.resolve(host, addr);
    }
    if insecure_tls {
        b = b.danger_accept_invalid_certs(true);
    } else {
        // Bundled Mozilla roots, same as the delivery WSS client. reqwest's default
        // (`rustls-platform-verifier`) needs a Kotlin class in the APK that the
        // cargo-makepad build does not ship, so every coord HTTPS call failed on
        // Android with ClassNotFoundException.
        b = b.use_preconfigured_tls(webpki_client_config());
    }
    b.build().map_err(|e| e.to_string())
}

impl CoordHttpClient {
    pub fn new(base_url: &str, insecure_tls: bool) -> Result<Self, String> {
        let base = base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err("coord base url empty".into());
        }
        let (host, port) = parse_coord_authority(&base)?;
        Ok(Self {
            base,
            host,
            port,
            insecure_tls,
            preferred: Mutex::new(None),
        })
    }

    fn dial_plan(&self) -> Vec<DialChoice> {
        let mut plan = Vec::new();
        if let Ok(g) = self.preferred.lock() {
            if let Some(pref) = *g {
                plan.push(pref);
            }
        }
        for addr in coord_dial_overrides(&self.host, self.port) {
            let choice = DialChoice::Override(addr);
            if !plan.iter().any(|c| matches!(c, DialChoice::Override(a) if *a == addr)) {
                plan.push(choice);
            }
        }
        if !plan.iter().any(|c| matches!(c, DialChoice::SystemDns)) {
            plan.push(DialChoice::SystemDns);
        }
        plan
    }

    fn client_for(&self, choice: DialChoice) -> Result<reqwest::blocking::Client, String> {
        match choice {
            DialChoice::Override(addr) => {
                build_http_client(&self.host, self.insecure_tls, Some(addr))
            }
            DialChoice::SystemDns => build_http_client(&self.host, self.insecure_tls, None),
        }
    }

    fn remember(&self, choice: DialChoice) {
        if let Ok(mut g) = self.preferred.lock() {
            *g = Some(choice);
        }
    }

    /// Try pinned / IPv6-first dials before plain system DNS so a blackholed
    /// public IPv4 cannot stall every bridge pending poll.
    fn send_with_transport_retry(
        &self,
        build: impl Fn(&reqwest::blocking::Client) -> reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response, String> {
        let mut last = "coord: no dial candidates".to_string();
        for choice in self.dial_plan() {
            let http = match self.client_for(choice) {
                Ok(c) => c,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            match build(&http).send() {
                Ok(resp) => {
                    self.remember(choice);
                    return Ok(resp);
                }
                Err(e) if e.is_connect() || e.is_timeout() || e.is_request() => {
                    std::thread::sleep(std::time::Duration::from_millis(80));
                    match build(&http).send() {
                        Ok(resp) => {
                            self.remember(choice);
                            return Ok(resp);
                        }
                        Err(e2) => last = e2.to_string(),
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Err(last)
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
        ident: &DecryptedIdentity,
        endpoints: &[CoordEndpoint],
        ipv4: Option<&str>,
        ipv6: Option<&str>,
    ) -> Result<CoordPeerRecord, String> {
        let wire = normalize_contact_identity_wire(&ident.identity_wire())?;
        let ch_url = format!("{}/v1/register/challenge", self.base);
        let ch: serde_json::Value = self
            .send_with_transport_retry(|http| {
                self.with_headers(http.post(&ch_url))
                    .json(&serde_json::json!({ "public_key_hex": wire }))
            })?
            .json()
            .map_err(|e| e.to_string())?;
        let nonce_hex = ch["nonce_hex"].as_str().ok_or("missing nonce_hex")?;
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(&hex::decode(nonce_hex).map_err(|e| e.to_string())?);

        let sig = sign_coord_registration(ident, &nonce, &wire)?;
        let sig_hex = hex::encode(sig);

        let reg_url = format!("{}/v1/register", self.base);
        let resp = self.send_with_transport_retry(|http| {
            self.with_headers(http.post(&reg_url))
                .json(&serde_json::json!({
                    "public_key_hex": wire,
                    "nonce_hex": nonce_hex,
                    "signature_hex": sig_hex,
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

    pub fn heartbeat(&self, identity_wire: &str) -> Result<CoordPeerRecord, String> {
        let wire = normalize_contact_identity_wire(identity_wire)?;
        let url = format!("{}/v1/heartbeat", self.base);
        let resp = self.send_with_transport_retry(|http| {
            self.with_headers(http.post(&url))
                .json(&serde_json::json!({ "public_key_hex": wire }))
        })?;
        if !resp.status().is_success() {
            return Err(format!("heartbeat HTTP {}", resp.status()));
        }
        let v: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
        serde_json::from_value(v["peer"].clone()).map_err(|e| e.to_string())
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let resp =
            self.send_with_transport_retry(|http| self.with_headers(http.post(url)).json(body))?;
        if !resp.status().is_success() {
            return Err(format!(
                "POST {} HTTP {}: {}",
                url,
                resp.status(),
                resp.text().unwrap_or_default()
            ));
        }
        resp.json().map_err(|e| e.to_string())
    }

    pub fn get_json(&self, url: &str) -> Result<serde_json::Value, String> {
        let resp = self.send_with_transport_retry(|http| self.with_headers(http.get(url)))?;
        if !resp.status().is_success() {
            return Err(format!(
                "GET {} HTTP {}: {}",
                url,
                resp.status(),
                resp.text().unwrap_or_default()
            ));
        }
        resp.json().map_err(|e| e.to_string())
    }

    pub fn lookup(&self, identity_wire: &str) -> Result<CoordPeerRecord, String> {
        let pk = normalize_contact_identity_wire(identity_wire)?;
        let encoded = crate::identity::percent_encode_uri_component(&pk);
        let url = format!("{}/v1/peers/{}", self.base, encoded);
        let resp = self.send_with_transport_retry(|http| self.with_headers(http.get(&url)))?;
        let status = resp.status();
        let body = resp.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("lookup HTTP {}: {}", status, truncate_body(&body)));
        }
        serde_json::from_str(&body)
            .map_err(|e| format!("lookup JSON parse: {e} (body: {})", truncate_body(&body)))
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
