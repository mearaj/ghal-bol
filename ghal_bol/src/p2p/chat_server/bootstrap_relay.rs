/// Prefer TCP on LAN; QUIC bootstrap/mDNS dials often time out on phones.
pub(crate) fn is_tcp_multiaddr(ma: &Multiaddr) -> bool {
    ma.iter().any(|p| matches!(p, Protocol::Tcp(_)))
}

pub(crate) fn is_quic_multiaddr(ma: &Multiaddr) -> bool {
    ma.to_string().contains("quic-v1")
}

fn issue_bootstrap_dials(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    nodes: &[(PeerId, Multiaddr)],
    log_label: &str,
    force: bool,
) {
    use std::collections::HashMap;
    let now_ms = chrono_now_ms();
    let mut by_peer: HashMap<PeerId, Vec<Multiaddr>> = HashMap::new();
    for (peer, ma) in nodes {
        if !crate::p2p::network_transport::is_trusted_bootstrap_dial_addr(ma) {
            continue;
        }
        by_peer.entry(*peer).or_default().push(ma.clone());
    }
    for (peer, mut addrs) in by_peer {
        if has_tracked_bootstrap_tcp(session, peer) {
            if force || session.should_log_dial_skip(peer, now_ms, 8_000) {
                native_log::info("dial", format!("skip {log_label} {peer}: already has tracked TCP"));
            }
            continue;
        }
        if !session.should_issue_bootstrap_dial(peer, now_ms, force) {
            if force || session.should_log_dial_skip(peer, now_ms, 8_000) {
                native_log::info("dial", format!("skip {log_label} {peer}: throttled"));
            }
            continue;
        }
        addrs.sort_by_key(|ma| session.bootstrap_family_rank(ma, peer, now_ms));
        addrs.dedup_by(|a, b| a.to_string() == b.to_string());
        if session.bootstrap_ipv6_degraded(peer, now_ms) {
            addrs.retain(|ma| !ma.to_string().contains("/ip6/"));
        }
        // Wi‑Fi/LAN with a reachable IPv4 relay base: skip IPv6 bootstrap dials that often
        // log `Network is unreachable` and duplicate TCP to the same relay.
        if session.network_profile_snapshot().has_active_lan() {
            let has_v4 = addrs.iter().any(|ma| ma.to_string().contains("/ip4/"));
            if has_v4 {
                addrs.retain(|ma| !ma.to_string().contains("/ip6/"));
            }
        }
        // One dial per address family per pass — happy-eyeballs without prune storms mid-reservation.
        let mut dialed_v4 = false;
        let mut dialed_v6 = false;
        for ma in addrs {
            let is_v6 = ma.to_string().contains("/ip6/");
            if is_v6 {
                if dialed_v6 {
                    continue;
                }
                dialed_v6 = true;
            } else {
                if dialed_v4 {
                    continue;
                }
                dialed_v4 = true;
            }
            if let Ok(mut m) = session.bootstrap_relay_addr.write() {
                m.entry(peer).or_insert_with(|| ma.clone());
            }
            native_log::info("dial", format!("{log_label} {peer} via {ma}"));
            if let Err(e) = swarm.dial(ma.clone()) {
                native_log::warn("dial", format!("{log_label} {peer} {ma}: {e}"));
            }
        }
    }
}

/// Dial coord relay bootstrap nodes (base TCP multiaddrs from GET /v1/relay).
fn dial_coord_relays(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    nodes: &[(PeerId, Multiaddr)],
) {
    issue_bootstrap_dials(swarm, session, nodes, "coord relay", false);
}

/// After a network handover: dial coord relay(s) only when not already connected — never tear down an active relay link
/// (TRANSPORT.md: handover must not drop in-flight DM over relay circuits).
fn ensure_coord_relays_connected(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    nodes: &[(PeerId, Multiaddr)],
) {
    session.refresh_bootstrap_connected_flag(swarm);
    issue_bootstrap_dials(swarm, session, nodes, "coord relay dial", true);
}

/// Immediate bootstrap redial from cached `GET /v1/relay` addrs (no HTTP).
fn redial_ghalbol_bootstrap_from_cache(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    log_label: &str,
) -> bool {
    let Some((relay_peer, addrs)) = session
        .ghalbol_relay_state
        .read()
        .ok()
        .and_then(|g| g.clone())
    else {
        return false;
    };
    let nodes =
        crate::p2p::network_transport::resolve_relay_bootnodes(&relay_peer.to_string(), &addrs);
    if nodes.is_empty() {
        return false;
    }
    issue_bootstrap_dials(swarm, session, &nodes, log_label, true);
    true
}

/// Per-relay throttle for `listen_on(/p2p-circuit)`.
const RELAY_RESERVE_THROTTLE_MS: i64 = 10_000;
/// libp2p closes the old circuit listener during reservation renewal — ignore only this gap.
const RELAY_RENEWAL_GAP_MS: i64 = 3_000;
/// Faster probe retries on CGNAT while bootstrap TCP is still pending (cellular can be slow).
const CGNAT_PROBE_THROTTLE_MS: i64 = 2_500;
/// Do not re-issue `listen_on` while a reservation is in flight (libp2p cancels the prior attempt).
const RELAY_RESERVE_IN_FLIGHT_TIMEOUT_MS: i64 = 30_000;
/// Wait for happy-eyeballs dual-stack bootstrap links before picking the HOP anchor.
const RELAY_BOOTSTRAP_SETTLE_MS: i64 = 450;
/// After bootstrap TCP is up, issue `listen_on` even if Identify was not observed yet
/// (startup `bootstrap_publishable_listen` may drain Identify before the main loop).
const RELAY_TCP_HOP_FALLBACK_MS: i64 = 800;
/// Brief wait for a relay circuit before `node_ready` when WAN/coord mode is on.
/// AGENTS.md: emit `node_ready` after ~3s — do **not** block startup on full WAN. If the relay
/// circuit is not up yet, `begin_wan_recovery()` runs immediately after this wait and recovery
/// continues event-driven on `coord_tick` / `dm_upkeep` (never a long startup stall).
const BOOTSTRAP_LISTEN_MAX_SECS: u64 = 3;

fn note_bootstrap_tcp_since(session: &SessionState, relay: PeerId, now_ms: i64) {
    if let Ok(mut m) = session.bootstrap_tcp_since_ms.write() {
        m.entry(relay).or_insert(now_ms);
    }
}

/// Bootstrap HOP is ready for a single `listen_on(/p2p-circuit)`.
fn bootstrap_hop_ready_for_listen(session: &SessionState, relay: PeerId, now_ms: i64) -> bool {
    if session.is_bootstrap_identified(relay) {
        return true;
    }
    session
        .bootstrap_tcp_since_ms
        .read()
        .ok()
        .and_then(|m| m.get(&relay).copied())
        .is_some_and(|since| now_ms.saturating_sub(since) >= RELAY_TCP_HOP_FALLBACK_MS)
}

/// Live bootstrap TCP address(es) for a relay — the HOP libp2p pins for `listen_on(/p2p-circuit)`.
fn bootstrap_hop_anchor(session: &SessionState, relay: PeerId) -> Option<Multiaddr> {
    let conns = session
        .bootstrap_tcp_conns
        .read()
        .ok()
        .and_then(|m| m.get(&relay).cloned())?;
    if conns.is_empty() {
        return None;
    }
    if conns.len() == 1 {
        return conns.values().next().cloned();
    }
    let now_ms = chrono_now_ms();
    conns
        .values()
        .min_by_key(|ma| session.bootstrap_family_rank(ma, relay, now_ms))
        .cloned()
}

/// Prune dual-stack HOP links if needed, then return the anchor for one `listen_on`.
fn bootstrap_hop_anchor_for_reserve(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
) -> Option<Multiaddr> {
    if bootstrap_relay_conn_count(session, relay) > 1 {
        return prune_duplicate_relay_bootstrap_connections(swarm, session, relay);
    }
    bootstrap_hop_anchor(session, relay)
}

fn attempt_wan_relay_reserve(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    force: bool,
) {
    run_bootstrap_relay_reserve_pass(swarm, session);
    if relay_circuit_listening(swarm) {
        return;
    }
    let now_ms = chrono_now_ms();
    if !force && any_relay_reserve_in_flight(session, now_ms) {
        return;
    }
    let _ = try_relay_reservations(swarm, session, force);
}

fn relay_reserve_in_flight_blocks(session: &SessionState, relay: PeerId, now_ms: i64) -> bool {
    let Ok(m) = session.relay_reserve_in_flight_ms.read() else {
        return false;
    };
    let Some(start) = m.get(&relay).copied() else {
        return false;
    };
    now_ms.saturating_sub(start) < RELAY_RESERVE_IN_FLIGHT_TIMEOUT_MS
}

