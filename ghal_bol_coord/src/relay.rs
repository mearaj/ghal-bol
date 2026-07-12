//! Circuit Relay v2 node (NAT traversal) co-located with the coordination server.
//!
//! Why this lives here: `ghal_bol_coord` is the always-on, publicly reachable host
//! (`coord.ghalbol.com`). The HTTP side stays a lightweight presence "phone book"; this
//! relay only carries brief NAT-traversal traffic so that two peers behind NAT/CGNAT can
//! reserve a circuit and then (via DCUtR on the client) upgrade to a direct connection.
//!
//! The HTTP API advertises this relay's PeerId + dialable multiaddrs at `GET /v1/relay`,
//! so clients dial it, reserve a circuit, and register that `/p2p-circuit` address in coord
//! presence. Protocol stack (tcp + noise + yamux + relay/identify/ping) and libp2p version
//! are kept identical to the `ghal_bol` client.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm, identify, identity, ping, relay};
use tokio::sync::mpsc;

use crate::agent_pk::parse_pk_from_agent_version;
use crate::presence::PresenceStore;
use crate::relay_live::RelayLiveRegistry;
use crate::relay_nat::{self, MappedPort};
use crate::AppState;

/// Relay coordinates advertised to clients via `GET /v1/relay`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RelayInfo {
    pub peer_id: String,
    pub addrs: Vec<String>,
}

/// Runtime config for the relay node (env-driven, see [`RelayConfig::from_env`]).
#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub enabled: bool,
    /// Raw TCP listen socket (publicly reachable port on the relay host).
    pub listen: SocketAddr,
    /// Dialable base multiaddrs advertised to clients (without `/p2p/<id>`), e.g.
    /// `/dns4/coord.ghalbol.com/tcp/4002`. Empty when dynamic — filled after UPnP bind.
    pub public_addrs: Vec<String>,
    /// DNS hostname for `/dns4|6/<host>/tcp/<port>` when `public_addrs` is not set explicitly.
    pub public_host: Option<String>,
    /// Bind ephemeral local port + optional UPnP (home NAT). Port `0` in `listen` implies this.
    pub dynamic_listen: bool,
    /// Request UPnP/NAT-PMP external port mapping (default on when `dynamic_listen`).
    pub upnp: bool,
    /// Persisted ed25519 identity so the relay PeerId is stable across restarts.
    pub key_path: PathBuf,
}

fn env_truthy(key: &str) -> Option<bool> {
    std::env::var(key).ok().map(|s| {
        let t = s.trim().to_ascii_lowercase();
        t == "1" || t == "true" || t == "yes" || t == "on"
    })
}

fn env_falsy(key: &str) -> Option<bool> {
    std::env::var(key).ok().map(|s| {
        let t = s.trim().to_ascii_lowercase();
        t == "0" || t == "false" || t == "no" || t == "off"
    })
}

/// Build `/dns6` + `/dns4` relay bootstrap addrs for a hostname and TCP port.
pub fn build_dns_public_addrs(host: &str, port: u16) -> Vec<String> {
    vec![
        format!("/dns6/{host}/tcp/{port}"),
        format!("/dns4/{host}/tcp/{port}"),
    ]
}

