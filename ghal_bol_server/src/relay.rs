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
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm, identify, identity, ping, relay};

use crate::agent_pk::parse_pk_from_agent_version;
use crate::presence::PresenceStore;
use crate::relay_live::RelayLiveRegistry;

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
                    // Advertise both IP families, IPv6 first (preferred when reachable). A
                    // dual-stack resolver returns the host's AAAA + A; a DNS64/IPv6-only carrier
                    // synthesizes an IPv6 mapping for the A record. Clients keep whichever family
                    // routes on their network (see `network_transport::resolve_relay_bootnodes`).
                    public_addrs.push(format!("/dns6/{host}/tcp/{}", listen.port()));
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
/// `relay::Config::default()` caps **every** circuit at **120 s** and **128 KiB**, allows only
/// **16 concurrent circuits** relay-wide, and installs **rate limiters** (~1 circuit / reservation
/// per peer every **2 minutes**). Clients legitimately retry DM reconnect every ~2 s (outbox /
/// coord upkeep), so those default limiters surface as `relay circuit DENIED …
/// ResourceLimitExceeded` and WAN chat never completes. Lift caps for a chat relay and **clear**
/// the default rate limiters — abuse is bounded by `max_circuits*` pool sizes instead.
fn relay_config() -> relay::Config {
    relay::Config {
        max_reservations: 4096,
        max_reservations_per_peer: 16,
        reservation_duration: Duration::from_secs(60 * 60),
        reservation_rate_limiters: vec![],
        max_circuits: 4096,
        max_circuits_per_peer: 16,
        // 0 disables the byte cap entirely (see libp2p CopyFuture: enforced only when > 0).
        max_circuit_bytes: 0,
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
                    .with_agent_version(format!(
                        "ghal_bol_server_relay/{}",
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
pub fn start(
    cfg: RelayConfig,
    presence: Arc<PresenceStore>,
) -> Result<Option<RelayInfo>, Box<dyn std::error::Error + Send + Sync>> {
    if !cfg.enabled {
        tracing::info!("relay disabled (GHAL_BOL_RELAY_ENABLE=0)");
        return Ok(None);
    }

    let keypair = load_or_create_keypair(&cfg.key_path)?;
    let peer_id = keypair.public().to_peer_id();
    let mut swarm = build_swarm(keypair)?;

    // Listen on the configured address and additionally on the counterpart IP family (same port,
    // same scope), so the relay accepts both IPv4 and IPv6 client connections (dual-stack; IPv6
    // preferred when both work). The default `0.0.0.0:4002` covers IPv4; `[::]:<port>` covers IPv6.
    let primary_listen = tcp_listen_multiaddr(cfg.listen);
    swarm.listen_on(primary_listen.clone())?;
    if let Some(counterpart) = counterpart_listen_addr(cfg.listen) {
        let counterpart_listen = tcp_listen_multiaddr(counterpart);
        if let Err(e) = swarm.listen_on(counterpart_listen.clone()) {
            tracing::warn!(
                listen = %counterpart_listen,
                error = %e,
                "relay counterpart-family listen failed — continuing single-stack"
            );
        }
    }

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

    let live = presence.relay_live().clone();
    let ctx = Arc::new(RelayLoopCtx {
        presence,
        relay_info: info.clone(),
        live,
    });
    tokio::spawn(run_relay(swarm, ctx));
    Ok(Some(info))
}

struct RelayLoopCtx {
    presence: Arc<PresenceStore>,
    relay_info: RelayInfo,
    live: RelayLiveRegistry,
}

impl RelayLoopCtx {
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

    fn try_register_relay_presence(&self, peer_id: PeerId) {
        if !self.live.is_peer_live(peer_id) {
            return;
        }
        let Some(pk) = self.pk_for_peer(peer_id) else {
            tracing::debug!(%peer_id, "relay reservation accepted — awaiting identify pk");
            return;
        };
        let Some(circuit_ma) = relay_circuit_multiaddr(&self.relay_info, peer_id) else {
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

async fn run_relay(mut swarm: Swarm<RelayBehaviour>, ctx: Arc<RelayLoopCtx>) {
    loop {
        match swarm.select_next_some().await {
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