fn mark_relay_reserve_in_flight(session: &SessionState, relay: PeerId, now_ms: i64) {
    if let Ok(mut m) = session.relay_reserve_in_flight_ms.write() {
        m.insert(relay, now_ms);
    }
}

fn clear_relay_reserve_in_flight(session: &SessionState, relay: PeerId) {
    if let Ok(mut m) = session.relay_reserve_in_flight_ms.write() {
        m.remove(&relay);
    }
}

fn note_relay_reservation_accepted(session: &SessionState, relay: PeerId, now_ms: i64) {
    if let Ok(mut m) = session.relay_reservation_accepted_ms.write() {
        m.insert(relay, now_ms);
    }
}

fn any_relay_reservation_accepted_recently(session: &SessionState, now_ms: i64, window_ms: i64) -> bool {
    session
        .relay_reservation_accepted_ms
        .read()
        .ok()
        .is_some_and(|m| {
            m.values()
                .any(|t| now_ms.saturating_sub(*t) < window_ms)
        })
}

fn any_relay_reserve_in_flight(session: &SessionState, now_ms: i64) -> bool {
    session
        .relay_reserve_in_flight_ms
        .read()
        .ok()
        .map(|m| {
            m.iter().any(|(_, start)| {
                now_ms.saturating_sub(*start) < RELAY_RESERVE_IN_FLIGHT_TIMEOUT_MS
            })
        })
        .unwrap_or(false)
}

fn schedule_bootstrap_relay_reserve(session: &SessionState, relay: PeerId, now_ms: i64) {
    if let Ok(mut m) = session.bootstrap_reserve_after_ms.write() {
        m.insert(relay, now_ms.saturating_add(RELAY_BOOTSTRAP_SETTLE_MS));
    }
}

fn note_bootstrap_tcp_conn(
    session: &SessionState,
    relay: PeerId,
    conn_id: ConnectionId,
    remote: &Multiaddr,
) {
    if remote.to_string().contains("/p2p-circuit") {
        return;
    }
    if let Ok(mut m) = session.bootstrap_tcp_conns.write() {
        m.entry(relay).or_default().insert(conn_id, remote.clone());
    }
}

fn drop_bootstrap_tcp_conn(session: &SessionState, relay: PeerId, conn_id: ConnectionId) {
    if let Ok(mut m) = session.bootstrap_tcp_conns.write() {
        if let Some(inner) = m.get_mut(&relay) {
            inner.remove(&conn_id);
            if inner.is_empty() {
                m.remove(&relay);
            }
        }
    }
}

fn has_tracked_bootstrap_tcp(session: &SessionState, relay: PeerId) -> bool {
    bootstrap_relay_conn_count(session, relay) > 0
}

fn bootstrap_relay_conn_count(session: &SessionState, relay: PeerId) -> usize {
    session
        .bootstrap_tcp_conns
        .read()
        .ok()
        .and_then(|m| m.get(&relay).map(|c| c.len()))
        .unwrap_or(0)
}

/// libp2p relay client pins HOP to one bootstrap TCP link — keep the best family only.
fn prune_duplicate_relay_bootstrap_connections(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
) -> Option<Multiaddr> {
    let conns = session
        .bootstrap_tcp_conns
        .read()
        .ok()
        .and_then(|m| m.get(&relay).cloned())?;
    if conns.is_empty() {
        return None;
    }
    if conns.len() == 1 {
        return conns.values().next().cloned();
    }
    let now_ms = chrono_now_ms();
    // Keep all HOP links while reserve/listen is in flight or circuit is already up.
    if any_relay_reserve_in_flight(session, now_ms) || relay_circuit_listening(swarm) {
        return bootstrap_hop_anchor(session, relay);
    }
    if !bootstrap_hop_ready_for_listen(session, relay, now_ms) {
        return bootstrap_hop_anchor(session, relay);
    }
    let mut ranked: Vec<(ConnectionId, Multiaddr, u8)> = conns
        .iter()
        .map(|(id, ma)| {
            (
                *id,
                ma.clone(),
                session.bootstrap_family_rank(ma, relay, now_ms),
            )
        })
        .collect();
    ranked.sort_by_key(|(_, _, rank)| *rank);
    let (_keep_id, keep_ma, _) = ranked[0].clone();
    for (drop_id, drop_ma, _) in ranked.into_iter().skip(1) {
        native_log::info(
            "relay",
            format!("prune duplicate bootstrap TCP {relay}: drop {drop_ma}, keep {keep_ma}"),
        );
        let _ = swarm.close_connection(drop_id);
        drop_bootstrap_tcp_conn(session, relay, drop_id);
    }
    if let Ok(mut m) = session.bootstrap_relay_addr.write() {
        m.insert(relay, keep_ma.clone());
    }
    Some(keep_ma)
}

/// After happy-eyeballs settle: one `listen_on(/p2p-circuit)` on the live HOP anchor only.
fn run_bootstrap_relay_reserve_pass(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    if !crate::coord_runtime::wan_discovery_via_coord_only() || relay_circuit_listening(swarm) {
        return;
    }
    let now_ms = chrono_now_ms();
    let relays: Vec<PeerId> = session
        .bootstrap_tcp_conns
        .read()
        .ok()
        .map(|m| m.keys().copied().collect())
        .unwrap_or_default();
    if relays.is_empty() {
        if session.should_log_dial_skip(PeerId::random(), now_ms, 8_000) {
            native_log::info("relay", "skip reserve pass: no bootstrap TCP links available");
        }
    }
    for relay in relays {
        if !session.is_bootstrap_peer(relay) || !swarm.is_connected(&relay) {
            continue;
        }
        if !bootstrap_hop_ready_for_listen(session, relay, now_ms) {
            continue;
        }
        if relay_reserve_in_flight_blocks(session, relay, now_ms) {
            continue;
        }
        let _ = try_relay_reservation(swarm, session, relay, false);
    }
}

fn relay_reserve_throttle_ms(session: &SessionState) -> i64 {
    let profile = session.network_profile_snapshot();
    if (profile.on_mobile_data_path() || profile.needs_relay_for_wan())
        && !session.any_bootstrap_connected.load(Ordering::Relaxed)
    {
        CGNAT_PROBE_THROTTLE_MS
    } else {
        RELAY_RESERVE_THROTTLE_MS
    }
}

/// Bootstrap TCP came up — track link, defer reservation until happy-eyeballs settles.
fn on_bootstrap_tcp_connected(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer_id: PeerId,
    connection_id: ConnectionId,
    endpoint: &ConnectedPoint,
) {
    if !session.is_bootstrap_peer(peer_id) {
        return;
    }
    let remote = endpoint.get_remote_address().clone();
    native_log::info(
        "swarm",
        format!("bootstrap connection {peer_id} via {remote}"),
    );
    session.note_bootstrap_connected();
    note_bootstrap_tcp_conn(session, peer_id, connection_id, &remote);
    note_bootstrap_tcp_since(session, peer_id, chrono_now_ms());
    let now_ms = chrono_now_ms();
    schedule_bootstrap_relay_reserve(session, peer_id, now_ms);
    if bootstrap_relay_conn_count(session, peer_id) > 1
        && bootstrap_hop_ready_for_listen(session, peer_id, now_ms)
        && !relay_circuit_listening(swarm)
    {
        let _ = prune_duplicate_relay_bootstrap_connections(swarm, session, peer_id);
    }
    // Reservation runs from Identify + `ensure_wan_relay_circuit` after HOP settle — not here
    // (libp2p relay client needs Identify on the bootstrap link; premature `listen_on` races
    // happy-eyeballs and can pin the wrong HOP).
    if session.wan_recovery_active.load(Ordering::Relaxed) && !relay_circuit_listening(swarm) {
        let _ = ensure_wan_relay_circuit(swarm, session, None, false);
    }
}

/// True once an IPv4/dns4 relay circuit is listening (stable WAN endpoint; ignore `/dns6/…`).
fn relay_circuit_listening(swarm: &Swarm<ChatBehaviour>) -> bool {
    swarm
        .listeners()
        .any(crate::p2p::network_transport::is_coord_ipv4_relay_listen)
}

