//! Human-readable connectivity diagnostics for App log / journalctl triage.
//!
//! Maps raw HTTP/libp2p errors to **why** WAN/LAN stalled and what must happen next
//! (see `docs/TRANSPORT.md` § End-to-end WAN phases).

/// Outcome bucket for a coord `GET /v1/peers/{pk}` attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordLookupCategory {
    Ok,
    PeerNotOnCoord,
    CoordHttpUnreachable,
    CoordHttpOther,
    NoDialableAddrs,
}

impl CoordLookupCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::PeerNotOnCoord => "peer_not_on_coord",
            Self::CoordHttpUnreachable => "coord_http_unreachable",
            Self::CoordHttpOther => "coord_http_error",
            Self::NoDialableAddrs => "peer_on_coord_no_dial_addrs",
        }
    }
}

pub fn classify_coord_lookup_error(err: &str) -> CoordLookupCategory {
    let e = err.to_ascii_lowercase();
    if e.contains("404") || e.contains("peer_not_on_server") || e.contains("not found") {
        return CoordLookupCategory::PeerNotOnCoord;
    }
    if e.contains("error sending request")
        || e.contains("connection refused")
        || e.contains("connection reset")
        || e.contains("timed out")
        || e.contains("timeout")
        || e.contains("dns")
        || e.contains("tls")
        || e.contains("certificate")
        || e.contains("unreachable")
    {
        return CoordLookupCategory::CoordHttpUnreachable;
    }
    CoordLookupCategory::CoordHttpOther
}

/// Explain a coord lookup failure for operators (not end users).
pub fn explain_coord_lookup_failure(
    category: CoordLookupCategory,
    self_coord_registered: bool,
) -> (&'static str, &'static str) {
    match category {
        CoordLookupCategory::PeerNotOnCoord => {
            let reason = if self_coord_registered {
                "remote peer has no presence record on coord (GET /v1/peers → 404)"
            } else {
                "remote peer not on coord; this device is also not registered yet"
            };
            let action = if self_coord_registered {
                "WAN blocked until remote completes: bootstrap TCP → reservation accepted → \
                 coord registered. libp2p relay v2 requires destination reservation before \
                 circuit dial (rust-libp2p #2513 NoReservation). Check remote App log for \
                 reservation/register — not opening chat room or sending new messages."
            } else {
                "Finish own WAN path first: relay circuit listening + coord register, then \
                 lookup retries automatically."
            };
            (reason, action)
        }
        CoordLookupCategory::CoordHttpUnreachable => (
            "coord HTTPS unreachable (register/lookup HTTP failed before response body)",
            "Check internet, VPN, DNS, firewall, coord.ghalbol.com/nginx — not a libp2p dial \
             failure. LAN/mDNS may still work for on-LAN peers.",
        ),
        CoordLookupCategory::NoDialableAddrs => (
            "peer registered on coord but returned zero TCP/relay circuit endpoints",
            "Remote registered LAN/CGNAT TCP only or stale record — remote needs relay \
             /p2p-circuit in POST /v1/register.",
        ),
        CoordLookupCategory::CoordHttpOther => (
            "coord HTTP error (non-404)",
            "Inspect full error string; may be auth, 5xx, or malformed response.",
        ),
        CoordLookupCategory::Ok => ("lookup ok", "dial in progress"),
    }
}

pub fn explain_coord_register_failure(err: &str, has_relay_endpoint: bool) -> (&'static str, &'static str) {
    if err.contains("no listen endpoints") {
        return (
            "coord register skipped — no publishable WAN endpoint yet",
            if has_relay_endpoint {
                "Internal endpoint build failed despite relay listener — check \
                 coord_register_listen_snapshot."
            } else {
                "Wait for reservation accepted + /p2p-circuit in swarm.listeners(), then \
                 register retries on coord_tick."
            },
        );
    }
    let cat = classify_coord_lookup_error(err);
    if cat == CoordLookupCategory::CoordHttpUnreachable {
        return (
            "coord register HTTP transport failed (challenge or POST never completed)",
            "Same as lookup unreachable — fix HTTPS path to coord before WAN peer discovery.",
        );
    }
    (
        "coord register rejected or incomplete",
        "Read error detail; register must succeed on at least one configured coord URL.",
    )
}

