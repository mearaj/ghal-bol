//! Connect worker — TCP listener + mDNS + session table (identity-wire keyed).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use super::types::ConnectOutboundCmd;
use crate::p2p::native_log;

struct ConnectHolder {
    stop: Arc<AtomicBool>,
    join: JoinHandle<()>,
    cmd_tx: std::sync::mpsc::Sender<ConnectOutboundCmd>,
    connected: Arc<Mutex<HashSet<String>>>,
}

fn holder_mx() -> &'static Mutex<Option<ConnectHolder>> {
    static H: OnceLock<Mutex<Option<ConnectHolder>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

pub fn contact_has_lan_connect_path(identity_wire: &str) -> bool {
    let wire = identity_wire.trim();
    if wire.is_empty() {
        return false;
    }
    let Ok(g) = holder_mx().lock() else {
        return false;
    };
    let Some(h) = g.as_ref() else {
        return false;
    };
    let Ok(set) = h.connected.lock() else {
        return false;
    };
    set.iter().any(|w| w.eq_ignore_ascii_case(wire))
}

pub fn connect_start(identity_wire: &str, contacts: &[String]) -> Result<(), String> {
    if let Ok(g) = holder_mx().lock() {
        if g.is_some() {
            return Ok(());
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    let connected = Arc::new(Mutex::new(HashSet::new()));
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<ConnectOutboundCmd>();
    let stop_t = Arc::clone(&stop);
    let connected_t = Arc::clone(&connected);
    let ident_wire = identity_wire.to_string();
    let contacts_owned: Vec<String> = contacts.to_vec();
    let join = std::thread::Builder::new()
        .name("ghal_bol_connect".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    native_log::error("connect", format!("tokio runtime: {e}"));
                    return;
                }
            };
            rt.block_on(connect_sidecar(
                ident_wire,
                contacts_owned,
                stop_t,
                connected_t,
                cmd_rx,
            ));
        })
        .map_err(|e| e.to_string())?;
    if let Ok(mut g) = holder_mx().lock() {
        *g = Some(ConnectHolder {
            stop,
            join,
            cmd_tx,
            connected,
        });
    }
    native_log::info("connect", "native connect sidecar started");
    Ok(())
}

pub fn connect_stop() {
    if let Ok(mut g) = holder_mx().lock() {
        if let Some(h) = g.take() {
            h.stop.store(true, Ordering::SeqCst);
            let _ = h.cmd_tx.send(ConnectOutboundCmd::Stop);
            let _ = h.join.join();
        }
    }
}

async fn connect_sidecar(
    identity_wire: String,
    contacts: Vec<String>,
    stop: Arc<AtomicBool>,
    connected: Arc<Mutex<HashSet<String>>>,
    cmd_rx: std::sync::mpsc::Receiver<ConnectOutboundCmd>,
) {
    use super::lan_discovery::{spawn_event_forwarder, LanDiscovery, LanDiscoveryEvent};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    let listener = match TcpListener::bind("0.0.0.0:0").await {
        Ok(l) => l,
        Err(e) => {
            native_log::error("connect", format!("listen failed: {e}"));
            return;
        }
    };
    let local_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    native_log::info("connect", format!("sidecar TCP listen port={local_port}"));

    let mut discovery = match LanDiscovery::new() {
        Ok(d) => d,
        Err(e) => {
            native_log::error("connect", format!("mdns: {e}"));
            return;
        }
    };
    for c in &contacts {
        let _ = discovery.register_contact(c);
    }
    if let Err(e) = discovery.publish_listener(&identity_wire, local_port) {
        native_log::warn("connect", format!("mdns publish: {e}"));
    }
    let discovery = Arc::new(Mutex::new(discovery));
    let (mdns_tx, mut mdns_rx) = mpsc::unbounded_channel::<LanDiscoveryEvent>();
    spawn_event_forwarder(Arc::clone(&discovery), mdns_tx);

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        while let Ok(cmd) = cmd_rx.try_recv() {
            if matches!(cmd, ConnectOutboundCmd::Stop) {
                return;
            }
        }
        while let Ok(ev) = mdns_rx.try_recv() {
            match ev {
                LanDiscoveryEvent::Discovered { host, port, .. } => {
                    native_log::info("connect", format!("mdns discovered {host}:{port}"));
                }
                LanDiscoveryEvent::Expired { identity_commitment } => {
                    native_log::info("connect", format!("mdns expired {identity_commitment}"));
                }
            }
        }
        tokio::select! {
            accept = listener.accept() => {
                if let Ok((stream, addr)) = accept {
                    native_log::info("connect", format!("inbound TCP from {addr}"));
                    drop(stream);
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
        }
    }
    let _ = connected;
}