/// Outbound peer relay dials need our bootstrap TCP HOP up — otherwise circuits cancel each other.
fn own_bootstrap_ready_for_peer_relay_dial(session: &SessionState) -> bool {
    session
        .any_bootstrap_connected
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// Throttle key for global "defer coord lookup until bootstrap up" logs (not a dial target).
fn bootstrap_defer_log_peer() -> PeerId {
    static PEER: std::sync::OnceLock<PeerId> = std::sync::OnceLock::new();
    *PEER.get_or_init(|| {
        use std::str::FromStr;
        PeerId::from_str("12D3KooWEywitWCf3SYpaHbLSmP2CMyRUAQH7qF8JmUp6q6B7Ekk")
            .expect("bootstrap defer log peer")
    })
}

/// Relays eligible for circuit reservation (connected bootstrap HOP, not already circuit-listening).
fn eligible_relays_for_reservation(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
) -> Vec<PeerId> {
    let Some(conns) = session.bootstrap_tcp_conns.read().ok() else {
        return Vec::new();
    };
    conns
        .keys()
        .filter(|p| {
            session.is_bootstrap_peer(**p)
                && swarm.is_connected(p)
                && bootstrap_hop_anchor(session, **p).is_some()
                && !swarm.listeners().any(|l| {
                    crate::p2p::network_transport::is_coord_ipv4_relay_listen(l)
                        && l.to_string().contains(&format!("/p2p/{p}"))
                })
        })
        .copied()
        .collect()
}

fn ghalbol_relay_peer(session: &SessionState) -> Option<PeerId> {
    session
        .ghalbol_relay_state
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|(p, _)| *p))
}

/// libp2p relay-client WAN circuit — **single entry** for reserve / retry / recovery.
///
/// Encodes rust-libp2p relay-client constraints (TRANSPORT.md § Client, § CGNAT):
/// - HOP pins to **one** bootstrap TCP link → prune before `listen_on`.
/// - Each new `listen_on(/p2p-circuit)` **cancels** the prior in-flight reservation.
/// - Bootstrap path: **Identify** on the HOP link before `listen_on`.
/// - CGNAT probe `listen_on` only when bootstrap TCP is **not** up (never parallel).
fn ensure_wan_relay_circuit(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: Option<&[(PeerId, Multiaddr)]>,
    force: bool,
) -> bool {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return false;
    }
    if listen_ready_for_node(session, true, swarm) {
        return true;
    }
    let now_ms = chrono_now_ms();
    // Always advance the deferred reserve pass; only block issuing a *new* listen_on while one
    // is in flight (libp2p cancels the prior reservation if we re-issue too soon).
    run_bootstrap_relay_reserve_pass(swarm, session);
    if relay_circuit_listening(swarm) {
        return true;
    }
    if !force && any_relay_reserve_in_flight(session, now_ms) {
        return false;
    }

    session.refresh_bootstrap_connected_flag(swarm);
    let bootstrap_up = session.any_bootstrap_connected.load(Ordering::Relaxed);
    let mut issued_bootstrap_dial = false;

    if let Some(relays) = coord_relays {
        if !bootstrap_up && !relays.is_empty() {
            if force {
                ensure_coord_relays_connected(swarm, session, relays);
            } else {
                dial_coord_relays(swarm, session, relays);
            }
            issued_bootstrap_dial = true;
        } else if !relays.is_empty() && session.should_log_dial_skip(relays[0].0, now_ms, 8_000) {
             native_log::info("relay", "skip base coord relay dial: bootstrap is already up");
        }
    }

    run_bootstrap_relay_reserve_pass(swarm, session);
    if relay_circuit_listening(swarm) {
        return true;
    }

    // Do not race CGNAT probe `listen_on` against a base TCP dial issued above — wait for HOP.
    if issued_bootstrap_dial {
        return false;
    }

    if !bootstrap_up {
        if swarm_has_lan_dm_listen(swarm) {
            return false;
        }
        let profile = session.network_profile_snapshot();
        if profile.on_mobile_data_path() || profile.needs_relay_for_wan() {
            if try_ghalbol_probe_style_circuit_listen(swarm, session, force) {
                native_log::info(
                    "relay",
                    "CGNAT probe-style relay reservation initiated (bootstrap TCP pending)",
                );
                return true;
            }
        }
        return false;
    }

    let issued = try_relay_reservations(swarm, session, force);
    if issued > 0 {
        native_log::info(
            "relay",
            format!(
                "reservation listen_on issued for {issued} bootstrap(s) — await ReservationReqAccepted"
            ),
        );
    }
    issued > 0
}

/// When coord + ghalbol relay are configured, reserve only on our reliable relay (TRANSPORT.md).
fn relays_to_try_for_reservation(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
) -> Vec<PeerId> {
    let eligible = eligible_relays_for_reservation(swarm, session);
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return eligible;
    }
    let Some(ghalbol) = ghalbol_relay_peer(session) else {
        return eligible;
    };
    if eligible.iter().any(|p| *p == ghalbol) {
        return vec![ghalbol];
    }
    // Not connected yet — caller dials base TCP then retries after ConnectionEstablished/identify.
    Vec::new()
}

/// Issue `listen_on(/p2p-circuit)` on the advertised ghalbol relay — same as `relay_probe`:
/// the relay client dials through the circuit multiaddr; no separate bootstrap dial first.
fn try_ghalbol_probe_style_circuit_listen(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    _force: bool,
) -> bool {
    if !crate::coord_runtime::wan_discovery_via_coord_only() || relay_circuit_listening(swarm) {
        return false;
    }
    let Some(ghalbol) = ghalbol_relay_peer(session) else {
        return false;
    };
    let nodes = session
        .ghalbol_relay_state
        .read()
        .ok()
        .and_then(|g| g.clone())
        .map(|(peer, addrs)| {
            crate::p2p::network_transport::resolve_relay_bootnodes(&peer.to_string(), &addrs)
        })
        .unwrap_or_default();
    let now_ms = chrono_now_ms();
    let mut candidates: Vec<(PeerId, Multiaddr)> = nodes
        .into_iter()
        .filter(|(p, ma)| {
            *p == ghalbol && crate::p2p::network_transport::is_trusted_bootstrap_dial_addr(ma)
        })
        .collect();
    candidates.sort_by_key(|(p, ma)| session.bootstrap_family_rank(ma, *p, now_ms));
    let Some((_, base_ma)) = candidates.into_iter().next() else {
        return false;
    };

    let already_listening = swarm.listeners().any(|ma| {
        ma.to_string().contains("/p2p-circuit")
            && ma.to_string().contains(&format!("/p2p/{ghalbol}"))
    });
    if already_listening {
        return false;
    }
    if session.any_bootstrap_connected.load(Ordering::Relaxed) {
        return false;
    }

    if relay_reserve_in_flight_blocks(session, ghalbol, now_ms) {
        return false;
    }

    let throttle_ms = relay_reserve_throttle_ms(session);
    if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
        if let Some(last) = m.get(&ghalbol).copied() {
            if now_ms.saturating_sub(last) < throttle_ms {
                return false;
            }
        }
        m.insert(ghalbol, now_ms);
    } else {
        return false;
    }

    let mut base = base_ma;
    if !base.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        base.push(Protocol::P2p(ghalbol));
    }
    let Some(listen_ma) = crate::p2p::network_transport::relay_circuit_listen_addr(&base) else {
        return false;
    };
    match swarm.listen_on(listen_ma.clone()) {
        Ok(_) => {
            mark_relay_reserve_in_flight(session, ghalbol, now_ms);
            native_log::info(
                "relay",
                format!("ghalbol circuit listen (probe path) via {listen_ma}"),
            );
            true
        }
        Err(e) => {
            native_log::warn("relay", format!("ghalbol circuit listen {listen_ma}: {e}"));
            clear_relay_reserve_in_flight(session, ghalbol);
            if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
                m.insert(ghalbol, now_ms);
            }
            false
        }
    }
}

/// Issue a relay reservation once identify has completed on a bootstrap link (handshake ready).
fn try_relay_reservation_after_identify(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
) {
    if relay_circuit_listening(swarm) {
        return;
    }
    if !session.is_bootstrap_peer(relay) {
        return;
    }
    schedule_bootstrap_relay_reserve(session, relay, chrono_now_ms());
    run_bootstrap_relay_reserve_pass(swarm, session);
}

/// Reserve a relay circuit on EVERY eligible bootstrap, in parallel.
///
/// Serializing onto one relay at a time (the previous "one-at-a-time" scheme) let a single
/// bootstrap whose reservation is *pending but never accepted* block all the others: WAN
/// reachability then took minutes or never came up. The per-relay throttle inside
/// `try_relay_reservation` (`RELAY_RESERVE_THROTTLE_MS`) already prevents 1s `listen_on` storms,
/// so fanning out is both safe and necessary — a granting relay is found in seconds.
/// Returns the number of relays a fresh `listen_on` was issued for this pass.
fn try_relay_reservations(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    force: bool,
) -> usize {
    if relay_circuit_listening(swarm) {
        return 0;
    }
    let now_ms = chrono_now_ms();
    if !force && any_relay_reserve_in_flight(session, now_ms) {
        if session.should_log_dial_skip(ghalbol_relay_peer(session).unwrap_or(PeerId::random()), now_ms, 8_000) {
            native_log::info("relay", "skip relay reservations: another reservation pass is in flight");
        }
        return 0;
    }
    run_bootstrap_relay_reserve_pass(swarm, session);
    if relay_circuit_listening(swarm) {
        return 0;
    }
    let mut issued = 0usize;
    let relays = relays_to_try_for_reservation(swarm, session);
    if relays.is_empty() {
        if force || session.should_log_dial_skip(PeerId::random(), now_ms, 8_000) {
            native_log::info("relay", "skip relay reservations: no eligible relays");
        }
    }
    for peer in relays {
        if try_relay_reservation(swarm, session, peer, force) {
            issued += 1;
        }
    }
    issued
}

