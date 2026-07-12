pub(crate) struct SessionState {
    identity: crate::DecryptedIdentity,
    peers: RwLock<PeerTables>,
    connected: RwLock<HashSet<PeerId>>,
    /// Messages we sent that are not yet `ack_received` by the peer.
    outbox: RwLock<HashMap<String, PendingOutbound>>,
    /// Wire ack cleared outbox but poll has not patched transcript yet — block re-merge from pending rows.
    outbound_ack_pending_poll: RwLock<HashSet<String>>,
    /// Dedupe inbound `text` emits (retries / duplicate frames).
    seen_inbound_ids: RwLock<HashMap<String, i64>>,
    /// FFI/UI: `peer_identified` at most once per remote libp2p peer.
    identified_emitted: RwLock<HashSet<PeerId>>,
    /// FFI/UI: `chat_ready` at most once per remote libp2p peer.
    chat_ready_emitted: RwLock<HashSet<PeerId>>,
    /// Dialable listen addresses accumulated across listeners (coord/mDNS).
    published_listen: RwLock<Vec<Multiaddr>>,
    /// Throttle routed dials / stream-open attempts (ms since epoch).
    routed_dial_attempt_ms: RwLock<HashMap<PeerId, i64>>,
    /// Relay-circuit coord dials — separate from peer-id routed dials so LAN upkeep NoAddresses
    /// spam does not block WAN `/p2p-circuit` handshakes (different throttle semantics).
    circuit_coord_dial_last_ms: RwLock<HashMap<PeerId, i64>>,
    stream_open_log_emitted: RwLock<HashSet<PeerId>>,
    /// Prevent concurrent open_stream storms per peer (causes "receiver is gone"/oneshot canceled).
    stream_open_inflight: RwLock<HashSet<PeerId>>,
    /// Back off stream open retries after transient failures (e.g. zombie link "receiver is gone").
    stream_open_backoff_until_ms: RwLock<HashMap<PeerId, i64>>,
    /// Throttle `DialFailed` UI events for the same peer (upkeep retries every ~1s).
    stream_open_fail_log_ms: RwLock<HashMap<PeerId, i64>>,
    /// Coord relay peer ids (for logging + relay reservation).
    bootstrap_peer_ids: RwLock<HashSet<PeerId>>,
    relay_reserve_requested: RwLock<HashSet<PeerId>>,
    /// Throttle `listen_on(/p2p-circuit)` attempts per relay peer.
    /// Repeated listen attempts create large listen/behaviour churn and can delay WAN readiness.
    relay_reserve_last_attempt_ms: RwLock<HashMap<PeerId, i64>>,
    /// One in-flight `listen_on(/p2p-circuit)` per relay until accepted or timeout.
    relay_reserve_in_flight_ms: RwLock<HashMap<PeerId, i64>>,
    /// Last `ReservationReqAccepted` per relay — libp2p closes the old circuit listener during
    /// renewal; must not treat that gap as "circuit gone" and re-issue `listen_on`.
    relay_reservation_accepted_ms: RwLock<HashMap<PeerId, i64>>,
    /// Live bootstrap TCP links per coord relay (for HOP anchor + prune).
    bootstrap_tcp_conns: RwLock<HashMap<PeerId, HashMap<ConnectionId, Multiaddr>>>,
    /// Identify completed on bootstrap HOP (libp2p relay client expects this before reserve).
    bootstrap_identified: RwLock<HashSet<PeerId>>,
    /// First bootstrap TCP connect time per relay (fallback when Identify was drained at startup).
    bootstrap_tcp_since_ms: RwLock<HashMap<PeerId, i64>>,
    /// Defer circuit reservation until happy-eyeballs links settle (`RELAY_BOOTSTRAP_SETTLE_MS`).
    bootstrap_reserve_after_ms: RwLock<HashMap<PeerId, i64>>,
    /// Remote multiaddr per connected coord relay (relay reservation retries).
    bootstrap_relay_addr: RwLock<HashMap<PeerId, Multiaddr>>,
    /// At least one coord relay peer has a live libp2p connection.
    any_bootstrap_connected: AtomicBool,
    /// Throttle repeated coord lookups per contact public key (UI can spam register/send bursts).
    last_coord_lookup_ms: RwLock<HashMap<String, i64>>,
    /// Backoff coord lookups when peer isn't registered yet (HTTP 404 peer_not_on_server).
    /// Key: recipient public_key_hex.
    coord_lookup_backoff: RwLock<HashMap<String, CoordLookupBackoff>>,
    bootstrap_dial_err_log_ms: RwLock<HashMap<PeerId, i64>>,
    /// Throttle redundant `swarm.dial` to the same coord relay (refetch/redial storms).
    bootstrap_dial_last_ms: RwLock<HashMap<PeerId, i64>>,
    /// IPv6 bootstrap dial failed (unreachable) — prefer IPv4 for HOP/prune until expiry.
    bootstrap_ipv6_unreachable_ms: RwLock<HashMap<PeerId, i64>>,
    /// Peers we rejected on connect (relay/bootstrap noise); suppress disconnect logs.
    incidental_rejects: RwLock<HashSet<PeerId>>,
    /// Inbound texts needing `ack_read` while foreground chat is open (retried until confirmed).
    pending_read_acks: RwLock<VecDeque<PendingReadAck>>,
    /// Inbound texts whose `ack_received` failed to send (retried until stream is ready).
    pending_delivery_acks: RwLock<VecDeque<PendingDeliveryAck>>,
    /// Inbound message ids for which the peer sent `ack_received` after our `ack_read`.
    read_ack_confirmed: RwLock<HashSet<String>>,
    /// Inbound message ids for which we already sent `ack_received` (wire retries must not re-send).
    delivery_ack_sent: RwLock<HashSet<String>>,
    /// Call signaling frames waiting for DM stream (same transient errors as text send).
    pending_call_signals: RwLock<VecDeque<PendingCallSignal>>,
    foreground_peer: RwLock<Option<PeerId>>,
    transcript_path: Option<String>,
    app_namespace: Option<String>,
    /// History replay once per remote peer per session (avoids reordering the open chat).
    history_replay_done: RwLock<HashSet<PeerId>>,
    network_profile: RwLock<crate::p2p::network_transport::LocalNetworkProfile>,
    /// Fast relay/coord/bootstrap loop after Wi‑Fi ↔ mobile (or OS connectivity callback).
    wan_recovery_active: AtomicBool,
    /// Co-located Ghal Bol relay `(peer_id, base_addrs from GET /v1/relay)` for refresh.
    ghal_bol_relay_state: RwLock<Option<(PeerId, Vec<String>)>>,
    ghal_bol_relay_last_fetch_ms: RwLock<i64>,
    /// Rate-limit diagnostic logs for dial skips (avoid log storms).
    dial_skip_log_ms: RwLock<HashMap<PeerId, i64>>,
    /// mDNS discovered this DM peer on the local LAN (WAN-first dial otherwise).
    peers_on_local_lan: RwLock<HashMap<PeerId, i64>>,
    /// Count of currently-open **direct** (non-relay-circuit) connections per peer.
    /// Lets a peer freshly seen on the LAN decide whether it still needs a direct
    /// LAN link (it is connected only over a relay circuit) — see `handle_mdns_discovered_list`.
    peers_direct_conns: RwLock<HashMap<PeerId, u32>>,
    /// Live relay-circuit `ConnectionId`s per DM peer — tracked while relay link is up (parallel with direct LAN).
    dm_relay_conn_ids: RwLock<HashMap<PeerId, HashSet<ConnectionId>>>,
    /// Live direct (non-relay) `ConnectionId`s per DM peer — close selectively on LAN↔WAN handover.
    dm_direct_conn_ids: RwLock<HashMap<PeerId, HashSet<ConnectionId>>>,
    /// Peers with a fresh relay `InboundCircuitEstablished` — next `ConnectionEstablished` is relay.
    dm_relay_circuit_pending: RwLock<HashSet<PeerId>>,
    /// Ephemeral `/ip4/0.0.0.0/tcp/0` listener ids — removed on LAN handover so mDNS advertises one port.
    lan_ephemeral_tcp_listener_ids: RwLock<Vec<ListenerId>>,
    /// Live mDNS LAN TCP candidates per peer (libp2p emits addrs one-by-one — not a dial cache).
    peer_mdns_lan_candidate_addrs: RwLock<HashMap<PeerId, Vec<Multiaddr>>>,
    /// All ranked LAN TCP candidates failed — WAN/coord only until fresh mDNS merge.
    lan_candidates_exhausted: RwLock<HashSet<PeerId>>,
    /// DM contacts whose connection just dropped — reconnect is urgent until this deadline (ms).
    /// While urgent, coord lookup bypasses the `peer_not_on_server` backoff and we retry every
    /// upkeep tick so a transient drop does not turn into a multi-second message delay.
    dm_reconnect_urgent: RwLock<HashMap<String, i64>>,
    /// Last encrypted DM frame in either direction (detect zombie libp2p links).
    dm_wire_activity_ms: RwLock<HashMap<PeerId, i64>>,
    /// When a DM peer is connected but the chat stream writer is missing.
    dm_no_writer_since_ms: RwLock<HashMap<PeerId, i64>>,
    /// Peers with an open `/ghal-bol/msg/1.0.0` writer (protonet `chatStreams` — upkeep noop while set).
    dm_stream_has_writer: RwLock<HashSet<PeerId>>,
    /// Monotonic per-peer writer generation. Multiple inbound stream handlers can exist for one
    /// contact (symmetric connect race, parallel LAN+WAN). When a newer mux installs the writer
    /// (adopt / reopen), the **older** handler's teardown must not clear the live writer — it
    /// compares its generation and skips cleanup if a newer one took over. Prevents the
    /// adopt-then-stale-LAN-close race that silently killed the relay writer (one-way acks).
    dm_writer_generation: RwLock<HashMap<PeerId, u64>>,
    /// Source of monotonic writer generations (`claim_dm_writer_generation`).
    dm_writer_gen_counter: std::sync::atomic::AtomicU64,
    /// Periodic relay reservation refresh even when a circuit is already listening.
    relay_keepalive_last_ms: RwLock<i64>,
    /// Back off relay-circuit dials after `ResourceLimitExceeded` from the coord relay.
    relay_circuit_dial_backoff_until: RwLock<HashMap<PeerId, i64>>,
    /// Outbound relay-circuit dial start time — blocks replacement dials for CIRCUIT_DIAL_IN_FLIGHT_MS.
    dm_circuit_dial_in_flight_ms: RwLock<HashMap<PeerId, i64>>,
    /// Direct LAN TCP dial start time — blocks coord relay for LAN_DIAL_IN_FLIGHT_MS.
    dm_lan_dial_in_flight_ms: RwLock<HashMap<PeerId, i64>>,
    /// When a DM peer was hot-registered — wait for mDNS on Wi‑Fi before coord relay dials.
    dm_peer_registered_ms: RwLock<HashMap<PeerId, i64>>,
    /// Last coord lookup outcome per DM public key (for actionable dial-skip logs).
    coord_lookup_last_category:
        RwLock<HashMap<String, crate::p2p::connectivity_diag::CoordLookupCategory>>,
    /// Last WAN coord listen fingerprint — detects relay/public path churn without iface change.
    last_wan_listen_fp: RwLock<Vec<String>>,
    /// Debounce full peer rediscovery after burst presence-wake notifies.
    last_presence_wake_ms: RwLock<i64>,
    /// Throttle mDNS behaviour recreation after LAN handover (multicast iface rebind).
    last_mdns_restart_ms: RwLock<i64>,
    /// Throttle Wi‑Fi LAN reopen attempts (connectivity callback only).
    last_lan_recovery_ms: RwLock<i64>,
    /// Full `kick_lan_dm_rediscovery_after_handover` queued while a relay circuit dial is in flight.
    pending_full_lan_kick_reason: RwLock<Option<String>>,
    /// DM peers waiting for fresh mDNS + ephemeral listen after a LAN event (Wi‑Fi flap, mDNS expire).
    lan_listen_rediscovery_peers: RwLock<HashSet<PeerId>>,
    /// Drop asymmetric/zombie libp2p links after `open_stream` timeout (swarm loop disconnect).
    pending_dm_link_reset: RwLock<HashSet<PeerId>>,
    /// One-shot bypass of `WAN_MUX_RECONCILE_THROTTLE_MS` after relay `InboundCircuitEstablished`
    /// (TRANSPORT.md § Asymmetric mux — Wi‑Fi side must not wait 5s while remote re-dials on relay).
    asymmetric_relay_recover_urgent: RwLock<HashSet<PeerId>>,
    /// Fresh relay `InboundCircuitEstablished` during `lan_listen_rediscovery` — mobile peer re-dialed
    /// on WAN while we still hold lingering direct (flutter_linux.log 2026-07-02 05:41:22).
    relay_inbound_handover_peers: RwLock<HashSet<PeerId>>,
    /// Throttle repetitive coord lookup INFO logs (especially peer_not_on_coord).
    coord_lookup_info_log_ms: RwLock<HashMap<String, i64>>,
    /// Active native voice-call media sessions, keyed by `call_id`. Each entry holds the
    /// per-call controls (mute/stop + stats) and the channel into the engine for inbound
    /// (peer → engine) sealed packets. See `docs/GHAL_BOL_CALL_NATIVE_V2.md`.
    call_media: Mutex<HashMap<String, CallMediaEntry>>,
    /// Active native **video**-call sessions, keyed by `call_id`. Parallel to
    /// `call_media` (voice); a call may have both. See `docs/GHAL_BOL_VIDEO_NATIVE_V1.md`.
    call_video: Mutex<HashMap<String, CallVideoEntry>>,
    /// Node-local X25519 secret for DM transport KEM (not identity key material).
    dm_transport_local_sk: x25519_dalek::StaticSecret,
    /// Peer identity wire → peer transport X25519 public key (`TransportKemHello`).
    dm_peer_transport_pks: RwLock<HashMap<String, [u8; 32]>>,
    /// Peer identity wires we successfully sent `TransportKemHello` toward.
    dm_transport_hello_sent: RwLock<HashSet<String>>,
}

