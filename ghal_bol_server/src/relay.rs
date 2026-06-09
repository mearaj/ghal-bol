//! Circuit Relay v2 node (NAT traversal) co-located with the coordination server.
//!
//! Why this lives here: `ghal_bol_server` is the always-on, publicly reachable host
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
use std::time::Duration;

use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use libp2p::{identify, identity, ping, relay, Multiaddr, Swarm};

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
    /// `/dns4/coord.ghalbol.com/tcp/4002`.
    pub public_addrs: Vec<String>,
    /// Persisted ed25519 identity so the relay PeerId is stable across restarts.
    pub key_path: PathBuf,
}

impl RelayConfig {
    /// Defaults: enabled, TCP `0.0.0.0:4002`, identity under the server data dir.
    /// Env: `GHAL_BOL_RELAY_ENABLE` (0/1), `GHAL_BOL_RELAY_LISTEN` (`ip:port`),
    /// `GHAL_BOL_RELAY_PUBLIC_HOST` (→ `/dns4/<host>/tcp/<port>`),
    /// `GHAL_BOL_RELAY_PUBLIC_ADDRS` (comma-separated multiaddrs, overrides host).
    pub fn from_env(data_dir: &Path) -> Self {
        let enabled = std::env::var("GHAL_BOL_RELAY_ENABLE")
            .ok()
            .map(|s| {
                let t = s.trim().to_ascii_lowercase();
                !(t == "0" || t == "false" || t == "no" || t == "off")
            })
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
        if public_addrs.is_empty() {
            if let Ok(host) = std::env::var("GHAL_BOL_RELAY_PUBLIC_HOST") {
                let host = host.trim();
                if !host.is_empty() {
                    public_addrs.push(format!("/dns4/{host}/tcp/{}", listen.port()));
                }
            }
        }

        Self {
            enabled,
            listen,
            public_addrs,
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
/// `relay::Config::default()` caps **every** circuit at **120 s** and **128 KiB** and allows only
/// **16 concurrent circuits** relay-wide. For two peers behind NAT/CGNAT whose DCUtR hole-punch
/// fails, the relay is the *only* data path, so those defaults tear the conversation down after ~2
/// minutes (the client logs `Limit { duration: 120s, data_in_bytes: 131072 }` then `dm peer
/// disconnected`). A chat link must persist, so we lift the per-circuit caps (0 bytes = unlimited)
/// and raise the pool sizes. Rate limiters from `Config::default()` are kept to bound abuse.
fn relay_config() -> relay::Config {
    relay::Config {
        // 0 disables the byte cap entirely (see libp2p CopyFuture: enforced only when > 0).
        max_circuit_bytes: 0,
        // Long enough that an active conversation is never torn down by the relay; the client
        // re-reserves on its own cadence. Stays well under the u32::MAX-seconds protocol limit.
        max_circuit_duration: Duration::from_secs(24 * 60 * 60),
        // Headroom for many concurrent peers plus reconnect/DCUtR churn.
        max_reservations: 4096,
        max_reservations_per_peer: 16,
        max_circuits: 4096,
        max_circuits_per_peer: 16,
        ..relay::Config::default()
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
                // Same identify protocol as the ghal_bol client so observed-addr exchange
                // (used by client AutoNAT/DCUtR) interoperates.
                identify::Config::new_with_signed_peer_record("/ghal-bol/1.0.0".to_string(), key)
                    .with_agent_version(format!("ghal_bol_server_relay/{}", env!("CARGO_PKG_VERSION"))),
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
pub fn start(
    cfg: RelayConfig,
) -> Result<Option<RelayInfo>, Box<dyn std::error::Error + Send + Sync>> {
    if !cfg.enabled {
        tracing::info!("relay disabled (GHAL_BOL_RELAY_ENABLE=0)");
        return Ok(None);
    }

    let keypair = load_or_create_keypair(&cfg.key_path)?;
    let peer_id = keypair.public().to_peer_id();
    let mut swarm = build_swarm(keypair)?;

    let listen_ma: Multiaddr =
        format!("/ip4/{}/tcp/{}", cfg.listen.ip(), cfg.listen.port()).parse()?;
    swarm.listen_on(listen_ma)?;

    if cfg.public_addrs.is_empty() {
        tracing::warn!(
            "relay has no public address advertised — set GHAL_BOL_RELAY_PUBLIC_HOST (or \
             GHAL_BOL_RELAY_PUBLIC_ADDRS) so clients can reserve a circuit"
        );
    }

    // Register the publicly reachable address(es) as *external* addresses of this node.
    //
    // This is required for reservations to work: libp2p's `relay::Behaviour` fills a reservation
    // reply with the relay's *external addresses* (confirmed via `ExternalAddrConfirmed`), NOT its
    // raw `listen_on` addresses. We listen on `0.0.0.0:<port>`, which only expands to local/private
    // interface addresses (127.0.0.1, 192.168.x, docker 172.x, …) and is never added to the
    // external-address set. Behind a tunnel (bore) the kernel cannot observe the public address at
    // all. Without this call the reservation reply carries zero addresses and every client rejects
    // it with `Reservation(Protocol(NoAddressesInReservation))` — the relay then never grants a
    // circuit, so coord registration (which gates on a `/p2p-circuit` endpoint) and WAN DM stall.
    for addr in &cfg.public_addrs {
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

    let info = RelayInfo {
        peer_id: peer_id.to_string(),
        addrs: cfg.public_addrs.clone(),
    };

    tracing::info!(
        peer_id = %info.peer_id,
        listen = %cfg.listen,
        advertised = ?info.addrs,
        "relay v2 node started"
    );

    tokio::spawn(run_relay(swarm));
    Ok(Some(info))
}

async fn run_relay(mut swarm: Swarm<RelayBehaviour>) {
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "relay listening");
            }
            SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(event)) => {
                tracing::debug!(?event, "relay event");
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                tracing::debug!(%peer_id, "relay client connected");
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                tracing::debug!(%peer_id, "relay client disconnected");
            }
            _ => {}
        }
    }
}
