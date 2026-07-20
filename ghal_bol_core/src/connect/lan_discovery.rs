//! mDNS LAN discovery via `mdns-sd` (`_ghalbol._tcp.local`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use flume::Receiver;
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

use super::id_commitment::identity_commitment_hex;
use super::types::CONNECT_MDNS_SERVICE;

#[derive(Clone, Debug)]
pub enum LanDiscoveryEvent {
    Discovered {
        identity_commitment: String,
        host: String,
        port: u16,
    },
    Expired { identity_commitment: String },
}

pub struct LanDiscovery {
    daemon: ServiceDaemon,
    event_rx: Receiver<ServiceEvent>,
    /// identity_wire → commitment hex
    contact_commitments: HashMap<String, String>,
    /// commitment → identity_wire
    commitment_to_wire: HashMap<String, String>,
}

impl LanDiscovery {
    pub fn new() -> Result<Self, String> {
        let daemon = ServiceDaemon::new().map_err(|e| format!("mdns daemon: {e}"))?;
        let event_rx = daemon
            .browse(CONNECT_MDNS_SERVICE)
            .map_err(|e| format!("mdns browse: {e}"))?;
        Ok(Self {
            daemon,
            event_rx,
            contact_commitments: HashMap::new(),
            commitment_to_wire: HashMap::new(),
        })
    }

    pub fn register_contact(&mut self, identity_wire: &str) -> Result<(), String> {
        let idc = identity_commitment_hex(identity_wire)?;
        self.commitment_to_wire
            .insert(idc.clone(), identity_wire.to_string());
        self.contact_commitments
            .insert(identity_wire.to_string(), idc);
        Ok(())
    }


    pub fn publish_listener(&self, identity_wire: &str, port: u16) -> Result<(), String> {
        let idc = identity_commitment_hex(identity_wire)?;
        let host = mdns_local_hostname();
        let mut props = HashMap::new();
        props.insert("v".to_string(), "1".to_string());
        props.insert("idc".to_string(), idc.clone());
        let info = ServiceInfo::new(
            CONNECT_MDNS_SERVICE,
            &idc,
            &host,
            "",
            port,
            Some(props),
        )
        .map_err(|e| format!("mdns service info: {e}"))?;
        self.daemon
            .register(info)
            .map_err(|e| format!("mdns register: {e}"))?;
        Ok(())
    }

    /// Drain pending mDNS events into connect-layer discovery events (non-blocking).
    pub fn drain_events(&self) -> Vec<LanDiscoveryEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    if let Some(ev) = self.resolved_to_event(info.as_ref()) {
                        out.push(ev);
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    if let Some(idc) = fullname.split('.').next() {
                        out.push(LanDiscoveryEvent::Expired {
                            identity_commitment: idc.to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub fn wire_for_commitment(&self, identity_commitment: &str) -> Option<String> {
        self.commitment_to_wire
            .get(identity_commitment)
            .cloned()
    }

    fn resolved_to_event(&self, info: &ResolvedService) -> Option<LanDiscoveryEvent> {
        let idc = info
            .get_property_val_str("idc")
            .map(str::to_string)
            .or_else(|| info.get_fullname().split('.').next().map(str::to_string))?;
        if !self.commitment_to_wire.contains_key(&idc) {
            return None;
        }
        let port = info.get_port();
        let host = pick_lan_ip(info)?;
        Some(LanDiscoveryEvent::Discovered {
            identity_commitment: idc,
            host,
            port,
        })
    }
}

/// Hostname for mDNS-SD registration (`mdns-sd` requires a `.local.` suffix).
fn mdns_local_hostname() -> String {
    let raw = std::env::var("HOSTNAME").unwrap_or_else(|_| "ghalbol".to_string());
    let trimmed = raw.trim().trim_end_matches('.');
    if trimmed.to_ascii_lowercase().ends_with(".local") {
        format!("{trimmed}.")
    } else {
        format!("{trimmed}.local.")
    }
}

fn pick_lan_ip(info: &ResolvedService) -> Option<String> {
    for v4 in info.get_addresses_v4() {
        if !v4.is_loopback() {
            return Some(v4.to_string());
        }
    }
    info.get_addresses_v4().into_iter().next().map(|v4| v4.to_string())
}

/// Async bridge: forward mDNS events to a tokio channel.
pub fn spawn_event_forwarder(
    discovery: Arc<std::sync::Mutex<LanDiscovery>>,
    tx: mpsc::UnboundedSender<LanDiscoveryEvent>,
) {
    std::thread::Builder::new()
        .name("ghal_bol_mdns".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(200));
                let events = discovery
                    .lock()
                    .ok()
                    .map(|d| d.drain_events())
                    .unwrap_or_default();
                for ev in events {
                    let _ = tx.send(ev);
                }
            }
        })
        .ok();
}
