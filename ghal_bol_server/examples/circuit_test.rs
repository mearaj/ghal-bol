//! End-to-end circuit forwarding test against a live Ghal Bol relay.
//!
//! Spawns TWO secp256k1 libp2p nodes (same stack as the ghal_bol client):
//!   * responder — reserves a circuit on the relay and stays listening.
//!   * dialer    — dials the responder *through* the relay circuit.
//!
//! Prints whether the relayed connection is actually established (HOP/STOP forwarded),
//! isolating relay-server circuit forwarding from the full app.
//!
//! Usage:
//!   cargo run -p ghal_bol_server --example circuit_test -- <relay-base-addr> <relay-peer-id>
//!   e.g. cargo run -p ghal_bol_server --example circuit_test -- \
//!        /dns4/coord.ghalbol.com/tcp/4002 12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF

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

fn build_swarm() -> Result<libp2p::Swarm<Behaviour>, Box<dyn std::error::Error + Send + Sync>> {
    let id_keys = identity::Keypair::generate_secp256k1();
    let swarm = libp2p::SwarmBuilder::with_existing_identity(id_keys)
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
            identify: identify::Behaviour::new(
                identify::Config::new("/ghal-bol/1.0.0".to_string(), key.public())
                    .with_agent_version("ghal_bol_circuit_test/secp256k1".to_string()),
            ),
            ping: ping::Behaviour::new(ping::Config::new()),
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(120)))
        .build();
    Ok(swarm)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: circuit_test <relay-base-addr> <relay-peer-id>");
        std::process::exit(2);
    }
    let relay_base: Multiaddr = args[0].parse()?;
    let relay_peer: PeerId = args[1].parse()?;

    // Dial-only mode: 3rd arg is an external responder peer id (e.g. a real app node).
    if let Some(target) = args.get(2) {
        let responder_peer: PeerId = target.parse()?;
        let mut dialer = build_swarm()?;
        let dialer_peer = *dialer.local_peer_id();
        let mut circuit_addr = relay_base.clone();
        if !circuit_addr.iter().any(|p| matches!(p, Protocol::P2p(_))) {
            circuit_addr.push(Protocol::P2p(relay_peer));
        }
        circuit_addr.push(Protocol::P2pCircuit);
        circuit_addr.push(Protocol::P2p(responder_peer));
        println!("dialer {dialer_peer} dialing {circuit_addr}");
        dialer.dial(circuit_addr)?;
        let deadline = tokio::time::sleep(Duration::from_secs(30));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => {
                    println!("\n==> RESULT: FAIL — relayed connection not established in 30s");
                    std::process::exit(1);
                }
                ev = dialer.select_next_some() => match ev {
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } if peer_id == responder_peer => {
                        println!("dialer => established to responder via {}", endpoint.get_remote_address());
                        println!("\n==> RESULT: OK — circuit forwarding works (relayed connection up)");
                        std::process::exit(0);
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        println!("dialer => established to {peer_id} via {}", endpoint.get_remote_address());
                    }
                    SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                        println!("dialer OUTGOING ERROR to {peer_id:?}: {error}");
                        println!("\n==> RESULT: FAIL — relayed dial errored");
                        std::process::exit(1);
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::RelayClient(e)) => {
                        println!("dialer relay-client: {e:?}");
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- responder: reserve + stay ----
    let mut responder = build_swarm()?;
    let responder_peer = *responder.local_peer_id();
    let mut reserve_addr = relay_base.clone();
    if !reserve_addr.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        reserve_addr.push(Protocol::P2p(relay_peer));
    }
    reserve_addr.push(Protocol::P2pCircuit);
    responder.listen_on(reserve_addr.clone())?;
    println!("responder {responder_peer} reserving on {reserve_addr}");

    // Wait until reservation accepted (NewListenAddr with /p2p-circuit).
    let reserved = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match responder.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. }
                    if address.iter().any(|p| matches!(p, Protocol::P2pCircuit)) =>
                {
                    println!("responder RESERVED at {address}");
                    return true;
                }
                SwarmEvent::OutgoingConnectionError { error, .. } => {
                    println!("responder conn error: {error}");
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    if !reserved {
        println!("==> RESULT: FAIL — responder could not reserve");
        std::process::exit(1);
    }

    // Optional: re-issue listen_on(circuit) to mimic the app's periodic re-reservation, and/or
    // wait before the dialer dials, to test whether inbound STOP survives time/re-reserve.
    let re_listen = std::env::var("RELISTEN").is_ok();
    let wait_secs: u64 = std::env::var("WAIT_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(0);

    // Drive the responder swarm in the background so it can answer STOP/circuit.
    tokio::spawn(async move {
        if re_listen {
            tokio::time::sleep(Duration::from_secs(2)).await;
            println!("responder RE-listen_on(circuit) (simulating app re-reserve)");
            let _ = responder.listen_on(reserve_addr.clone());
        }
        loop {
            let ev = responder.select_next_some().await;
            match ev {
                SwarmEvent::NewListenAddr { address, .. }
                    if address.iter().any(|p| matches!(p, Protocol::P2pCircuit)) =>
                {
                    println!("responder re-RESERVED at {address}");
                }
                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                    println!("responder <= connection from {peer_id} via {}", endpoint.get_remote_address());
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    println!("responder XX connection closed {peer_id} cause={cause:?}");
                }
                SwarmEvent::Behaviour(BehaviourEvent::RelayClient(e)) => {
                    println!("responder relay-client: {e:?}");
                }
                _ => {}
            }
        }
    });

    if wait_secs > 0 {
        println!("waiting {wait_secs}s before dialer dials ...");
        tokio::time::sleep(Duration::from_secs(wait_secs)).await;
    }

    // ---- dialer: dial responder through the relay ----
    let mut dialer = build_swarm()?;
    let dialer_peer = *dialer.local_peer_id();
    let mut circuit_addr = relay_base.clone();
    if !circuit_addr.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        circuit_addr.push(Protocol::P2p(relay_peer));
    }
    circuit_addr.push(Protocol::P2pCircuit);
    circuit_addr.push(Protocol::P2p(responder_peer));
    println!("dialer {dialer_peer} dialing {circuit_addr}");
    dialer.dial(circuit_addr)?;

    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => {
                println!("\n==> RESULT: FAIL — relayed connection not established in 30s (circuit forwarding broken)");
                std::process::exit(1);
            }
            ev = dialer.select_next_some() => match ev {
                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } if peer_id == responder_peer => {
                    println!("dialer => established to responder via {}", endpoint.get_remote_address());
                    println!("\n==> RESULT: OK — circuit forwarding works (relayed connection up)");
                    std::process::exit(0);
                }
                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                    println!("dialer => established to {peer_id} via {}", endpoint.get_remote_address());
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    println!("dialer OUTGOING ERROR to {peer_id:?}: {error}");
                    println!("\n==> RESULT: FAIL — relayed dial errored (circuit forwarding broken)");
                    std::process::exit(1);
                }
                SwarmEvent::Behaviour(BehaviourEvent::RelayClient(e)) => {
                    println!("dialer relay-client: {e:?}");
                }
                _ => {}
            }
        }
    }
}
