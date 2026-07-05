//! Per-integrator path and environment configuration (multi-app isolation).

use std::collections::HashMap;
use std::path::PathBuf;

use super::client_api::UiWakeKind;
use super::paths::{
    default_socket_path_for_app_namespace, runtime_dir_for_app_namespace,
    sanitize_app_namespace_segment,
};

/// Integrator identity + runtime paths. Must match the daemon process env.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegratorConfig {
    pub app_namespace: String,
    pub socket_path: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
}

impl IntegratorConfig {
    pub fn new(app_namespace: impl Into<String>) -> Self {
        Self {
            app_namespace: app_namespace.into(),
            socket_path: None,
            runtime_dir: None,
        }
    }

    pub fn with_socket_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.socket_path = Some(path.into());
        self
    }

    pub fn with_runtime_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.runtime_dir = Some(dir.into());
        self
    }

    pub fn sanitized_namespace(&self) -> String {
        sanitize_app_namespace_segment(&self.app_namespace)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.runtime_dir
            .clone()
            .unwrap_or_else(|| runtime_dir_for_app_namespace(&self.app_namespace))
    }

    pub fn socket_path(&self) -> PathBuf {
        self.socket_path.clone().unwrap_or_else(|| {
            default_socket_path_for_app_namespace(&self.app_namespace)
        })
    }

    pub fn daemon_spawn_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert(
            "GHAL_BOL_APP_NAMESPACE".to_string(),
            self.app_namespace.clone(),
        );
        if let Some(p) = &self.socket_path {
            env.insert(
                "GHAL_BOL_DAEMON_SOCKET".to_string(),
                p.to_string_lossy().into_owned(),
            );
        }
        if let Some(d) = &self.runtime_dir {
            env.insert(
                "GHAL_BOL_RUNTIME_DIR".to_string(),
                d.to_string_lossy().into_owned(),
            );
        }
        env
    }

    pub fn ui_presence_path(&self) -> PathBuf {
        self.runtime_dir().join(UiWakeKind::UI_PRESENCE_FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_isolated() {
        let a = IntegratorConfig::new("com.app.a");
        let b = IntegratorConfig::new("com.app.b");
        assert_ne!(a.socket_path(), b.socket_path());
        assert_ne!(a.runtime_dir(), b.runtime_dir());
    }

    #[test]
    fn spawn_env_includes_namespace() {
        let cfg = IntegratorConfig::new("com.example.chat");
        let env = cfg.daemon_spawn_env();
        assert_eq!(
            env.get("GHAL_BOL_APP_NAMESPACE").map(String::as_str),
            Some("com.example.chat")
        );
    }
}
