//! Standalone relay-reservation probe.
//!
//! Dials the advertised Ghal Bol relay and attempts a circuit reservation, printing exactly
//! what happens (accepted / failed / timeout). This isolates the relay-server path from the
//! full client app, so we can tell whether the coord/relay server actually grants reservations
//! through whatever tunnel (bore) is in front of it.
//!
//! Usage:
//!   cargo run -p ghal_bol_server --example relay_probe -- \
//!     /ip4/159.223.110.159/tcp/1245/p2p/12D3KooW.../p2p-circuit
//!
//! Or pass a base addr + peer id:
//!   cargo run -p ghal_bol_server --example relay_probe -- \
//!     /ip4/159.223.110.159/tcp/1245 12D3KooW...

use std::time::Duration;

use futures::StreamExt;
use libp2p::core::multiaddr::Protocol;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, identify, identity, ping, relay};

#[derive(libp2p::swarm::NetworkBehaviour)]
struct Behaviour {
    relay_client: relay::client::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: relay_probe <relay-multiaddr-with-/p2p-circuit> | <base-addr> <peer-id>");
        std::process::exit(2);
    }

    let circuit_addr: Multiaddr = if args.len() == 1 {
        let ma: Multiaddr = args[0].parse()?;
        if !ma.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
            return Err("multiaddr must include /p2p-circuit".into());
        }
        ma
    } else {
        let base: Multiaddr = args[0].parse()?;
        let peer: PeerId = args[1].parse()?;
        let mut ma = base;
        if !ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
            ma.push(Protocol::P2p(peer));
        }
        ma.push(Protocol::P2pCircuit);
        ma
    };

    let id_keys = if std::env::var("PROBE_SECP256K1").is_ok() {
        println!("probe key type: secp256k1");
        identity::Keypair::generate_secp256k1()
    } else {
        println!("probe key type: ed25519");
        identity::Keypair::generate_ed25519()
    };
    println!("probe peer id : {}", id_keys.public().to_peer_id());
    println!("relay target  : {circuit_addr}");

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default().nodelay(true),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_dns()?
        .with_relay_client(libp2p::noise::Config::new, libp2p::yamux::Config::default)?
        .with_behaviour(|key, relay_client| Behaviour {
            relay_client,
            identify: identify::Behaviour::new(identify::Config::new(
                "/ghal-bol/1.0.0".to_string(),
                key.public(),
            )),
            ping: ping::Behaviour::new(ping::Config::new()),
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    swarm.listen_on(circuit_addr.clone())?;
    println!("listen_on issued — waiting up to 30s for ReservationReqAccepted ...\n");

    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => {
                println!("\n==> RESULT: TIMEOUT — no reservation accepted in 30s");
                std::process::exit(1);
            }
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        if address.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
                            println!("==> RESULT: RESERVATION ACCEPTED — listening at {address}");
                            std::process::exit(0);
                        }
                        println!("[listen] {address}");
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        println!("[conn] established to {peer_id} via {}", endpoint.get_remote_address());
                    }
                    SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                        println!("[conn] OUTGOING ERROR to {peer_id:?}: {error}");
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::RelayClient(ev)) => {
                        println!("[relay-client] {ev:?}");
                        if matches!(ev, relay::client::Event::ReservationReqAccepted { .. }) {
                            println!("\n==> RESULT: RESERVATION ACCEPTED (event)");
                            std::process::exit(0);
                        }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Identify(ev)) => {
                        if let identify::Event::Received { peer_id, info, .. } = ev {
                            println!("[identify] {peer_id} proto={} agent={}", info.protocol_version, info.agent_version);
                        }
                    }
                    other => {
                        println!("[event] {other:?}");
                    }
                }
            }
        }
    }
}
