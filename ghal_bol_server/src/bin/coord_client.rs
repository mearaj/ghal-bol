//! Manual coordination API client — register, heartbeat, lookup against a live server.
//!
//! ```bash
//! cargo build -p ghal_bol_server --release
//! ./target/release/coord_client http://127.0.0.1:8765 health
//! ./target/release/coord_client http://127.0.0.1:8765 demo-two-peers
//! ./target/release/coord_client -k https://YOUR.ngrok-free.dev demo-two-peers
//! ```

use ghal_bol_server::registration_message_digest;
use secp256k1::{Secp256k1, SecretKey};
use std::env;
use std::process::ExitCode;

fn parse_cli(args: &[String]) -> Option<(bool, String, String, usize)> {
    let mut i = 1usize;
    let mut insecure_tls = env::var("COORD_INSECURE_TLS")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    while i < args.len() && args[i] == "-k" {
        insecure_tls = true;
        i += 1;
    }
    let base = args.get(i)?.trim_end_matches('/').to_string();
    i += 1;
    let cmd = args.get(i)?.clone();
    i += 1;
    Some((insecure_tls, base, cmd, i))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let Some((insecure_tls, base, cmd, i)) = parse_cli(&args) else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let cmd = cmd.as_str();

    let client = match build_client(insecure_tls) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };

    let code = match cmd {
        "health" => run_health(&client, &base).await,
        "demo-two-peers" => run_demo_two_peers(&client, &base).await,
        "register" => {
            let host = args.get(i).map(|s| s.as_str()).unwrap_or("127.0.0.1");
            let port: u16 = args
                .get(i + 1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(4433);
            let seed = args
                .get(i + 2)
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0x42);
            run_register(&client, &base, seed, host, port).await
        }
        "lookup" => {
            let Some(pk) = args.get(i) else {
                eprintln!("lookup requires public_key_hex");
                return ExitCode::from(2);
            };
            run_lookup(&client, &base, pk).await
        }
        "list" => run_list(&client, &base).await,
        "heartbeat" => {
            let Some(pk) = args.get(i) else {
                eprintln!("heartbeat requires public_key_hex");
                return ExitCode::from(2);
            };
            run_heartbeat(&client, &base, pk).await
        }
        _ => {
            eprintln!("unknown command: {cmd}");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    ExitCode::from(code)
}

const USAGE: &str = r#"Usage:
  coord_client [-k] <base_url> health
  coord_client [-k] <base_url> demo-two-peers
  coord_client [-k] <base_url> register [quic_host] [quic_port] [seed_byte]
  coord_client [-k] <base_url> lookup <public_key_hex>
  coord_client [-k] <base_url> list
  coord_client [-k] <base_url> heartbeat <public_key_hex>

  -k  skip TLS certificate verify (ngrok / self-signed)

Examples:
  coord_client http://127.0.0.1:8765 health
  coord_client -k https://YOUR.ngrok-free.dev demo-two-peers
"#;

fn build_client(insecure_tls: bool) -> Result<reqwest::Client, reqwest::Error> {
    let mut b = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15));
    if insecure_tls {
        b = b.danger_accept_invalid_certs(true);
    }
    b.build()
}

struct PeerKeys {
    secret: SecretKey,
    public_key_hex: String,
}

fn peer_from_seed(seed: u8) -> PeerKeys {
    let secp = Secp256k1::new();
    let secret = SecretKey::from_byte_array([seed; 32]).expect("valid test key");
    let public_key_hex = hex::encode(secret.public_key(&secp).serialize());
    PeerKeys {
        secret,
        public_key_hex,
    }
}

async fn run_health(client: &reqwest::Client, base: &str) -> u8 {
    match client.get(format!("{base}/health")).send().await {
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            println!("HTTP {status}\n{body}");
            if status.is_success() { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("health request failed: {e}");
            1
        }
    }
}

async fn register_peer(
    client: &reqwest::Client,
    base: &str,
    peer: &PeerKeys,
    quic_host: &str,
    quic_port: u16,
) -> Result<serde_json::Value, String> {
    let secp = Secp256k1::new();
    let pk = &peer.public_key_hex;

    let ch = client
        .post(format!("{base}/v1/register/challenge"))
        .json(&serde_json::json!({ "public_key_hex": pk }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !ch.status().is_success() {
        return Err(format!("challenge HTTP {}: {}", ch.status(), ch.text().await.unwrap_or_default()));
    }
    let ch_body: serde_json::Value = ch.json().await.map_err(|e| e.to_string())?;
    let nonce_hex = ch_body["nonce_hex"].as_str().ok_or("missing nonce_hex")?;
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).map_err(|e| e.to_string())?);

    let msg = registration_message_digest(&nonce, pk);
    let sig = secp.sign_ecdsa(msg, &peer.secret);

    let reg = client
        .post(format!("{base}/v1/register"))
        .json(&serde_json::json!({
            "public_key_hex": pk,
            "nonce_hex": nonce_hex,
            "signature_hex": hex::encode(sig.serialize_der()),
            "endpoints": [{ "scheme": "quic", "host": quic_host, "port": quic_port }],
            "ipv4": quic_host,
            "transport_capabilities": ["quic", "sync-v1"]
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !reg.status().is_success() {
        return Err(format!("register HTTP {}: {}", reg.status(), reg.text().await.unwrap_or_default()));
    }
    reg.json().await.map_err(|e| e.to_string())
}

async fn run_register(
    client: &reqwest::Client,
    base: &str,
    seed: u8,
    host: &str,
    port: u16,
) -> u8 {
    let peer = peer_from_seed(seed);
    match register_peer(client, base, &peer, host, port).await {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            0
        }
        Err(e) => {
            eprintln!("register failed: {e}");
            1
        }
    }
}

async fn run_lookup(client: &reqwest::Client, base: &str, pk: &str) -> u8 {
    match client
        .get(format!("{base}/v1/peers/{pk}"))
        .send()
        .await
    {
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            println!("HTTP {status}\n{body}");
            if status.is_success() { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("lookup failed: {e}");
            1
        }
    }
}

async fn run_list(client: &reqwest::Client, base: &str) -> u8 {
    match client.get(format!("{base}/v1/peers")).send().await {
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            println!("HTTP {status}\n{body}");
            if status.is_success() { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("list failed: {e}");
            1
        }
    }
}

async fn run_heartbeat(client: &reqwest::Client, base: &str, pk: &str) -> u8 {
    match client
        .post(format!("{base}/v1/heartbeat"))
        .json(&serde_json::json!({ "public_key_hex": pk }))
        .send()
        .await
    {
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            println!("HTTP {status}\n{body}");
            if status.is_success() { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("heartbeat failed: {e}");
            1
        }
    }
}

async fn run_demo_two_peers(client: &reqwest::Client, base: &str) -> u8 {
    let a = peer_from_seed(0x0a);
    let b = peer_from_seed(0x0b);
    println!("Registering peer A …");
    if register_peer(client, base, &a, "10.0.0.1", 4433).await.is_err() {
        return 1;
    }
    println!("Registering peer B …");
    if register_peer(client, base, &b, "10.0.0.2", 4444).await.is_err() {
        return 1;
    }
    println!("\nA looks up B:");
    if run_lookup(client, base, &b.public_key_hex).await != 0 {
        return 1;
    }
    println!("\nB looks up A:");
    if run_lookup(client, base, &a.public_key_hex).await != 0 {
        return 1;
    }
    println!("\nOnline peers:");
    run_list(client, base).await
}