/// Request a relay reservation on a connected bootstrap (NAT traversal for phones).
/// Returns `true` only when a fresh `listen_on(/p2p-circuit)` was issued this call.
/// Uses the **live HOP TCP multiaddr**, never the dial-cache addr (dual-stack can differ).
fn try_relay_reservation(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    relay: PeerId,
    force: bool,
) -> bool {
    if !session.is_bootstrap_peer(relay) {
        return false;
    }
    let now_ms = chrono_now_ms();
    if !bootstrap_hop_ready_for_listen(session, relay, now_ms) {
        if force || session.should_log_dial_skip(relay, now_ms, 8_000) {
            native_log::info("relay", format!("skip relay reserve {relay}: HOP not ready"));
        }
        return false;
    }
    if force {
        // Handover may clear throttle timestamps; never clear in-flight — a new `listen_on`
        // cancels the working reservation (rust-libp2p #6165 / TRANSPORT.md).
        if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
            m.remove(&relay);
        }
    } else if relay_reserve_in_flight_blocks(session, relay, now_ms) {
        if session.should_log_dial_skip(relay, now_ms, 8_000) {
            native_log::info("relay", format!("skip relay reserve {relay}: already in flight"));
        }
        return false;
    }
    if let Ok(mut m) = session.relay_reserve_in_flight_ms.write() {
        if let Some(start) = m.get(&relay).copied() {
            if now_ms.saturating_sub(start) >= RELAY_RESERVE_IN_FLIGHT_TIMEOUT_MS {
                m.remove(&relay);
                if let Ok(mut g) = session.relay_reserve_requested.write() {
                    g.remove(&relay);
                }
                native_log::warn(
                    "relay",
                    format!("reservation in-flight timeout on {relay} — retry"),
                );
            }
        }
    }
    // If we are already listening on this IPv4 circuit, do not re-issue listens.
    let already_listening = swarm.listeners().any(|ma| {
        crate::p2p::network_transport::is_coord_ipv4_relay_listen(ma)
            && ma.to_string().contains(&format!("/p2p/{relay}"))
    });
    if already_listening {
        if force || session.should_log_dial_skip(relay, now_ms, 8_000) {
            native_log::info("relay", format!("skip relay reserve {relay}: already listening"));
        }
        return false;
    }
    // Per-relay time throttle — skipped on `force` (left-LAN / WAN recovery handover).
    if !force {
        if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
            if let Some(last) = m.get(&relay).copied() {
                if now_ms.saturating_sub(last) < RELAY_RESERVE_THROTTLE_MS {
                    if session.should_log_dial_skip(relay, now_ms, 8_000) {
                        native_log::info("relay", format!("skip relay reserve {relay}: throttled"));
                    }
                    return false;
                }
            }
            m.insert(relay, now_ms);
        } else {
            return false;
        }
    } else if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
        m.insert(relay, now_ms);
    }

    let Some(relay_addr) = bootstrap_hop_anchor_for_reserve(swarm, session, relay) else {
        if force || session.should_log_dial_skip(relay, now_ms, 8_000) {
            native_log::info("relay", format!("skip relay reserve {relay}: no HOP anchor"));
        }
        return false;
    };
    if relay_addr.to_string().contains("/p2p-circuit") {
        if force || session.should_log_dial_skip(relay, now_ms, 8_000) {
            native_log::info("relay", format!("skip relay reserve {relay}: HOP is circuit addr"));
        }
        return false;
    }

    let Some(listen_ma) = crate::p2p::network_transport::relay_circuit_listen_addr(&relay_addr) else {
        native_log::warn(
            "relay",
            format!("no circuit listen addr for {relay} from HOP {relay_addr}"),
        );
        return false;
    };
    match swarm.listen_on(listen_ma.clone()) {
        Ok(_) => {
            mark_relay_reserve_in_flight(session, relay, now_ms);
            native_log::info(
                "relay",
                format!("reserving circuit on {relay} via {listen_ma}"),
            );
            true
        }
        Err(e) => {
            native_log::warn("relay", format!("relay reserve listen {listen_ma}: {e}"));
            clear_relay_reserve_in_flight(session, relay);
            if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
                m.insert(relay, now_ms);
            }
            false
        }
    }
}

/// Poll/UI only needs TCP dialable listen addrs (LAN or relay circuit), not every relay transport variant.
fn should_emit_listening_event(addr: &Multiaddr) -> bool {
    crate::p2p::network_transport::is_dm_listen_tcp_multiaddr(addr)
        && (!crate::p2p::network_transport::is_relay_circuit_multiaddr(addr)
            || crate::p2p::network_transport::is_coord_ipv4_relay_listen(addr))
}

/// Expand `0.0.0.0` DM TCP listeners into concrete RFC1918 addrs for coord + mDNS publish.
fn swarm_listen_addrs_for_coord(swarm: &Swarm<ChatBehaviour>) -> Vec<Multiaddr> {
    let mut out = Vec::new();
    for ma in swarm.listeners() {
        if crate::p2p::network_transport::is_relay_circuit_multiaddr(ma)
            && !crate::p2p::network_transport::is_coord_ipv4_relay_listen(ma)
        {
            continue;
        }
        let expanded = expand_listen_addresses(ma);
        if expanded.is_empty() {
            out.push(ma.clone());
        } else {
            out.extend(expanded);
        }
    }
    out
}

/// Coord registration must be based on what libp2p is *actually* listening on.
/// Using only the cached `published_listen` snapshot can temporarily drop relay circuits
/// during churn, which makes coord think we have "no endpoints" and flaps registration.
fn coord_register_listen_snapshot(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
) -> Vec<Multiaddr> {
    let mut out = session.published_listen_snapshot();
    for ma in swarm_listen_addrs_for_coord(swarm) {
        if !out.iter().any(|e| e == &ma) {
            out.push(ma);
        }
    }
    out
}

/// Drop relay circuits libp2p has closed; merge live swarm listeners into the publish cache.
fn sync_published_listen_from_swarm(session: &SessionState, swarm: &Swarm<ChatBehaviour>) -> bool {
    let live: std::collections::HashSet<Multiaddr> = swarm.listeners().cloned().collect();
    let Ok(mut v) = session.published_listen.write() else {
        return false;
    };
    let before = v.clone();
    v.retain(|ma| {
        if crate::p2p::network_transport::is_relay_circuit_multiaddr(ma) {
            return live.contains(ma) && crate::p2p::network_transport::is_coord_ipv4_relay_listen(ma);
        }
        true
    });
    for ma in live {
        if !crate::p2p::network_transport::is_dm_listen_tcp_multiaddr(&ma)
            || crate::p2p::network_transport::is_relay_circuit_multiaddr(&ma)
        {
            continue;
        }
        let expanded = expand_listen_addresses(&ma);
        let addrs: Vec<Multiaddr> = if expanded.is_empty() {
            vec![ma]
        } else {
            expanded
        };
        for a in addrs {
            if !v.iter().any(|x| x == &a) {
                v.push(a);
            }
        }
    }
    *v != before
}

/// On full network handover (left LAN / mobile-data): reopen streams when outbox pending on a live link.
fn nudge_dm_streams_pending_outbox_on_wan_handover(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
) {
    for peer in session.dm_peer_ids() {
        if !session.should_dial_libp2p_peer(peer) {
            continue;
        }
        if !swarm.is_connected(&peer) || session.dm_peer_stream_up(peer) {
            continue;
        }
        if session.peer_has_pending_outbox(peer) {
            native_log::info(
                "outbox",
                format!("WAN handover — reopen DM stream for {peer} (pending outbox)"),
            );
            session.request_dm_stream_reopen(peer);
        }
    }
}