/// One active call's transport bridge state held in [`SessionState::call_media`].
struct CallMediaEntry {
    peer_id: PeerId,
    controls: crate::call_media::MediaControls,
    /// Inbound sealed packets (peer → our engine). Cloned by the RX stream handler.
    wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

/// One active video call's transport bridge state held in [`SessionState::call_video`].
struct CallVideoEntry {
    peer_id: PeerId,
    controls: crate::call_video::VideoControls,
    /// Inbound sealed video chunks (peer → our engine). Cloned by the RX stream handler.
    wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

#[derive(Clone, Copy, Debug)]
struct CoordLookupBackoff {
    next_allowed_ms: i64,
    step_ms: i64,
}

struct PeerTables {
    by_peer_id: HashMap<PeerId, DmPeer>,
}

impl PeerTables {
    fn retain_invalid_dm_peer_ids(&mut self) {
        self.by_peer_id.retain(|peer, dm| {
            if dm.has_send_keys() {
                return true;
            }
            crate::peer_id_util::identity_wire_from_peer_id(peer).is_some()
        });
    }
}

impl SessionState {
    fn new(
        identity: crate::DecryptedIdentity,
        dm_peers_list: &[DmPeer],
        bootstrap_peer_ids: HashSet<PeerId>,
        transcript_path: Option<String>,
        app_namespace: Option<String>,
        network_profile: crate::p2p::network_transport::LocalNetworkProfile,
        ghal_bol_relay_state: Option<(PeerId, Vec<String>)>,
    ) -> Result<Self, ChatServerError> {
        let mut tables = PeerTables {
            by_peer_id: HashMap::new(),
        };
        for p in dm_peers_list {
            if let Some(pk) = p
                .public_key_hex
                .as_deref()
                .map(str::trim)
                .filter(|s| crate::contacts_v1::is_valid_public_key_hex(s))
            {
                if let Ok(dm) = DmPeer::from_public_key_hex(pk.to_string()) {
                    tables.by_peer_id.insert(dm.peer_id, dm);
                    continue;
                }
            }
            if let Some(pk) = identity_wire_from_peer_id(&p.peer_id) {
                tables.by_peer_id.insert(
                    p.peer_id,
                    DmPeer {
                        peer_id: p.peer_id,
                        public_key_hex: Some(pk),
                    },
                );
            } else if p.public_key_hex.is_some() {
                tables.by_peer_id.insert(p.peer_id, p.clone());
            } else {
                native_log::debug(
                    "session",
                    format!(
                        "skip dm peer {} at start: no identity wire and not an embedded-key peer id",
                        p.peer_id
                    ),
                );
            }
        }
        tables.retain_invalid_dm_peer_ids();
        Ok(Self {
            identity,
            peers: RwLock::new(tables),
            connected: RwLock::new(HashSet::new()),
            outbox: RwLock::new(HashMap::new()),
            outbound_ack_pending_poll: RwLock::new(HashSet::new()),
            seen_inbound_ids: RwLock::new(HashMap::new()),
            identified_emitted: RwLock::new(HashSet::new()),
            chat_ready_emitted: RwLock::new(HashSet::new()),
            published_listen: RwLock::new(Vec::new()),
            routed_dial_attempt_ms: RwLock::new(HashMap::new()),
            circuit_coord_dial_last_ms: RwLock::new(HashMap::new()),
            stream_open_log_emitted: RwLock::new(HashSet::new()),
            stream_open_inflight: RwLock::new(HashSet::new()),
            stream_open_backoff_until_ms: RwLock::new(HashMap::new()),
            stream_open_fail_log_ms: RwLock::new(HashMap::new()),
            bootstrap_peer_ids: RwLock::new(bootstrap_peer_ids),
            relay_reserve_requested: RwLock::new(HashSet::new()),
            relay_reserve_last_attempt_ms: RwLock::new(HashMap::new()),
            relay_reserve_in_flight_ms: RwLock::new(HashMap::new()),
            relay_reservation_accepted_ms: RwLock::new(HashMap::new()),
            bootstrap_tcp_conns: RwLock::new(HashMap::new()),
            bootstrap_identified: RwLock::new(HashSet::new()),
            bootstrap_tcp_since_ms: RwLock::new(HashMap::new()),
            bootstrap_reserve_after_ms: RwLock::new(HashMap::new()),
            bootstrap_relay_addr: RwLock::new(HashMap::new()),
            any_bootstrap_connected: AtomicBool::new(false),
            last_coord_lookup_ms: RwLock::new(HashMap::new()),
            coord_lookup_backoff: RwLock::new(HashMap::new()),
            bootstrap_dial_err_log_ms: RwLock::new(HashMap::new()),
            bootstrap_dial_last_ms: RwLock::new(HashMap::new()),
            bootstrap_ipv6_unreachable_ms: RwLock::new(HashMap::new()),
            incidental_rejects: RwLock::new(HashSet::new()),
            pending_read_acks: RwLock::new(VecDeque::new()),
            pending_delivery_acks: RwLock::new(VecDeque::new()),
            read_ack_confirmed: RwLock::new(HashSet::new()),
            delivery_ack_sent: RwLock::new(HashSet::new()),
            pending_call_signals: RwLock::new(VecDeque::new()),
            foreground_peer: RwLock::new(None),
            transcript_path,
            app_namespace,
            history_replay_done: RwLock::new(HashSet::new()),
            network_profile: RwLock::new(network_profile),
            wan_recovery_active: AtomicBool::new(false),
            ghal_bol_relay_state: RwLock::new(ghal_bol_relay_state),
            ghal_bol_relay_last_fetch_ms: RwLock::new(0),
            dial_skip_log_ms: RwLock::new(HashMap::new()),
            peers_on_local_lan: RwLock::new(HashMap::new()),
            peers_direct_conns: RwLock::new(HashMap::new()),
            dm_relay_conn_ids: RwLock::new(HashMap::new()),
            dm_direct_conn_ids: RwLock::new(HashMap::new()),
            dm_relay_circuit_pending: RwLock::new(HashSet::new()),
            lan_ephemeral_tcp_listener_ids: RwLock::new(Vec::new()),
            peer_mdns_lan_candidate_addrs: RwLock::new(HashMap::new()),
            lan_candidates_exhausted: RwLock::new(HashSet::new()),
            dm_reconnect_urgent: RwLock::new(HashMap::new()),
            dm_wire_activity_ms: RwLock::new(HashMap::new()),
            dm_no_writer_since_ms: RwLock::new(HashMap::new()),
            dm_stream_has_writer: RwLock::new(HashSet::new()),
            dm_writer_generation: RwLock::new(HashMap::new()),
            dm_writer_gen_counter: std::sync::atomic::AtomicU64::new(1),
            relay_keepalive_last_ms: RwLock::new(0),
            relay_circuit_dial_backoff_until: RwLock::new(HashMap::new()),
            dm_circuit_dial_in_flight_ms: RwLock::new(HashMap::new()),
            dm_lan_dial_in_flight_ms: RwLock::new(HashMap::new()),
            dm_peer_registered_ms: RwLock::new(HashMap::new()),
            coord_lookup_last_category: RwLock::new(HashMap::new()),
            last_wan_listen_fp: RwLock::new(Vec::new()),
            last_presence_wake_ms: RwLock::new(0),
            last_mdns_restart_ms: RwLock::new(0),
            last_lan_recovery_ms: RwLock::new(0),
            pending_full_lan_kick_reason: RwLock::new(None),
            lan_listen_rediscovery_peers: RwLock::new(HashSet::new()),
            pending_dm_link_reset: RwLock::new(HashSet::new()),
            asymmetric_relay_recover_urgent: RwLock::new(HashSet::new()),
            relay_inbound_handover_peers: RwLock::new(HashSet::new()),
            coord_lookup_info_log_ms: RwLock::new(HashMap::new()),
            call_media: Mutex::new(HashMap::new()),
            call_video: Mutex::new(HashMap::new()),
            dm_transport_local_sk: {
                let (sk, _) = crate::transport_kem_v1::generate_transport_keypair();
                sk
            },
            dm_peer_transport_pks: RwLock::new(HashMap::new()),
            dm_transport_hello_sent: RwLock::new(HashSet::new()),
        })
    }

    /// True while a native media session for `call_id` is registered.
    fn call_media_active(&self, call_id: &str) -> bool {
        self.call_media
            .lock()
            .map(|m| m.contains_key(call_id))
            .unwrap_or(false)
    }

    fn call_media_register(
        &self,
        call_id: String,
        peer_id: PeerId,
        controls: crate::call_media::MediaControls,
        wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        if let Ok(mut m) = self.call_media.lock() {
            m.insert(
                call_id,
                CallMediaEntry {
                    peer_id,
                    controls,
                    wire_in_tx,
                },
            );
        }
    }

    /// Channel into the engine for inbound packets of `call_id`, but only when the
    /// stream comes from the libp2p peer this call was started with (RX stream handler).
    fn call_media_wire_in_for_peer(
        &self,
        call_id: &str,
        peer: PeerId,
    ) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.call_media.lock().ok().and_then(|m| {
            m.get(call_id).and_then(|e| {
                if e.peer_id == peer {
                    Some(e.wire_in_tx.clone())
                } else {
                    None
                }
            })
        })
    }

    /// Stop and remove one media session; returns whether it existed.
    fn call_media_stop(&self, call_id: &str) -> bool {
        let entry = self
            .call_media
            .lock()
            .ok()
            .and_then(|mut m| m.remove(call_id));
        if let Some(e) = entry {
            e.controls.request_stop();
            true
        } else {
            false
        }
    }

    fn call_media_stop_all(&self) {
        if let Ok(mut m) = self.call_media.lock() {
            for (_, e) in m.drain() {
                e.controls.request_stop();
            }
        }
    }

    fn call_media_set_mic_muted(&self, call_id: &str, muted: bool) -> bool {
        self.call_media
            .lock()
            .ok()
            .and_then(|m| m.get(call_id).map(|e| e.controls.set_mic_muted(muted)))
            .is_some()
    }

    fn call_video_active(&self, call_id: &str) -> bool {
        self.call_video
            .lock()
            .map(|m| m.contains_key(call_id))
            .unwrap_or(false)
    }

    fn call_video_register(
        &self,
        call_id: String,
        peer_id: PeerId,
        controls: crate::call_video::VideoControls,
        wire_in_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        if let Ok(mut m) = self.call_video.lock() {
            m.insert(
                call_id,
                CallVideoEntry {
                    peer_id,
                    controls,
                    wire_in_tx,
                },
            );
        }
    }

    /// Channel into the video engine for inbound chunks of `call_id`, but only when
    /// the stream comes from the libp2p peer this call was started with.
    fn call_video_wire_in_for_peer(
        &self,
        call_id: &str,
        peer: PeerId,
    ) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.call_video.lock().ok().and_then(|m| {
            m.get(call_id).and_then(|e| {
                if e.peer_id == peer {
                    Some(e.wire_in_tx.clone())
                } else {
                    None
                }
            })
        })
    }

    fn call_video_stop(&self, call_id: &str) -> bool {
        let entry = self
            .call_video
            .lock()
            .ok()
            .and_then(|mut m| m.remove(call_id));
        crate::call_video::clear_decoded_frames(call_id);
        if let Some(e) = entry {
            e.controls.request_stop();
            true
        } else {
            false
        }
    }

    fn call_video_stop_all(&self) {
        if let Ok(mut m) = self.call_video.lock() {
            for (call_id, e) in m.drain() {
                e.controls.request_stop();
                crate::call_video::clear_decoded_frames(&call_id);
            }
        }
    }

    fn call_video_set_camera_off(&self, call_id: &str, off: bool) -> bool {
        self.call_video
            .lock()
            .ok()
            .and_then(|m| m.get(call_id).map(|e| e.controls.set_camera_off(off)))
            .is_some()
    }

    fn note_peer_on_local_lan(&self, peer: PeerId) {
        let now = chrono_now_ms();
        let Ok(mut m) = self.peers_on_local_lan.write() else {
            return;
        };
        m.insert(peer, now);
        m.retain(|_, t| now.saturating_sub(*t) < PEER_LAN_SEEN_TTL_MS);
    }

