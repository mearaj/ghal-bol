/// Routine lookup/dial noise — do not surface to Flutter as link-down.
fn is_transient_swarm_dial_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("no addresses")
        || e.contains("disconnectedandnotdialing")
        || e.contains("dialpeerconditionfalse")
        || e.contains("connection refused")
        || e.contains("connection reset")
        || e.contains("timed out")
        || e.contains("transport error")
}

fn handle_swarm_event(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    event: SwarmEvent<ChatBehaviourEvent>,
    emit: &mut dyn FnMut(GossipChatEvent),
) {
    match event {
        SwarmEvent::Behaviour(ChatBehaviourEvent::Identify(
            libp2p::identify::Event::Received { peer_id, info, .. },
        )) => {
            ingest_identify_listen_addrs(swarm, session, peer_id, &info.listen_addrs, "identify");
            if session.is_bootstrap_peer(peer_id) {
                session.note_bootstrap_identified(peer_id);
                try_relay_reservation_after_identify(swarm, session, peer_id);
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Identify(libp2p::identify::Event::Pushed {
            ..
        })) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Identify(_)) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Dcutr(libp2p::dcutr::Event {
            remote_peer_id,
            result: Ok(connection_id),
            ..
        })) => {
            if session.is_dm_contact(remote_peer_id) {
                native_log::info(
                    "dcutr",
                    format!(
                        "direct connection upgrade to {remote_peer_id} (conn {connection_id:?})"
                    ),
                );
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Dcutr(libp2p::dcutr::Event {
            remote_peer_id,
            result: Err(e),
            ..
        })) => {
            if session.is_dm_contact(remote_peer_id) {
                native_log::warn(
                    "dcutr",
                    format!("hole punch to DM peer {remote_peer_id} failed: {e}"),
                );
            } else if session.should_dial_libp2p_peer(remote_peer_id) {
                native_log::debug(
                    "dcutr",
                    format!("hole punch to {remote_peer_id} failed: {e}"),
                );
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Autonat(
            libp2p::autonat::Event::StatusChanged { new, .. },
        )) => match new {
            libp2p::autonat::NatStatus::Public(addr) => {
                native_log::info("autonat", format!("public reachability via {addr}"));
                let _ = session.merge_published_listen(vec![addr.clone()]);
                crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                    &session.published_listen_snapshot(),
                );
            }
            libp2p::autonat::NatStatus::Private => {
                native_log::info("autonat", "behind NAT — relay+dcutr path");
            }
            libp2p::autonat::NatStatus::Unknown => {}
        },
        SwarmEvent::Behaviour(ChatBehaviourEvent::Autonat(_)) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Upnp(libp2p::upnp::Event::NewExternalAddr(
            external_addr,
        ))) => {
            native_log::info("upnp", format!("external addr {external_addr}"));
            let _ = session.merge_published_listen(vec![external_addr.clone()]);
            crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                &session.published_listen_snapshot(),
            );
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Upnp(_)) => {}
        SwarmEvent::Behaviour(ChatBehaviourEvent::Mdns(libp2p::mdns::Event::Discovered(list))) => {
            handle_mdns_discovered_list(swarm, session, list, emit);
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Mdns(libp2p::mdns::Event::Expired(list))) => {
            // Drop LAN preference only when the expired addr is still our cached direct TCP
            // target. Phones rebind listen ports often — libp2p emits Expired(old) after
            // Discovered(new); wiping the whole peer on every expire caused stale-port dials.
            let mut peers_left = std::collections::HashSet::new();
            for (peer, addr) in list {
                if session.note_peer_mdns_lan_addr_expired(peer, &addr) {
                    native_log::info(
                        "mdns",
                        format!("expired {peer} at {addr} — LAN path dropped"),
                    );
                    peers_left.insert(peer);
                }
            }
            if !peers_left.is_empty() && crate::coord_runtime::wan_discovery_via_coord_only() {
                notify_relay_refresh();
                if !wan_recovery_satisfied(session, swarm) {
                    session.begin_wan_recovery();
                }
            }
            for peer in peers_left {
                if !session.is_dm_contact(peer) {
                    continue;
                }
                session.request_lan_listen_rediscovery(peer);
                apply_peer_left_local_lan(swarm, session, peer);
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Stream(_)) => {}
        SwarmEvent::ListenerClosed {
            addresses, reason, ..
        } => {
            // A relay reservation that fails surfaces here as the circuit listener closing with an
            // error reason (e.g. resource limit, connection reset over the tunnel, timeout). Without
            // this log the failure is invisible and "reserving circuit …" appears to loop forever.
            let relay_listener = addresses
                .iter()
                .any(crate::p2p::network_transport::is_relay_circuit_multiaddr);
            let kind = if relay_listener {
                "relay circuit"
            } else {
                "listener"
            };
            match &reason {
                Ok(()) => native_log::info(
                    "listen",
                    format!("{kind} closed cleanly addrs={addresses:?}"),
                ),
                Err(e) => native_log::warn(
                    "relay",
                    format!("{kind} closed with error: {e} addrs={addresses:?}"),
                ),
            }
            if relay_listener {
                let closed_ipv4 = addresses
                    .iter()
                    .any(crate::p2p::network_transport::is_coord_ipv4_relay_listen);
                let still_ipv4 = swarm
                    .listeners()
                    .any(crate::p2p::network_transport::is_coord_ipv4_relay_listen);
                if !closed_ipv4 {
                    native_log::debug(
                        "relay",
                        format!("ignore non-IPv4 relay listener close addrs={addresses:?}"),
                    );
                } else if still_ipv4 {
                    native_log::debug(
                        "relay",
                        "IPv4 relay circuit still listening — ignore spurious listener close",
                    );
                    let _ = sync_published_listen_from_swarm(session, swarm);
                } else if matches!(reason, Ok(())) {
                    let _ = sync_published_listen_from_swarm(session, swarm);
                    if relay_circuit_listening(swarm) {
                        let _ = sync_published_listen_from_swarm(session, swarm);
                        return;
                    }
                    let now_ms = chrono_now_ms();
                    let bootstrap_up = session.any_bootstrap_connected.load(Ordering::Relaxed);
                    if bootstrap_up && any_relay_reserve_in_flight(session, now_ms) {
                        return;
                    }
                    if bootstrap_up
                        && any_relay_reservation_accepted_recently(session, now_ms, RELAY_RENEWAL_GAP_MS)
                    {
                        native_log::debug(
                            "relay",
                            "relay circuit closed cleanly during renewal — not re-issuing listen_on",
                        );
                        return;
                    }
                    // Bootstrap HOP is down → the relay reservation is invalid until the hop
                    // reconnects and re-issues `listen_on` (libp2p does not auto-restore it),
                    // regardless of how recently it was accepted. Redial the hop now and fall
                    // through to the self-guarded recovery — no clock-based "reservation may
                    // persist" guess (AGENTS.md golden rule 9: event-driven, no grace-window).
                    if !bootstrap_up {
                        let _ = redial_ghalbol_bootstrap_from_cache(
                            swarm,
                            session,
                            "coord relay hop reconnect",
                        );
                    }
                    kick_relay_ipv4_circuit_recovery(
                        swarm,
                        session,
                        &addresses,
                        if !bootstrap_up {
                            "relay circuit closed cleanly (bootstrap down) — re-reserve"
                        } else {
                            "relay circuit closed cleanly — re-reserve"
                        },
                    );
                } else {
                    kick_relay_ipv4_circuit_recovery(
                        swarm,
                        session,
                        &addresses,
                        &format!(
                            "relay circuit listener closed ({reason:?}) — IPv4 circuit gone; re-reserve"
                        ),
                    );
                }
            }
        }
        SwarmEvent::ListenerError { error, .. } => {
            native_log::warn("relay", format!("listener error: {error}"));
        }
        SwarmEvent::NewListenAddr { address, .. } => {
            let is_relay = crate::p2p::network_transport::is_relay_circuit_multiaddr(&address);
            if is_relay && !crate::p2p::network_transport::is_coord_ipv4_relay_listen(&address) {
                native_log::debug(
                    "listen",
                    format!("ignore non-IPv4 relay circuit listen {address}"),
                );
            } else {
                native_log::info("listen", format!("listening on {address}"));
                if is_relay {
                    let _ = session.merge_published_listen(vec![address.clone()]);
                    native_log::info("relay", format!("relay listen addr {address}"));
                    let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
                    crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                        &session.published_listen_snapshot(),
                    );
                    apply_wan_coord_effects(
                        &crate::wan_coord::on_relay_circuit_listening(),
                        None,
                        None,
                    );
                    sync_wan_coord_local_snapshot(swarm);
                    finish_wan_recovery_if_ready(session, swarm);
                } else {
                    let expanded = expand_listen_addresses(&address);
                    let _ = session.merge_published_listen(expanded);
                    let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
                }
                if should_emit_listening_event(&address) {
                    emit(GossipChatEvent::Listening(address.clone()));
                }
            }
        }
        SwarmEvent::ConnectionClosed {
            peer_id,
            connection_id,
            endpoint,
            ..
        } => {
            if session.consume_incidental_reject(peer_id) {
                return;
            }
            if session.is_bootstrap_peer(peer_id) {
                drop_bootstrap_tcp_conn(session, peer_id, connection_id);
                if bootstrap_relay_conn_count(session, peer_id) == 0 {
                    session.clear_bootstrap_relay_session(peer_id);
                    if ghalbol_relay_peer(session) == Some(peer_id) {
                        // HOP TCP lost → reservation is invalid until the hop reconnects and
                        // re-reserves. Redial now and run recovery unconditionally; the bootstrap
                        // reconnect event re-issues `listen_on` (no 120s "reservation may persist"
                        // grace — AGENTS.md golden rule 9). Storm-guarded by reserve throttles.
                        let _ = redial_ghalbol_bootstrap_from_cache(
                            swarm,
                            session,
                            "coord relay hop lost redial",
                        );
                        native_log::warn(
                            "relay",
                            format!(
                                "coord relay bootstrap TCP lost ({peer_id}) — reservation invalid until reconnected; peers cannot dial our /p2p-circuit"
                            ),
                        );
                        let _ = sync_published_listen_from_swarm(session, swarm);
                        apply_wan_coord_effects(
                            &crate::wan_coord::on_relay_bootstrap_lost(
                                relay_circuit_listening(swarm),
                            ),
                            Some(peer_id),
                            None,
                        );
                        notify_relay_refresh();
                        session.begin_wan_recovery();
                        session.mark_dm_reconnect_urgent_unless_live_direct_stream();
                    }
                }
                native_log::debug("swarm", format!("bootstrap connection closed {peer_id}"));
                session.refresh_bootstrap_connected_flag(swarm);
                sync_wan_coord_local_snapshot(swarm);
            } else if session.is_dm_contact(peer_id) {
                session.clear_circuit_dial_in_flight(peer_id);
                let is_relay = dm_connection_is_relay(session, peer_id, &endpoint);
                if is_relay {
                    session.forget_dm_relay_connection(peer_id, connection_id);
                } else {
                    session.forget_dm_direct_connection(peer_id, connection_id);
                }
                session.drop_connection_path(peer_id, is_relay);
                if !is_relay && !session.peer_has_direct_connection(peer_id) {
                    session.forget_peer_on_local_lan(peer_id);
                }
                if swarm.is_connected(&peer_id) {
                    native_log::info(
                        "swarm",
                        format!(
                            "dm connection closed {peer_id} via {} — other path still open",
                            endpoint.get_remote_address()
                        ),
                    );
                    // TRANSPORT.md § Asymmetric mux — relay path dropped while another link lingers.
                    // Reopen chat stream on the surviving mux; do not tear down direct here (upkeep
                    // `reconcile_stale_lan_mux_for_wan` owns asymmetric close-direct policy).
                    if is_relay {
                        session.request_dm_stream_reopen(peer_id);
                        notify_coord_lookup();
                        if session.peer_has_pending_wire_work(peer_id)
                            || session.is_foreground_peer(peer_id)
                        {
                            if let Some(pk) = session
                                .dm_peer_for_libp2p(peer_id)
                                .and_then(|d| d.public_key_hex.clone())
                            {
                                session.refresh_dm_reconnect_urgent(&pk);
                            }
                        }
                    }
                } else {
                    native_log::info("swarm", format!("dm connection closed {peer_id}"));
                }
            } else {
                session.note_disconnected(&peer_id);
            }
        }
        SwarmEvent::ConnectionEstablished {
            peer_id,
            connection_id,
            endpoint,
            ..
        } => {
            session.register_inbound_dialer_if_needed(peer_id, &endpoint);
            if !session.is_kept_peer(peer_id) {
                session.mark_incidental_reject(peer_id);
                let _ = swarm.disconnect_peer_id(peer_id);
                return;
            }
            if session.is_dm_contact(peer_id) {
                if let Some(pk) = secp256k1_public_key_hex_from_peer_id(&peer_id) {
                    session.clear_peer_coord_absent_state(&pk);
                    session.clear_dm_reconnect_urgent(&pk);
                }
                session.clear_lan_dial_in_flight(peer_id);
                session.clear_relay_circuit_dial_backoff(peer_id);
                let is_relay = dm_connection_is_relay(session, peer_id, &endpoint);
                session.clear_circuit_dial_in_flight(peer_id);
                if is_relay {
                    session.note_dm_relay_connection(peer_id, connection_id);
                } else {
                    session.note_dm_direct_connection(peer_id, connection_id);
                    session.note_connection_path(peer_id, false);
                    if let Some(ip) = crate::p2p::network_transport::ipv4_from_ma_str(
                        &endpoint.get_remote_address().to_string(),
                    ) {
                        if ip.is_private() {
                            session.note_peer_on_local_lan(peer_id);
                        }
                    }
                }
                native_log::info(
                    "swarm",
                    format!(
                        "dm connection established {peer_id} via {} ({})",
                        endpoint.get_remote_address(),
                        if is_relay { "relay" } else { "direct" }
                    ),
                );
            } else {
                native_log::debug(
                    "swarm",
                    format!(
                        "bootstrap connection {peer_id} via {}",
                        endpoint.get_remote_address()
                    ),
                );
            }
            if session.is_bootstrap_peer(peer_id) {
                on_bootstrap_tcp_connected(swarm, session, peer_id, connection_id, &endpoint);
            }
        }
        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            let Some(peer) = peer_id else {
                return;
            };
            if session.is_bootstrap_peer(peer) {
                let err_s = error.to_string();
                let is_ghalbol_relay = ghalbol_relay_peer(session) == Some(peer);
                let now_ms = chrono_now_ms();
                if err_s.contains("Network is unreachable") || err_s.contains("network unreachable")
                {
                    if err_s.contains("/ip6/") {
                        session.note_bootstrap_ipv6_unreachable(peer, now_ms);
                        if is_ghalbol_relay && !swarm.is_connected(&peer) {
                            if let Ok(mut m) = session.bootstrap_dial_last_ms.write() {
                                m.remove(&peer);
                            }
                            let v4_nodes: Vec<(PeerId, Multiaddr)> = session
                                .ghalbol_relay_state
                                .read()
                                .ok()
                                .and_then(|g| g.clone())
                                .map(|(relay_peer, addrs)| {
                                    crate::p2p::network_transport::resolve_relay_bootnodes(
                                        &relay_peer.to_string(),
                                        &addrs,
                                    )
                                })
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|(p, ma)| {
                                    *p == peer
                                        && ma.to_string().contains("/ip4/")
                                        && crate::p2p::network_transport::is_trusted_bootstrap_dial_addr(
                                            ma,
                                        )
                                })
                                .collect();
                            if !v4_nodes.is_empty() {
                                issue_bootstrap_dials(
                                    swarm,
                                    session,
                                    &v4_nodes,
                                    "coord relay ipv4 fallback",
                                    true,
                                );
                            }
                        }
                    }
                }
                if is_ghalbol_relay
                    && (err_s.contains("Connection refused")
                        || err_s.contains("connection refused"))
                {
                    let relay_addr = session
                        .bootstrap_relay_addr
                        .read()
                        .ok()
                        .and_then(|m| m.get(&peer).cloned())
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| peer.to_string());
                    crate::coord_runtime::invalidate_cached_ghalbol_relay(
                        session
                            .transcript_path
                            .as_deref()
                            .and_then(|tp| Path::new(tp).parent()),
                    );
                    notify_relay_refresh();
                    native_log::warn(
                        "relay",
                        format!(
                            "Ghal Bol relay TCP unreachable at {relay_addr} — WAN is down until \
                             the relay port is reachable (dev: restart ./ghal_bol_server/deploy/run_server.sh \
                             so bore picks a fresh port; prod: check coord.ghalbol.com:4002). \
                             Cleared stale relay cache; will refetch GET /v1/relay."
                        ),
                    );
                } else if session.should_log_bootstrap_dial_err(peer, chrono_now_ms()) {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue=bootstrap_dial_error | peer={peer} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                }
            } else if session.should_dial_libp2p_peer(peer) {
                let err_s = format!("{error}");
                if session.prefers_mobile_coord_strategy()
                    && crate::coord_runtime::coord_is_configured()
                    && !err_s.contains("/p2p-circuit")
                    && (err_s.contains("/ip6/::1/")
                        || err_s.contains("192.168.")
                        || err_s.contains("/ip4/127.0.0.1/"))
                {
                    apply_peer_left_local_lan(swarm, session, peer);
                }
                let lan_tcp_fail = !err_s.contains("/p2p-circuit")
                    && (session.peer_on_local_lan(peer)
                        || session.peer_mdns_lan_addr(peer).is_some());
                if lan_tcp_fail {
                    // Parallel LAN+WAN: LAN failure must not clear relay circuit in-flight (TRANSPORT.md).
                    let failed = failed_dial_multiaddr_from_error(&err_s);
                    if err_s.contains("Timeout") {
                        session.clear_lan_dial_in_flight(peer);
                        if !session.try_mdns_lan_failover_dial(swarm, peer, failed.as_ref()) {
                            notify_dm_presence_wake();
                            if dm_connect_is_urgent(session, peer, chrono_now_ms()) {
                                notify_coord_lookup();
                            }
                        }
                    } else if !session.try_mdns_lan_failover_dial(swarm, peer, failed.as_ref()) {
                        session.clear_lan_dial_in_flight(peer);
                        if !swarm.is_connected(&peer) && session.lan_candidates_exhausted(peer) {
                            kick_lan_after_lan_dial_path_failed(
                                swarm,
                                session,
                                peer,
                                "LAN candidates exhausted",
                            );
                            notify_coord_lookup();
                        }
                    }
                } else if err_s.contains("/p2p-circuit") {
                    session.clear_circuit_dial_in_flight(peer);
                    session.clear_lan_dial_in_flight(peer);
                    if let Some(pk) = session
                        .dm_peer_for_libp2p(peer)
                        .and_then(|d| d.public_key_hex.clone())
                    {
                        apply_wan_coord_effects(
                            &crate::wan_coord::on_remote_circuit_dial_failed(&pk, &err_s),
                            None,
                            None,
                        );
                        if !session.peer_coord_absent_never_connected(&pk, peer) {
                            session.mark_dm_reconnect_urgent(&pk);
                        }
                    }
                } else {
                    session.clear_circuit_dial_in_flight(peer);
                    session.clear_lan_dial_in_flight(peer);
                }
                let (issue, next) = crate::p2p::connectivity_diag::explain_outgoing_dial_error(&err_s);
                if err_s.contains("ResourceLimitExceeded") || err_s.contains("resource limit") {
                    session.note_relay_circuit_dial_rate_limited(peer, chrono_now_ms());
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else if err_s.contains("Relay has no reservation") {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else if err_s.contains("p2p-circuit") {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else if is_transient_swarm_dial_error(&err_s) {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | transient dial error: {error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                } else {
                    native_log::warn(
                        "dial",
                        format!(
                            "issue={issue} | peer={peer} | next={next} | error={error} | ctx={}",
                            session.diag_ctx()
                        ),
                    );
                }
            }
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Relay(
            libp2p::relay::client::Event::ReservationReqAccepted { relay_peer_id, .. },
        )) => {
            clear_relay_reserve_in_flight(session, relay_peer_id);
            note_relay_reservation_accepted(session, relay_peer_id, chrono_now_ms());
            native_log::info("relay", format!("reservation accepted on {relay_peer_id}"));
            let relay_addrs: Vec<Multiaddr> = swarm
                .listeners()
                .filter(|ma| crate::p2p::network_transport::is_relay_circuit_multiaddr(ma))
                .cloned()
                .collect();
            if !relay_addrs.is_empty() {
                let _ = session.merge_published_listen(relay_addrs);
            }
            let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
            crate::coord_runtime::rebuild_coord_endpoints_from_listen(
                &session.published_listen_snapshot(),
            );
            apply_wan_coord_effects(
                &crate::wan_coord::on_reservation_accepted(),
                Some(relay_peer_id),
                None,
            );
            sync_wan_coord_local_snapshot(swarm);
            finish_wan_recovery_if_ready(session, swarm);
        }
        SwarmEvent::Behaviour(ChatBehaviourEvent::Relay(ev)) => match ev {
            libp2p::relay::client::Event::InboundCircuitEstablished { src_peer_id, .. } => {
                native_log::info(
                    "relay",
                    format!("relay-client: inbound circuit from {src_peer_id}"),
                );
                if session.is_dm_contact(src_peer_id) {
                    session.note_relay_circuit_pending_peer(src_peer_id);
                    // Remote peer re-dialed on relay after leaving LAN — kick WAN mux recovery on
                    // the Wi‑Fi side (asymmetric inbound-only duplicate mux, 07:23 logs).
                    if peer_wan_asymmetric_mux_likely(session, src_peer_id)
                        || peer_needs_wan_mux_reopen(session, src_peer_id)
                    {
                        session.request_dm_stream_reopen(src_peer_id);
                        notify_coord_lookup();
                    }
                }
            }
            libp2p::relay::client::Event::OutboundCircuitEstablished { relay_peer_id, .. } => {
                native_log::info(
                    "relay",
                    format!("relay-client: outbound circuit via {relay_peer_id}"),
                );
                // ConnectionEstablished may follow with bare `/p2p/<dest>` (#5741) — refresh pending.
                let now_ms = chrono_now_ms();
                for dest in session.peers_with_circuit_dial_in_flight(now_ms) {
                    if session.is_dm_contact(dest) {
                        session.note_relay_circuit_pending_peer(dest);
                    }
                }
            }
            other => native_log::info("relay", format!("relay-client: {other:?}")),
        },
        _ => {}
    }
}

/// Collect TCP / relay-circuit listen addrs before `node_ready`.
///
/// Must run **all** relay-relevant swarm events through `handle_swarm_event` — dropping
/// Identify/Relay here leaves bootstrap TCP up with no `listen_on` (reservation never starts).
async fn bootstrap_publishable_listen(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    timeout: Duration,
) {
    let coord_mode = crate::coord_runtime::wan_discovery_via_coord_only();
    let deadline = time::Instant::now() + timeout;
    while time::Instant::now() < deadline {
        if listen_ready_for_node(session, coord_mode, swarm) {
            let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
            return;
        }
        tokio::select! {
            ev = swarm.select_next_some() => {
                handle_swarm_event(swarm, session, ev, &mut |_ev| {});
                if coord_mode
                    && session.any_bootstrap_connected.load(Ordering::Relaxed)
                {
                    attempt_wan_relay_reserve(swarm, session, false);
                }
                if listen_ready_for_node(session, coord_mode, swarm) {
                    let _ = session.merge_published_listen(swarm.listeners().cloned().collect());
                    return;
                }
            }
            _ = time::sleep(Duration::from_millis(40)) => {
                if coord_mode
                    && session.any_bootstrap_connected.load(Ordering::Relaxed)
                {
                    attempt_wan_relay_reserve(swarm, session, false);
                }
            }
        }
    }
    if !listen_ready_for_node(session, coord_mode, swarm) {
        if coord_mode {
            native_log::warn(
                "listen",
                "no relay circuit before node_ready — WAN peers cannot dial this device until \
                 reservation accepted (check coord relay connectivity)",
            );
        } else {
            native_log::warn(
                "listen",
                "no publishable TCP listen addr before node_ready — peers may not find this device yet",
            );
        }
    }
}