impl RelayConfig {
    /// Defaults: enabled, TCP `0.0.0.0:4002`, identity under the server data dir.
    /// Env: `GHAL_BOL_RELAY_ENABLE` (0/1), `GHAL_BOL_RELAY_LISTEN` (`ip:port`, port `0` = ephemeral),
    /// `GHAL_BOL_RELAY_DYNAMIC` (1 = ephemeral + UPnP for home NAT),
    /// `GHAL_BOL_RELAY_UPNP` (0 to skip router mapping),
    /// `GHAL_BOL_RELAY_PUBLIC_HOST` (→ `/dns4/<host>/tcp/<port>`),
    /// `GHAL_BOL_RELAY_PUBLIC_ADDRS` (comma-separated multiaddrs, overrides host).
    pub fn from_env(data_dir: &Path) -> Self {
        let enabled = env_falsy("GHAL_BOL_RELAY_ENABLE")
            .map(|f| !f)
            .unwrap_or(true);

        let listen: SocketAddr = std::env::var("GHAL_BOL_RELAY_LISTEN")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or_else(|| "0.0.0.0:4002".parse().expect("valid default relay listen"));

        let mut public_addrs: Vec<String> = std::env::var("GHAL_BOL_RELAY_PUBLIC_ADDRS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let public_host = std::env::var("GHAL_BOL_RELAY_PUBLIC_HOST")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let dynamic_listen =
            env_truthy("GHAL_BOL_RELAY_DYNAMIC").unwrap_or(false) || listen.port() == 0;

        let upnp = env_falsy("GHAL_BOL_RELAY_UPNP")
            .map(|f| !f)
            .unwrap_or(dynamic_listen);

        // Static installs know the WAN port up front; dynamic defers until UPnP returns.
        if public_addrs.is_empty() && !dynamic_listen {
            if let Some(ref host) = public_host {
                public_addrs = build_dns_public_addrs(host, listen.port());
            }
        }

        Self {
            enabled,
            listen,
            public_addrs,
            public_host,
            dynamic_listen,
            upnp,
            key_path: data_dir.join("relay_ed25519.key"),
        }
    }
}

#[derive(libp2p::swarm::NetworkBehaviour)]
struct RelayBehaviour {
    relay: relay::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
}

/// Relay limits tuned for a real chat link, not the libp2p defaults.
///
/// `relay::Config::default()` caps **every** circuit at **120 s** and **128 KiB**, allows only
/// **16 concurrent circuits** relay-wide, and installs **rate limiters** (~1 circuit / reservation
/// per peer every **2 minutes**). Clients legitimately retry DM reconnect every ~2 s (outbox /
/// coord upkeep), so those default limiters surface as `relay circuit DENIED …
/// ResourceLimitExceeded` and WAN chat never completes. Lift caps for a chat relay and **clear**
/// the default rate limiters — abuse is bounded by `max_circuits*` pool sizes instead.
///
/// Env (optional bandwidth caps):
/// - `GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES` — max bytes relayed per circuit (0 = unlimited)
/// - `GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER` — concurrent circuits per peer (default 16)
fn relay_config() -> relay::Config {
    let max_circuit_bytes = std::env::var("GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let max_circuits_per_peer = std::env::var("GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(16);
    relay::Config {
        max_reservations: 4096,
        max_reservations_per_peer: 16,
        reservation_duration: Duration::from_secs(60 * 60),
        reservation_rate_limiters: vec![],
        max_circuits: 4096,
        max_circuits_per_peer,
        max_circuit_bytes,
        max_circuit_duration: Duration::from_secs(24 * 60 * 60),
        circuit_src_rate_limiters: vec![],
    }
}

/// Load a persisted ed25519 identity, or generate + persist a new one.
fn load_or_create_keypair(path: &Path) -> std::io::Result<identity::Keypair> {
    if let Ok(bytes) = std::fs::read(path) {
        if let Ok(kp) = identity::Keypair::from_protobuf_encoding(&bytes) {
            return Ok(kp);
        }
        tracing::warn!(path = %path.display(), "relay key unreadable — regenerating");
    }
    let kp = identity::Keypair::generate_ed25519();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let enc = kp
        .to_protobuf_encoding()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(path, &enc)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(kp)
}

/// Build a `/ip4|/ip6/.../tcp/<port>` listen multiaddr from a socket address (family-aware).
fn tcp_listen_multiaddr(sa: SocketAddr) -> Multiaddr {
    let s = match sa.ip() {
        std::net::IpAddr::V4(ip) => format!("/ip4/{ip}/tcp/{}", sa.port()),
        std::net::IpAddr::V6(ip) => format!("/ip6/{ip}/tcp/{}", sa.port()),
    };
    s.parse().expect("valid tcp listen multiaddr")
}

/// Counterpart-family listen address preserving scope (wildcard↔wildcard, loopback↔loopback). A
/// specific interface IP has no safe counterpart (`None`) — never widen it into a public wildcard.
fn counterpart_listen_addr(primary: SocketAddr) -> Option<SocketAddr> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    let port = primary.port();
    match primary.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port))
        }
        IpAddr::V4(ip) if ip.is_loopback() => {
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port))
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port))
        }
        IpAddr::V6(ip) if ip.is_loopback() => {
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        }
        _ => None,
    }
}

