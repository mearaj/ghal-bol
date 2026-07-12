#[derive(NetworkBehaviour)]
pub struct ChatBehaviour {
    pub relay: libp2p::relay::client::Behaviour,
    /// Disabled when coord is configured — DCUtR hole-punch dials polluted identify addrs and
    /// races stream-first coord/mDNS explicit dials (TRANSPORT.md § Stream-first).
    pub dcutr: Toggle<libp2p::dcutr::Behaviour>,
    pub identify: libp2p::identify::Behaviour,
    pub autonat: libp2p::autonat::Behaviour,
    pub upnp: Toggle<libp2p::upnp::tokio::Behaviour>,
    pub mdns: Toggle<libp2p::mdns::tokio::Behaviour>,
    /// Keepalive: periodic pings keep otherwise-idle DM/relay connections active so libp2p's
    /// `idle_connection_timeout` does not silently drop a live chat link (and the next message
    /// pay a full reconnect). Ping failure also detects a dead route faster.
    pub ping: libp2p::ping::Behaviour,
    pub stream: stream::Behaviour,
}

/// LAN mDNS re-query cadence. libp2p's `Config::default()` queries only every **5 minutes**
/// (and TTLs addrs for 6 minutes), so after a LAN link drops both peers sit on
/// `no mDNS candidate yet` for minutes before re-discovering each other — LAN looks "broken"
/// whenever WAN/relay is also down. We query every few seconds instead so a dropped LAN link is
/// re-discovered within seconds, fully independent of WAN. The ephemeral TCP **listen port stays
/// stable** — discovery is still event-driven from the mDNS `Discovered` handler (TRANSPORT.md
/// § "Ephemeral LAN TCP ports", § "LAN re-discovery cadence"); we only make the query loop fast,
/// we do **not** rebind the port or destructively restart mDNS on a tick (that caused the
/// `mdns restarted … no mdns discovered` storm — AGENTS.md anti-patterns).
const LAN_MDNS_QUERY_INTERVAL_SECS: u64 = 5;

/// mDNS config with a fast re-query interval — see [`LAN_MDNS_QUERY_INTERVAL_SECS`]. Used for both
/// the initial behaviour and the post-handover restart so both paths recover LAN quickly.
fn ghal_bol_mdns_config() -> libp2p::mdns::Config {
    libp2p::mdns::Config {
        query_interval: Duration::from_secs(LAN_MDNS_QUERY_INTERVAL_SECS),
        ..Default::default()
    }
}

/// TCP-only transport when `GHAL_BOL_MINIMAL_SWARM` is set (local integration runs).
#[cfg(all(not(target_os = "android"), not(feature = "test-minimal-swarm")))]
fn minimal_swarm_mode() -> bool {
    std::env::var_os("GHAL_BOL_MINIMAL_SWARM")
        .is_some_and(|v| !matches!(v.to_str(), Some("0" | "false" | "no")))
}

#[inline(never)]
fn chat_behaviour(
    key: &libp2p::identity::Keypair,
    relay: libp2p::relay::client::Behaviour,
) -> ChatBehaviour {
    let local_peer_id = key.public().to_peer_id();
    let pk = crate::session_runtime::unlocked_identity_clone()
        .map(|i| i.identity_wire())
        .unwrap_or_default();
    let identify_cfg =
        libp2p::identify::Config::new_with_signed_peer_record("/ghal-bol/1.0.0".to_string(), key)
            .with_agent_version(format!("ghal_bol/{};pk={}", env!("CARGO_PKG_VERSION"), pk))
            // Push on every ephemeral LAN listen port floods the coord relay identify handler
            // (server logs: "at capacity") → phase D pk never parsed → peer 404 on coord.
            .with_push_listen_addr_updates(false);
    let mdns =
        match libp2p::mdns::tokio::Behaviour::new(ghal_bol_mdns_config(), local_peer_id) {
            Ok(b) => {
                native_log::info("mdns", "enabled");
                Toggle::from(Some(b))
            }
            Err(e) => {
                native_log::warn("mdns", format!("disabled: {e}"));
                Toggle::from(None)
            }
        };
    #[cfg(feature = "test-minimal-swarm")]
    let upnp = Toggle::from(None);
    #[cfg(not(feature = "test-minimal-swarm"))]
    let upnp = Toggle::from(Some(libp2p::upnp::tokio::Behaviour::default()));
    // Keepalive ping: interval must be shorter than `SWARM_IDLE_CONNECTION_TIMEOUT_SECS`
    // (45s on Android) so a healthy-but-idle chat connection is never dropped between messages.
    let ping = libp2p::ping::Behaviour::new(
        libp2p::ping::Config::new()
            .with_interval(Duration::from_secs(PING_INTERVAL_SECS))
            .with_timeout(Duration::from_secs(PING_TIMEOUT_SECS)),
    );
    let dcutr = if crate::coord_runtime::wan_discovery_via_coord_only() {
        native_log::info(
            "dcutr",
            "disabled — coord stream-first uses mDNS LAN + relay circuit only (no hole punch)",
        );
        Toggle::from(None)
    } else {
        Toggle::from(Some(libp2p::dcutr::Behaviour::new(local_peer_id)))
    };
    native_log::info(
        "p2p",
        "behaviours: relay+identify+autonat+upnp+mdns+ping (+dcutr when no coord)",
    );
    ChatBehaviour {
        relay,
        dcutr,
        identify: libp2p::identify::Behaviour::new(identify_cfg),
        autonat: libp2p::autonat::Behaviour::new(local_peer_id, libp2p::autonat::Config::default()),
        upnp,
        mdns,
        ping,
        stream: stream::Behaviour::new(),
    }
}