/// Re-register on coord, refresh relay reservations, and reconnect DM peers after any network
/// or public-reachability change (interface handover, OS callback, or new relay circuit).
fn refresh_coord_reachability_after_network_change(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
    log_ctx: &str,
    full_handover: bool,
) {
    native_log::info("net", log_ctx.to_string());
    if !full_handover {
        refresh_coord_presence_soft(swarm, session, coord_relays);
        return;
    }
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        let _ = sync_published_listen_from_swarm(session, swarm);
        crate::coord_runtime::rebuild_coord_endpoints_from_listen(&coord_register_listen_snapshot(
            swarm, session,
        ));
        return;
    }
    session.clear_coord_lookup_backoff_all();
    clear_wan_listen_state_for_handover(session);
    if !session.network_profile_snapshot().has_active_lan() {
        session.purge_mdns_lan_candidates_for_dm_peers();
    }
    let _ = sync_published_listen_from_swarm(session, swarm);
    crate::coord_runtime::coord_invalidate_presence_on_network_change();
    crate::coord_runtime::rebuild_coord_endpoints_from_listen(&coord_register_listen_snapshot(
        swarm, session,
    ));
    if !wan_recovery_satisfied(session, swarm) {
        session.begin_wan_recovery();
    }
    notify_relay_refresh();
    for pk in session.dm_public_keys() {
        session.mark_dm_reconnect_urgent(&pk);
    }
    notify_dm_presence_wake();
    notify_stream_reopen();
    nudge_dm_streams_pending_outbox_on_wan_handover(swarm, session);
    crate::coord_runtime::schedule_register_presence_force();
    ensure_coord_relays_connected(swarm, session, coord_relays);
    retry_stalled_relay_reservations(swarm, session, true);
}

/// Wi‑Fi/LAN return after leaving WAN/mobile — refresh mDNS + DM without coord invalidation.
fn handle_lan_path_restored(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
    old_mode: &str,
    new_mode: &str,
) {
    native_log::info(
        "net",
        format!("LAN restored {old_mode} -> {new_mode} (soft handover)"),
    );
    let _ = sync_published_listen_from_swarm(session, swarm);
    let lan_history = session.dm_peers_with_lan_history();
    session.purge_mdns_lan_candidates_for_dm_peers();
    let rediscover = lan_rediscovery_peer_set(session, lan_history);
    for peer in session.dm_peer_ids() {
        session.clear_lan_candidates_exhausted(peer);
    }
    for peer in rediscover {
        session.request_lan_listen_rediscovery(peer);
    }
    ensure_lan_tcp_listen(swarm, session, true);
    restart_mdns_behaviour(swarm, session, true);
    apply_wan_coord_effects(
        &crate::wan_coord::on_lan_path_restored(),
        None,
        Some(session),
    );
    let _ = ensure_wan_relay_circuit(swarm, session, Some(coord_relays), false);
    refresh_coord_presence_soft(swarm, session, coord_relays);
    let need_bootstrap = ghalbol_relay_peer(session)
        .map(|relay| !has_tracked_bootstrap_tcp(session, relay))
        .unwrap_or(true);
    if need_bootstrap {
        ensure_coord_relays_connected(swarm, session, coord_relays);
    }
}

/// On-LAN DHCP / interface drift — listen sync + mDNS/LAN DM rediscovery (Wi‑Fi flap while profile stays `lan`).
fn handle_lan_interface_drift(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
    old_mode: &str,
    new_mode: &str,
) {
    native_log::info(
        "net",
        format!("LAN interface drift {old_mode} -> {new_mode} (listen sync only)"),
    );
    kick_lan_dm_rediscovery_after_handover(swarm, session, "interface drift", false);
    refresh_coord_presence_soft(swarm, session, coord_relays);
    ensure_coord_relays_connected(swarm, session, coord_relays);
}

/// True when Wi‑Fi is linked enough to run LAN listen + mDNS (profile may lag after toggle).
fn wifi_lan_handover_active(session: &SessionState) -> bool {
    if session.network_profile_snapshot().has_active_lan() {
        return true;
    }
    let detected = detected_network_with_platform_hints();
    detected.has_wifi_iface || detected.has_rfc1918_on_wifi
}

/// Android ConnectivityManager or Linux sysfs operstate — authoritative when if_addrs lags.
fn platform_wifi_linked(session: &SessionState) -> bool {
    platform_wifi_linked_from_profile(&session.network_profile_snapshot())
}

/// Interim LAN recovery while a relay circuit dial is in flight — mDNS + stream reopen only.
/// Does **not** replace full kick: `try_flush_pending_full_lan_kick` runs fresh ephemeral TCP
/// once `circuit_dial_in_flight` clears (TRANSPORT.md § “Deferred full LAN kick”).
fn soft_lan_rediscovery_nudge(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    reason: &str,
) {
    native_log::info("net", format!("LAN soft rediscovery — {reason}"));
    for peer in session.dm_peer_ids() {
        if peer_eligible_for_lan_handover(session, peer) {
            session.request_lan_listen_rediscovery(peer);
        }
    }
    restart_mdns_behaviour(swarm, session, false);
    notify_stream_reopen();
    notify_dm_presence_wake();
}

/// Relay dropped or interface drift while still on LAN — reopen listen, restart mDNS, allow LAN dials again.
fn kick_lan_dm_rediscovery_after_handover(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    reason: &str,
    force: bool,
) {
    let now_ms = chrono_now_ms();
    if !force {
        if !platform_wifi_linked(session) {
            return;
        }
        // Throttle full handover — dial fail must not close/rebind listen every tick.
        if !session.should_run_lan_recovery(now_ms) {
            notify_dm_presence_wake();
            return;
        }
    } else if !platform_wifi_linked(session) {
        return;
    }
    native_log::info("net", format!("LAN DM rediscovery — {reason}"));
    session.clear_pending_full_lan_kick();
    let lan_history = session.dm_peers_with_lan_history();
    session.purge_mdns_lan_candidates_for_dm_peers();
    for peer in session.dm_peer_ids() {
        session.clear_lan_candidates_exhausted(peer);
        session.clear_lan_dial_in_flight(peer);
    }
    let rediscover = lan_rediscovery_peer_set(session, lan_history);
    for peer in rediscover {
        session.request_lan_listen_rediscovery(peer);
    }
    ensure_lan_tcp_listen(swarm, session, true);
    restart_mdns_behaviour(swarm, session, true);
    session.mark_dm_reconnect_urgent_unless_live_direct_stream();
    notify_dm_presence_wake();
    session.clear_coord_lookup_backoff_all();
    notify_stream_reopen();
}

/// After full DM disconnect — reopen streams and rediscover; LAN mDNS and coord lookup run in parallel (TRANSPORT.md § Parallel LAN + WAN).
fn recover_dm_peer_after_disconnect(session: &SessionState, peer: PeerId) {
    notify_stream_reopen();
    if session.peer_on_local_lan(peer) {
        session.request_lan_listen_rediscovery(peer);
    }
    notify_dm_presence_wake();
    if crate::coord_runtime::coord_is_configured() {
        notify_coord_lookup();
    }
}

/// Full handover kick only when the LAN dial path failed (event), not on transient libp2p churn.
fn kick_lan_after_lan_dial_path_failed(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    reason: &str,
) {
    if swarm.is_connected(&peer) {
        return;
    }
    if !platform_wifi_linked(session) {
        return;
    }
    kick_lan_dm_rediscovery_after_handover(swarm, session, reason, false);
}

/// Coord presence + DM rediscovery without tearing down bootstrap HOP / reservation state.
/// Used when only the published listen set drifted (e.g. libp2p opened a `/dns6/.../p2p-circuit`).
fn refresh_coord_presence_soft(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
) {
    let _ = sync_published_listen_from_swarm(session, swarm);
    if wifi_lan_handover_active(session) {
        ensure_lan_tcp_listen(swarm, session, false);
    }
    let snap = coord_register_listen_snapshot(swarm, session);
    crate::coord_runtime::rebuild_coord_endpoints_from_listen(&snap);
    if !relay_circuit_listening(swarm) {
        session.begin_wan_recovery();
        ensure_wan_relay_circuit(swarm, session, Some(coord_relays), false);
    }
}

/// Drop stale relay listen addrs after a network handover. Keep LAN TCP when still on LAN.
fn clear_wan_listen_state_for_handover(session: &SessionState) {
    let on_lan = session.network_profile_snapshot().has_active_lan();
    if let Ok(mut v) = session.published_listen.write() {
        v.retain(|ma| !crate::p2p::network_transport::is_relay_circuit_multiaddr(ma));
        if crate::coord_runtime::wan_discovery_via_coord_only() && !on_lan {
            v.retain(|ma| {
                !crate::p2p::network_transport::ipv4_from_ma_str(&ma.to_string())
                    .is_some_and(|ip| ip.is_private())
            });
        }
    }
    if let Ok(mut g) = session.relay_reserve_requested.write() {
        g.clear();
    }
    if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
        m.clear();
    }
    if let Ok(mut m) = session.relay_reserve_in_flight_ms.write() {
        m.clear();
    }
    // Keep live bootstrap HOP tracking — clearing here while TCP is still up breaks
    // `bootstrap_hop_anchor` until a new ConnectionEstablished (which never fires).
    if let Ok(mut m) = session.bootstrap_reserve_after_ms.write() {
        m.clear();
    }
    if let Ok(mut m) = session.bootstrap_dial_last_ms.write() {
        m.clear();
    }
}