/// Build advertised relay multiaddrs from config + optional UPnP mapping.
fn finalize_public_addrs(
    cfg: &RelayConfig,
    listen: SocketAddr,
    mapping: Option<&MappedPort>,
) -> Vec<String> {
    if !cfg.public_addrs.is_empty() {
        return cfg.public_addrs.clone();
    }
    let advertise_port = mapping
        .map(|m| m.external_port)
        .unwrap_or_else(|| listen.port());
    // Dynamic + no UPnP yet: do not publish the local ephemeral port as WAN-reachable.
    if cfg.dynamic_listen && mapping.is_none() {
        return Vec::new();
    }
    let mut addrs = cfg
        .public_host
        .as_ref()
        .map(|host| build_dns_public_addrs(host, advertise_port))
        .unwrap_or_default();
    if let Some(mapping) = mapping {
        if let Some(ip_ma) = mapping.external_multiaddr() {
            if !addrs.iter().any(|a| a == &ip_ma) {
                addrs.push(ip_ma);
            }
        }
        if let Some(lan_ma) = mapping.local_lan_multiaddr() {
            if !addrs.iter().any(|a| a == &lan_ma) {
                addrs.push(lan_ma);
            }
        }
    }
    addrs
}

fn unix_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn publish_upnp_addrs(
    listen: SocketAddr,
    cfg: &RelayConfig,
    peer_id: &str,
    app_state: Option<&Arc<AppState>>,
    addr_tx: &mpsc::Sender<Vec<String>>,
    mapping: Option<&MappedPort>,
) {
    let addrs = finalize_public_addrs(cfg, listen, mapping);
    if addrs.is_empty() {
        return;
    }
    let _ = addr_tx.try_send(addrs.clone());
    if let Some(state) = app_state {
        state.set_relay_info(RelayInfo {
            peer_id: peer_id.to_string(),
            addrs,
        });
    }
}

fn clear_upnp_advertised(
    peer_id: &str,
    app_state: Option<&Arc<AppState>>,
    addr_tx: &mpsc::Sender<Vec<String>>,
) {
    let _ = addr_tx.try_send(Vec::new());
    if let Some(state) = app_state {
        state.set_relay_info(RelayInfo {
            peer_id: peer_id.to_string(),
            addrs: Vec::new(),
        });
    }
}

/// Startup worker: UPnP unknown duration — retry until mapped (TRANSPORT.md § Event-driven async).
fn spawn_upnp_startup_worker(
    listen: SocketAddr,
    cfg: RelayConfig,
    peer_id: String,
    app_state: Option<Arc<AppState>>,
    addr_tx: mpsc::Sender<Vec<String>>,
    current_mapping: std::sync::Arc<std::sync::Mutex<Option<MappedPort>>>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            match relay_nat::map_relay_port_with_retries(listen, 3).await {
                Ok(mapping) => {
                    tracing::info!(
                        external_port = mapping.external_port,
                        external = %mapping.external_ip,
                        "relay UPnP mapped on background retry"
                    );
                    if let Ok(mut g) = current_mapping.lock() {
                        *g = Some(mapping.clone());
                    }
                    publish_upnp_addrs(
                        listen,
                        &cfg,
                        &peer_id,
                        app_state.as_ref(),
                        &addr_tx,
                        Some(&mapping),
                    );
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "relay UPnP background retry — router not responding yet");
                }
            }
        }
    });
}

/// Storm-throttled remap on client `/v1/relay` refetch — not a periodic poll.
const UPNP_REMAP_STORM_MS: i64 = 10_000;