/// Why `try_routed_dial` has no addresses in peerstore yet.
pub fn explain_no_dial_addrs(
    last_lookup: Option<CoordLookupCategory>,
    peer_on_lan: bool,
    self_coord_registered: bool,
) -> (&'static str, &'static str) {
    match last_lookup {
        Some(CoordLookupCategory::PeerNotOnCoord) => (
            "no dial addrs — coord has no record for this peer yet",
            "Normal while remote is still on relay reservation/register. Outbox will send once \
             lookup returns /p2p-circuit. Do not require opening chat room or sending new text.",
        ),
        Some(CoordLookupCategory::CoordHttpUnreachable) => (
            "no dial addrs — coord lookup could not reach HTTP API",
            "Fix coord HTTPS; blind peer-id dial is disabled on mobile-data/CGNAT when coord is configured.",
        ),
        Some(CoordLookupCategory::NoDialableAddrs) => (
            "no dial addrs — peer on coord but no relay/tcp endpoints",
            "Remote must register libp2p /p2p-circuit endpoint.",
        ),
        Some(CoordLookupCategory::CoordHttpOther) => (
            "no dial addrs — last coord lookup failed (non-404)",
            "See prior coord log line for detail.",
        ),
        Some(CoordLookupCategory::Ok) | None => {
            if peer_on_lan {
                (
                    "no dial addrs — waiting for mDNS/identify LAN addresses",
                    "Peer marked on LAN; mDNS discovery or identify ingest should supply RFC1918 TCP.",
                )
            } else if !self_coord_registered {
                (
                    "no dial addrs — self not coord-registered; WAN lookup may be deferred",
                    "Complete own relay circuit + register first.",
                )
            } else {
                (
                    "no dial addrs — coord lookup not run yet or backoff active",
                    "dm_upkeep/coord_tick will lookup; pending outbox marks urgent reconnect.",
                )
            }
        }
    }
}

pub fn explain_outgoing_dial_error(err: &str) -> (&'static str, &'static str) {
    let e = err.to_ascii_lowercase();
    if e.contains("no reservation") || e.contains("noreservation") {
        return (
            "relay circuit dial rejected — destination not listening on relay",
            "Per libp2p relay v2 (#2513): callee must have active reservation on same relay. \
             If lookup was ok, remote reservation may have expired (bootstrap TCP dropped).",
        );
    }
    if e.contains("resourcelimitexceeded") || e.contains("resource limit") {
        return (
            "relay circuit denied — ResourceLimitExceeded on coord relay",
            "Server relay rate limiters or circuit pool full — redeploy ghal_bol_server with \
             cleared circuit_src_rate_limiters; client backs off 90s.",
        );
    }
    if e.contains("p2p-circuit") {
        return (
            "relay circuit dial failed",
            "Check server journal for circuit ACCEPTED/DENIED; verify /p2p-circuit multiaddr \
             from coord matches live reservation.",
        );
    }
    (
        "outgoing libp2p dial failed",
        "See error detail; may be transient (throttled, already dialing).",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_404_as_peer_not_on_coord() {
        assert_eq!(
            classify_coord_lookup_error("lookup HTTP 404 Not Found"),
            CoordLookupCategory::PeerNotOnCoord
        );
    }

    #[test]
    fn classifies_transport_as_unreachable() {
        assert_eq!(
            classify_coord_lookup_error("error sending request for url (https://coord.example/v1/peers/x)"),
            CoordLookupCategory::CoordHttpUnreachable
        );
    }
}
