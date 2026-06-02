//! Newline-delimited JSON request/response loop over a Unix domain socket.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use crate::coord_runtime;
use crate::daemon::paths::default_socket_path;
use crate::p2p_runtime;
use crate::session_runtime;

pub fn run_daemon(socket_path: &Path) -> Result<(), String> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    if socket_path.exists() {
        let _ = fs::remove_file(socket_path);
    }
    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("bind {}: {e}", socket_path.display()))?;
    eprintln!("ghal_bol_daemon listening on {}", socket_path.display());

    let shutting_down = Arc::new(AtomicBool::new(false));
    for stream in listener.incoming() {
        if shutting_down.load(Ordering::SeqCst) {
            break;
        }
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("accept: {e}");
                continue;
            }
        };
        let shutdown = Arc::clone(&shutting_down);
        thread::spawn(move || {
            if let Err(e) = handle_client(stream, shutdown) {
                eprintln!("client: {e}");
            }
        });
    }
    Ok(())
}

fn handle_client(stream: UnixStream, shutting_down: Arc<AtomicBool>) -> Result<(), String> {
    let stream = Arc::new(Mutex::new(stream));
    let reader_stream = Arc::clone(&stream);
    let reader = BufReader::new(UnixStreamReader(reader_stream));
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read: {e}"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: Value =
            serde_json::from_str(line).map_err(|e| format!("json request: {e}"))?;
        let id = req.get("id").cloned();
        let method = req
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        if method == "shutdown" {
            let resp = rpc_ok(id, serde_json::json!({ "ok": true }));
            write_response(&stream, &resp)?;
            shutting_down.store(true, Ordering::SeqCst);
            p2p_runtime::p2p_stop();
            session_runtime::lock_identity();
            break;
        }

        let result = dispatch(&method, &params);
        let resp = match result {
            Ok(v) => rpc_ok(id, v),
            Err(e) => rpc_err(id, &e),
        };
        write_response(&stream, &resp)?;
    }
    Ok(())
}

struct UnixStreamReader(Arc<Mutex<UnixStream>>);

impl std::io::Read for UnixStreamReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().read(buf)
    }
}

fn write_response(stream: &Arc<Mutex<UnixStream>>, resp: &Value) -> Result<(), String> {
    let mut line = serde_json::to_string(resp).map_err(|e| format!("encode: {e}"))?;
    line.push('\n');
    let mut guard = stream.lock().map_err(|_| "stream mutex poisoned".to_string())?;
    guard
        .write_all(line.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    guard.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

fn rpc_ok(id: Option<Value>, result: Value) -> Value {
    serde_json::json!({ "id": id, "result": result })
}

fn rpc_err(id: Option<Value>, error: &str) -> Value {
    serde_json::json!({ "id": id, "error": error })
}

fn dispatch(method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "ping" => Ok(serde_json::json!({ "ok": true, "pong": true })),
        "unlock" => {
            let ns = param_str(params, "app_namespace")?;
            let password = param_str(params, "password")?;
            Ok(session_runtime::unlock_identity(&ns, &password))
        }
        "lock" => {
            session_runtime::lock_identity();
            Ok(serde_json::json!({ "ok": true }))
        }
        "session_unlocked" => Ok(serde_json::json!({
            "ok": true,
            "unlocked": session_runtime::session_unlocked(),
        })),
        "p2p_start" => {
            let config = params
                .get("config")
                .cloned()
                .unwrap_or_else(|| params.clone());
            Ok(p2p_runtime::p2p_start(&config))
        }
        "p2p_stop" => {
            p2p_runtime::p2p_stop();
            Ok(serde_json::json!({ "ok": true }))
        }
        "p2p_is_running" => Ok(p2p_runtime::p2p_is_running()),
        "p2p_notify_network_change" => Ok(p2p_runtime::p2p_notify_network_change()),
        "p2p_poll" => Ok(match p2p_runtime::p2p_poll_event() {
            Some(ev) => serde_json::json!({ "ok": true, "event": ev }),
            None => serde_json::json!({ "ok": true, "event": null }),
        }),
        "p2p_send_text_dm" => {
            let recipient = param_str(params, "recipient_public_key_hex")?;
            let text = param_str(params, "text")?;
            Ok(p2p_runtime::p2p_send_text_dm(&recipient, &text))
        }
        "p2p_call_signal" => {
            Ok(p2p_runtime::p2p_call_signal(params))
        }
        "p2p_requeue_outbound_dm" => {
            let message_id = param_str(params, "message_id")?;
            let recipient = param_str(params, "recipient_public_key_hex")?;
            let text = param_str(params, "text")?;
            Ok(p2p_runtime::p2p_requeue_outbound_dm(&message_id, &recipient, &text))
        }
        "p2p_send_ack_dm" => {
            let recipient = param_str(params, "recipient_public_key_hex")?;
            let ref_id = param_str(params, "ref_id")?;
            let ack_kind = param_str(params, "ack_kind")?;
            Ok(p2p_runtime::p2p_send_ack_dm(&recipient, &ref_id, &ack_kind))
        }
        "p2p_dial_bootstrap" => {
            let addrs: Vec<String> = params
                .get("bootstrap_peers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            let parsed: Vec<crate::dm_transport::DmDialAddr> = addrs
                .iter()
                .filter_map(|s| crate::dm_transport::DmDialAddr::parse(s))
                .collect();
            Ok(p2p_runtime::p2p_dial_bootstrap_peers(&parsed))
        }
        "p2p_register_dm_peer" => {
            let pk = param_str(params, "public_key_hex")?;
            Ok(p2p_runtime::p2p_register_dm_peer(&pk))
        }
        "p2p_set_app_ack_read_enabled" => {
            let enabled = params
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Ok(p2p_runtime::p2p_set_app_ack_read_enabled(enabled))
        }
        "p2p_set_foreground_peer" => {
            let pk = params
                .get("public_key_hex")
                .or_else(|| params.get("peer_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            Ok(p2p_runtime::p2p_set_foreground_peer(pk))
        }
        "coord_set_base_url" => {
            let url = param_str(params, "base_url")?;
            let insecure = params
                .get("insecure_tls")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(coord_runtime::coord_set_base_url_json(&url, insecure))
        }
        "coord_lookup_peer" => {
            let pk = param_str(params, "public_key_hex")?;
            Ok(coord_runtime::coord_lookup_peer_json(&pk))
        }
        "coord_register_now" => Ok(coord_runtime::coord_register_now_json()),
        other => Err(format!("unknown method: {other}")),
    }
}

fn param_str(params: &Value, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("missing param: {key}"))
}

/// Try connecting to an existing daemon; returns true if it answers ping.
pub fn probe_existing_daemon(socket_path: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path) else {
        return false;
    };
    let req = serde_json::json!({ "id": 0, "method": "ping", "params": {} });
    let Ok(mut line) = serde_json::to_string(&req) else {
        return false;
    };
    line.push('\n');
    if stream.write_all(line.as_bytes()).is_err() {
        return false;
    }
    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    if reader.read_line(&mut buf).is_err() {
        return false;
    }
    let Ok(resp) = serde_json::from_str::<Value>(&buf) else {
        return false;
    };
    resp.get("result")
        .and_then(|r| r.get("pong"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub fn socket_path_from_env_or_default() -> std::path::PathBuf {
    std::env::var("GHAL_BOL_DAEMON_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path)
}

use std::path::PathBuf;