fn spawn_upnp_remap_worker(
    listen: SocketAddr,
    cfg: RelayConfig,
    peer_id: String,
    app_state: Option<Arc<AppState>>,
    addr_tx: mpsc::Sender<Vec<String>>,
    mut remap_rx: mpsc::Receiver<()>,
    current_mapping: std::sync::Arc<std::sync::Mutex<Option<MappedPort>>>,
) {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST_REMAP_MS: AtomicI64 = AtomicI64::new(0);

    tokio::spawn(async move {
        while remap_rx.recv().await.is_some() {
            let now = unix_ms_now();
            let last = LAST_REMAP_MS.load(Ordering::Relaxed);
            if last > 0 && now.saturating_sub(last) < UPNP_REMAP_STORM_MS {
                continue;
            }
            LAST_REMAP_MS.store(now, Ordering::Relaxed);
            let previous_port = current_mapping
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|m| m.external_port));
            match relay_nat::remap_after_client_bootstrap_failure(listen, previous_port).await {
                Ok(mapping) => {
                    tracing::info!(
                        external_port = mapping.external_port,
                        external = %mapping.external_ip,
                        previous_port = ?previous_port,
                        "relay UPnP remapped after client bootstrap-failure signal"
                    );
                    if previous_port != Some(mapping.external_port) {
                        tracing::warn!(
                            old = ?previous_port,
                            new = mapping.external_port,
                            "relay UPnP external port changed — GET /v1/relay updated"
                        );
                    }
                    if let Ok(mut g) = current_mapping.lock() {
                        *g = Some(mapping.clone());
                    }
                    publish_upnp_addrs(
                        listen,
                        &cfg,
                        &peer_id,
                        app_state.as_ref(),
                        &addr_tx,
                        Some(&mapping),
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "relay UPnP remap failed — clearing advertised addrs until router responds"
                    );
                    if let Ok(mut g) = current_mapping.lock() {
                        *g = None;
                    }
                    clear_upnp_advertised(&peer_id, app_state.as_ref(), &addr_tx);
                }
            }
        }
    });
}

fn register_external_addrs(swarm: &mut Swarm<RelayBehaviour>, addrs: &[String]) {
    for addr in addrs {
        match addr.parse::<Multiaddr>() {
            Ok(ma) => {
                swarm.add_external_address(ma.clone());
                tracing::info!(%ma, "relay external address registered (reservations advertise this)");
            }
            Err(e) => {
                tracing::warn!(addr = %addr, error = %e, "invalid relay public addr — skipping");
            }
        }
    }
}
fn build_swarm(
    keypair: identity::Keypair,
) -> Result<Swarm<RelayBehaviour>, Box<dyn std::error::Error + Send + Sync>> {
    let swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default().nodelay(true),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_dns()?
        .with_behaviour(|key| RelayBehaviour {
            relay: relay::Behaviour::new(key.public().to_peer_id(), relay_config()),
            identify: identify::Behaviour::new(
                // Same identify protocol as the ghal_bol_core client so observed-addr exchange
                // (used by client AutoNAT/DCUtR) interoperates.
                identify::Config::new_with_signed_peer_record("/ghal-bol/1.0.0".to_string(), key)
                    .with_agent_version(format!(
                        "ghal_bol_coord_relay/{}",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .with_push_listen_addr_updates(false),
            ),
            ping: ping::Behaviour::new(ping::Config::new()),
        })?
        // Keep relayed connections alive between client keepalive pings (client pings ~15s).
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(600)))
        .build();
    Ok(swarm)
}

