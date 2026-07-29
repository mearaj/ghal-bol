//! Integrator SDK: JSON-RPC client over the daemon Unix socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::IntegratorConfig;
use super::client_api::DaemonMethod;

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("connect {path}: {source}")]
    Connect {
        path: String,
        source: std::io::Error,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("rpc: {0}")]
    Rpc(String),
    #[error("daemon disconnected")]
    Disconnected,
}

struct RpcSocket {
    stream: Arc<Mutex<UnixStream>>,
    next_id: AtomicU64,
}

impl RpcSocket {
    fn connect(path: &Path) -> Result<Self, RpcError> {
        let stream = UnixStream::connect(path).map_err(|source| RpcError::Connect {
            path: path.display().to_string(),
            source,
        })?;
        Ok(Self {
            stream: Arc::new(Mutex::new(stream)),
            next_id: AtomicU64::new(1),
        })
    }

    fn call(&self, method: DaemonMethod, params: &Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = json!({ "id": id, "method": method.wire_name(), "params": params });
        let line = serde_json::to_string(&req)?;
        let mut guard = self.stream.lock().map_err(|_| RpcError::Disconnected)?;
        writeln!(guard, "{line}")?;
        guard.flush()?;
        let mut reader = BufReader::new(&mut *guard);
        let mut buf = String::new();
        loop {
            buf.clear();
            if reader.read_line(&mut buf)? == 0 {
                return Err(RpcError::Disconnected);
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let raw: Value = serde_json::from_str(trimmed)?;
            if raw.get("id") != Some(&json!(id)) {
                continue;
            }
            if let Some(err) = raw.get("error") {
                return Err(RpcError::Rpc(err.to_string()));
            }
            return Ok(raw.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// Dual-socket daemon client (main + state) for integrator apps.
pub struct DaemonClient {
    pub config: IntegratorConfig,
    main: RpcSocket,
    state: RpcSocket,
}

impl DaemonClient {
    pub fn connect(config: IntegratorConfig) -> Result<Self, RpcError> {
        let path = config.socket_path();
        let main = RpcSocket::connect(&path)?;
        let state = RpcSocket::connect(&path)?;
        Ok(Self {
            config,
            main,
            state,
        })
    }

    pub fn ping(&self) -> Result<bool, RpcError> {
        let r = self.main.call(DaemonMethod::Ping, &Value::Null)?;
        Ok(r.get("pong").and_then(|v| v.as_bool()) == Some(true))
    }

    pub fn call(&self, method: DaemonMethod, params: &Value) -> Result<Value, RpcError> {
        self.main.call(method, params)
    }

    pub fn call_state(&self, method: DaemonMethod, params: &Value) -> Result<Value, RpcError> {
        self.state.call(method, params)
    }

    pub fn unlock(&self, password: &str) -> Result<Value, RpcError> {
        self.call(
            DaemonMethod::Unlock,
            &json!({
                "app_namespace": self.config.app_namespace,
                "password": password,
            }),
        )
    }

    pub fn session_unlocked(&self) -> Result<bool, RpcError> {
        let r = self.call(DaemonMethod::SessionUnlocked, &Value::Null)?;
        Ok(r.get("unlocked").and_then(|v| v.as_bool()) == Some(true))
    }

    pub fn poll_event(&self) -> Result<Option<Value>, RpcError> {
        let r = self.call_state(DaemonMethod::P2pPoll, &Value::Null)?;
        Ok(r.get("event").cloned().filter(|v| !v.is_null()))
    }

    pub fn sync_ui_session(
        &self,
        ui_visible: bool,
        room_public_key_hex: Option<&str>,
    ) -> Result<Value, RpcError> {
        let mut params = json!({ "ui_visible": ui_visible });
        if let Some(pk) = room_public_key_hex {
            let pk = pk.trim();
            if !pk.is_empty() {
                params["room_public_key_hex"] = json!(pk);
            }
        }
        self.call_state(DaemonMethod::P2pSyncUiSession, &params)
    }

    pub fn take_unlock_wake(&self) -> Result<bool, RpcError> {
        let r = self.call_state(DaemonMethod::P2pTakeUnlockWake, &Value::Null)?;
        Ok(r.get("wake").and_then(|v| v.as_bool()) == Some(true))
    }

    pub fn take_incoming_call_wake(&self) -> Result<bool, RpcError> {
        let r = self.call_state(DaemonMethod::P2pTakeIncomingCallWake, &Value::Null)?;
        Ok(r.get("wake").and_then(|v| v.as_bool()) == Some(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_fails_when_daemon_absent() {
        let cfg = IntegratorConfig::new("com.test.nope");
        assert!(matches!(
            DaemonClient::connect(cfg),
            Err(RpcError::Connect { .. })
        ));
    }
}