/// Keepalive ping cadence. Interval is well under the idle-connection timeout so a live but
/// quiet chat link stays up; timeout bounds detection of a dead route.
const PING_INTERVAL_SECS: u64 = 8;
const PING_TIMEOUT_SECS: u64 = 15;

/// Shorter on Android so dead Wi‑Fi TCP does not block bootstrap redial for minutes.
#[cfg(target_os = "android")]
const SWARM_IDLE_CONNECTION_TIMEOUT_SECS: u64 = 45;

#[cfg(not(target_os = "android"))]
const SWARM_IDLE_CONNECTION_TIMEOUT_SECS: u64 = 300;

/// Phones: TCP+noise only (no QUIC/TLS stack) — avoids common Android libp2p build failures.
#[cfg(target_os = "android")]
#[inline(never)]
fn build_swarm(config: &GossipChatConfig) -> Result<Swarm<ChatBehaviour>, ChatServerError> {
    native_log::info("p2p", "swarm transport: android tcp+noise");
    let swarm = SwarmBuilder::with_existing_identity(config.keypair.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| ChatServerError::Transport(format!("tcp: {e}")))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| ChatServerError::Transport(format!("relay client: {e}")))?
        .with_behaviour(|key, relay| Ok(chat_behaviour(key, relay)))
        .map_err(|e| ChatServerError::Transport(format!("behaviour: {e}")))?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(Duration::from_secs(SWARM_IDLE_CONNECTION_TIMEOUT_SECS))
        })
        .build();
    Ok(swarm)
}

/// TCP-only swarm for CI integration tests (`test-minimal-swarm` feature).
#[cfg(all(not(target_os = "android"), feature = "test-minimal-swarm"))]
#[inline(never)]
fn build_swarm(config: &GossipChatConfig) -> Result<Swarm<ChatBehaviour>, ChatServerError> {
    native_log::info("p2p", "swarm transport: minimal tcp+noise (integration)");
    let swarm = SwarmBuilder::with_existing_identity(config.keypair.clone())
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| ChatServerError::Transport(e.to_string()))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| ChatServerError::Transport(format!("relay client: {e}")))?
        .with_behaviour(|key, relay| Ok(chat_behaviour(key, relay)))
        .map_err(|e| ChatServerError::Transport(e.to_string()))?
        .with_swarm_config(|c| {
            c.with_idle_connection_timeout(Duration::from_secs(SWARM_IDLE_CONNECTION_TIMEOUT_SECS))
        })
        .build();
    Ok(swarm)
}

#[cfg(all(not(target_os = "android"), not(feature = "test-minimal-swarm")))]
#[inline(never)]
fn build_swarm(config: &GossipChatConfig) -> Result<Swarm<ChatBehaviour>, ChatServerError> {
    let keypair = config.keypair.clone();
    let swarm = if minimal_swarm_mode() {
        native_log::info("p2p", "swarm transport: minimal tcp+noise (env)");
        SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|e| ChatServerError::Transport(e.to_string()))?
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .map_err(|e| ChatServerError::Transport(format!("relay client: {e}")))?
            .with_behaviour(|key, relay| Ok(chat_behaviour(key, relay)))
            .map_err(|e| ChatServerError::Transport(e.to_string()))?
            .with_swarm_config(|c| {
                c.with_idle_connection_timeout(Duration::from_secs(
                    SWARM_IDLE_CONNECTION_TIMEOUT_SECS,
                ))
            })
            .build()
    } else {
        // TCP uses noise only (same as Android) so phones and desktop can DM on LAN/coord TCP.
        native_log::info("p2p", "swarm transport: tcp+noise+quic");
        SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .map_err(|e| ChatServerError::Transport(e.to_string()))?
            .with_quic()
            .with_dns()
            .map_err(|e| ChatServerError::Transport(format!("dns: {e}")))?
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .map_err(|e| ChatServerError::Transport(format!("relay client: {e}")))?
            .with_behaviour(|key, relay| Ok(chat_behaviour(key, relay)))
            .map_err(|e| ChatServerError::Transport(e.to_string()))?
            .with_swarm_config(|c| {
                c.with_idle_connection_timeout(Duration::from_secs(
                    SWARM_IDLE_CONNECTION_TIMEOUT_SECS,
                ))
            })
            .build()
    };
    Ok(swarm)
}

fn listen_swarm_transports(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
) -> Result<(), ChatServerError> {
    listen_lan_ephemeral_tcp(swarm, session, false)?;
    #[cfg(all(not(target_os = "android"), not(feature = "test-minimal-swarm")))]
    if !minimal_swarm_mode() {
        listen_ephemeral(swarm, "/ip4/0.0.0.0/udp/0/quic-v1")?;
        listen_ephemeral(swarm, "/ip6/::/udp/0/quic-v1")?;
        listen_ephemeral(swarm, "/ip6/::/tcp/0")?;
    }
    Ok(())
}

fn parse_ma(s: &str) -> Result<Multiaddr, ChatServerError> {
    s.parse()
        .map_err(|e: libp2p::multiaddr::Error| ChatServerError::Multiaddr(e.to_string()))
}

fn dial_opts_peer_hint(ma: &Multiaddr) -> Option<PeerId> {
    peer_id_from_multiaddr(ma)
}

