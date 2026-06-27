/// Blocking-IO friendly run used by **`p2p_ffi`**: poll outbound commands and emit events on std channels.
pub async fn run_gossip_chat_node_with_std_io(
    config: GossipChatConfig,
    identity: crate::DecryptedIdentity,
    outbound_rx: std::sync::mpsc::Receiver<OutboundCmd>,
    events_tx: std::sync::mpsc::Sender<GossipChatEvent>,
    stop: Arc<AtomicBool>,
) -> Result<(), ChatServerError> {
    let bootstrap = config.bootstrap_peers.clone();
    native_log::info("p2p", "building swarm");
    let mut swarm = build_swarm(&config)?;
    crate::coord_runtime::coord_set_local_peer_id(*swarm.local_peer_id());
    native_log::info("p2p", "swarm built, opening chat stream accept");
    let mut control = swarm.behaviour().stream.new_control();
    let protocol = StreamProtocol::new(STREAM_PROTOCOL);
    let mut incoming = control
        .accept(protocol.clone())
        .map_err(|e| ChatServerError::Transport(format!("accept stream: {e}")))?;
    let mut call_incoming = control
        .accept(StreamProtocol::new(CALL_STREAM_PROTOCOL))
        .map_err(|e| ChatServerError::Transport(format!("accept call stream: {e}")))?;
    let mut call_video_incoming = control
        .accept(StreamProtocol::new(CALL_VIDEO_STREAM_PROTOCOL))
        .map_err(|e| ChatServerError::Transport(format!("accept call video stream: {e}")))?;
    let writers: StreamWriters = Arc::new(Mutex::new(HashMap::new()));

    let coord_only = crate::coord_runtime::wan_discovery_via_coord_only();
    let mut coord_relays: Vec<(PeerId, Multiaddr)> = Vec::new();
    let mut ghalbol_relay_initial: Option<(PeerId, Vec<String>)> = None;
    if let Some(tp) = config.transcript_path.as_deref() {
        if let Some(data_dir) = Path::new(tp).parent() {
            crate::coord_runtime::purge_legacy_relay_cache_files(data_dir);
        }
    }
    if coord_only {
        // Fetch co-located relay(s) from every configured coord server.
        let all_relays = tokio::task::spawn_blocking(|| {
            crate::coord_runtime::fetch_all_ghalbol_relays(false)
        })
        .await
        .ok()
        .unwrap_or_default();
        for (relay_peer, relay_addrs) in &all_relays {
            let relay_nodes =
                crate::p2p::network_transport::resolve_relay_bootnodes(relay_peer, relay_addrs);
            if relay_nodes.is_empty() {
                native_log::warn(
                    "relay",
                    format!(
                        "ghalbol relay {relay_peer} advertised but no dialable public addr yet — will retry via coord_tick"
                    ),
                );
            } else {
                native_log::info(
                    "relay",
                    format!(
                        "ghalbol relay {relay_peer}: {} dial addr(s) — preferred for reservation",
                        relay_nodes.len()
                    ),
                );
                merge_relay_nodes_into_coord_relays(&mut coord_relays, &relay_nodes);
            }
            if let Ok(relay_pid) = relay_peer.parse::<PeerId>() {
                if ghalbol_relay_initial.is_none() {
                    ghalbol_relay_initial = Some((relay_pid, relay_addrs.clone()));
                }
            }
        }
        native_log::info(
            "coord",
            format!(
                "coord URL set — peer discovery via server; dialing {} coord relay node(s) for circuit reservation",
                coord_relays.len()
            ),
        );
        if coord_relays.is_empty() {
            native_log::warn(
                "relay",
                "no coord relay dial addr at startup — will refetch /v1/relay every few seconds; \
                 LAN works via mDNS; WAN needs coord GET /v1/relay with a dialable relay addr",
            );
            notify_relay_refresh();
        }
    }
    crate::p2p::network_transport::refresh_os_network_truth();
    let net = crate::p2p::network_transport::detect_local_network_profile();
    let mut bootstrap_peer_ids: HashSet<PeerId> = coord_relays.iter().map(|(p, _)| *p).collect();
    if let Some((relay_pid, _)) = &ghalbol_relay_initial {
        bootstrap_peer_ids.insert(*relay_pid);
    }
    let session = Arc::new(SessionState::new(
        identity,
        &config.dm_peers,
        bootstrap_peer_ids,
        config.transcript_path.clone(),
        config.app_namespace.clone(),
        net.clone(),
        ghalbol_relay_initial,
    )?);
    native_log::info(
        "p2p",
        format!(
            "swarm up: dm_peers={} invite_bootstrap={} coord_relays={} coord_only={coord_only}",
            session.dm_peer_ids().len(),
            bootstrap.len(),
            coord_relays.len()
        ),
    );
    native_log::info(
        "net",
        format!(
            "profile={} hint={} wifi={} cellular={} tether={} usb={} lan={} cgnat={} public4={} global6={}",
            net.mode_label(),
            net.dial_hint(),
            net.has_wifi_iface,
            net.has_cellular_iface,
            net.has_tether_iface,
            net.has_usb_iface,
            net.has_rfc1918_ipv4,
            net.has_cgnat_ipv4,
            net.has_public_ipv4,
            net.has_global_ipv6
        ),
    );
    listen_swarm_transports(&mut swarm, session.as_ref())?;
    // Start relay TCP + CGNAT probe before the brief listen wait so cellular bootstrap is not idle.
    if coord_only && !coord_relays.is_empty() {
        dial_coord_relays(&mut swarm, &session, &coord_relays);
        session.begin_wan_recovery();
    }
    // Wait for bootstrap TCP + one HOP-anchored `listen_on` before node_ready (coord WAN).
    let listen_wait = if coord_only {
        Duration::from_secs(BOOTSTRAP_LISTEN_MAX_SECS)
    } else {
        Duration::from_millis(800)
    };
    bootstrap_publishable_listen(&mut swarm, &session, listen_wait).await;
    crate::coord_runtime::rebuild_coord_endpoints_from_listen(&session.published_listen_snapshot());
    if coord_only && !coord_relays.is_empty() {
        dial_coord_relays(&mut swarm, &session, &coord_relays);
    }
    ensure_wan_relay_circuit(&mut swarm, session.as_ref(), Some(&coord_relays), false);
    if coord_only && !relay_circuit_listening(&swarm) {
        if !listen_ready_for_node(session.as_ref(), true, &swarm) {
            native_log::info("net", "WAN not ready at startup — begin recovery pass");
            session.begin_wan_recovery();
        }
    }
    dial_bootstrap_peers(&mut swarm, &bootstrap, &mut |ev| {
        let _ = events_tx.send(ev);
    });
    if let (Some(path), Some(ns)) = (&config.transcript_path, &config.app_namespace) {
        let path = Path::new(path);
        let ns = ns.trim();
        if !path.as_os_str().is_empty() && !ns.is_empty() {
            bootstrap_outbox_from_transcript(session.as_ref(), path, ns);
        }
    }
    // Bootstrap consumes NewListenAddr without emitting; replay snapshot so poll/tests see TCP listen.
    for addr in session.published_listen_snapshot() {
        if should_emit_listening_event(&addr) {
            let _ = events_tx.send(GossipChatEvent::Listening(addr));
        }
    }
    let _ = events_tx.send(GossipChatEvent::NodeReady);
    native_log::info("p2p", "node ready");
    let session_for_swarm = Arc::clone(&session);

    let mut poll_tick = time::interval(Duration::from_millis(25));
    // If bootstrap peers drop (idle timeout / network quirks), relay reservations and coord
    // registration can silently stall until we re-dial. This must be frequent enough to keep
    // background connectivity from "taking minutes" to recover.
    let mut redial_tick = time::interval(Duration::from_secs(BOOTSTRAP_REDIAL_INTERVAL_SECS));
    redial_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut coord_tick = time::interval(Duration::from_secs(COORD_LOOKUP_INTERVAL_SECS));
    coord_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut dm_upkeep_tick = time::interval(Duration::from_secs(1));
    dm_upkeep_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stream_upkeep_tick = time::interval(Duration::from_millis(500));
    stream_upkeep_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut flow_snapshot_tick = time::interval(Duration::from_secs(30));
    flow_snapshot_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut network_tick = time::interval(Duration::from_secs(NETWORK_PROFILE_POLL_SECS));
    network_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // First snapshot soon after node_ready so exports show initial listen/coord state.
    flow_snapshot_tick.tick().await;
    network_tick.tick().await;

    loop {
        if stop.load(Ordering::SeqCst) {
            session.call_media_stop_all();
            session.call_video_stop_all();
            crate::p2p::call_active::clear();
            break Ok(());
        }
        drain_outbound_queue(
            &outbound_rx,
            &mut swarm,
            Arc::clone(&session),
            Arc::clone(&writers),
            control.clone(),
            Some(events_tx.clone()),
            MAX_OUTBOUND_CMDS_PER_TICK,
        )
        .await;
        select! {
            biased;
            _ = stream_upkeep_tick.tick() => {
                spawn_reopen_dm_chat_streams(
                    &swarm,
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    control.clone(),
                    Some(events_tx.clone()),
                );
                if session.pending_read_ack_len() > 0 {
                    let connected_now: Vec<PeerId> = session
                        .connected_peers()
                        .into_iter()
                        .filter(|p| session.is_dm_contact(*p) && swarm.is_connected(p))
                        .collect();
                    if !connected_now.is_empty() {
                        let session3 = Arc::clone(&session);
                        let writers3 = Arc::clone(&writers);
                        tokio::spawn(async move {
                            run_ack_upkeep(session3, writers3, &connected_now).await;
                        });
                    }
                }
            }
            _ = dm_upkeep_tick.tick() => {
                let now_ms = chrono_now_ms();
                for peer in session.expire_stale_circuit_dials(now_ms) {
                    if !swarm.is_connected(&peer) {
                        let _ = swarm.disconnect_peer_id(peer);
                        native_log::info(
                            "dial",
                            format!(
                                "reset libp2p dial state for {peer} after relay-circuit timeout"
                            ),
                        );
                    }
                }
                session.expire_stale_lan_dials(now_ms);
                reconcile_all_stale_lan_mux_for_wan(&mut swarm, session.as_ref(), &writers);
                apply_pending_dm_link_resets(&mut swarm, session.as_ref(), &writers);
                let coord_lookup_wake = take_coord_lookup_notify();
                if take_dm_presence_wake_notify() && session.should_run_presence_wake(now_ms) {
                    native_log::info(
                        "net",
                        "presence wake — immediate rediscovery for known DM peers",
                    );
                    session.wake_all_dm_peers_rediscovery(now_ms);
                }
                lan_handover_upkeep_if_needed(&mut swarm, session.as_ref());
                upkeep_dm_peers(
                    &mut swarm,
                    Arc::clone(&session),
                    control.clone(),
                    Arc::clone(&writers),
                    Some(events_tx.clone()),
                );
                // Coord register + lookup run in parallel with LAN handover (TRANSPORT.md).
                // The lookup pass is scale-safe (TRANSPORT.md § "Instant connect at any roster
                // size"): active-intent peers are looked up uncapped, idle contacts swept LRU under
                // a per-tick cap, so thousands of stale peers can never delay a reachable one.
                if crate::coord_runtime::coord_is_configured() {
                    let listen = coord_register_listen_snapshot(&swarm, session.as_ref());
                    crate::coord_runtime::coord_register_tick(&listen);
                }
                run_dm_coord_lookup_pass(&mut swarm, session.as_ref(), now_ms, coord_lookup_wake);
                let connected_now: Vec<PeerId> = session
                    .connected_peers()
                    .into_iter()
                    .filter(|p| session.is_dm_contact(*p) && swarm.is_connected(p))
                    .collect();
                if let (Some(path), Some(ns)) = (
                    &session.transcript_path,
                    &session.app_namespace,
                ) {
                    transcript_sync_outbound_tick(
                        session.as_ref(),
                        Path::new(path),
                        ns.trim(),
                    );
                }
                resync_pending_outbox(
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    connected_now.clone(),
                    Some(events_tx.clone()),
                    Some(control.clone()),
                )
                .await;
                flush_pending_call_signals(
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    connected_now.clone(),
                    Some(events_tx.clone()),
                )
                .await;
                if session.pending_read_ack_len() > 0 && !connected_now.is_empty() {
                    run_ack_upkeep(
                        Arc::clone(&session),
                        Arc::clone(&writers),
                        &connected_now,
                    )
                    .await;
                }
            }
            _ = network_tick.tick() => {
                crate::p2p::network_transport::refresh_os_network_truth();
                let recovering_before = session.wan_recovery_active.load(Ordering::Relaxed);
                let mut handover = false;
                #[cfg(target_os = "linux")]
                if crate::linux_network::poll_wifi_link_up_transition() {
                    notify_network_change();
                }
                let forced = take_network_change_notify();
                if forced {
                    let detected = detected_network_with_platform_hints();
                    let net = network_profile_for_swarm(&swarm, detected);
                    let (old_mode, new_mode, changed) = if let Ok(mut cur) = session.network_profile.write() {
                        let old_key = crate::p2p::network_transport::network_handover_key(&*cur);
                        let old_mode = cur.mode_label().to_string();
                        let lan_restored = net.has_active_lan() && !cur.has_active_lan();
                        let new_key = crate::p2p::network_transport::network_handover_key(&net);
                        *cur = net;
                        let new_mode = cur.mode_label().to_string();
                        (old_mode, new_mode, old_key != new_key || lan_restored)
                    } else {
                        continue;
                    };
                    if changed {
                        handle_network_path_change(
                            &mut swarm,
                            session.as_ref(),
                            &coord_relays,
                            &old_mode,
                            &new_mode,
                        );
                        handover = true;
                    } else {
                        let profile = session.network_profile_snapshot();
                        if profile.on_mobile_data_path()
                            || (!profile.has_active_lan() && profile.needs_relay_for_wan())
                        {
                            refresh_coord_reachability_after_network_change(
                                &mut swarm,
                                session.as_ref(),
                                &coord_relays,
                                "connectivity notify — mobile/WAN path refresh",
                                true,
                            );
                            handover = true;
                        } else if try_recover_lan_after_wifi_available(
                            &mut swarm,
                            session.as_ref(),
                            &coord_relays,
                            true,
                        ) {
                            handover = true;
                        }
                    }
                } else if let Some((old_mode, new_mode)) =
                    session.refresh_network_path_if_changed(&swarm)
                {
                    handle_network_path_change(
                        &mut swarm,
                        session.as_ref(),
                        &coord_relays,
                        &old_mode,
                        &new_mode,
                    );
                    handover = true;
                } else if try_recover_lan_after_wifi_available(
                    &mut swarm,
                    session.as_ref(),
                    &coord_relays,
                    false,
                ) {
                    handover = true;
                } else if !recovering_before {
                    try_wan_relay_recovery(&mut swarm, session.as_ref());
                }
                if crate::coord_runtime::coord_is_configured() {
                    let _ = sync_published_listen_from_swarm(session.as_ref(), &swarm);
                    let snap = coord_register_listen_snapshot(&swarm, session.as_ref());
                    let fp =
                        crate::p2p::network_transport::wan_coord_listen_fingerprint(&snap);
                    let fp_changed = session.wan_listen_fp_changed(&fp);
                    if fp_changed {
                        refresh_coord_reachability_after_network_change(
                            &mut swarm,
                            session.as_ref(),
                            &coord_relays,
                            "WAN listen addrs changed — refresh coord presence",
                            false,
                        );
                    }
                }
                if session.wan_recovery_active.load(Ordering::Relaxed) {
                    run_wan_recovery_pass(&mut swarm, session.as_ref(), &coord_relays);
                }
                let recovering_after = session.wan_recovery_active.load(Ordering::Relaxed);
                if handover || (recovering_before && !recovering_after) {
                    // Force-wake burst, still bounded (TRANSPORT.md § "Instant connect at any
                    // roster size") so a huge roster can't flood coord or block the swarm loop.
                    run_dm_coord_lookup_pass(&mut swarm, session.as_ref(), chrono_now_ms(), true);
                    spawn_reopen_dm_chat_streams(
                        &swarm,
                        Arc::clone(&session),
                        Arc::clone(&writers),
                        control.clone(),
                        Some(events_tx.clone()),
                    );
                }
            }
            _ = coord_tick.tick() => {
                run_bootstrap_relay_reserve_pass(&mut swarm, session.as_ref());
                let force_relay = take_relay_refresh_notify();
                maybe_refresh_ghalbol_relay(
                    &mut swarm,
                    session.as_ref(),
                    &mut coord_relays,
                    force_relay,
                )
                .await;
                // If WAN reachability is not yet established (no relay circuit / not registered on
                // coord), pursue it — even while on a Wi‑Fi/LAN. Being on a LAN means mDNS covers
                // on‑LAN peers, but contacts on mobile data are OFF‑LAN and can only reach us via a
                // relay circuit + coord registration. Gating WAN recovery on `has_active_lan()` made
                // a Wi‑Fi device permanently invisible to off‑LAN contacts (coord_registered=false,
                // no /p2p-circuit). LAN is additive, never a replacement for WAN.
                if crate::coord_runtime::wan_discovery_via_coord_only()
                    && !wan_recovery_satisfied(session.as_ref(), &swarm)
                    && !session.wan_recovery_active.load(Ordering::Relaxed)
                {
                    native_log::info("net", "WAN not ready — begin recovery pass");
                    session.begin_wan_recovery();
                }
                if session.wan_recovery_active.load(Ordering::Relaxed) {
                    run_wan_recovery_pass(&mut swarm, session.as_ref(), &coord_relays);
                } else {
                    let listen = coord_register_listen_snapshot(&swarm, session.as_ref());
                    crate::coord_runtime::coord_register_tick(&listen);
                    let force_keepalive = session.should_relay_keepalive(chrono_now_ms());
                    ensure_wan_relay_circuit(
                        &mut swarm,
                        session.as_ref(),
                        Some(&coord_relays),
                        force_keepalive,
                    );
                }
                if crate::coord_runtime::coord_http_degraded() {
                    if !session.any_bootstrap_connected.load(Ordering::Relaxed) {
                        dial_coord_relays(&mut swarm, session.as_ref(), &coord_relays);
                    }
                    notify_coord_lookup();
                }
            }
            _ = flow_snapshot_tick.tick() => {
                log_connectivity_snapshot(&swarm, session.as_ref(), &writers);
            }
            _ = redial_tick.tick() => {
                if !session.any_bootstrap_connected.load(Ordering::Relaxed) {
                    dial_coord_relays(&mut swarm, &session, &coord_relays);
                    let mut tcp_first: Vec<Multiaddr> = Vec::new();
                    let mut other: Vec<Multiaddr> = Vec::new();
                    for ma in &bootstrap {
                        if ma.is_empty() {
                            continue;
                        }
                        if is_tcp_multiaddr(ma) {
                            tcp_first.push(ma.clone());
                        } else if !is_quic_multiaddr(ma) {
                            other.push(ma.clone());
                        }
                    }
                    for ma in tcp_first.iter().chain(other.iter()) {
                        if !crate::p2p::network_transport::is_trusted_bootstrap_dial_addr(ma) {
                            continue;
                        }
                        let skip = dial_opts_peer_hint(ma)
                            .is_some_and(|pid| swarm.is_connected(&pid));
                        if skip {
                            continue;
                        }
                        if let Err(e) = swarm.dial(ma.clone()) {
                            native_log::debug("dial", format!("bootstrap redial {ma}: {e}"));
                        }
                    }
                }
                let force = session.wan_recovery_active.load(Ordering::Relaxed);
                retry_stalled_relay_reservations(&mut swarm, session.as_ref(), force);
            }
            _ = poll_tick.tick() => {
                // Delivery acks only on the fast tick; read ack retries are ~1 s (DESIGN.md).
                if session.pending_delivery_ack_len() > 0 {
                    let connected_now: Vec<PeerId> = session
                        .connected_peers()
                        .into_iter()
                        .filter(|p| session.is_dm_contact(*p) && swarm.is_connected(p))
                        .collect();
                    if !connected_now.is_empty() {
                        run_delivery_ack_upkeep(
                            Arc::clone(&session),
                            Arc::clone(&writers),
                            &connected_now,
                        )
                        .await;
                    }
                }
            }
            incoming_pair = incoming.next() => {
                if let Some((peer, stream)) = incoming_pair {
                    native_log::info("stream", format!("inbound chat stream from {peer}"));
                    let session2 = Arc::clone(&session);
                    let writers2 = Arc::clone(&writers);
                    let events_tx2 = events_tx.clone();
                    let control2 = control.clone();
                    tokio::spawn(handle_inbound_stream(
                        peer,
                        stream,
                        session2,
                        writers2,
                        Some(events_tx2),
                        control2,
                    ));
                }
            }
            call_pair = call_incoming.next() => {
                if let Some((peer, stream)) = call_pair {
                    native_log::info("call_media", format!("inbound media stream from {peer}"));
                    tokio::spawn(handle_inbound_call_stream(peer, stream, Arc::clone(&session)));
                }
            }
            call_video_pair = call_video_incoming.next() => {
                if let Some((peer, stream)) = call_video_pair {
                    native_log::info("call_video", format!("inbound video stream from {peer}"));
                    tokio::spawn(handle_inbound_call_video_stream(peer, stream, Arc::clone(&session)));
                }
            }
            ev = swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { peer_id, connection_id, endpoint, .. } = &ev {
                    if let Some(pk) = session.register_inbound_dialer_if_needed(*peer_id, endpoint)
                    {
                        let _ = events_tx.send(GossipChatEvent::PeerIdentified {
                            peer_id: *peer_id,
                            public_key_hex: pk,
                        });
                    }
                    if session.is_bootstrap_peer(*peer_id) {
                        // Coord relay bootstrap only — not a chat contact.
                    } else if session.is_dm_contact(*peer_id) {
                        let pid = *peer_id;
                        let is_relay = dm_connection_is_relay(session.as_ref(), pid, endpoint);
                        if is_relay {
                            session.note_dm_relay_connection(pid, *connection_id);
                        } else {
                            session.note_dm_direct_connection(pid, *connection_id);
                            session.note_connection_path(pid, false);
                        }
                        native_log::info("swarm", format!("dm peer connected {pid}"));
                        session.note_connected(pid);
                        if let Some(pk) = secp256k1_public_key_hex_from_peer_id(&pid) {
                            session.clear_dm_reconnect_urgent(&pk);
                        }
                        session.clear_relay_circuit_dial_backoff(pid);
                        let _ = events_tx.send(GossipChatEvent::PeerConnected(pid));
                        let session2 = Arc::clone(&session);
                        let writers2 = Arc::clone(&writers);
                        let events_tx2 = events_tx.clone();
                        let control2 = control.clone();
                        tokio::spawn(async move {
                            on_dm_peer_connected(
                                session2,
                                control2,
                                writers2,
                                pid,
                                Some(events_tx2),
                            )
                            .await;
                        });
                    }
                }
                if let SwarmEvent::ConnectionClosed { peer_id, .. } = &ev {
                    if !session.consume_incidental_reject(*peer_id) {
                        if session.is_dm_contact(*peer_id) {
                            // libp2p may hold several parallel links (mDNS burst, relay+LAN).
                            // Only treat as a full peer drop when no connection remains.
                            if swarm.is_connected(peer_id) {
                                native_log::debug(
                                    "swarm",
                                    format!("dm peer connection path closed {peer_id}"),
                                );
                            } else {
                                native_log::info("swarm", format!("dm peer disconnected {peer_id}"));
                                // Do not tear down an active call here — brief relay/direct churn
                                // and coord blips recover in seconds; hangup signals end calls.
                                let _ = events_tx.send(GossipChatEvent::PeerDisconnected(*peer_id));
                                if let Some(pk) = secp256k1_public_key_hex_from_peer_id(peer_id) {
                                    session.refresh_dm_reconnect_urgent(&pk);
                                }
                                recover_dm_peer_after_disconnect(session.as_ref(), *peer_id);
                                session.note_disconnected(peer_id);
                                if let Ok(mut g) = writers.lock() {
                                    g.remove(peer_id);
                                }
                            }
                        } else {
                            session.note_disconnected(peer_id);
                        }
                    }
                }
                handle_swarm_event(&mut swarm, &session_for_swarm, ev, &mut |ev| {
                    let _ = events_tx.send(ev);
                });
                if take_stream_reopen_notify() {
                    spawn_reopen_dm_chat_streams(
                        &swarm,
                        Arc::clone(&session),
                        Arc::clone(&writers),
                        control.clone(),
                        Some(events_tx.clone()),
                    );
                }
                drain_outbound_queue(
                    &outbound_rx,
                    &mut swarm,
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    control.clone(),
                    Some(events_tx.clone()),
                    MAX_OUTBOUND_CMDS_PER_TICK,
                )
                .await;
            }
        }
    }
}