fn listen_ready_for_node(
    session: &SessionState,
    coord_mode: bool,
    swarm: &Swarm<ChatBehaviour>,
) -> bool {
    if coord_mode {
        return swarm
            .listeners()
            .any(crate::p2p::network_transport::is_coord_ipv4_relay_listen);
    }
    let snap = session.published_listen_snapshot();
    if snap
        .iter()
        .any(crate::p2p::network_transport::is_coord_ipv4_relay_listen)
    {
        return true;
    }
    !crate::p2p::network_transport::tcp_dm_publish_addrs(snap).is_empty()
}

fn try_wan_relay_recovery(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    let _ = ensure_wan_relay_circuit(swarm, session, None, false);
}

/// IPv4 relay circuit listener is gone — clear stale reserve bookkeeping and pursue recovery.
/// No-op when another IPv4 circuit is still listening (libp2p renewal in flight).
fn kick_relay_ipv4_circuit_recovery(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    closed_addrs: &[Multiaddr],
    log_ctx: &str,
) {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return;
    }
    let _ = sync_published_listen_from_swarm(session, swarm);
    if relay_circuit_listening(swarm) {
        return;
    }
    let now_ms = chrono_now_ms();
    let bootstrap_up = session.any_bootstrap_connected.load(Ordering::Relaxed);
    if bootstrap_up {
        if any_relay_reserve_in_flight(session, now_ms) {
            return;
        }
        if any_relay_reservation_accepted_recently(session, now_ms, RELAY_RENEWAL_GAP_MS) {
            return;
        }
    }
    native_log::warn("relay", log_ctx.to_string());
    if let Some(ghalbol) = ghalbol_relay_peer(session) {
        clear_relay_reserve_in_flight(session, ghalbol);
        if let Ok(mut g) = session.relay_reserve_requested.write() {
            g.remove(&ghalbol);
        }
        if let Ok(mut m) = session.relay_reserve_last_attempt_ms.write() {
            m.remove(&ghalbol);
        }
    }
    if let Ok(mut v) = session.published_listen.write() {
        for addr in closed_addrs {
            v.retain(|ma| ma != addr);
        }
        v.retain(|ma| !crate::p2p::network_transport::is_relay_circuit_multiaddr(ma));
    }
    notify_relay_refresh();
    session.begin_wan_recovery();
    apply_wan_coord_effects(
        &crate::wan_coord::on_relay_circuit_lost(),
        None,
        None,
    );
    session.mark_dm_reconnect_urgent_unless_live_direct_stream();
    let _ = ensure_wan_relay_circuit(swarm, session, None, false);
}

fn wan_recovery_satisfied(session: &SessionState, swarm: &Swarm<ChatBehaviour>) -> bool {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return true;
    }
    if !listen_ready_for_node(session, true, swarm) {
        return false;
    }
    // When coord HTTP is unreachable, a relay circuit is enough for WAN;
    // keep retrying coord register in the background without blocking recovery completion.
    if crate::coord_runtime::coord_http_degraded() {
        return true;
    }
    crate::coord_runtime::coord_is_registered()
}

fn finish_wan_recovery_if_ready(session: &SessionState, swarm: &Swarm<ChatBehaviour>) {
    if !session.wan_recovery_active.load(Ordering::Relaxed) {
        return;
    }
    if wan_recovery_satisfied(session, swarm) {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        let msg = if crate::coord_runtime::coord_http_degraded() {
            "WAN recovery complete — relay circuit listening (coord HTTP degraded)"
        } else {
            "WAN recovery complete — relay circuit + coord registered"
        };
        native_log::info("net", msg);
    }
}

fn run_wan_recovery_pass(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
) {
    if !session.wan_recovery_active.load(Ordering::Relaxed) {
        return;
    }
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        session.wan_recovery_active.store(false, Ordering::Relaxed);
        return;
    }
    // On an active Wi‑Fi/LAN we keep existing bootstrap links and never force redial churn (that
    // disrupts working Wi‑Fi paths). But we STILL pursue a relay circuit + coord registration:
    // off‑LAN contacts (mobile data) can only reach us over WAN, so LAN must never abort recovery.
    if !listen_ready_for_node(session, true, swarm) {
        if coord_relays.is_empty() {
            notify_relay_refresh();
        } else {
            ensure_wan_relay_circuit(swarm, session, Some(coord_relays), true);
        }
    }
    let listen = coord_register_listen_snapshot(swarm, session);
    // Never call blocking coord HTTP (try_restore_relay_presence_from_coord) from the tokio
    // swarm loop — reqwest::blocking drops an internal runtime and panics. coord_register_tick
    // schedules relay-presence poll + register on background std threads (TRANSPORT.md § event-driven).
    crate::coord_runtime::coord_register_tick(&listen);
    finish_wan_recovery_if_ready(session, swarm);
}

/// Close only direct LAN TCP links to `peer` — relay circuits stay up (TRANSPORT.md § Parallel LAN + WAN).
fn close_direct_dm_connections(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
) {
    for conn_id in session.drain_dm_direct_connection_ids(peer) {
        native_log::info(
            "net",
            format!("close direct DM link {peer} ({conn_id:?}) — relay kept"),
        );
        let _ = swarm.close_connection(conn_id);
    }
}

/// Remote peer left local LAN or we left Wi‑Fi — phase E–F via wan_coord; drop direct only.
fn apply_peer_left_local_lan(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
) {
    session.forget_peer_on_local_lan(peer);
    close_direct_dm_connections(swarm, session, peer);
    session.request_dm_stream_reopen(peer);
    if let Some(pk) = session
        .dm_peer_for_libp2p(peer)
        .and_then(|d| d.public_key_hex.clone())
        .filter(|pk| pk.len() == 66)
    {
        apply_wan_coord_effects(
            &crate::wan_coord::on_peer_off_local_lan(&pk),
            None,
            None,
        );
        session.mark_dm_reconnect_urgent(&pk);
    }
}

/// Wi‑Fi → mobile-data handover — TRANSPORT.md § “LAN ↔ WAN handover” (parallel; keep relay links).
fn apply_left_lan_handover(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    for peer in session.dm_peer_ids() {
        session.forget_peer_on_local_lan(peer);
        session.clear_lan_dial_in_flight(peer);
        close_direct_dm_connections(swarm, session, peer);
        session.request_dm_stream_reopen(peer);
    }
    session.purge_all_mdns_lan_state();
    clear_wan_listen_state_for_handover(session);
    session.begin_wan_recovery();
    apply_wan_coord_effects(
        &crate::wan_coord::on_left_lan(),
        None,
        Some(session),
    );
    let _ = ensure_wan_relay_circuit(swarm, session, None, true);
    crate::coord_runtime::schedule_register_presence_force();
    native_log::info(
        "net",
        "left LAN — mDNS purged; WAN phases B–F via wan_coord (relay links kept)",
    );
}

fn handle_network_path_change(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
    old_mode: &str,
    new_mode: &str,
) {
    let profile = session.network_profile_snapshot();
    // Leaving LAN — one coordinated handover (WAN reserve + stream reopen). Do not run LAN
    // interface drift on the same tick Wi‑Fi drops (Android often flickers lan→lan first).
    if !profile.has_active_lan() && old_mode == "lan" {
        apply_left_lan_handover(swarm, session);
        return;
    }
    if old_mode == "lan" && new_mode != "lan" {
        apply_left_lan_handover(swarm, session);
        return;
    }
    if old_mode != "lan" && new_mode == "lan" {
        handle_lan_path_restored(swarm, session, coord_relays, old_mode, new_mode);
        return;
    }
    if old_mode == "lan" && new_mode == "lan" && profile.has_active_lan() && platform_wifi_linked(session)
    {
        handle_lan_interface_drift(swarm, session, coord_relays, old_mode, new_mode);
        return;
    }
    refresh_coord_reachability_after_network_change(
        swarm,
        session,
        coord_relays,
        &format!("network path changed {old_mode} -> {new_mode}"),
        true,
    );
}

/// When coord is set, WAN DM needs a relay circuit. Reservations can stall; retry on connected bootstraps.
/// Minimum interval between `GET /v1/relay` refetches when the relay is already connected.
const GHALBOL_RELAY_REFETCH_MS: i64 = 30_000;
/// Aggressive refetch while no relay dial addr is known (coord may have just enabled relay).
const GHALBOL_RELAY_REFETCH_EMPTY_MS: i64 = 5_000;