    fn peer_on_local_lan(&self, peer: PeerId) -> bool {
        let now = chrono_now_ms();
        self.peers_on_local_lan
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|t| now.saturating_sub(t) < PEER_LAN_SEEN_TTL_MS)
    }

    /// A peer left the LAN (mDNS `Expired`): drop its LAN preference so dial ranking
    /// returns to WAN-first immediately instead of waiting out `PEER_LAN_SEEN_TTL_MS`.
    /// Returns `true` if the peer was actually marked on-LAN.
    fn forget_peer_on_local_lan(&self, peer: PeerId) -> bool {
        let Ok(mut m) = self.peers_on_local_lan.write() else {
            return false;
        };
        let removed = m.remove(&peer).is_some();
        if removed {
            if let Ok(mut lan) = self.peer_mdns_lan_candidate_addrs.write() {
                lan.remove(&peer);
            }
        }
        removed
    }

    fn should_run_lan_recovery(&self, now_ms: i64) -> bool {
        let Ok(mut last) = self.last_lan_recovery_ms.write() else {
            return true;
        };
        if *last > 0 && now_ms.saturating_sub(*last) < LAN_RECOVERY_MIN_MS {
            return false;
        }
        *last = now_ms;
        true
    }

    /// On `lan → mobile-data` handover — drop cached LAN TCP addrs (not live direct conn counters).
    fn purge_all_mdns_lan_state(&self) {
        if let Ok(mut m) = self.peer_mdns_lan_candidate_addrs.write() {
            m.clear();
        }
        if let Ok(mut e) = self.lan_candidates_exhausted.write() {
            e.clear();
        }
        if let Ok(mut m) = self.peers_on_local_lan.write() {
            m.clear();
        }
    }

    fn merge_mdns_lan_candidate(&self, peer: PeerId, addr: &Multiaddr) -> bool {
        if !is_direct_lan_tcp_mdns_candidate(addr) {
            return false;
        }
        let mut added = false;
        if let Ok(mut m) = self.peer_mdns_lan_candidate_addrs.write() {
            let v = m.entry(peer).or_default();
            if !v.iter().any(|a| a == addr) {
                v.push(addr.clone());
                added = true;
            }
        }
        if added {
            if let Ok(mut e) = self.lan_candidates_exhausted.write() {
                e.remove(&peer);
            }
            self.clear_lan_listen_rediscovery(peer);
        }
        added
    }

    fn remove_mdns_lan_candidate(&self, peer: PeerId, failed: Option<&Multiaddr>) {
        let Some(failed_ma) = failed else {
            return;
        };
        if let Ok(mut m) = self.peer_mdns_lan_candidate_addrs.write() {
            if let Some(v) = m.get_mut(&peer) {
                v.retain(|a| a != failed_ma);
                if v.is_empty() {
                    m.remove(&peer);
                }
            }
        }
    }

    fn lan_dial_in_flight_start_ms(&self, peer: PeerId) -> Option<i64> {
        self.dm_lan_dial_in_flight_ms
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
    }

    /// Claim the LAN dial slot before `swarm.dial` so back-to-back mDNS events cannot parallel-dial.
    fn try_claim_lan_dial_slot(&self, peer: PeerId, now_ms: i64) -> bool {
        if self.lan_dial_in_flight_blocks(peer, now_ms) {
            return false;
        }
        self.mark_lan_dial_in_flight(peer, now_ms);
        true
    }

    fn chat_ready_seen(&self, peer: PeerId) -> bool {
        self.chat_ready_emitted
            .read()
            .ok()
            .is_some_and(|g| g.contains(&peer))
    }

    /// Ghost/stale contact: coord 404 and this peer never reached `chat_ready` — throttle, not urgent flood.
    fn peer_coord_absent_never_connected(&self, pk_hex: &str, peer: PeerId) -> bool {
        self.coord_lookup_category_for_pk(pk_hex)
            == Some(crate::p2p::connectivity_diag::CoordLookupCategory::PeerNotOnCoord)
            && !self.chat_ready_seen(peer)
    }

    /// mDNS `Expired` for one multiaddr — only drop LAN state when the expired addr is still
    /// our cached direct TCP target (libp2p often expires an old port after advertising a new one).
    fn note_peer_mdns_lan_addr_expired(&self, peer: PeerId, expired: &Multiaddr) -> bool {
        if crate::p2p::network_transport::is_relay_circuit_multiaddr(expired) {
            return false;
        }
        let had = self
            .peer_mdns_lan_candidate_addrs
            .read()
            .ok()
            .and_then(|m| m.get(&peer).cloned())
            .is_some_and(|v| v.iter().any(|a| a == expired));
        if !had {
            return false;
        }
        self.remove_mdns_lan_candidate(peer, Some(expired));
        if self.peer_mdns_lan_addr(peer).is_some() {
            return false;
        }
        if self.forget_peer_on_local_lan(peer) {
            self.request_lan_listen_rediscovery(peer);
            return true;
        }
        false
    }

    pub(crate) fn request_lan_listen_rediscovery(&self, peer: PeerId) {
        if let Ok(mut s) = self.lan_listen_rediscovery_peers.write() {
            s.insert(peer);
        }
    }

    fn clear_lan_listen_rediscovery(&self, peer: PeerId) {
        if let Ok(mut s) = self.lan_listen_rediscovery_peers.write() {
            s.remove(&peer);
        }
        self.clear_relay_inbound_handover_peer(peer);
    }

    /// After LAN→WAN handover — drop mDNS candidates + on-LAN TTL that block asymmetric mux recovery.
    fn clear_peer_stale_lan_cache(&self, peer: PeerId) {
        // Live mDNS means the peer is still on our LAN — keep on-LAN TTL (TRANSPORT.md § Parallel LAN + WAN).
        if self.peer_mdns_lan_addr(peer).is_some() {
            if let Ok(mut e) = self.lan_candidates_exhausted.write() {
                e.remove(&peer);
            }
            return;
        }
        self.forget_peer_on_local_lan(peer);
        if let Ok(mut m) = self.peer_mdns_lan_candidate_addrs.write() {
            m.remove(&peer);
        }
        if let Ok(mut e) = self.lan_candidates_exhausted.write() {
            e.remove(&peer);
        }
    }

    fn lan_listen_rediscovery_requested(&self, peer: PeerId) -> bool {
        self.lan_listen_rediscovery_peers
            .read()
            .ok()
            .is_some_and(|s| s.contains(&peer))
    }

    /// DM contacts that had mDNS or `peers_on_local_lan` before a handover purge.
    fn dm_peers_with_lan_history(&self) -> Vec<PeerId> {
        let on_lan: HashSet<PeerId> = self
            .peers_on_local_lan
            .read()
            .ok()
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default();
        let with_mdns: HashSet<PeerId> = self
            .peer_mdns_lan_candidate_addrs
            .read()
            .ok()
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default();
        self.dm_peer_ids()
            .into_iter()
            .filter(|p| on_lan.contains(p) || with_mdns.contains(p))
            .collect()
    }

    /// Next LAN TCP addr for failover only — first publishable candidate in mDNS discovery order.
    fn peer_mdns_lan_addr(&self, peer: PeerId) -> Option<Multiaddr> {
        let v = self
            .peer_mdns_lan_candidate_addrs
            .read()
            .ok()
            .and_then(|m| m.get(&peer).cloned())?;
        v.iter()
            .filter(|ma| is_direct_lan_tcp_mdns_candidate(ma))
            .find(|ma| crate::p2p::network_transport::is_publishable_listen_addr(ma))
            .or_else(|| v.iter().find(|ma| is_direct_lan_tcp_mdns_candidate(ma)))
            .cloned()
    }

    fn lan_candidates_exhausted(&self, peer: PeerId) -> bool {
        self.lan_candidates_exhausted
            .read()
            .ok()
            .is_some_and(|s| s.contains(&peer))
    }

    fn mark_lan_candidates_exhausted(&self, peer: PeerId) {
        if let Ok(mut s) = self.lan_candidates_exhausted.write() {
            s.insert(peer);
        }
        self.clear_lan_listen_rediscovery(peer);
    }

    fn clear_lan_candidates_exhausted(&self, peer: PeerId) {
        if let Ok(mut s) = self.lan_candidates_exhausted.write() {
            s.remove(&peer);
        }
    }

    fn note_pending_full_lan_kick(&self, reason: &str) {
        if let Ok(mut g) = self.pending_full_lan_kick_reason.write() {
            *g = Some(reason.to_string());
        }
    }

    fn pending_interface_drift_lan_kick(&self) -> bool {
        self.pending_full_lan_kick_reason
            .read()
            .ok()
            .and_then(|g| g.clone())
            .is_some_and(|r| r == "interface drift")
    }

    fn take_pending_full_lan_kick_reason(&self) -> Option<String> {
        self.pending_full_lan_kick_reason
            .write()
            .ok()
            .and_then(|mut g| g.take())
    }

    fn clear_pending_full_lan_kick(&self) {
        if let Ok(mut g) = self.pending_full_lan_kick_reason.write() {
            *g = None;
        }
    }

    /// Drop cached LAN TCP addrs after handover — stale ports must not dial before fresh mDNS.
    fn purge_mdns_lan_candidates_for_dm_peers(&self) {
        let peers: Vec<PeerId> = self.dm_peer_ids();
        if let Ok(mut m) = self.peer_mdns_lan_candidate_addrs.write() {
            for peer in &peers {
                m.remove(peer);
            }
        }
        if let Ok(mut lan) = self.peers_on_local_lan.write() {
            for peer in &peers {
                lan.remove(peer);
            }
        }
    }

    fn any_dm_circuit_dial_in_flight(&self, now_ms: i64) -> bool {
        self.dm_peer_ids()
            .iter()
            .any(|p| self.circuit_dial_in_flight_blocks(*p, now_ms))
    }

    fn peer_has_relay_connection(&self, peer: PeerId) -> bool {
        self.dm_relay_conn_ids
            .read()
            .ok()
            .is_some_and(|m| m.get(&peer).is_some_and(|s| !s.is_empty()))
    }

    /// Any registered DM contact without a live chat stream while on LAN (Wi‑Fi flap / switch).
    fn any_dm_peer_needs_lan_rediscovery(&self) -> bool {
        if !wifi_lan_handover_active(self) {
            return false;
        }
        self.dm_peer_ids()
            .iter()
            .any(|p| self.should_dial_libp2p_peer(*p) && !self.dm_peer_stream_up(*p))
    }

    /// Signing identity wire for a connected libp2p peer — roster entry first, then derive from
    /// embedded-key PeerId (stream can open before roster merge fills the row).
    pub(crate) fn signing_pk_for_libp2p_peer(&self, peer: PeerId) -> Option<String> {
        if let Some(wire) = self
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
            .filter(|pk| crate::contacts_v1::is_valid_public_key_hex(pk))
        {
            return Some(wire);
        }
        if let Some(wire) = identity_wire_from_peer_id(&peer) {
            return Some(wire);
        }
        let tables = self.peers.read().ok()?;
        crate::peer_id_util::identity_wire_matching_peer_id(
            &peer,
            tables
                .by_peer_id
                .values()
                .filter_map(|dm| dm.public_key_hex.as_deref()),
        )
    }

    pub(crate) fn peer_has_pending_outbox(&self, peer: PeerId) -> bool {
        self.signing_pk_for_libp2p_peer(peer)
            .is_some_and(|pk| self.has_pending_outbox_for_pk(&pk))
    }

    fn clear_routed_dial_throttle(&self, peer: PeerId) {
        if let Ok(mut g) = self.routed_dial_attempt_ms.write() {
            g.remove(&peer);
        }
    }

    /// Track a newly-established connection's path so we know whether a peer has a
    /// **direct** (non-relay) link. `is_relay` is derived from the connection's remote
    /// multiaddr (`/p2p-circuit`).
    fn note_connection_path(&self, peer: PeerId, is_relay: bool) {
        if is_relay {
            return;
        }
        if let Ok(mut m) = self.peers_direct_conns.write() {
            *m.entry(peer).or_insert(0) += 1;
        }
    }

    /// A connection closed; if it was a direct (non-relay) link, decrement the count.
    fn drop_connection_path(&self, peer: PeerId, is_relay: bool) {
        if is_relay {
            return;
        }
        if let Ok(mut m) = self.peers_direct_conns.write() {
            if let Some(n) = m.get_mut(&peer) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    m.remove(&peer);
                }
            }
        }
    }

    fn note_dm_relay_connection(&self, peer: PeerId, conn_id: ConnectionId) {
        if let Ok(mut m) = self.dm_relay_conn_ids.write() {
            m.entry(peer).or_default().insert(conn_id);
        }
        self.clear_relay_circuit_pending_peer(peer);
    }

    fn forget_dm_relay_connection(&self, peer: PeerId, conn_id: ConnectionId) {
        if let Ok(mut m) = self.dm_relay_conn_ids.write() {
            if let Some(set) = m.get_mut(&peer) {
                set.remove(&conn_id);
                if set.is_empty() {
                    m.remove(&peer);
                }
            }
        }
    }

    fn note_dm_direct_connection(&self, peer: PeerId, conn_id: ConnectionId) {
        if let Ok(mut m) = self.dm_direct_conn_ids.write() {
            m.entry(peer).or_default().insert(conn_id);
        }
    }

    fn forget_dm_direct_connection(&self, peer: PeerId, conn_id: ConnectionId) {
        if let Ok(mut m) = self.dm_direct_conn_ids.write() {
            if let Some(set) = m.get_mut(&peer) {
                set.remove(&conn_id);
                if set.is_empty() {
                    m.remove(&peer);
                }
            }
        }
    }

    fn note_relay_circuit_pending_peer(&self, peer: PeerId) {
        if let Ok(mut s) = self.dm_relay_circuit_pending.write() {
            s.insert(peer);
        }
    }

    fn take_relay_circuit_pending_peer(&self, peer: PeerId) -> bool {
        let Ok(mut s) = self.dm_relay_circuit_pending.write() else {
            return false;
        };
        s.remove(&peer)
    }

    fn peers_with_circuit_dial_in_flight(&self, now_ms: i64) -> Vec<PeerId> {
        let Ok(m) = self.dm_circuit_dial_in_flight_ms.read() else {
            return Vec::new();
        };
        m.iter()
            .filter(|(_, start)| now_ms.saturating_sub(**start) < CIRCUIT_DIAL_IN_FLIGHT_MS)
            .map(|(peer, _)| *peer)
            .collect()
    }

    fn clear_relay_circuit_pending_peer(&self, peer: PeerId) {
        if let Ok(mut s) = self.dm_relay_circuit_pending.write() {
            s.remove(&peer);
        }
    }

    fn drain_dm_direct_connection_ids(&self, peer: PeerId) -> Vec<ConnectionId> {
        let Ok(mut m) = self.dm_direct_conn_ids.write() else {
            return Vec::new();
        };
        m.remove(&peer)
            .map(|s| s.into_iter().collect())
            .unwrap_or_default()
    }

    fn note_lan_ephemeral_tcp_listener(&self, id: ListenerId) {
        if let Ok(mut v) = self.lan_ephemeral_tcp_listener_ids.write() {
            v.push(id);
        }
    }

    fn drain_lan_ephemeral_tcp_listener_ids(&self) -> Vec<ListenerId> {
        let Ok(mut v) = self.lan_ephemeral_tcp_listener_ids.write() else {
            return Vec::new();
        };
        std::mem::take(&mut *v)
    }

    /// True when at least one direct (non-relay) connection to `peer` is open.
    fn peer_has_direct_connection(&self, peer: PeerId) -> bool {
        self.peers_direct_conns
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|n| n > 0)
    }

    fn should_log_dial_skip(&self, peer: PeerId, now_ms: i64, min_interval_ms: i64) -> bool {
        let Ok(mut m) = self.dial_skip_log_ms.write() else {
            return true;
        };
        let last = m.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < min_interval_ms {
            return false;
        }
        m.insert(peer, now_ms);
        true
    }

    fn should_log_read_ack_seed_skip(&self, peer: PeerId, now_ms: i64) -> bool {
        const MIN_MS: i64 = 5_000;
        static M: OnceLock<RwLock<HashMap<PeerId, i64>>> = OnceLock::new();
        let Ok(mut m) = M.get_or_init(|| RwLock::new(HashMap::new())).write() else {
            return true;
        };
        let last = m.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < MIN_MS {
            return false;
        }
        m.insert(peer, now_ms);
        true
    }

    fn diag_ctx(&self) -> String {
        let profile = self.network_profile_snapshot().mode_label().to_string();
        let coord_cfg = crate::coord_runtime::coord_is_configured();
        let coord_reg = crate::coord_runtime::coord_is_registered();
        let boot = self.any_bootstrap_connected.load(Ordering::Relaxed);
        let wan_recovery = self.wan_recovery_active.load(Ordering::Relaxed);
        let outbox = self.outbox.read().ok().map(|m| m.len()).unwrap_or(0);
        let pending_delivery = self
            .pending_delivery_acks
            .read()
            .ok()
            .map(|q| q.len())
            .unwrap_or(0);
        let pending_read = self
            .pending_read_acks
            .read()
            .ok()
            .map(|q| q.len())
            .unwrap_or(0);
        let relay_listen = self
            .published_listen_snapshot()
            .iter()
            .any(|ma| crate::p2p::network_transport::is_relay_circuit_multiaddr(ma));
        format!(
            "profile={profile} coord_cfg={coord_cfg} coord_reg={coord_reg} bootstrap_ok={boot} relay_listen={relay_listen} wan_recovery={wan_recovery} outbox={outbox} pending_delivery_acks={pending_delivery} pending_read_acks={pending_read}"
        )
    }

    fn try_begin_stream_open(&self, peer: PeerId) -> bool {
        let now_ms = chrono_now_ms();
        if self.stream_open_backoff_active(peer, now_ms) {
            return false;
        }
        let Ok(mut g) = self.stream_open_inflight.write() else {
            return true;
        };
        g.insert(peer)
    }

    fn end_stream_open(&self, peer: PeerId) {
        let Ok(mut g) = self.stream_open_inflight.write() else {
            return;
        };
        g.remove(&peer);
    }

    fn stream_open_backoff_active(&self, peer: PeerId, now_ms: i64) -> bool {
        self.stream_open_backoff_until_ms
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|until| now_ms < until)
    }

    fn clear_stream_open_backoff(&self, peer: PeerId) {
        if let Ok(mut m) = self.stream_open_backoff_until_ms.write() {
            m.remove(&peer);
        }
        if let Ok(mut m) = self.stream_open_fail_log_ms.write() {
            m.remove(&peer);
        }
        if let Ok(mut g) = self.stream_open_log_emitted.write() {
            g.remove(&peer);
        }
    }

    fn note_stream_open_failure(&self, peer: PeerId, err: &str) {
        const BACKOFF_MS: i64 = 3_000;
        const ZOMBIE_BACKOFF_MS: i64 = 5_000;
        const NO_WRITER_MS: i64 = 6_000;
        const FRESH_LINK_MS: i64 = 30_000;
        let now_ms = chrono_now_ms();
        let err_lc = err.to_lowercase();
        if (err_lc.contains("oneshot canceled") || err_lc.contains("receiver is gone"))
            && self.dm_has_stream_writer(peer)
        {
            if let Ok(mut g) = self.stream_open_inflight.write() {
                g.remove(&peer);
            }
            return;
        }
        let had_ready = self.chat_ready_seen(peer);
        let fresh_link = self
            .dm_wire_activity_ms
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|t| now_ms.saturating_sub(t) < FRESH_LINK_MS);
        if err_lc.contains("timed out") {
            let stale_direct = peer_has_stale_direct_lan_conn(self, peer);
            if stale_direct {
                self.request_dm_link_reset(peer);
                if let Some(pk) = self
                    .dm_peer_for_libp2p(peer)
                    .and_then(|d| d.public_key_hex.clone())
                {
                    self.mark_dm_reconnect_urgent(&pk);
                }
                notify_coord_lookup();
            } else if had_ready || fresh_link || self.peer_has_relay_connection(peer) {
                self.request_dm_stream_reopen(peer);
            } else {
                self.request_dm_link_reset(peer);
            }
        }
        let reset = stream_open_needs_connection_reset(err);
        let backoff = if reset { ZOMBIE_BACKOFF_MS } else { BACKOFF_MS };
        if let Ok(mut m) = self.stream_open_backoff_until_ms.write() {
            m.insert(peer, now_ms.saturating_add(backoff));
        }
        self.set_dm_stream_writer(peer, false);
        self.clear_chat_ready_emitted(peer);
        if reset {
            if let Ok(mut m) = self.dm_no_writer_since_ms.write() {
                m.insert(peer, now_ms.saturating_sub(NO_WRITER_MS));
            }
            self.request_dm_stream_reopen(peer);
        }
    }

    fn should_emit_stream_open_dial_failed(&self, peer: PeerId, now_ms: i64) -> bool {
        const MIN_LOG_MS: i64 = 5_000;
        let Ok(mut m) = self.stream_open_fail_log_ms.write() else {
            return true;
        };
        let last = m.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < MIN_LOG_MS {
            return false;
        }
        m.insert(peer, now_ms);
        true
    }

    /// Drop the per-contact stream writer; next `dm_upkeep` reopens on the existing libp2p link.
    /// Protonet: `chatStreams.Delete` on reset — never `disconnect_peer` while route may still work.
    fn request_dm_stream_reopen(&self, peer: PeerId) {
        self.set_dm_stream_writer(peer, false);
        self.clear_chat_ready_emitted(peer);
        if let Ok(mut g) = self.stream_open_inflight.write() {
            g.remove(&peer);
        }
        notify_stream_reopen();
    }

    fn request_dm_link_reset(&self, peer: PeerId) {
        if let Ok(mut s) = self.pending_dm_link_reset.write() {
            s.insert(peer);
        }
    }

    fn take_pending_dm_link_resets(&self) -> Vec<PeerId> {
        let Ok(mut s) = self.pending_dm_link_reset.write() else {
            return Vec::new();
        };
        s.drain().collect()
    }

    /// Consume one-shot urgent reconcile for `peer` (set on relay inbound circuit).
    fn take_asymmetric_relay_recover_urgent(&self, peer: PeerId) -> bool {
        let Ok(mut g) = self.asymmetric_relay_recover_urgent.write() else {
            return false;
        };
        g.remove(&peer)
    }

    fn mark_asymmetric_relay_recover_urgent(&self, peer: PeerId) {
        if let Ok(mut g) = self.asymmetric_relay_recover_urgent.write() {
            g.insert(peer);
        }
    }

    /// Mobile peer re-dialed inbound on relay during our LAN rediscovery window.
    pub(crate) fn mark_relay_inbound_handover_peer(&self, peer: PeerId) {
        if let Ok(mut g) = self.relay_inbound_handover_peers.write() {
            g.insert(peer);
        }
    }

    pub(crate) fn relay_inbound_handover_active(&self, peer: PeerId) -> bool {
        self.relay_inbound_handover_peers
            .read()
            .ok()
            .is_some_and(|g| g.contains(&peer))
    }

    pub(crate) fn clear_relay_inbound_handover_peer(&self, peer: PeerId) {
        if let Ok(mut g) = self.relay_inbound_handover_peers.write() {
            g.remove(&peer);
        }
    }

    fn begin_wan_recovery(&self) {
        self.wan_recovery_active.store(true, Ordering::Relaxed);
    }

    fn refresh_bootstrap_connected_flag(&self, swarm: &Swarm<ChatBehaviour>) {
        let any = self.bootstrap_peer_ids.read().ok().is_some_and(|g| {
            g.iter()
                .any(|p| has_tracked_bootstrap_tcp(self, *p) && swarm.is_connected(p))
        });
        self.any_bootstrap_connected.store(any, Ordering::Relaxed);
    }

    fn network_profile_snapshot(&self) -> crate::p2p::network_transport::LocalNetworkProfile {
        self.network_profile
            .read()
            .ok()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    /// Re-detect interfaces; returns `(old_mode, new_mode)` when dial/coord strategy should change.
    fn refresh_network_path_if_changed(
        &self,
        swarm: &Swarm<ChatBehaviour>,
    ) -> Option<(String, String)> {
        let detected = detected_network_with_platform_hints();
        let new = network_profile_for_swarm(swarm, detected);
        let Ok(mut cur) = self.network_profile.write() else {
            return None;
        };
        let old_key = crate::p2p::network_transport::network_handover_key(&*cur);
        let new_key = crate::p2p::network_transport::network_handover_key(&new);
        let lan_restored = new.has_active_lan() && !cur.has_active_lan();
        if old_key == new_key && !lan_restored {
            return None;
        }
        let old_mode = cur.mode_label().to_string();
        *cur = new;
        let new_mode = cur.mode_label().to_string();
        Some((old_mode, new_mode))
    }

    /// Mobile/CGNAT without active Wi‑Fi LAN — prefer coord/relay; skip blind peerstore dials.
    /// Wi‑Fi + RFC1918 keeps routed dials enabled so LAN/mDNS paths stay smooth.
    fn prefers_mobile_coord_strategy(&self) -> bool {
        self.network_profile_snapshot().avoid_blind_routed_dial()
    }

    fn should_coord_lookup_pk(&self, pk_hex: &str, now_ms: i64, min_interval_ms: i64) -> bool {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return false;
        }
        let handover_coord_degraded = wifi_lan_handover_active(self)
            && crate::coord_runtime::coord_http_degraded();
        // On LAN with dead coord HTTP, hammering lookup every upkeep tick hides mDNS recovery.
        let lookup_min_ms = if handover_coord_degraded {
            15_000
        } else {
            min_interval_ms
        };
        if self.is_pk_reconnect_urgent(pk, now_ms) {
            let Ok(mut m) = self.last_coord_lookup_ms.write() else {
                return true;
            };
            let last = m.get(pk).copied().unwrap_or(0);
            // Urgent reconnect must not be throttled to 15s — that stalls WAN after handover.
            let min_gap = 800;
            if now_ms.saturating_sub(last) < min_gap {
                return false;
            }
            m.insert(pk.to_string(), now_ms);
            return true;
        }
        if let Ok(m) = self.coord_lookup_backoff.read() {
            if let Some(b) = m.get(pk) {
                if now_ms < b.next_allowed_ms {
                    return false;
                }
            }
        }
        let Ok(mut m) = self.last_coord_lookup_ms.write() else {
            return true;
        };
        let last = m.get(pk).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < lookup_min_ms {
            return false;
        }
        m.insert(pk.to_string(), now_ms);
        true
    }

    /// Like [`should_coord_lookup_pk`] but **ignores the 404 / unreachable backoff** — for peers
    /// with active intent (pending outbox or the foreground chat). Enforces only `min_interval_ms`
    /// so coord is not hammered every tick. **Intent beats backoff** (TRANSPORT.md § prime
    /// directive): a peer the user is actively trying to reach must never be silenced by a stale
    /// "peer offline" backoff — if they are reachable now, we find them within seconds.
    fn should_coord_lookup_intent_pk(&self, pk_hex: &str, now_ms: i64, min_interval_ms: i64) -> bool {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return false;
        }
        let Ok(mut m) = self.last_coord_lookup_ms.write() else {
            return true;
        };
        let last = m.get(pk).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < min_interval_ms {
            return false;
        }
        m.insert(pk.to_string(), now_ms);
        true
    }

    /// Read-only ms of the last coord lookup for `pk` (0 if never). Used as an
    /// LRU key so the bounded background sweep always picks the most-stale-by-time
    /// contacts first — a huge idle roster is swept fairly, never starved.
    fn coord_lookup_last_ms(&self, pk_hex: &str) -> i64 {
        self.last_coord_lookup_ms
            .read()
            .ok()
            .and_then(|m| m.get(pk_hex.trim()).copied())
            .unwrap_or(0)
    }

    fn note_coord_lookup_not_found(&self, pk_hex: &str, now_ms: i64) {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return;
        }
        let Ok(mut m) = self.coord_lookup_backoff.write() else {
            return;
        };
        let prev = m.get(pk).copied();
        // Fast initial retries, then back off hard (coord can't return what isn't registered).
        let mut next_step = match prev {
            None => 1_000,
            Some(p) => (p.step_ms.saturating_mul(2)).clamp(1_000, 30_000),
        };
        // Fast retries only while urgent reconnect; otherwise back off (ghost 404 contacts).
        if self.dm_peer(pk).is_some() && self.is_pk_reconnect_urgent(pk, now_ms) {
            next_step = next_step.min(3_000);
        }
        m.insert(
            pk.to_string(),
            CoordLookupBackoff {
                next_allowed_ms: now_ms.saturating_add(next_step),
                step_ms: next_step,
            },
        );
        crate::coord_runtime::sync_coord_lookup_peer_not_found(pk, next_step as u64, now_ms);
    }

    /// Longer lookup backoff when coord HTTPS transport fails (throttle only — no dial cache).
    fn note_coord_lookup_http_unreachable(&self, pk_hex: &str, now_ms: i64) {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return;
        }
        let Ok(mut m) = self.coord_lookup_backoff.write() else {
            return;
        };
        let prev = m.get(pk).copied();
        let next_step = match prev {
            None => 15_000,
            Some(p) => (p.step_ms.saturating_mul(2)).clamp(15_000, 60_000),
        };
        m.insert(
            pk.to_string(),
            CoordLookupBackoff {
                next_allowed_ms: now_ms.saturating_add(next_step),
                step_ms: next_step,
            },
        );
    }

    /// A DM connection just closed — mark its key urgent so reconnect is attempted immediately
    /// (bypassing the coord 404 backoff) for a bounded window. See AGENTS.md override rules.
    /// Does not extend an active window (avoids perpetual urgent from repeated upkeep ticks).
    fn mark_dm_reconnect_urgent(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return;
        }
        // A fresh drop invalidates any prior "peer_not_on_server" backoff: the peer was just
        // here, so try coord again right away instead of waiting out the exponential gap.
        self.clear_coord_lookup_backoff(pk);
        let now = chrono_now_ms();
        let deadline = now.saturating_add(DM_RECONNECT_URGENT_WINDOW_MS);
        if let Ok(mut m) = self.dm_reconnect_urgent.write() {
            if m.get(pk).is_some_and(|d| now < *d) {
                return;
            }
            m.insert(pk.to_string(), deadline);
        }
    }

    /// Force-refresh urgent window (disconnect, outbox restore at node_ready) even if still active.
    fn refresh_dm_reconnect_urgent(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return;
        }
        self.clear_coord_lookup_backoff(pk);
        if let Ok(mut m) = self.dm_reconnect_urgent.write() {
            m.insert(
                pk.to_string(),
                chrono_now_ms().saturating_add(DM_RECONNECT_URGENT_WINDOW_MS),
            );
        }
    }

    /// WAN/relay churn must not disturb contacts with a live direct LAN DM stream.
    fn mark_dm_reconnect_urgent_unless_live_direct_stream(&self) {
        for peer in self.dm_peer_ids() {
            // Direct stream is only "live LAN" when mDNS still has a candidate — not a zombie link.
            if self.dm_peer_stream_up(peer)
                && self.peer_has_direct_connection(peer)
                && self.peer_mdns_lan_addr(peer).is_some()
            {
                continue;
            }
            if let Some(pk) = self
                .dm_peer_for_libp2p(peer)
                .and_then(|d| d.public_key_hex.clone())
                .filter(|pk| crate::contacts_v1::is_valid_public_key_hex(pk))
            {
                self.mark_dm_reconnect_urgent(&pk);
            }
        }
    }

    fn is_pk_reconnect_urgent(&self, pk_hex: &str, now_ms: i64) -> bool {
        let pk = pk_hex.trim();
        self.dm_reconnect_urgent
            .read()
            .ok()
            .and_then(|m| m.get(pk).copied())
            .is_some_and(|deadline| now_ms < deadline)
    }

    fn is_peer_reconnect_urgent(&self, peer: PeerId, now_ms: i64) -> bool {
        self.dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex)
            .is_some_and(|pk| self.is_pk_reconnect_urgent(&pk, now_ms))
    }

    /// DM keys still inside their urgent-reconnect window (expired entries are dropped).
    fn urgent_reconnect_pks(&self, now_ms: i64) -> Vec<String> {
        let Ok(mut m) = self.dm_reconnect_urgent.write() else {
            return Vec::new();
        };
        m.retain(|_, deadline| now_ms < *deadline);
        m.keys().cloned().collect()
    }

    fn clear_dm_reconnect_urgent(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if pk.is_empty() {
            return;
        }
        if let Ok(mut m) = self.dm_reconnect_urgent.write() {
            m.remove(pk);
        }
    }

    fn clear_coord_lookup_backoff(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return;
        }
        if let Ok(mut m) = self.coord_lookup_backoff.write() {
            m.remove(pk);
        }
        crate::coord_runtime::clear_coord_lookup_backoff_for_pk(pk);
    }

    fn note_dm_wire_activity(&self, peer: PeerId) {
        if let Ok(mut m) = self.dm_wire_activity_ms.write() {
            m.insert(peer, chrono_now_ms());
        }
    }

    /// Recent inbound/outbound DM frames on the live chat mux (not idle outbox alone).
    fn dm_mux_recently_active(&self, peer: PeerId, now_ms: i64) -> bool {
        const ACTIVE_MS: i64 = 15_000;
        self.dm_wire_activity_ms
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|t| now_ms.saturating_sub(t) < ACTIVE_MS)
    }

    /// Last confirmed **inbound** DM frame (acks, text, call signals from peer).
    fn note_dm_inbound_activity(&self, peer: PeerId) {
        self.note_dm_wire_activity(peer);
    }

    fn set_dm_stream_writer(&self, peer: PeerId, open: bool) {
        if let Ok(mut s) = self.dm_stream_has_writer.write() {
            if open {
                s.insert(peer);
                if let Ok(mut m) = self.dm_no_writer_since_ms.write() {
                    m.remove(&peer);
                }
                self.clear_lan_listen_rediscovery(peer);
            } else {
                s.remove(&peer);
            }
        }
    }

    fn dm_has_stream_writer(&self, peer: PeerId) -> bool {
        self.dm_stream_has_writer
            .read()
            .ok()
            .is_some_and(|s| s.contains(&peer))
    }

    /// Claim a fresh writer generation for `peer` (called when a stream installs the mux writer).
    /// Any handler holding an older generation will skip its teardown (see frames.rs).
    fn claim_dm_writer_generation(&self, peer: PeerId) -> u64 {
        let generation = self
            .dm_writer_gen_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut m) = self.dm_writer_generation.write() {
            m.insert(peer, generation);
        }
        generation
    }

    /// True only while `generation` is still the live writer generation for `peer`; clears the
    /// record when it matches so a stale handler tears down once and never clobbers a newer mux.
    fn release_dm_writer_generation_if_current(&self, peer: PeerId, generation: u64) -> bool {
        let Ok(mut m) = self.dm_writer_generation.write() else {
            return true;
        };
        if m.get(&peer).copied() == Some(generation) {
            m.remove(&peer);
            true
        } else {
            false
        }
    }

    /// Evidence the currently-owned writer is **not** draining outbound work: a pending delivery
    /// ack we could not send, or an outbound text queued longer than `min_ms` and still unacked.
    /// Used to decide whether a live duplicate inbound stream should take over the writer.
    ///
    /// Transcript-restored ghost rows (never on wire this session) must **not** count — they have
    /// ancient `created_at_ms` and falsely trip reconcile every upkeep tick (flutter_linux.log
    /// 2026-06-28: `close stale direct` every ~5s while relay inbound still delivered).
    pub(crate) fn peer_outbound_stuck_for(&self, peer: PeerId, now_ms: i64, min_ms: i64) -> bool {
        if self.has_pending_delivery_acks_older_than(peer, now_ms, min_ms) {
            return true;
        }
        let Some(pk) = self
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
        else {
            return false;
        };
        let pk = pk.trim().to_lowercase();
        self.outbox.read().ok().is_some_and(|g| {
            g.values().any(|p| {
                if !p.recipient_public_key_hex.trim().eq_ignore_ascii_case(&pk) {
                    return false;
                }
                // On wire, no delivery ack yet — sustained stuck (first wire time, not resync bumps).
                if p.on_wire {
                    let since = if p.first_on_wire_ms > 0 {
                        p.first_on_wire_ms
                    } else {
                        p.last_send_ms
                    };
                    return now_ms.saturating_sub(since) >= min_ms;
                }
                // Never attempted on wire this session (transcript ghost at bootstrap).
                if p.last_send_ms == 0 {
                    return false;
                }
                // Send path retrying without reaching wire — only fresh user intent.
                let fresh_intent = now_ms.saturating_sub(p.created_at_ms) < min_ms.saturating_mul(4);
                fresh_intent && now_ms.saturating_sub(p.last_send_ms) >= min_ms
            })
        })
    }

    /// Protonet-style: open chat stream = healthy. Upkeep/coord/identify must not touch this peer.
    fn dm_peer_stream_up(&self, peer: PeerId) -> bool {
        self.dm_has_stream_writer(peer)
    }

    /// Connected but no live DM stream writer for long enough — reopen stream on next upkeep tick.
    ///
    /// An open stream with idle hub / no inbound frames is normal (protonet-as-reference
    /// `chatStreams` hit → 1s upkeep noop). Do not infer staleness from outbox or inbound silence.
    fn dm_link_needs_recovery(&self, peer: PeerId, now_ms: i64) -> bool {
        if self.dm_peer_stream_up(peer) {
            return false;
        }
        const NO_WRITER_MS: i64 = 6_000;
        let Ok(mut m) = self.dm_no_writer_since_ms.write() else {
            return true;
        };
        let first = m.entry(peer).or_insert(now_ms);
        now_ms.saturating_sub(*first) >= NO_WRITER_MS
    }

    fn should_relay_keepalive(&self, now_ms: i64) -> bool {
        const INTERVAL_MS: i64 = 45_000;
        let Ok(mut last) = self.relay_keepalive_last_ms.write() else {
            return false;
        };
        if now_ms.saturating_sub(*last) < INTERVAL_MS {
            return false;
        }
        *last = now_ms;
        true
    }

    fn note_relay_circuit_dial_rate_limited(&self, peer: PeerId, now_ms: i64) {
        const BACKOFF_MS: i64 = 90_000;
        if let Ok(mut m) = self.relay_circuit_dial_backoff_until.write() {
            m.insert(peer, now_ms.saturating_add(BACKOFF_MS));
        }
    }

    fn relay_circuit_dial_backoff_active(&self, peer: PeerId, now_ms: i64) -> bool {
        let Ok(mut m) = self.relay_circuit_dial_backoff_until.write() else {
            return false;
        };
        if let Some(until) = m.get(&peer).copied() {
            if now_ms < until {
                return true;
            }
            m.remove(&peer);
        }
        false
    }

    fn clear_relay_circuit_dial_backoff(&self, peer: PeerId) {
        if let Ok(mut m) = self.relay_circuit_dial_backoff_until.write() {
            m.remove(&peer);
        }
    }

    fn mark_circuit_dial_in_flight(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut m) = self.dm_circuit_dial_in_flight_ms.write() {
            m.insert(peer, now_ms);
        }
    }

    fn clear_circuit_dial_in_flight(&self, peer: PeerId) {
        if let Ok(mut m) = self.dm_circuit_dial_in_flight_ms.write() {
            m.remove(&peer);
        }
    }

    /// True while a relay-circuit dial to this peer must not be replaced (avoids oneshot cancel).
    fn circuit_dial_in_flight_blocks(&self, peer: PeerId, now_ms: i64) -> bool {
        let Ok(m) = self.dm_circuit_dial_in_flight_ms.read() else {
            return false;
        };
        let limit_ms = if self.is_peer_reconnect_urgent(peer, now_ms)
            || self.peer_has_pending_wire_work(peer)
        {
            CIRCUIT_DIAL_IN_FLIGHT_URGENT_MS
        } else {
            CIRCUIT_DIAL_IN_FLIGHT_MS
        };
        m.get(&peer)
            .is_some_and(|start| now_ms.saturating_sub(*start) < limit_ms)
    }

    /// Drop stale in-flight circuit dials so a hung hop does not block retries forever.
    /// Returns peers whose in-flight window expired (caller may reset libp2p dial state).
    fn expire_stale_circuit_dials(&self, now_ms: i64) -> Vec<PeerId> {
        let Ok(mut m) = self.dm_circuit_dial_in_flight_ms.write() else {
            return Vec::new();
        };
        let mut expired = Vec::new();
        m.retain(|peer, start| {
            let limit_ms = if self.is_peer_reconnect_urgent(*peer, now_ms)
                || self.peer_has_pending_wire_work(*peer)
            {
                CIRCUIT_DIAL_IN_FLIGHT_URGENT_MS
            } else {
                CIRCUIT_DIAL_IN_FLIGHT_MS
            };
            let keep = now_ms.saturating_sub(*start) < limit_ms;
            if !keep {
                native_log::warn(
                    "dial",
                    format!(
                        "relay-circuit dial to {peer} timed out ({limit_ms}ms) — retry allowed"
                    ),
                );
                expired.push(*peer);
            }
            keep
        });
        expired
    }

    fn mark_lan_dial_in_flight(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut m) = self.dm_lan_dial_in_flight_ms.write() {
            m.insert(peer, now_ms);
        }
    }

    fn clear_lan_dial_in_flight(&self, peer: PeerId) {
        if let Ok(mut m) = self.dm_lan_dial_in_flight_ms.write() {
            m.remove(&peer);
        }
    }

    /// True while a direct LAN TCP dial to this peer is in flight — blocks stacking another LAN dial only.
    fn lan_dial_in_flight_blocks(&self, peer: PeerId, now_ms: i64) -> bool {
        let Ok(m) = self.dm_lan_dial_in_flight_ms.read() else {
            return false;
        };
        m.get(&peer)
            .is_some_and(|start| now_ms.saturating_sub(*start) < LAN_DIAL_IN_FLIGHT_MS)
    }

    fn expire_stale_lan_dials(&self, now_ms: i64) {
        let Ok(mut m) = self.dm_lan_dial_in_flight_ms.write() else {
            return;
        };
        m.retain(|_peer, start| now_ms.saturating_sub(*start) < LAN_DIAL_IN_FLIGHT_MS + 5_000);
    }

    fn set_coord_lookup_category(
        &self,
        pk_hex: &str,
        cat: crate::p2p::connectivity_diag::CoordLookupCategory,
    ) {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return;
        }
        if let Ok(mut m) = self.coord_lookup_last_category.write() {
            m.insert(pk.to_string(), cat);
        }
    }

    fn coord_lookup_category_for_peer(
        &self,
        peer: PeerId,
    ) -> Option<crate::p2p::connectivity_diag::CoordLookupCategory> {
        let pk = self
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())?;
        self.coord_lookup_category_for_pk(&pk)
    }

    fn coord_lookup_category_for_pk(
        &self,
        pk_hex: &str,
    ) -> Option<crate::p2p::connectivity_diag::CoordLookupCategory> {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return None;
        }
        self.coord_lookup_last_category
            .read()
            .ok()
            .and_then(|m| m.get(pk).copied())
    }

    /// Throttle coord HTTP for peers absent from coord (404) or transport errors.
    fn should_skip_coord_lookup_pk(&self, pk_hex: &str, now_ms: i64) -> bool {
        let Some(cat) = self.coord_lookup_category_for_pk(pk_hex) else {
            return false;
        };
        if cat != crate::p2p::connectivity_diag::CoordLookupCategory::PeerNotOnCoord
            && cat != crate::p2p::connectivity_diag::CoordLookupCategory::CoordHttpUnreachable
        {
            return false;
        }
        if self.is_pk_reconnect_urgent(pk_hex, now_ms) {
            return false;
        }
        if let Ok(m) = self.coord_lookup_backoff.read() {
            if let Some(b) = m.get(pk_hex.trim()) {
                return now_ms < b.next_allowed_ms;
            }
        }
        false
    }

    /// Clear stale 404/backoff so the next upkeep tick can find a peer who just registered.
    fn clear_peer_coord_absent_state(&self, pk_hex: &str) {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return;
        }
        self.clear_coord_lookup_backoff(pk);
        if let Ok(mut m) = self.coord_lookup_last_category.write() {
            if m.get(pk) == Some(&crate::p2p::connectivity_diag::CoordLookupCategory::PeerNotOnCoord) {
                m.remove(pk);
            }
        }
    }

    /// Self presence wake or handover — reachable contacts get urgent rediscovery; 404 ghosts get one backoff-cleared retry only.
    fn wake_all_dm_peers_rediscovery(&self, now_ms: i64) {
        for pk in self.dm_public_keys() {
            if let Ok(peer) = peer_id_from_identity_wire(&pk) {
                if self.dm_peer_stream_up(peer) {
                    continue;
                }
            }
            if self.coord_lookup_category_for_pk(&pk)
                == Some(crate::p2p::connectivity_diag::CoordLookupCategory::PeerNotOnCoord)
            {
                self.clear_coord_lookup_backoff(&pk);
                continue;
            }
            self.clear_peer_coord_absent_state(&pk);
            self.refresh_dm_reconnect_urgent(&pk);
            if let Ok(peer) = peer_id_from_identity_wire(&pk) {
                self.clear_lan_candidates_exhausted(peer);
                self.clear_lan_dial_in_flight(peer);
                if self.network_profile_snapshot().has_active_lan() {
                    self.request_lan_listen_rediscovery(peer);
                }
            }
        }
        if let Ok(mut t) = self.last_presence_wake_ms.write() {
            *t = now_ms;
        }
    }

    fn should_restart_mdns(&self, now_ms: i64) -> bool {
        const MIN_MS: i64 = 8_000;
        let Ok(mut last) = self.last_mdns_restart_ms.write() else {
            return true;
        };
        if *last > 0 && now_ms.saturating_sub(*last) < MIN_MS {
            return false;
        }
        *last = now_ms;
        true
    }

    fn should_run_presence_wake(&self, now_ms: i64) -> bool {
        let last = self
            .last_presence_wake_ms
            .read()
            .ok()
            .map(|t| *t)
            .unwrap_or(0);
        last == 0 || now_ms.saturating_sub(last) >= PRESENCE_WAKE_RUN_DEBOUNCE_MS
    }

    fn should_log_coord_lookup_info(
        &self,
        pk_hex: &str,
        now_ms: i64,
        min_interval_ms: i64,
    ) -> bool {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return true;
        }
        let Ok(mut m) = self.coord_lookup_info_log_ms.write() else {
            return true;
        };
        let last = m.get(pk).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < min_interval_ms {
            return false;
        }
        m.insert(pk.to_string(), now_ms);
        true
    }

    fn pending_outbox_count(&self) -> usize {
        self.outbox.read().ok().map(|g| g.len()).unwrap_or(0)
    }

    /// True when relay/public listen addrs changed since the last snapshot (including first circuit).
    fn wan_listen_fp_changed(&self, fp: &[String]) -> bool {
        if fp.is_empty() {
            return false;
        }
        let Ok(mut last) = self.last_wan_listen_fp.write() else {
            return false;
        };
        let changed = last.is_empty() || *last != *fp;
        *last = fp.to_vec();
        changed
    }

    fn is_kept_peer(&self, peer: PeerId) -> bool {
        self.is_dm_contact(peer) || self.is_bootstrap_peer(peer)
    }

    fn mark_incidental_reject(&self, peer: PeerId) {
        if let Ok(mut g) = self.incidental_rejects.write() {
            g.insert(peer);
        }
    }

    fn consume_incidental_reject(&self, peer: PeerId) -> bool {
        self.incidental_rejects
            .write()
            .ok()
            .is_some_and(|mut g| g.remove(&peer))
    }

    /// Returns false when the same bootstrap relay was dialed too recently (pending TCP).
    fn should_issue_bootstrap_dial(&self, peer: PeerId, now_ms: i64, force: bool) -> bool {
        const THROTTLE_MS: i64 = 10_000;
        const FORCE_MIN_MS: i64 = 3_000;
        let gap = if force { FORCE_MIN_MS } else { THROTTLE_MS };
        let Ok(mut m) = self.bootstrap_dial_last_ms.write() else {
            return true;
        };
        let last = m.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < gap {
            return false;
        }
        m.insert(peer, now_ms);
        true
    }

    fn should_log_bootstrap_dial_err(&self, peer: PeerId, now_ms: i64) -> bool {
        let Ok(mut g) = self.bootstrap_dial_err_log_ms.write() else {
            return true;
        };
        let last = g.get(&peer).copied().unwrap_or(0);
        if now_ms.saturating_sub(last) < 30_000 {
            return false;
        }
        g.insert(peer, now_ms);
        true
    }

    fn bootstrap_ipv6_degraded(&self, peer: PeerId, now_ms: i64) -> bool {
        const DEGRADED_MS: i64 = 600_000;
        self.bootstrap_ipv6_unreachable_ms
            .read()
            .ok()
            .and_then(|m| m.get(&peer).copied())
            .is_some_and(|t| now_ms.saturating_sub(t) < DEGRADED_MS)
    }

    fn note_bootstrap_ipv6_unreachable(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut m) = self.bootstrap_ipv6_unreachable_ms.write() {
            m.insert(peer, now_ms);
        }
    }

    fn bootstrap_family_rank(&self, ma: &Multiaddr, peer: PeerId, now_ms: i64) -> u8 {
        let profile = self.network_profile_snapshot();
        crate::p2p::network_transport::relay_bootstrap_family_rank(
            ma,
            profile.on_mobile_data_path(),
            self.bootstrap_ipv6_degraded(peer, now_ms),
        )
    }

    fn note_bootstrap_connected(&self) {
        self.any_bootstrap_connected.store(true, Ordering::Relaxed);
    }

    fn is_bootstrap_peer(&self, peer: PeerId) -> bool {
        self.bootstrap_peer_ids
            .read()
            .ok()
            .is_some_and(|g| g.contains(&peer))
    }

    fn note_bootstrap_identified(&self, relay: PeerId) {
        if let Ok(mut g) = self.bootstrap_identified.write() {
            g.insert(relay);
        }
    }

    fn is_bootstrap_identified(&self, relay: PeerId) -> bool {
        self.bootstrap_identified
            .read()
            .ok()
            .is_some_and(|g| g.contains(&relay))
    }

    /// Reset per-relay bootstrap/reserve state when the last HOP TCP link drops.
    fn clear_bootstrap_relay_session(&self, relay: PeerId) {
        if let Ok(mut g) = self.bootstrap_identified.write() {
            g.remove(&relay);
        }
        if let Ok(mut m) = self.bootstrap_tcp_since_ms.write() {
            m.remove(&relay);
        }
        clear_relay_reserve_in_flight(self, relay);
        if let Ok(mut m) = self.bootstrap_reserve_after_ms.write() {
            m.remove(&relay);
        }
        if let Ok(mut g) = self.relay_reserve_requested.write() {
            g.remove(&relay);
        }
    }

    /// Someone who added us via QR dials in — accept without a reciprocal contact row.
    ///
    /// Returns `Some(public_key_hex)` when we can immediately learn keys from the libp2p `PeerId`.
    fn register_inbound_dialer_if_needed(
        &self,
        peer: PeerId,
        endpoint: &ConnectedPoint,
    ) -> Option<String> {
        if self.is_kept_peer(peer) {
            return None;
        }
        if !matches!(endpoint, ConnectedPoint::Listener { .. }) {
            return None;
        }
        if self.ensure_dm_peer_from_libp2p(peer) {
            native_log::info(
                "session",
                format!(
                    "accepted inbound dialer {peer} (libp2p identity → DM keys; stream protocol only)"
                ),
            );
            return self.dm_peer_for_libp2p(peer).and_then(|d| d.public_key_hex);
        }
        None
    }

    /// Registered DM contact (invite or inbound dial) — not an incidental relay peer.
    fn is_dm_contact(&self, peer: PeerId) -> bool {
        self.should_dial_libp2p_peer(peer)
    }

    fn should_routed_dial(&self, peer: PeerId, now_ms: i64, min_interval_ms: i64) -> bool {
        let Ok(g) = self.routed_dial_attempt_ms.read() else {
            return true;
        };
        let last = g.get(&peer).copied().unwrap_or(0);
        now_ms.saturating_sub(last) >= min_interval_ms
    }

    fn note_routed_dial_attempt(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut g) = self.routed_dial_attempt_ms.write() {
            g.insert(peer, now_ms);
        }
    }

    fn try_mdns_lan_failover_dial(
        &self,
        swarm: &mut Swarm<ChatBehaviour>,
        peer: PeerId,
        failed: Option<&Multiaddr>,
    ) -> bool {
        self.remove_mdns_lan_candidate(peer, failed);
        self.clear_lan_dial_in_flight(peer);
        self.clear_routed_dial_throttle(peer);
        let Some(next) = self.peer_mdns_lan_addr(peer) else {
            if self.network_profile_snapshot().has_active_lan() {
                let now_ms = chrono_now_ms();
                let parallel_wan = dm_connect_is_urgent(self, peer, now_ms);
                native_log::info(
                    "mdns",
                    format!(
                        "LAN dial failed for {peer} — waiting for fresh mDNS{}",
                        if parallel_wan {
                            " (WAN coord lookup in parallel)"
                        } else {
                            " (coord deferred — idle peer)"
                        }
                    ),
                );
                notify_dm_presence_wake();
                if parallel_wan {
                    notify_coord_lookup();
                }
                return false;
            }
            self.mark_lan_candidates_exhausted(peer);
            native_log::info(
                "mdns",
                format!("LAN candidates exhausted for {peer} — coord relay next"),
            );
            notify_coord_lookup();
            return false;
        };
        if failed.is_some_and(|f| f == &next) {
            return false;
        }
        dial_mdns_lan_addr(swarm, self, peer, next)
    }

    fn should_circuit_coord_dial(&self, peer: PeerId, now_ms: i64, min_interval_ms: i64) -> bool {
        let Ok(g) = self.circuit_coord_dial_last_ms.read() else {
            return true;
        };
        let last = g.get(&peer).copied().unwrap_or(0);
        now_ms.saturating_sub(last) >= min_interval_ms
    }

    fn note_circuit_coord_dial_attempt(&self, peer: PeerId, now_ms: i64) {
        if let Ok(mut g) = self.circuit_coord_dial_last_ms.write() {
            g.insert(peer, now_ms);
        }
    }

    fn log_stream_open_once(&self, peer: PeerId) -> bool {
        let Ok(mut g) = self.stream_open_log_emitted.write() else {
            return true;
        };
        g.insert(peer)
    }

    /// Returns true when new dialable addresses were added.
    fn merge_published_listen(&self, addrs: Vec<Multiaddr>) -> bool {
        let Ok(mut v) = self.published_listen.write() else {
            return false;
        };
        v.retain(|ma| crate::p2p::network_transport::is_dm_listen_tcp_multiaddr(ma));
        let before = v.len();
        for ma in addrs {
            if !crate::p2p::network_transport::is_dm_listen_tcp_multiaddr(&ma) {
                continue;
            }
            if !v.iter().any(|x| x == &ma) {
                v.push(ma);
            }
        }
        v.len() > before
    }

    fn published_listen_snapshot(&self) -> Vec<Multiaddr> {
        self.published_listen
            .read()
            .ok()
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    fn try_emit_peer_identified(
        &self,
        peer: PeerId,
        public_key_hex: String,
        events_tx: &Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    ) {
        let first = self
            .identified_emitted
            .write()
            .ok()
            .is_some_and(|mut g| g.insert(peer));
        if !first {
            return;
        }
        if let Some(tx) = events_tx {
            let _ = tx.send(GossipChatEvent::PeerIdentified {
                peer_id: peer,
                public_key_hex,
            });
        }
    }

    fn track_outbound(&self, pending: PendingOutbound) {
        let mut entry = pending;
        // Eligible for resync on the next upkeep tick (do not wait a full resend interval).
        entry.last_send_ms = chrono_now_ms().saturating_sub(OUTBOX_RESEND_INTERVAL_MS);
        if let Ok(mut g) = self.outbox.write() {
            g.insert(entry.message_id.clone(), entry);
        }
    }

    fn complete_outbound(&self, message_id: &str) {
        let id = message_id.trim();
        if id.is_empty() {
            return;
        }
        if let Ok(mut g) = self.outbox.write() {
            g.remove(id);
        }
    }

    /// Outbox cleared on wire ack; hold transcript→outbox re-merge until poll patches delivery
    /// (GHAL_BOL_DM_MSG_V1.md — poll owns outbound tick patches; no dual wire transcript writer).
    fn finalize_outbound_ack(&self, message_id: &str) {
        let id = message_id.trim();
        if id.is_empty() {
            return;
        }
        self.complete_outbound(id);
        if let Ok(mut g) = self.outbound_ack_pending_poll.write() {
            g.insert(id.to_string());
            if g.len() > SEEN_INBOUND_MAX {
                let trim = SEEN_INBOUND_MAX / 8;
                let mut keys: Vec<String> = g.iter().cloned().collect();
                keys.sort();
                for k in keys.into_iter().take(trim) {
                    g.remove(&k);
                }
            }
        }
    }

    fn outbound_ack_blocks_transcript_merge(&self, message_id: &str) -> bool {
        let id = message_id.trim();
        if id.is_empty() {
            return false;
        }
        self.outbound_ack_pending_poll
            .read()
            .ok()
            .is_some_and(|g| g.contains(id))
    }

    /// Drop poll-merge holds once the transcript no longer lists a message as pending.
    fn purge_outbound_ack_pending_poll(&self, still_pending_ids: &HashSet<String>) {
        let Ok(mut g) = self.outbound_ack_pending_poll.write() else {
            return;
        };
        g.retain(|id| still_pending_ids.contains(id));
    }

    /// Transcript still marks delivery pending — allow re-merge into the outbox on reconnect
    /// (`DESIGN.md`: `:p2p` owns resend; poll-merge hold is only until poll patches delivered).
    fn release_outbox_merge_blocks_for_transcript_pending(
        &self,
        still_pending_ids: &HashSet<String>,
    ) {
        let Ok(mut g) = self.outbound_ack_pending_poll.write() else {
            return;
        };
        for id in still_pending_ids {
            g.remove(id);
        }
    }

    fn outbox_due_for_resend(&self, now_ms: i64) -> Vec<PendingOutbound> {
        let Ok(g) = self.outbox.read() else {
            return Vec::new();
        };
        let mut due: Vec<PendingOutbound> = g
            .values()
            .filter(|p| now_ms.saturating_sub(p.last_send_ms) >= OUTBOX_RESEND_INTERVAL_MS)
            .cloned()
            .collect();
        due.sort_by_key(|p| p.last_send_ms);
        due
    }

    /// Returns true the first time this message is marked on-wire.
    fn mark_outbox_sent(&self, message_id: &str, now_ms: i64) -> bool {
        let Ok(mut g) = self.outbox.write() else {
            return false;
        };
        if let Some(p) = g.get_mut(message_id) {
            let first_wire = !p.on_wire;
            p.on_wire = true;
            if first_wire {
                p.first_on_wire_ms = now_ms;
            }
            p.last_send_ms = now_ms;
            return first_wire;
        }
        false
    }

    fn mark_outbox_send_failed(&self, message_id: &str, now_ms: i64) {
        let Ok(mut g) = self.outbox.write() else {
            return;
        };
        if let Some(p) = g.get_mut(message_id) {
            p.on_wire = false;
            p.first_on_wire_ms = 0;
            p.last_send_ms = now_ms;
        }
    }

    /// Full peer disconnect: pending rows must be eligible for instant burst on the next stream-open
    /// (DESIGN.md — delivery does not wait for hub room open).
    fn reset_outbox_wire_state_for_peer(&self, peer: PeerId) {
        let Some(pk) = self
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
            .filter(|pk| crate::contacts_v1::is_valid_public_key_hex(pk))
            .or_else(|| identity_wire_from_peer_id(&peer))
        else {
            return;
        };
        let Ok(mut g) = self.outbox.write() else {
            return;
        };
        let now = chrono_now_ms();
        let eligible = now.saturating_sub(OUTBOX_RESEND_INTERVAL_MS);
        for p in g.values_mut() {
            if !p.recipient_public_key_hex.eq_ignore_ascii_case(&pk) {
                continue;
            }
            p.on_wire = false;
            p.first_on_wire_ms = 0;
            p.last_send_ms = eligible;
        }
    }

    fn release_outbox_merge_blocks_for_peer_pending_transcript(&self, peer: PeerId) {
        let (Some(path), Some(ns)) = (&self.transcript_path, &self.app_namespace) else {
            return;
        };
        let Some(pk) = self.signing_pk_for_libp2p_peer(peer) else {
            return;
        };
        let Ok(rows) =
            crate::dm_transcript_v1::pending_outbound_rows(Path::new(path), ns.trim())
        else {
            return;
        };
        let peer_s = peer.to_string();
        let pending: HashSet<String> = rows
            .into_iter()
            .filter(|r| {
                let ck = r.conversation_key.as_str();
                ck == peer_s || ck == pk
            })
            .map(|r| r.message_id)
            .collect();
        self.release_outbox_merge_blocks_for_transcript_pending(&pending);
    }

    fn outbox_contains(&self, message_id: &str) -> bool {
        let id = message_id.trim();
        self.outbox.read().ok().is_some_and(|g| g.contains_key(id))
    }

    fn has_pending_outbox_for_pk(&self, pk_hex: &str) -> bool {
        let pk = match crate::public_key_util::normalize_contact_identity_wire(pk_hex.trim()) {
            Ok(w) => w,
            Err(_) => return false,
        };
        self.outbox.read().ok().is_some_and(|g| {
            g.values().any(|p| {
                crate::public_key_util::same_contact_pk(p.recipient_public_key_hex.trim(), &pk)
            })
        })
    }

    /// Should a pending-outbox contact be looked up in the **uncapped priority** coord tier?
    ///
    /// Active intent (a freshly-sent message, foreground chat) always qualifies — the send path
    /// arms the urgent window (`mark_dm_reconnect_urgent`), so "intent beats backoff" (TRANSPORT.md
    /// § prime directive #4) still holds. But a contact that coord reports `PeerNotOnCoord` (offline
    /// / never registered) whose only claim is an **old transcript-restored** pending row — with no
    /// recent user action — must **not** sit in the uncapped priority tier every tick: at thousands
    /// of stale contacts that is the 404 storm that starves reachable peers. Such ghosts fall
    /// through to the bounded LRU **background** sweep instead (TRANSPORT.md § scale invariant).
    fn pending_outbox_eligible_for_wire(&self, pk_hex: &str, now_ms: i64) -> bool {
        let pk = pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return false;
        }
        if !self.has_pending_outbox_for_pk(pk) {
            return false;
        }
        if self.coord_lookup_category_for_pk(pk)
            == Some(crate::p2p::connectivity_diag::CoordLookupCategory::PeerNotOnCoord)
        {
            return self.is_pk_reconnect_urgent(pk, now_ms);
        }
        true
    }

    fn remember_inbound_id(&self, message_id: &str, now_ms: i64) -> bool {
        let id = message_id.trim();
        if id.is_empty() {
            return true;
        }
        let Ok(mut g) = self.seen_inbound_ids.write() else {
            return true;
        };
        if g.contains_key(id) {
            return false;
        }
        if g.len() >= SEEN_INBOUND_MAX {
            let trim = SEEN_INBOUND_MAX / 8;
            let mut oldest: Vec<(String, i64)> = g.iter().map(|(k, v)| (k.clone(), *v)).collect();
            oldest.sort_by_key(|(_, ts)| *ts);
            for (k, _) in oldest.into_iter().take(trim) {
                g.remove(&k);
            }
        }
        g.insert(id.to_string(), now_ms);
        true
    }

    /// First local accept time for an inbound text id (never updated on duplicate resends).
    pub(crate) fn inbound_received_at_ms(&self, message_id: &str) -> Option<i64> {
        let id = message_id.trim();
        if id.is_empty() {
            return None;
        }
        self.seen_inbound_ids
            .read()
            .ok()
            .and_then(|g| g.get(id).copied())
    }

    fn note_connected(&self, peer: PeerId) {
        if let Ok(mut g) = self.connected.write() {
            g.insert(peer);
        }
        self.clear_stream_open_backoff(peer);
    }

    fn note_disconnected(&self, peer: &PeerId) {
        if let Ok(mut g) = self.connected.write() {
            g.remove(peer);
        }
        self.reset_outbox_wire_state_for_peer(*peer);
        self.release_outbox_merge_blocks_for_peer_pending_transcript(*peer);
        self.clear_chat_ready_emitted(*peer);
        if let Ok(mut g) = self.history_replay_done.write() {
            g.remove(peer);
        }
        self.set_dm_stream_writer(*peer, false);
        if let Ok(mut m) = self.dm_no_writer_since_ms.write() {
            m.remove(peer);
        }
        if let Ok(mut m) = self.dm_wire_activity_ms.write() {
            m.remove(peer);
        }
    }

    fn clear_chat_ready_emitted(&self, peer: PeerId) {
        if let Ok(mut g) = self.chat_ready_emitted.write() {
            g.remove(&peer);
        }
    }

    /// After network handover, drop 404 backoff so the next tick does a live coord lookup.
    fn clear_coord_lookup_backoff_all(&self) {
        if let Ok(mut m) = self.coord_lookup_backoff.write() {
            m.clear();
        }
    }

    fn connected_peers(&self) -> Vec<PeerId> {
        self.connected
            .read()
            .ok()
            .map(|g| g.iter().copied().collect())
            .unwrap_or_default()
    }

    fn libp2p_peer_connected(&self, peer: PeerId) -> bool {
        self.connected
            .read()
            .ok()
            .is_some_and(|g| g.contains(&peer))
    }

    /// libp2p PeerIds for configured DM contacts (for DM dial/upkeep).
    fn dm_peer_ids(&self) -> Vec<PeerId> {
        self.peers
            .read()
            .ok()
            .map(|t| t.by_peer_id.keys().copied().collect())
            .unwrap_or_default()
    }

    fn dm_public_keys(&self) -> Vec<String> {
        self.peers
            .read()
            .ok()
            .map(|t| {
                t.by_peer_id
                    .values()
                    .filter_map(|d| d.public_key_hex.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Only dial mDNS/coord peers we already know from the invite (never random LAN nodes).
    /// Requires a 66-char secp256k1 `public_key_hex` — never bare `peer_id_only` captures.
    fn should_dial_libp2p_peer(&self, peer: PeerId) -> bool {
        let Ok(tables) = self.peers.read() else {
            return false;
        };
        tables
            .by_peer_id
            .get(&peer)
            .is_some_and(|dm| dm.has_send_keys())
    }

    /// Target PeerId to open `/ghal-bol/msg/1.0.0` for this contact.
    pub(crate) fn libp2p_peer_for_identity_wire(&self, signing_pk_hex: &str) -> Option<PeerId> {
        self.resolve_send_peer(signing_pk_hex)
    }

    /// Target PeerId to open `/ghal-bol/msg/1.0.0` for this contact.
    ///
    /// Derived from the validated identity wire when embeddable; otherwise uses the registered
    /// transport PeerId hint. A stale `peer_id` beside the key must never override crypto identity.
    fn resolve_send_peer(&self, signing_pk_hex: &str) -> Option<PeerId> {
        let pk = signing_pk_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return None;
        }
        if let Ok(peer_id) = peer_id_from_identity_wire(pk) {
            self.ensure_dm_peer(pk, peer_id);
            return Some(peer_id);
        }
        self.dm_peer(pk).map(|d| d.peer_id)
    }

    /// Fill identity wire from libp2p PeerId when PeerId embeds the transport key.
    fn ensure_dm_peer_from_libp2p(&self, peer: PeerId) -> bool {
        if self
            .dm_peer_for_libp2p(peer)
            .is_some_and(|d| d.has_send_keys())
        {
            return true;
        }
        let Some(pk) = identity_wire_from_peer_id(&peer) else {
            return false;
        };
        self.ensure_dm_peer(&pk, peer);
        true
    }

    fn register_dm_peer_key(&self, peer_id_hint: Option<PeerId>, public_key_hex: &str) {
        let pk = public_key_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            if let Some(pid) = peer_id_hint {
                self.ensure_dm_peer_from_libp2p(pid);
            }
            return;
        }
        let derived_peer = peer_id_from_identity_wire(pk).ok();
        let peer_id = match (derived_peer, peer_id_hint) {
            (Some(d), Some(h)) if d != h => {
                native_log::warn(
                    "session",
                    format!("dm peer id corrected {h} -> {d} (identity wire is authoritative)"),
                );
                d
            }
            (Some(d), _) => d,
            (None, Some(h)) => h,
            (None, None) => return,
        };
        self.ensure_dm_peer(pk, peer_id);
        self.purge_invalid_dm_peer_ids();
        refresh_outbox_peer_ids(self);
    }

    fn ensure_dm_peer(&self, public_key_hex: &str, libp2p_peer: PeerId) {
        let pk = public_key_hex.trim();
        if !crate::contacts_v1::is_valid_public_key_hex(pk) {
            return;
        }
        if let Ok(derived) = peer_id_from_identity_wire(pk) {
            if derived != libp2p_peer {
                native_log::warn(
                    "session",
                    format!("reject dm keys for {libp2p_peer}: identity wire does not match peer id"),
                );
                return;
            }
        }
        let mut tables = match self.peers.write() {
            Ok(g) => g,
            Err(_) => return,
        };
        let stale: Vec<PeerId> = tables
            .by_peer_id
            .iter()
            .filter(|(pid, dm)| **pid != libp2p_peer && dm.public_key_hex.as_deref() == Some(pk))
            .map(|(pid, _)| *pid)
            .collect();
        for pid in stale {
            tables.by_peer_id.remove(&pid);
        }
        tables.retain_invalid_dm_peer_ids();
        let is_new = !tables.by_peer_id.contains_key(&libp2p_peer);
        let entry = tables
            .by_peer_id
            .entry(libp2p_peer)
            .or_insert_with(|| DmPeer::peer_id_only(libp2p_peer));
        entry.public_key_hex = Some(pk.to_string());
        if is_new {
            if let Ok(mut m) = self.dm_peer_registered_ms.write() {
                m.insert(libp2p_peer, chrono_now_ms());
            }
        }
    }

    /// Drop `peer_id_only` rows and non-secp256k1 libp2p ids left from old inbound captures.
    fn purge_invalid_dm_peer_ids(&self) {
        let Ok(mut tables) = self.peers.write() else {
            return;
        };
        tables.retain_invalid_dm_peer_ids();
    }

    fn dm_peer(&self, signing_pk_hex: &str) -> Option<DmPeer> {
        let pk = signing_pk_hex.trim();
        let tables = self.peers.read().ok()?;
        tables.by_peer_id.values().find_map(|dm| {
            if dm.public_key_hex.as_deref() == Some(pk) {
                Some(dm.clone())
            } else {
                None
            }
        })
    }

    fn dm_peer_for_libp2p(&self, peer: PeerId) -> Option<DmPeer> {
        self.peers.read().ok()?.by_peer_id.get(&peer).cloned()
    }

    fn dm_peer_for_conversation_key(&self, key: &str) -> Option<DmPeer> {
        let key = key.trim();
        if crate::contacts_v1::is_valid_public_key_hex(key) {
            return self.dm_peer(key);
        }
        if let Ok(pid) = key.parse::<PeerId>() {
            return self.dm_peer_for_libp2p(pid);
        }
        None
    }

    fn set_foreground_peer(&self, peer: Option<PeerId>) {
        if let Ok(mut g) = self.foreground_peer.write() {
            *g = peer;
        }
        let pk = peer.and_then(|p| self.signing_pk_for_libp2p_peer(p));
        sync_foreground_peer_now(pk);
    }

    fn current_foreground_peer(&self) -> Option<PeerId> {
        self.foreground_peer.read().ok().and_then(|g| *g)
    }

    fn is_foreground_peer(&self, peer: PeerId) -> bool {
        self.current_foreground_peer().is_some_and(|f| f == peer)
    }

    fn pending_read_ack_len(&self) -> usize {
        self.pending_read_acks.read().map(|q| q.len()).unwrap_or(0)
    }

    fn has_pending_read_acks_for(&self, peer: PeerId) -> bool {
        self.pending_read_acks
            .read()
            .ok()
            .is_some_and(|q| q.iter().any(|p| p.peer_id == peer))
    }

    fn has_pending_delivery_acks_for(&self, peer: PeerId) -> bool {
        self.pending_delivery_acks
            .read()
            .ok()
            .is_some_and(|q| q.iter().any(|p| p.peer_id == peer))
    }

    /// A delivery ack queued for `peer` at least `min_ms` ago and still unsent — sustained
    /// evidence the writer is not draining, unlike a frame that is merely in flight right now.
    fn has_pending_delivery_acks_older_than(&self, peer: PeerId, now_ms: i64, min_ms: i64) -> bool {
        self.pending_delivery_acks.read().ok().is_some_and(|q| {
            q.iter()
                .any(|p| p.peer_id == peer && now_ms.saturating_sub(p.queued_at_ms) >= min_ms)
        })
    }

    /// Outbox or delivery acks stuck — WAN mux/coord must not treat the link as stable.
    fn peer_has_pending_outbound_blockers(&self, peer: PeerId) -> bool {
        self.peer_has_pending_outbox(peer) || self.has_pending_delivery_acks_for(peer)
    }

    /// Any wire work — urgent reconnect / intent gating (includes read ack backlog).
    fn peer_has_pending_wire_work(&self, peer: PeerId) -> bool {
        self.peer_has_pending_outbound_blockers(peer) || self.has_pending_read_acks_for(peer)
    }

    /// Room enter / leave seed from transcript: disk `read_ack_sent: false` is authoritative.
    fn enqueue_read_ack_backlog(
        &self,
        peer_id: PeerId,
        inbound_id: &str,
        recipient_signing: &str,
    ) -> bool {
        let id = inbound_id.trim().to_string();
        if id.is_empty() {
            return false;
        }
        if let Ok(mut s) = self.read_ack_confirmed.write() {
            s.remove(&id);
        }
        let Ok(mut q) = self.pending_read_acks.write() else {
            return false;
        };
        if q.iter().any(|p| p.inbound_id == id) {
            return false;
        }
        if q.len() >= MAX_PENDING_READ_ACKS {
            q.pop_front();
        }
        q.push_back(PendingReadAck {
            peer_id,
            inbound_id: id,
            recipient_public_key_hex: recipient_signing.trim().to_string(),
            last_send_ms: 0,
        });
        true
    }

    fn pending_delivery_ack_len(&self) -> usize {
        self.pending_delivery_acks
            .read()
            .map(|q| q.len())
            .unwrap_or(0)
    }

    fn mark_read_ack_wire_sent(&self, inbound_id: &str) {
        let id = inbound_id.trim();
        if id.is_empty() {
            return;
        }
        let now = chrono_now_ms();
        if let Ok(mut q) = self.pending_read_acks.write() {
            for item in q.iter_mut() {
                if item.inbound_id == id {
                    item.last_send_ms = now;
                    break;
                }
            }
        }
    }

    /// Near-single-shot wire send: one immediate `ack_read`, then upkeep retries after interval.
    fn try_claim_read_ack_wire_send(
        &self,
        peer_id: PeerId,
        inbound_id: &str,
        recipient_signing: &str,
    ) -> bool {
        let id = inbound_id.trim().to_string();
        if id.is_empty() || self.is_read_ack_confirmed(&id) {
            return false;
        }
        let now = chrono_now_ms();
        let Ok(mut q) = self.pending_read_acks.write() else {
            return false;
        };
        if let Some(item) = q.iter_mut().find(|p| p.inbound_id == id) {
            if item.last_send_ms > 0
                && now.saturating_sub(item.last_send_ms) < OUTBOX_RESEND_INTERVAL_MS
            {
                return false;
            }
            item.last_send_ms = now;
            return true;
        }
        if q.len() >= MAX_PENDING_READ_ACKS {
            q.pop_front();
        }
        q.push_back(PendingReadAck {
            peer_id,
            inbound_id: id,
            recipient_public_key_hex: recipient_signing.trim().to_string(),
            last_send_ms: now,
        });
        true
    }

    fn release_read_ack_wire_claim(&self, inbound_id: &str) {
        let id = inbound_id.trim();
        if id.is_empty() {
            return;
        }
        if let Ok(mut q) = self.pending_read_acks.write() {
            for item in q.iter_mut() {
                if item.inbound_id == id {
                    item.last_send_ms = 0;
                    break;
                }
            }
        }
    }

    fn is_read_ack_confirmed(&self, inbound_id: &str) -> bool {
        let id = inbound_id.trim();
        if id.is_empty() {
            return false;
        }
        self.read_ack_confirmed
            .read()
            .ok()
            .is_some_and(|s| s.contains(id))
    }

    fn has_pending_read_ack(&self, inbound_id: &str) -> bool {
        let id = inbound_id.trim();
        if id.is_empty() || self.is_read_ack_confirmed(id) {
            return false;
        }
        self.pending_read_acks
            .read()
            .ok()
            .is_some_and(|q| q.iter().any(|p| p.inbound_id == id))
    }

    fn mark_read_ack_confirmed(&self, inbound_id: &str) {
        let id = inbound_id.trim();
        if id.is_empty() || self.is_read_ack_confirmed(id) {
            return;
        }
        if let Ok(mut s) = self.read_ack_confirmed.write() {
            s.insert(id.to_string());
            if s.len() > SEEN_INBOUND_MAX {
                let trim = SEEN_INBOUND_MAX / 8;
                let mut keys: Vec<String> = s.iter().cloned().collect();
                keys.sort();
                for k in keys.into_iter().take(trim) {
                    s.remove(&k);
                }
            }
        }
        if let Ok(mut q) = self.pending_read_acks.write() {
            q.retain(|p| p.inbound_id != id);
        }
        if let (Some(path), Some(ns)) = (&self.transcript_path, &self.app_namespace) {
            let path = path.trim();
            let ns = ns.trim();
            if !path.is_empty() && !ns.is_empty() {
                let _ = crate::dm_transcript_store::patch_inbound_read_ack_sent_at_path(
                    Path::new(path),
                    ns,
                    id,
                );
            }
        }
    }

    /// Queued read receipts (from in-room or post-enter backlog) — retried until sender confirms.
    fn read_acks_due_for_upkeep(&self, limit: usize) -> Vec<PendingReadAck> {
        let Ok(q) = self.pending_read_acks.read() else {
            return Vec::new();
        };
        let confirmed = self.read_ack_confirmed.read().ok();
        let now = chrono_now_ms();
        let mut due: Vec<PendingReadAck> = q
            .iter()
            .filter(|item| {
                if confirmed
                    .as_ref()
                    .is_some_and(|s| s.contains(&item.inbound_id))
                {
                    return false;
                }
                item.last_send_ms == 0
                    || now.saturating_sub(item.last_send_ms) >= OUTBOX_RESEND_INTERVAL_MS
            })
            .cloned()
            .collect();
        due.sort_by_key(|p| p.last_send_ms);
        due.truncate(limit);
        due
    }

    fn enqueue_delivery_ack(
        &self,
        peer_id: PeerId,
        inbound_id: &str,
        recipient_signing: &str,
        received_at_ms: i64,
    ) {
        let id = inbound_id.trim().to_string();
        if id.is_empty() {
            return;
        }
        let Ok(mut q) = self.pending_delivery_acks.write() else {
            return;
        };
        if q.len() >= MAX_PENDING_READ_ACKS {
            q.pop_front();
        }
        if q.iter().any(|p| p.inbound_id == id) {
            return;
        }
        q.push_back(PendingDeliveryAck {
            peer_id,
            inbound_id: id,
            recipient_public_key_hex: recipient_signing.trim().to_string(),
            received_at_ms,
            queued_at_ms: chrono_now_ms(),
        });
    }

    fn dequeue_delivery_ack(&self, inbound_id: &str) {
        let id = inbound_id.trim();
        if id.is_empty() {
            return;
        }
        if let Ok(mut q) = self.pending_delivery_acks.write() {
            q.retain(|p| p.inbound_id != id);
        }
    }

    fn is_delivery_ack_sent(&self, inbound_id: &str) -> bool {
        let id = inbound_id.trim();
        if id.is_empty() {
            return false;
        }
        self.delivery_ack_sent
            .read()
            .ok()
            .is_some_and(|s| s.contains(id))
    }

    fn clear_delivery_ack_sent(&self, inbound_id: &str) {
        let id = inbound_id.trim();
        if id.is_empty() {
            return;
        }
        if let Ok(mut s) = self.delivery_ack_sent.write() {
            s.remove(id);
        }
    }

    fn mark_delivery_ack_sent(&self, inbound_id: &str) {
        let id = inbound_id.trim().to_string();
        if id.is_empty() {
            return;
        }
        let Ok(mut s) = self.delivery_ack_sent.write() else {
            return;
        };
        if s.len() >= SEEN_INBOUND_MAX {
            let trim = SEEN_INBOUND_MAX / 8;
            let drop: Vec<String> = s.iter().take(trim).cloned().collect();
            for k in drop {
                s.remove(&k);
            }
        }
        s.insert(id);
    }

    fn delivery_acks_due_for_upkeep(&self, limit: usize) -> Vec<PendingDeliveryAck> {
        let Ok(q) = self.pending_delivery_acks.read() else {
            return Vec::new();
        };
        q.iter().take(limit).cloned().collect()
    }

    fn enqueue_pending_call_signal(&self, item: PendingCallSignal) {
        const MAX: usize = 128;
        let Ok(mut q) = self.pending_call_signals.write() else {
            return;
        };
        if q.len() >= MAX {
            q.pop_front();
        }
        q.push_back(item);
    }

    fn drain_pending_call_signals(&self, limit: usize) -> Vec<PendingCallSignal> {
        let Ok(mut q) = self.pending_call_signals.write() else {
            return Vec::new();
        };
        let n = limit.min(q.len());
        q.drain(0..n).collect()
    }

    fn requeue_pending_call_signal_front(&self, item: PendingCallSignal) {
        const MAX: usize = 128;
        let Ok(mut q) = self.pending_call_signals.write() else {
            return;
        };
        if q.len() >= MAX {
            return;
        }
        q.push_front(item);
    }

    /// Drop queued signals for a ended call (prevents ghost rings from stale invites).
    fn purge_pending_call_signals(&self, call_id: &str) {
        let cid = call_id.trim();
        if cid.is_empty() {
            return;
        }
        let Ok(mut q) = self.pending_call_signals.write() else {
            return;
        };
        q.retain(|c| c.call_id != cid);
    }
}
