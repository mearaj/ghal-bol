//! Newline-delimited JSON request/response loop over a Unix domain socket.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use crate::daemon::client_api::{DaemonMethod, dispatch_method};
use crate::daemon::paths::default_socket_path;
use crate::daemon::ui_session::UiSessionGuard;
use crate::p2p_runtime;
use crate::session_runtime;

pub fn run_daemon(socket_path: &Path) -> Result<(), String> {
    crate::rustls_init::ensure_rustls_crypto_provider();
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    if socket_path.exists() {
        let _ = fs::remove_file(socket_path);
    }
    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("bind {}: {e}", socket_path.display()))?;
    eprintln!(
        "ghal_bol_core_daemon listening on {}",
        socket_path.display()
    );

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
    let _ui_session = UiSessionGuard::begin();
    let stream = Arc::new(Mutex::new(stream));
    let reader_stream = Arc::clone(&stream);
    let reader = BufReader::new(UnixStreamReader(reader_stream));
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read: {e}"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: Value = serde_json::from_str(line).map_err(|e| format!("json request: {e}"))?;
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        if method == "shutdown" {
            let resp = rpc_ok(id, serde_json::json!({ "ok": true }));
            write_response(&stream, &resp)?;
            shutting_down.store(true, Ordering::SeqCst);
            p2p_runtime::p2p_stop();
            session_runtime::lock_identity();
            break;
        }

        let result = match DaemonMethod::parse(method) {
            Some(m) => dispatch_method(m, &params),
            None => Err(format!("unknown method: {method}")),
        };
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
    let mut guard = stream
        .lock()
        .map_err(|_| "stream mutex poisoned".to_string())?;
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

/// Try connecting to an existing daemon; returns true if it answers ping.
pub fn probe_existing_daemon(socket_path: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path) else {
        return false;
    };
    let req = serde_json::json!({
        "id": 0,
        "method": DaemonMethod::Ping.wire_name(),
        "params": {}
    });
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
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_socket_path)
}
