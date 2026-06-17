//! Two local libp2p nodes deliver one signed DM text frame end-to-end.
//!
//! Privacy model: **host** (scanned QR) knows nobody upfront; **guest** (scanner) knows only
//! the host's `public_key_hex` from the invite. Guest dials host via explicit bootstrap multiaddr
//! from host `Listening` (LAN test harness — production uses coord/relay + mDNS).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ghal_bol::create_keystore_v1;
use ghal_bol::p2p::{
    DmPeer, GossipChatConfig, GossipChatEvent, OutboundCmd, run_gossip_chat_node_with_std_io,
};
use libp2p::Multiaddr;
use libp2p::identity::PeerId;
use libp2p::multiaddr::Protocol;

fn public_key_hex(id: &ghal_bol::DecryptedIdentity) -> String {
    id.public_key_hex()
}

fn with_host_peer_id(ma: Multiaddr, peer: PeerId) -> Multiaddr {
    if ma.to_string().contains("/p2p/") {
        return ma;
    }
    ma.with(Protocol::P2p(peer))
}

fn pick_loopback_or_lan(addrs: &[Multiaddr]) -> Multiaddr {
    if let Some(a) = addrs.iter().find(|a| a.to_string().contains("/ip4/127.")) {
        return a.clone();
    }
    if let Some(a) = addrs
        .iter()
        .find(|a| a.to_string().contains("/ip4/192.168."))
    {
        return a.clone();
    }
    addrs.first().cloned().expect("no listen addr from host")
}

fn wait_listening(ev_rx: &mpsc::Receiver<GossipChatEvent>) -> Multiaddr {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut addrs = Vec::new();
    while Instant::now() < deadline {
        match ev_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(GossipChatEvent::Listening(a)) => {
                addrs.push(a);
                if addrs.iter().any(|x| {
                    let s = x.to_string();
                    s.contains("/ip4/127.") || s.contains("/ip4/192.168.")
                }) {
                    return pick_loopback_or_lan(&addrs);
                }
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if !addrs.is_empty() {
        return pick_loopback_or_lan(&addrs);
    }
    panic!("node did not emit Listening");
}

struct AsymmetricPair {
    stop_host: Arc<AtomicBool>,
    stop_guest: Arc<AtomicBool>,
    out_host_tx: mpsc::Sender<OutboundCmd>,
    out_guest_tx: mpsc::Sender<OutboundCmd>,
    ev_host_rx: mpsc::Receiver<GossipChatEvent>,
    ev_guest_rx: mpsc::Receiver<GossipChatEvent>,
    host_server: std::thread::JoinHandle<()>,
    guest_server: std::thread::JoinHandle<()>,
    host_pk: String,
}

impl AsymmetricPair {
    fn start() -> Self {
        common::init_integration_env();
        let (_ks_a, id_a) = create_keystore_v1("peer-a", None).expect("identity a");
        let (_ks_b, id_b) = create_keystore_v1("peer-b", None).expect("identity b");
        let host_pk = public_key_hex(&id_a);

        let cfg_a = GossipChatConfig::from_unlocked_identity("ghal-bol-test", &id_a).unwrap();

        let stop_host = Arc::new(AtomicBool::new(false));
        let (out_host_tx, out_host_rx) = mpsc::channel::<OutboundCmd>();
        let (ev_host_tx, ev_host_raw) = mpsc::channel::<GossipChatEvent>();
        let (fwd_tx, ev_host_rx) = mpsc::channel::<GossipChatEvent>();
        std::thread::spawn(move || {
            while let Ok(ev) = ev_host_raw.recv() {
                let _ = fwd_tx.send(ev);
            }
        });

        let host_peer = id_a
            .to_libp2p_keypair()
            .expect("host keypair")
            .public()
            .to_peer_id();
        let stop_host_t = Arc::clone(&stop_host);
        let id_a_t = id_a;
        let host_server = common::spawn_p2p_thread("two-peer-host", move || {
            common::block_on_local(run_gossip_chat_node_with_std_io(
                cfg_a,
                id_a_t,
                out_host_rx,
                ev_host_tx,
                stop_host_t,
            ))
            .expect("host gossip node");
        });

        let host_dial = with_host_peer_id(wait_listening(&ev_host_rx), host_peer);

        let mut cfg_b = GossipChatConfig::from_unlocked_identity("ghal-bol-test", &id_b).unwrap();
        cfg_b
            .dm_peers
            .push(DmPeer::from_public_key_hex(host_pk.clone()).expect("host dm peer"));

        let stop_guest = Arc::new(AtomicBool::new(false));
        let (out_guest_tx, out_guest_rx) = mpsc::channel::<OutboundCmd>();
        let (ev_guest_tx, ev_guest_rx) = mpsc::channel::<GossipChatEvent>();

        let stop_guest_t = Arc::clone(&stop_guest);
        let guest_server = common::spawn_p2p_thread("two-peer-guest", move || {
            common::block_on_local(run_gossip_chat_node_with_std_io(
                cfg_b,
                id_b,
                out_guest_rx,
                ev_guest_tx,
                stop_guest_t,
            ))
            .expect("guest gossip node");
        });

        out_guest_tx
            .send(OutboundCmd::DialBootstrapPeers {
                addrs: vec![host_dial],
            })
            .expect("dial bootstrap");

        Self {
            stop_host,
            stop_guest,
            out_host_tx,
            out_guest_tx,
            ev_host_rx,
            ev_guest_rx,
            host_server,
            guest_server,
            host_pk,
        }
    }

    fn wait_host_connected(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.ev_host_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(GossipChatEvent::PeerConnected(_)) => return,
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        panic!("host did not see guest connect");
    }

    fn wait_chat_ready_on_guest(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.ev_guest_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(GossipChatEvent::ChatReady { .. }) => return,
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        panic!("chat stream not ready on guest");
    }

    fn shutdown(self) {
        self.stop_guest.store(true, Ordering::SeqCst);
        self.stop_host.store(true, Ordering::SeqCst);
        drop(self.out_guest_tx);
        drop(self.out_host_tx);
        let _ = self.guest_server.join();
        let _ = self.host_server.join();
    }
}

#[test]
fn two_peers_deliver_text_over_stream() {
    let pair = AsymmetricPair::start();
    pair.wait_host_connected(Duration::from_secs(45));
    pair.wait_chat_ready_on_guest(Duration::from_secs(20));

    let (done_tx, done_rx) = mpsc::channel();
    pair.out_guest_tx
        .send(OutboundCmd::SendText {
            recipient_public_key_hex: pair.host_pk.clone(),
            text: "integration-hello".to_string(),
            message_id: ghal_bol::p2p::chat_server::new_msg_id_for_ffi(),
            done: Some(done_tx),
        })
        .unwrap();
    done_rx
        .recv_timeout(Duration::from_secs(15))
        .expect("send done")
        .expect("send ok");

    let mut got_text = false;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match pair.ev_host_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(GossipChatEvent::DmMessage { msg_kind, text, .. }) if msg_kind == "text" => {
                assert_eq!(text.as_deref(), Some("integration-hello"));
                got_text = true;
                break;
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let mut got_ack = false;
    let ack_deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < ack_deadline {
        match pair.ev_guest_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(GossipChatEvent::DmMessage { msg_kind, .. }) if msg_kind == "ack_received" => {
                got_ack = true;
                break;
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    pair.shutdown();
    assert!(got_text, "host did not receive dm text");
    assert!(
        got_ack,
        "guest (sender) did not receive ack_received from host"
    );
}
