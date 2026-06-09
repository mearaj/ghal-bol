//! Guest reaches host via **coord lookup only** (no bootstrap dial addrs).
//!
//! Mirrors production: host registers TCP endpoint on `ghal_bol_server`, guest has
//! `public_key_hex` from invite and dials after coord lookup / upkeep.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use ghal_bol::coord::{CoordEndpoint, CoordHttpClient};
use ghal_bol::coord_runtime;
use ghal_bol::create_keystore_v1;
use ghal_bol::p2p::{
    run_gossip_chat_node_with_std_io, DmPeer, GossipChatEvent, GossipChatConfig, OutboundCmd,
};
use ghal_bol::DmDialAddr;
use ghal_bol_server::{router, AppState, ServerConfig};
use tokio::net::TcpListener;

fn coord_base_url() -> String {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        common::spawn_p2p_thread("coord-test-server", move || {
            let rt = common::p2p_tokio_runtime();
            rt.block_on(async {
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("coord bind");
                let url = format!("http://{}", listener.local_addr().expect("addr"));
                let state = Arc::new(
                    AppState::open_in_memory(ServerConfig::default()).expect("coord mem db"),
                );
                let app: Router = router(state);
                ready_tx.send(url.clone()).expect("ready");
                axum::serve(listener, app).await.expect("coord serve");
            });
        });
        ready_rx.recv().expect("coord url")
    })
    .clone()
}

fn register_on_coord(
    client: &CoordHttpClient,
    secret: &secp256k1::SecretKey,
    public_key_hex: &str,
    host: &str,
    port: u16,
) {
    let endpoints = vec![CoordEndpoint {
        scheme: "tcp".into(),
        host: host.to_string(),
        port,
    }];
    client
        .register(secret, public_key_hex, &endpoints, Some(host), None)
        .expect("coord register");
}

fn wait_tcp_listen(ev_rx: &mpsc::Receiver<GossipChatEvent>) -> DmDialAddr {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut addrs = Vec::new();
    while Instant::now() < deadline {
        match ev_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(GossipChatEvent::Listening(a)) => {
                if let Some(dm) = DmDialAddr::parse(&a.to_string()) {
                    addrs.push(dm);
                    if addrs.iter().any(|x| {
                        x.host.starts_with("127.") || x.host.starts_with("192.168.")
                    }) {
                        return pick_loopback_or_lan_dm(&addrs);
                    }
                }
            }
            Ok(GossipChatEvent::NodeReady) if !addrs.is_empty() => {
                return pick_loopback_or_lan_dm(&addrs);
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if !addrs.is_empty() {
        return pick_loopback_or_lan_dm(&addrs);
    }
    panic!("host did not emit TCP Listening");
}

fn pick_loopback_or_lan_dm(addrs: &[DmDialAddr]) -> DmDialAddr {
    if let Some(a) = addrs.iter().find(|a| a.host.starts_with("127.")) {
        return a.clone();
    }
    if let Some(a) = addrs.iter().find(|a| a.host.starts_with("192.168.")) {
        return a.clone();
    }
    addrs.first().cloned().expect("no tcp listen addr")
}

#[test]
fn guest_dials_host_via_coord_lookup() {
    common::init_integration_env();
    let url = coord_base_url();
    coord_runtime::set_coord_base_url(&url, false);

    let (_ks_host, id_host) = create_keystore_v1("host-coord", None).expect("host id");
    let (_ks_guest, id_guest) = create_keystore_v1("guest-coord", None).expect("guest id");
    let host_pk = id_host.public_key_hex();
    let host_sk = id_host.secp256k1_secret().clone();

    let cfg_host = GossipChatConfig::from_unlocked_identity("coord-test", &id_host).unwrap();
    let stop_host = Arc::new(AtomicBool::new(false));
    let (out_host_tx, out_host_rx) = mpsc::channel();
    let (ev_host_tx, ev_host_rx) = mpsc::channel();

    let stop_host_t = Arc::clone(&stop_host);
    let host_thread = common::spawn_p2p_thread("coord-test-host", move || {
        common::block_on_local(run_gossip_chat_node_with_std_io(
            cfg_host, id_host, out_host_rx, ev_host_tx, stop_host_t,
        ))
        .expect("host gossip node");
    });

    let dm_listen = wait_tcp_listen(&ev_host_rx);
    let client = CoordHttpClient::new(&url, false).expect("coord client");
    let reg_host = if dm_listen.host == "0.0.0.0" {
        "127.0.0.1"
    } else {
        dm_listen.host.trim()
    };
    register_on_coord(&client, &host_sk, &host_pk, reg_host, dm_listen.port);

    let mut cfg_guest =
        GossipChatConfig::from_unlocked_identity("coord-test", &id_guest).unwrap();
    cfg_guest
        .dm_peers
        .push(DmPeer::from_public_key_hex(host_pk.clone()).expect("dm peer"));

    let stop_guest = Arc::new(AtomicBool::new(false));
    let (out_guest_tx, out_guest_rx) = mpsc::channel();
    let (ev_guest_tx, ev_guest_rx) = mpsc::channel();

    let stop_guest_t = Arc::clone(&stop_guest);
    let guest_thread = common::spawn_p2p_thread("coord-test-guest", move || {
        common::block_on_local(run_gossip_chat_node_with_std_io(
            cfg_guest, id_guest, out_guest_rx, ev_guest_tx, stop_guest_t,
        ))
        .expect("guest gossip node");
    });

    // No DialBootstrapPeers — guest must reach host via coord lookup / upkeep.
    let chat_deadline = Instant::now() + Duration::from_secs(60);
    let mut chat_ready = false;
    while Instant::now() < chat_deadline {
        match ev_guest_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(GossipChatEvent::ChatReady { .. }) => {
                chat_ready = true;
                break;
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    assert!(chat_ready, "guest did not reach ChatReady via coord lookup");

    let (done_tx, done_rx) = mpsc::channel();
    out_guest_tx
        .send(OutboundCmd::SendText {
            recipient_public_key_hex: host_pk.clone(),
            text: "coord-path-hello".to_string(),
            message_id: ghal_bol::p2p::chat_server::new_msg_id_for_ffi(),
            done: Some(done_tx),
        })
        .unwrap();
    done_rx.recv_timeout(Duration::from_secs(15)).unwrap().unwrap();

    let mut got = false;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match ev_host_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(GossipChatEvent::DmMessage {
                msg_kind,
                text,
                ..
            }) if msg_kind == "text" => {
                assert_eq!(text.as_deref(), Some("coord-path-hello"));
                got = true;
                break;
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    assert!(got, "host did not receive text over coord-discovered TCP");

    stop_guest.store(true, Ordering::SeqCst);
    stop_host.store(true, Ordering::SeqCst);
    drop(out_guest_tx);
    drop(out_host_tx);
    let _ = guest_thread.join();
    let _ = host_thread.join();
}