fn merge_relay_nodes_into_coord_relays(
    coord_relays: &mut Vec<(PeerId, Multiaddr)>,
    nodes: &[(PeerId, Multiaddr)],
) {
    for (peer, ma) in nodes {
        if !coord_relays.iter().any(|(_, a)| a == ma) {
            native_log::info(
                "relay",
                format!("ghalbol relay {peer}: resolved dial addr {ma} (refresh)"),
            );
            coord_relays.push((*peer, ma.clone()));
        }
    }
}

/// Re-fetch `/v1/relay`, merge dial addrs, and dial + reserve on the co-located relay.
async fn maybe_refresh_ghalbol_relay(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &mut Vec<(PeerId, Multiaddr)>,
    force: bool,
) {
    if !crate::coord_runtime::wan_discovery_via_coord_only() {
        return;
    }
    let now = chrono_now_ms();
    if !force {
        let last = session
            .ghalbol_relay_last_fetch_ms
            .read()
            .ok()
            .map(|g| *g)
            .unwrap_or(0);
        let relay_connected = session
            .ghalbol_relay_state
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|(p, _)| *p))
            .is_some_and(|p| swarm.is_connected(&p));
        if relay_connected && crate::coord_runtime::coord_is_registered() {
            return;
        }
        let need_relay = coord_relays.is_empty()
            || !listen_ready_for_node(session, true, swarm)
            || !crate::coord_runtime::coord_is_registered();
        let min_gap = if need_relay {
            GHALBOL_RELAY_REFETCH_EMPTY_MS
        } else {
            GHALBOL_RELAY_REFETCH_MS
        };
        if now.saturating_sub(last) < min_gap {
            return;
        }
    }
    let all_relays =
        tokio::task::spawn_blocking(crate::coord_runtime::fetch_all_ghalbol_relays)
            .await
            .ok()
            .unwrap_or_default();
    if let Ok(mut g) = session.ghalbol_relay_last_fetch_ms.write() {
        *g = now;
    }
    if all_relays.is_empty() {
        native_log::warn(
            "relay",
            "GET /v1/relay returned no dialable relay — WAN unreachable until coord advertises \
             a relay circuit (coord server must expose GET /v1/relay with dialable addrs)",
        );
        return;
    }
    let mut merged_nodes: Vec<(PeerId, Multiaddr)> = Vec::new();
    for (peer_str, addrs) in &all_relays {
        let Ok(relay_peer) = peer_str.parse::<PeerId>() else {
            continue;
        };
        if let Ok(mut g) = session.bootstrap_peer_ids.write() {
            g.insert(relay_peer);
        }
        let nodes = crate::p2p::network_transport::resolve_relay_bootnodes(peer_str, addrs);
        if nodes.is_empty() {
            native_log::warn(
                "relay",
                format!("ghalbol relay {relay_peer} refetch: no dialable public addr yet"),
            );
            continue;
        }
        native_log::info(
            "relay",
            format!(
                "ghalbol relay {relay_peer}: {} dial addr(s) after refetch",
                nodes.len()
            ),
        );
        merge_relay_nodes_into_coord_relays(coord_relays, &nodes);
        merge_relay_nodes_into_coord_relays(&mut merged_nodes, &nodes);
    }
    if let Some((peer_str, addrs)) = all_relays.first() {
        if let Ok(relay_peer) = peer_str.parse::<PeerId>() {
            crate::coord_runtime::coord_note_relay_bootstrap_addrs(addrs);
            if let Ok(mut g) = session.ghalbol_relay_state.write() {
                *g = Some((relay_peer, addrs.clone()));
            }
        }
    }
    if merged_nodes.is_empty() {
        return;
    }
    ensure_coord_relays_connected(swarm, session, &merged_nodes);
    if relay_circuit_listening(swarm) {
        let bootstrap_ok = ghalbol_relay_peer(session)
            .map(|p| has_tracked_bootstrap_tcp(session, p) && swarm.is_connected(&p))
            .unwrap_or(false);
        if bootstrap_ok && crate::coord_runtime::coord_link_recently_ok() {
            return;
        }
    }
    let force_res = force || session.wan_recovery_active.load(Ordering::Relaxed);
    ensure_wan_relay_circuit(swarm, session, Some(&merged_nodes), force_res);
}

fn retry_stalled_relay_reservations(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    force: bool,
) {
    let _ = ensure_wan_relay_circuit(swarm, session, None, force);
}

fn dial_bootstrap_peers(
    swarm: &mut Swarm<ChatBehaviour>,
    peers: &[Multiaddr],
    emit: &mut dyn FnMut(GossipChatEvent),
) {
    let mut tcp_first: Vec<Multiaddr> = Vec::new();
    let mut other: Vec<Multiaddr> = Vec::new();
    for ma in peers {
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
        native_log::debug("dial", format!("bootstrap dial {ma}"));
        if let Err(e) = swarm.dial(ma.clone()) {
            native_log::warn("dial", format!("bootstrap dial failed {ma}: {e}"));
            emit(GossipChatEvent::DialFailed {
                peer: dial_opts_peer_hint(ma),
                error: format!("{e}"),
            });
        }
    }
}

#[cfg(all(not(target_os = "android"), not(feature = "test-minimal-swarm")))]
fn listen_ephemeral(swarm: &mut Swarm<ChatBehaviour>, ma: &str) -> Result<(), ChatServerError> {
    let parsed = parse_ma(ma)?;
    match swarm.listen_on(parsed) {
        Ok(_) => Ok(()),
        Err(e) => {
            native_log::warn("listen", format!("listen skipped ({ma}): {e}"));
            Ok(())
        }
    }
}

fn close_stale_lan_ephemeral_tcp_listeners(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
) {
    for id in session.drain_lan_ephemeral_tcp_listener_ids() {
        if swarm.remove_listener(id) {
            native_log::info(
                "listen",
                "closed stale LAN ephemeral TCP listener (handover)",
            );
        }
    }
}

fn listen_lan_ephemeral_tcp(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    handover: bool,
) -> Result<(), ChatServerError> {
    if handover {
        close_stale_lan_ephemeral_tcp_listeners(swarm, session);
    }
    let parsed = parse_ma("/ip4/0.0.0.0/tcp/0")?;
    match swarm.listen_on(parsed) {
        Ok(id) => {
            session.note_lan_ephemeral_tcp_listener(id);
            Ok(())
        }
        Err(e) => {
            native_log::warn("listen", format!("LAN ephemeral TCP listen skipped: {e}"));
            Ok(())
        }
    }
}

fn swarm_has_lan_dm_listen(swarm: &Swarm<ChatBehaviour>) -> bool {
    swarm.listeners().any(|ma| {
        crate::p2p::network_transport::is_dm_listen_tcp_multiaddr(ma)
            && crate::p2p::network_transport::ipv4_from_ma_str(&ma.to_string())
                .is_some_and(|ip| ip.is_private() && !ip.is_loopback())
    })
}

/// Any ephemeral DM TCP listen (`0.0.0.0` or RFC1918) — libp2p often reports `0.0.0.0` before if_addrs catches up.
fn swarm_has_ephemeral_dm_tcp_listen(swarm: &Swarm<ChatBehaviour>) -> bool {
    swarm.listeners().any(|ma| {
        crate::p2p::network_transport::is_dm_listen_tcp_multiaddr(ma)
            && !crate::p2p::network_transport::is_relay_circuit_multiaddr(ma)
    })
}

fn detected_network_with_platform_hints() -> crate::p2p::network_transport::LocalNetworkProfile {
    let mut p = crate::p2p::network_transport::detect_local_network_profile();
    if ANDROID_WIFI_TRANSPORT.load(Ordering::Relaxed) {
        p.has_wifi_iface = true;
        // if_addrs can lag after Wi‑Fi toggle; ConnectivityManager is authoritative.
        p.has_rfc1918_on_wifi = true;
    }
    #[cfg(target_os = "linux")]
    if crate::linux_network::wifi_oper_up() {
        p.has_wifi_iface = true;
    }
    p
}

fn network_profile_for_swarm(
    swarm: &Swarm<ChatBehaviour>,
    detected: crate::p2p::network_transport::LocalNetworkProfile,
) -> crate::p2p::network_transport::LocalNetworkProfile {
    crate::p2p::network_transport::effective_network_profile(
        detected.clone(),
        swarm_has_lan_dm_listen(swarm),
        platform_wifi_linked_from_profile(&detected),
    )
}

fn platform_wifi_linked_from_profile(
    profile: &crate::p2p::network_transport::LocalNetworkProfile,
) -> bool {
    profile.os.wifi_link_up
        || ANDROID_WIFI_TRANSPORT.load(Ordering::Relaxed)
        || profile.has_rfc1918_on_wifi
        || profile.has_wifi_iface
}

