//! Production-style E2E: real `ghal_bol_server` process, real TCP, real SQLite on disk.
//!
//! Not in-process mocks. Two peers are two real secp256k1 identities hitting a live listener.

use ghal_bol_server::registration_message_digest;
use reqwest::StatusCode;
use secp256k1::{Secp256k1, SecretKey};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

struct TestPeer {
    secret: SecretKey,
    public_key_hex: String,
    quic_host: &'static str,
    quic_port: u16,
}

impl TestPeer {
    fn new(seed: u8, host: &'static str, port: u16) -> Self {
        let secp = Secp256k1::new();
        let secret = SecretKey::from_byte_array([seed; 32]).expect("test key");
        let public_key_hex = hex::encode(secret.public_key(&secp).serialize());
        Self {
            secret,
            public_key_hex,
            quic_host: host,
            quic_port: port,
        }
    }
}

struct RunningServer {
    child: Child,
    base_url: String,
    _data_dir: TempDir,
    db_dir: std::path::PathBuf,
}

impl RunningServer {
    async fn start() -> Self {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let db_dir = data_dir.path().join("ghalbol_server");
        std::fs::create_dir_all(&db_dir).expect("db dir");

        let port = reserve_tcp_port();
        let listen = format!("127.0.0.1:{port}");
        let base_url = format!("http://{listen}");

        let bin = env!("CARGO_BIN_EXE_ghal_bol_server");
        let child = Command::new(bin)
            .env("GHAL_BOL_SERVER_LISTEN", &listen)
            .env("GHAL_BOL_SERVER_DB", &db_dir)
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ghal_bol_server binary");

        wait_until_healthy(&base_url).await;

        Self {
            child,
            base_url,
            _data_dir: data_dir,
            db_dir,
        }
    }

    fn db_file(&self) -> std::path::PathBuf {
        self.db_dir.join("coord.db")
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn reserve_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local_addr")
        .port()
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client")
}

async fn wait_until_healthy(base_url: &str) {
    let client = http_client();
    for attempt in 0..80 {
        if let Ok(resp) = client.get(format!("{base_url}/health")).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    if body.get("database") == Some(&serde_json::Value::Bool(true)) {
                        return;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        if attempt == 79 {
            panic!("ghal_bol_server not healthy at {base_url}");
        }
    }
}

async fn register_peer(base_url: &str, peer: &TestPeer) -> serde_json::Value {
    let client = http_client();
    let secp = Secp256k1::new();
    let pk = &peer.public_key_hex;

    let ch = client
        .post(format!("{base_url}/v1/register/challenge"))
        .json(&serde_json::json!({ "public_key_hex": pk }))
        .send()
        .await
        .expect("challenge request");
    assert_eq!(ch.status(), StatusCode::OK, "challenge failed");
    let ch_body: serde_json::Value = ch.json().await.expect("challenge json");
    let nonce_hex = ch_body["nonce_hex"].as_str().expect("nonce_hex");
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&hex::decode(nonce_hex).unwrap());

    let msg = registration_message_digest(&nonce, pk);
    let sig = secp.sign_ecdsa(msg, &peer.secret);

    let reg = client
        .post(format!("{base_url}/v1/register"))
        .json(&serde_json::json!({
            "public_key_hex": pk,
            "nonce_hex": nonce_hex,
            "signature_hex": hex::encode(sig.serialize_der()),
            "endpoints": [{
                "scheme": "quic",
                "host": peer.quic_host,
                "port": peer.quic_port
            }],
            "ipv4": peer.quic_host
        }))
        .send()
        .await
        .expect("register request");
    assert_eq!(reg.status(), StatusCode::OK, "register failed: {}", reg.text().await.unwrap_or_default());
    let reg_json: serde_json::Value = reg.json().await.expect("register json");
    reg_json["peer"].clone()
}

async fn lookup_peer(base_url: &str, target: &TestPeer) -> serde_json::Value {
    let client = http_client();
    let resp = client
        .get(format!(
            "{}/v1/peers/{}",
            base_url, target.public_key_hex
        ))
        .send()
        .await
        .expect("lookup request");
    assert_eq!(resp.status(), StatusCode::OK);
    resp.json().await.expect("lookup json")
}

#[tokio::test]
async fn real_tcp_two_peers_register_and_discover_each_other() {
    let server = RunningServer::start().await;
    assert!(server.db_file().is_file(), "sqlite file should exist on disk");

    let peer_a = TestPeer::new(0x0a, "10.0.0.1", 4433);
    let peer_b = TestPeer::new(0x0b, "10.0.0.2", 4444);

    register_peer(&server.base_url, &peer_a).await;
    register_peer(&server.base_url, &peer_b).await;

    let a_sees_b = lookup_peer(&server.base_url, &peer_b).await;
    assert_eq!(a_sees_b["endpoints"][0]["host"], "10.0.0.2");
    assert_eq!(a_sees_b["endpoints"][0]["port"], 4444);

    let b_sees_a = lookup_peer(&server.base_url, &peer_a).await;
    assert_eq!(b_sees_a["endpoints"][0]["host"], "10.0.0.1");

    let list = http_client()
        .get(format!("{}/v1/peers", server.base_url))
        .send()
        .await
        .expect("list")
        .json::<serde_json::Value>()
        .await
        .expect("list json");
    assert_eq!(list["peers"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn real_tcp_server_restart_keeps_peers_in_sqlite() {
    let _data_dir = tempfile::tempdir().expect("tempdir");
    let data_dir = &_data_dir;
        let db_dir = data_dir.path().join("ghalbol_server");
    std::fs::create_dir_all(&db_dir).expect("db dir");
    let port = reserve_tcp_port();
    let listen = format!("127.0.0.1:{port}");
    let base_url = format!("http://{listen}");
    let bin = env!("CARGO_BIN_EXE_ghal_bol_server");

    let peer_a = TestPeer::new(0xaa, "192.168.50.1", 5501);

    // --- Process 1 ---
    let mut child1 = Command::new(bin)
            .env("GHAL_BOL_SERVER_LISTEN", &listen)
            .env("GHAL_BOL_SERVER_DB", &db_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
        .spawn()
        .expect("spawn1");
    wait_until_healthy(&base_url).await;
    register_peer(&base_url, &peer_a).await;
    let _ = child1.kill();
    let _ = child1.wait();

    assert!(db_dir.join("coord.db").is_file());

    // --- Process 2 (same DB path) ---
    let mut child2 = Command::new(bin)
            .env("GHAL_BOL_SERVER_LISTEN", &listen)
            .env("GHAL_BOL_SERVER_DB", &db_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
        .spawn()
        .expect("spawn2");
    wait_until_healthy(&base_url).await;

    let record = lookup_peer(&base_url, &peer_a).await;
    assert_eq!(record["endpoints"][0]["host"], "192.168.50.1");

    let peer_b = TestPeer::new(0xbb, "192.168.50.2", 5502);
    register_peer(&base_url, &peer_b).await;
    lookup_peer(&base_url, &peer_a).await;
    lookup_peer(&base_url, &peer_b).await;

    let _ = child2.kill();
    let _ = child2.wait();
}