/// Start the relay node (spawns its swarm loop on the current Tokio runtime) and return
/// the coordinates to advertise. Returns `Ok(None)` when disabled. Must be called from
/// within a Tokio runtime (the binary's `#[tokio::main]`).
pub async fn start(
    cfg: RelayConfig,
    presence: Arc<PresenceStore>,
    app_state: Option<Arc<AppState>>,
) -> Result<Option<RelayInfo>, Box<dyn std::error::Error + Send + Sync>> {
    if !cfg.enabled {
        tracing::info!("relay disabled (GHAL_BOL_RELAY_ENABLE=0)");
        return Ok(None);
    }

    let mut listen = cfg.listen;
    if cfg.dynamic_listen && listen.port() == 0 {
        listen = relay_nat::reserve_ephemeral_tcp_port(listen).await?;
        tracing::info!(%listen, "relay dynamic — ephemeral local TCP port");
    }

    let mut upnp_mapping: Option<MappedPort> = None;
    let mut upnp_pending = false;

    if cfg.upnp && cfg.dynamic_listen {
        match relay_nat::map_relay_port(listen).await {
            Ok(mapping) => {
                tracing::info!(
                    local = %listen,
                    external = %mapping.external_ip,
                    external_port = mapping.external_port,
                    upnp_internal = %mapping.local_addr,
                    "relay UPnP/NAT-PMP mapped — advertising external port"
                );
                upnp_mapping = Some(mapping);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "relay UPnP mapping failed on startup — retrying in background (WAN relay pending)"
                );
                upnp_pending = true;
            }
        }
    }

    let public_addrs = finalize_public_addrs(&cfg, listen, upnp_mapping.as_ref());
    let advertise_port = upnp_mapping
        .as_ref()
        .map(|m| m.external_port)
        .unwrap_or(listen.port());

    let keypair = load_or_create_keypair(&cfg.key_path)?;
    let peer_id = keypair.public().to_peer_id();
    let mut swarm = build_swarm(keypair)?;

    // Listen on the configured address and additionally on the counterpart IP family (same port,
    // same scope), so the relay accepts both IPv4 and IPv6 client connections (dual-stack; IPv6
    // preferred when both work). The default `0.0.0.0:4002` covers IPv4; `[::]:<port>` covers IPv6.
    let primary_listen = tcp_listen_multiaddr(listen);
    swarm.listen_on(primary_listen.clone())?;
    if let Some(counterpart) = counterpart_listen_addr(listen) {
        let counterpart_listen = tcp_listen_multiaddr(counterpart);
        if let Err(e) = swarm.listen_on(counterpart_listen.clone()) {
            tracing::warn!(
                listen = %counterpart_listen,
                error = %e,
                "relay counterpart-family listen failed — continuing single-stack"
            );
        }
    }

    if public_addrs.is_empty() && !upnp_pending {
        tracing::warn!(
            "relay has no public address advertised — set GHAL_BOL_RELAY_PUBLIC_HOST (or \
             GHAL_BOL_RELAY_PUBLIC_ADDRS) so clients can reserve a circuit"
        );
    } else if upnp_pending {
        tracing::info!(
            "relay WAN addrs pending — UPnP background retry (GET /v1/relay updates when mapped)"
        );
    }

    register_external_addrs(&mut swarm, &public_addrs);

    let info = RelayInfo {
        peer_id: peer_id.to_string(),
        addrs: public_addrs,
    };

    let mut addr_rx: Option<mpsc::Receiver<Vec<String>>> = None;
    if cfg.upnp && cfg.dynamic_listen {
        let (addr_tx, rx) = mpsc::channel(4);
        addr_rx = Some(rx);
        let (remap_tx, remap_rx) = mpsc::channel(8);
        let current_mapping = std::sync::Arc::new(std::sync::Mutex::new(upnp_mapping.clone()));
        if let Some(ref st) = app_state {
            st.set_upnp_remap_tx(remap_tx);
        }
        if upnp_pending {
            spawn_upnp_startup_worker(
                listen,
                cfg.clone(),
                info.peer_id.clone(),
                app_state.clone(),
                addr_tx.clone(),
                std::sync::Arc::clone(&current_mapping),
            );
        }
        spawn_upnp_remap_worker(
            listen,
            cfg.clone(),
            info.peer_id.clone(),
            app_state,
            addr_tx,
            remap_rx,
            current_mapping,
        );
    }

    tracing::info!(
        peer_id = %info.peer_id,
        listen = %listen,
        dynamic = cfg.dynamic_listen,
        upnp = upnp_mapping.is_some(),
        upnp_pending,
        advertise_port,
        advertised = ?info.addrs,
        "relay v2 node started"
    );

    let live = presence.relay_live().clone();
    let ctx = Arc::new(RelayLoopCtx {
        presence,
        relay_info: Arc::new(Mutex::new(info.clone())),
        live,
    });
    tokio::spawn(run_relay(swarm, ctx, addr_rx));
    Ok(Some(info))
}

struct RelayLoopCtx {
    presence: Arc<PresenceStore>,
    relay_info: Arc<Mutex<RelayInfo>>,
    live: RelayLiveRegistry,
}