/// Wi‑Fi returned while profile still `mobile-data`, relay lost on LAN, or Wi‑Fi flap with DM down.
fn try_recover_lan_after_wifi_available(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    coord_relays: &[(PeerId, Multiaddr)],
    connectivity_notify: bool,
) -> bool {
    // OS connectivity notify: full cold-start LAN sequence — if_addrs/profile lag after toggle.
    if connectivity_notify && platform_wifi_linked(session) {
        kick_lan_dm_rediscovery_after_handover(
            swarm,
            session,
            "Wi‑Fi back (connectivity notify)",
            true,
        );
        return true;
    }
    let wifi_linked = wifi_lan_handover_active(session);
    let on_lan = session.network_profile_snapshot().has_active_lan();
    let needs_lan = session.any_dm_peer_needs_lan_rediscovery();
    let dm_down_on_lan = on_lan && needs_lan;
    // Parallel LAN+WAN: missing relay circuit must not re-kick LAN while WAN recovery is stuck
    // (dev coord/ngrok down would purge mDNS every 5s — TRANSPORT.md § Hybrid coord presence).
    let relay_lost_on_lan = on_lan
        && !relay_circuit_listening(swarm)
        && needs_lan
        && !session.wan_recovery_active.load(Ordering::Relaxed);
    // Android connectivity notify: Wi‑Fi is back but profile may still read mobile-data for ticks.
    let wifi_notify_handover = connectivity_notify && wifi_linked;
    if on_lan && !relay_lost_on_lan && !dm_down_on_lan && !wifi_notify_handover {
        return false;
    }
    if !wifi_linked && !connectivity_notify {
        return false;
    }
    // Mobile-data off/on: profile key often unchanged — LAN rediscovery is wrong here; WAN
    // recovery runs from the connectivity-notify branch in `network_tick`.
    if connectivity_notify && !wifi_linked && !on_lan {
        return false;
    }
    let now_ms = chrono_now_ms();
    if !session.should_run_lan_recovery(now_ms) {
        return false;
    }
    kick_lan_dm_rediscovery_after_handover(
        swarm,
        session,
        if connectivity_notify && !wifi_linked {
            "connectivity notify (Wi‑Fi transport)"
        } else if wifi_notify_handover && !on_lan {
            "Wi‑Fi linked (connectivity notify)"
        } else if !on_lan {
            "Wi‑Fi linked"
        } else if relay_lost_on_lan {
            "relay lost on LAN"
        } else {
            "Wi‑Fi flap — DM disconnected on LAN"
        },
        connectivity_notify,
    );
    if let Some((old_mode, new_mode)) = session.refresh_network_path_if_changed(swarm) {
        handle_network_path_change(swarm, session, coord_relays, &old_mode, &new_mode);
        return true;
    }
    wifi_notify_handover || !on_lan
}

fn restart_mdns_behaviour(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, force: bool) {
    let now_ms = chrono_now_ms();
    if force {
        if let Ok(mut last) = session.last_mdns_restart_ms.write() {
            *last = now_ms;
        }
    } else if !session.should_restart_mdns(now_ms) {
        return;
    }
    let local_peer_id = *swarm.local_peer_id();
    match libp2p::mdns::tokio::Behaviour::new(ghal_bol_mdns_config(), local_peer_id) {
        Ok(b) => {
            swarm.behaviour_mut().mdns = libp2p::swarm::behaviour::toggle::Toggle::from(Some(b));
            native_log::info("mdns", "restarted after LAN handover");
        }
        Err(e) => native_log::warn("mdns", format!("restart failed: {e}")),
    }
}

/// Re-open ephemeral LAN TCP after interface handover when only WAN listeners remain.
/// `handover`: Wi‑Fi flap — always bind a fresh port (old 0.0.0.0 listener is often stale).
fn ensure_lan_tcp_listen(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, handover: bool) {
    let detected = detected_network_with_platform_hints();
    let on_lan = session.network_profile_snapshot().has_active_lan()
        || detected.has_wifi_iface
        || detected.has_rfc1918_on_wifi
        || ANDROID_WIFI_TRANSPORT.load(Ordering::Relaxed);
    if !on_lan {
        return;
    }
    if handover || !swarm_has_ephemeral_dm_tcp_listen(swarm) {
        if listen_lan_ephemeral_tcp(swarm, session, handover).is_ok() {
            native_log::info("net", "LAN handover — fresh ephemeral TCP listen for mDNS");
        }
    }
}

/// Run queued full LAN kick after in-flight relay circuit dial completes (TRANSPORT.md § Deferred full LAN kick).
fn try_flush_pending_full_lan_kick(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    now_ms: i64,
) {
    if session.any_dm_circuit_dial_in_flight(now_ms) {
        return;
    }
    let Some(reason) = session.take_pending_full_lan_kick_reason() else {
        return;
    };
    native_log::info(
        "net",
        format!("LAN DM rediscovery — deferred full kick ({reason})"),
    );
    kick_lan_dm_rediscovery_after_handover(swarm, session, &reason, false);
}

/// On LAN: full handover when link down + no mDNS candidate (parallel with WAN recovery).
fn lan_handover_upkeep_if_needed(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState) {
    let now_ms = chrono_now_ms();
    try_flush_pending_full_lan_kick(swarm, session, now_ms);
    if !wifi_lan_handover_active(session) {
        return;
    }
    if session.wan_recovery_active.load(Ordering::Relaxed) && !relay_circuit_listening(swarm) {
        let _ = ensure_wan_relay_circuit(swarm, session, None, false);
    }
    let mut handover_reason = "link down, no mDNS candidate yet";
    let needs_mdns_nudge = session.dm_peer_ids().iter().any(|p| {
        if !session.should_dial_libp2p_peer(*p) {
            return false;
        }
        if !peer_eligible_for_lan_handover(session, *p) {
            return false;
        }
        // Missing chat stream while libp2p-connected is upkeep_dm_peers' job — not a LAN handover.
        if swarm.is_connected(p) {
            return false;
        }
        let stale_lan_candidate = session.peer_mdns_lan_addr(*p).is_some();
        if stale_lan_candidate {
            handover_reason = "link down, stale mDNS candidate";
        }
        session.peer_mdns_lan_addr(*p).is_none() || stale_lan_candidate
    });
    if needs_mdns_nudge {
        if session.any_dm_circuit_dial_in_flight(now_ms) {
            if swarm_has_ephemeral_dm_tcp_listen(swarm) {
                // Full kick closes/rebinds ephemeral TCP — defer until circuit dial finishes so we
                // do not destabilize an in-flight WAN handshake (TRANSPORT.md § Deferred full LAN kick).
                session.note_pending_full_lan_kick(handover_reason);
                if session.should_restart_mdns(now_ms) {
                    soft_lan_rediscovery_nudge(
                        swarm,
                        session,
                        "circuit dial in flight — defer full TCP rebind",
                    );
                } else {
                    notify_stream_reopen();
                    notify_dm_presence_wake();
                }
            } else {
                // Parallel LAN+WAN: mDNS cannot advertise without a TCP listener — bind one now
                // even while a relay circuit dial is in flight (do not block LAN on WAN).
                ensure_lan_tcp_listen(swarm, session, false);
                if session.should_restart_mdns(now_ms) {
                    soft_lan_rediscovery_nudge(swarm, session, handover_reason);
                } else {
                    notify_stream_reopen();
                    notify_dm_presence_wake();
                }
            }
        } else if swarm_has_ephemeral_dm_tcp_listen(swarm) {
            // A valid ephemeral LAN listener is already open — we are simply waiting for mDNS to
            // discover the peer on a steady network. Re-running the FULL kick here (fresh TCP port
            // + purge candidates + mDNS restart) every recovery tick changes our advertised mDNS
            // port and wipes discovered addrs, so discovery can never converge and no LAN link
            // ever forms (AGENTS.md § "mdns restarted … every ~5–12s, no mdns discovered";
            // TRANSPORT.md § "Ephemeral LAN TCP ports"). Discovery is event-driven: keep the port
            // stable and only nudge the mDNS query (throttled). The full destructive kick stays for
            // genuine handover triggers (connectivity notify / relay-lost / dial-path-failed) and
            // for the no-listener branch below.
            if session.should_restart_mdns(now_ms) {
                soft_lan_rediscovery_nudge(swarm, session, handover_reason);
            } else {
                notify_stream_reopen();
                notify_dm_presence_wake();
            }
        } else {
            // No ephemeral LAN listener (e.g. one was closed by a real interface handover) — bind a
            // fresh one once so mDNS has a reachable TCP port to advertise.
            kick_lan_dm_rediscovery_after_handover(swarm, session, handover_reason, false);
        }
        return;
    }
}

fn new_msg_id() -> String {
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}
