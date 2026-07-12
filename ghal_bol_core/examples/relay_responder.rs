//! App-faithful relay responder: reserves a circuit on the given relay using the SAME libp2p
//! behaviour set + transport as the ghal_bol_core client, then stays alive to accept incoming
//! circuits. Prints its peer id so a dialer can reach it via `/p2p-circuit/p2p/<id>`.
//!
//! Used to isolate whether the full app swarm (autonat/mdns/upnp/stream/quic + signed identify)
//! refuses the relay's inbound STOP (cloud WAN bug repro).
//!
//! Usage:
//!   cargo run -p ghal_bol_core --example relay_responder -- <relay-base-addr> <relay-peer-id>

use std::time::Duration;

use futures::StreamExt;
use libp2p::core::multiaddr::Protocol;
use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, autonat, dcutr, identify, identity, mdns, ping, relay, upnp};
use libp2p_stream as stream;

#[derive(libp2p::swarm::NetworkBehaviour)]
struct App {
    relay: relay::client::Behaviour,
    dcutr: Toggle<dcutr::Behaviour>,
    identify: identify::Behaviour,
    autonat: autonat::Behaviour,
    upnp: Toggle<upnp::tokio::Behaviour>,
    mdns: Toggle<mdns::tokio::Behaviour>,
    ping: ping::Behaviour,
    stream: stream::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: relay_responder <relay-base-addr> <relay-peer-id>");
        std::process::exit(2);
    }
    let relay_base: Multiaddr = args[0].parse()?;
    let relay_peer: PeerId = args[1].parse()?;

    let key = identity::Keypair::generate_secp256k1();
    let pk_hex = "02".to_string() + &"ab".repeat(32); // dummy 66-hex agent pk tag
    let local = key.public().to_peer_id();

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_relay_client(libp2p::noise::Config::new, libp2p::yamux::Config::default)?
        .with_behaviour(|key, relay| {
            let id = key.public().to_peer_id();
            App {
                relay,
                dcutr: Toggle::from(None),
                identify: identify::Behaviour::new(
                    identify::Config::new_with_signed_peer_record(
                        "/ghal-bol/1.0.0".to_string(),
                        key,
                    )
                    .with_agent_version(format!("ghal_bol/0.0.0;pk={pk_hex}"))
                    .with_push_listen_addr_updates(false),
                ),
                autonat: autonat::Behaviour::new(id, autonat::Config::default()),
                upnp: Toggle::from(Some(upnp::tokio::Behaviour::default())),
                mdns: Toggle::from(
                    mdns::tokio::Behaviour::new(mdns::Config::default(), id).ok(),
                ),
                ping: ping::Behaviour::new(
                    ping::Config::new()
                        .with_interval(Duration::from_secs(8))
                        .with_timeout(Duration::from_secs(15)),
                ),
                stream: stream::Behaviour::new(),
            }
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(300)))
        .build();

    // App also listens on ephemeral TCP/QUIC.
    let _ = swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?);
    let _ = swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?);

    let mut reserve_addr = relay_base.clone();
    if !reserve_addr.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        reserve_addr.push(Protocol::P2p(relay_peer));
    }
    reserve_addr.push(Protocol::P2pCircuit);
    swarm.listen_on(reserve_addr.clone())?;
    println!("APP RESPONDER peer_id={local}");
    println!("reserving on {reserve_addr}");

    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. }
                if address.iter().any(|p| matches!(p, Protocol::P2pCircuit)) =>
            {
                println!("APP RESERVED at {address}");
            }
            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                println!("APP <= conn from {peer_id} via {}", endpoint.get_remote_address());
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                println!("APP XX conn closed {peer_id} cause={cause:?}");
            }
            SwarmEvent::Behaviour(AppEvent::Relay(e)) => {
                println!("APP relay-client: {e:?}");
            }
            _ => {}
        }
    }
}