impl RelayLoopCtx {
    fn relay_info_snapshot(&self) -> RelayInfo {
        self.relay_info
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| RelayInfo {
                peer_id: String::new(),
                addrs: Vec::new(),
            })
    }
    fn pk_for_peer(&self, peer_id: PeerId) -> Option<String> {
        self.live.pk_for_peer(peer_id)
    }

    fn note_peer_pk(&self, peer_id: PeerId, pk: String) {
        self.live.note_peer_pk(peer_id, pk);
    }

    fn on_identify(&self, peer_id: PeerId, agent_version: &str) {
        let Some(pk) = parse_pk_from_agent_version(agent_version) else {
            return;
        };
        self.note_peer_pk(peer_id, pk);
        self.try_register_relay_presence(peer_id);
    }

    fn on_reservation_accepted(&self, peer_id: PeerId, renewed: bool) {
        self.live.on_reservation_accepted(peer_id, renewed);
        self.try_register_relay_presence(peer_id);
    }

    fn end_reservation(&self, peer_id: PeerId) {
        self.clear_reservation(peer_id);
    }

    fn clear_reservation(&self, peer_id: PeerId) {
        let pk = self.pk_for_peer(peer_id);
        if let Some(pk) = pk.as_deref() {
            let store = Arc::clone(&self.presence);
            match store.remove_relay_circuit(pk) {
                Ok(removed) => tracing::info!(
                    public_key = %pk,
                    removed,
                    "coord presence cleared after relay reservation end"
                ),
                Err(e) => tracing::warn!(public_key = %pk, error = %e, "relay presence remove failed"),
            }
        }
        self.live.on_reservation_end(peer_id);
    }

    /// Re-touch the coord presence row for every peer with a live reservation so it never expires
    /// while the reservation is held. The relay grants hour-long reservations but coord rows expire
    /// after `presence_ttl` (90 s), and `ReservationReqAccepted{renewed:true}` only fires on the
    /// hourly libp2p renewal — far too rarely to keep the row fresh (and `try_register_relay_presence`
    /// early-returns on an unchanged circuit anyway). Without this, a relay-only (NAT'd) peer is a
    /// coord 404 ~90 s after reserving even though it stays dialable for the full hour. Quiet by
    /// design (no per-peer info log) — see TRANSPORT.md § "Relay presence keepalive".
    fn refresh_live_presence(&self) {
        let live = self.live.live_peers_with_pk();
        if live.is_empty() {
            return;
        }
        let mut refreshed = 0usize;
        for (peer_id, pk) in live {
            let Some(circuit_ma) =
                relay_circuit_multiaddr(&self.relay_info_snapshot(), peer_id)
            else {
                continue;
            };
            match self.presence.upsert_relay_circuit(&pk, circuit_ma) {
                Ok(_) => refreshed += 1,
                Err(e) => {
                    tracing::warn!(public_key = %pk, %peer_id, error = %e, "relay presence keepalive upsert failed")
                }
            }
        }
        if refreshed > 0 {
            tracing::debug!(refreshed, "relay presence keepalive — re-touched live circuits");
        }
    }

    fn try_register_relay_presence(&self, peer_id: PeerId) {
        if !self.live.is_peer_live(peer_id) {
            return;
        }
        let Some(pk) = self.pk_for_peer(peer_id) else {
            tracing::debug!(%peer_id, "relay reservation accepted — awaiting identify pk");
            return;
        };
        let Some(circuit_ma) = relay_circuit_multiaddr(&self.relay_info_snapshot(), peer_id) else {
            tracing::warn!(%peer_id, "relay circuit multiaddr build failed");
            return;
        };
        if let Ok(existing) = self.presence.get_stored(&pk) {
            let already = existing.endpoints.iter().any(|e| {
                e.scheme == "libp2p" && e.host.contains("/p2p-circuit") && e.host == circuit_ma
            });
            if already {
                return;
            }
        }
        let store = Arc::clone(&self.presence);
        match store.upsert_relay_circuit(&pk, circuit_ma) {
            Ok(peer) => tracing::info!(
                public_key = %pk,
                %peer_id,
                endpoints = peer.endpoints.len(),
                "coord presence registered from relay reservation"
            ),
            Err(e) => {
                tracing::warn!(public_key = %pk, %peer_id, error = %e, "relay presence upsert failed")
            }
        }
    }
}

fn relay_circuit_multiaddr(relay_info: &RelayInfo, client_peer: PeerId) -> Option<String> {
    let relay_pk = relay_info.peer_id.parse::<PeerId>().ok()?;
    let base = relay_info
        .addrs
        .iter()
        .find(|a| a.contains("/ip4/"))
        .or_else(|| relay_info.addrs.first())?;
    Some(format!(
        "{base}/p2p/{relay_pk}/p2p-circuit/p2p/{client_peer}"
    ))
}

async fn run_relay(
    mut swarm: Swarm<RelayBehaviour>,
    ctx: Arc<RelayLoopCtx>,
    mut addr_rx: Option<mpsc::Receiver<Vec<String>>>,
) {
    // Keep coord presence rows fresh while reservations are held. Must be well under the coord
    // `presence_ttl` (90 s default) so a relay-only peer never 404s between hourly libp2p
    // reservation renewals — see `RelayLoopCtx::refresh_live_presence`.
    let mut presence_keepalive = tokio::time::interval(Duration::from_secs(30));
    presence_keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let event = tokio::select! {
            _ = presence_keepalive.tick() => {
                ctx.refresh_live_presence();
                continue;
            }
            addrs = async {
                match addr_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if addr_rx.is_some() => {
                if let Some(addrs) = addrs {
                    register_external_addrs(&mut swarm, &addrs);
                    if let Ok(mut g) = ctx.relay_info.lock() {
                        g.addrs = addrs;
                    }
                } else {
                    addr_rx = None;
                }
                continue;
            }
            event = swarm.select_next_some() => event,
        };
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "relay listening");
            }
            // Reservation / circuit decisions at INFO so the relay's view of each client is visible
            // under the default `RUST_LOG=info` (a CGNAT phone stuck "waiting for
            // ReservationReqAccepted" while a Wi‑Fi peer reserves fine is diagnosed from here — see
            // docs/TRANSPORT.md § "CGNAT / mobile-data relay reservation").
            SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(event)) => match event {
                relay::Event::ReservationReqAccepted {
                    src_peer_id,
                    renewed,
                } => {
                    tracing::info!(%src_peer_id, renewed, "relay reservation ACCEPTED");
                    ctx.on_reservation_accepted(src_peer_id, renewed);
                }
                relay::Event::ReservationReqDenied {
                    src_peer_id,
                    status,
                } => {
                    tracing::warn!(%src_peer_id, ?status, "relay reservation DENIED");
                }
                relay::Event::ReservationTimedOut { src_peer_id } => {
                    tracing::info!(%src_peer_id, "relay reservation timed out");
                    ctx.end_reservation(src_peer_id);
                }
                relay::Event::ReservationClosed { src_peer_id } => {
                    tracing::info!(%src_peer_id, "relay reservation closed");
                    // Bootstrap happy-eyeballs closes spare TCP hops; libp2p emits
                    // ReservationClosed while the client re-reserves on another link.
                    // Lookup gates on live registry — stale SQLite rows are not dialable.
                    ctx.live.on_reservation_closed(src_peer_id);
                }
                relay::Event::CircuitReqAccepted {
                    src_peer_id,
                    dst_peer_id,
                } => {
                    tracing::info!(%src_peer_id, %dst_peer_id, "relay circuit ACCEPTED");
                }
                relay::Event::CircuitReqDenied {
                    src_peer_id,
                    dst_peer_id,
                    status,
                } => {
                    tracing::warn!(%src_peer_id, %dst_peer_id, ?status, "relay circuit DENIED");
                    let status_s = format!("{status:?}");
                    // Stale coord mirror: lookup may still list dst circuit while reservation is gone.
                    if status_s.contains("NoReservation") {
                        ctx.end_reservation(dst_peer_id);
                    } else if status_s.contains("ConnectionFailed") {
                        if !ctx.live.is_peer_live(dst_peer_id) {
                            ctx.clear_reservation(dst_peer_id);
                        }
                    }
                }
                relay::Event::CircuitClosed {
                    src_peer_id,
                    dst_peer_id,
                    error,
                } => {
                    tracing::info!(%src_peer_id, %dst_peer_id, ?error, "relay circuit closed");
                }
                other => {
                    tracing::debug!(?other, "relay event");
                }
            },
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                tracing::info!(
                    %peer_id,
                    remote = %endpoint.get_remote_address(),
                    "relay client connected"
                );
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                tracing::info!(%peer_id, ?cause, "relay client disconnected");
                // Do not clear coord presence here — bootstrap happy-eyeballs and HOP
                // prune close spare TCP links while the reservation stays live. Only
                // ReservationClosed / ReservationTimedOut are authoritative.
            }
            SwarmEvent::Behaviour(RelayBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
                ctx.on_identify(peer_id, &info.agent_version);
            }
            _ => {}
        }
    }
}
