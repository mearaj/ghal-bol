//! Process-wide coordination server URL, registration, and heartbeat loop.

use crate::coord::{CoordEndpoint, CoordHttpClient, endpoints_to_dial_multiaddr_strings};
use crate::dm_transport::{coord_endpoints_to_dial_addrs, DmDialAddr};
use libp2p::Multiaddr;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

const MAX_COORD_ENDPOINTS: usize = 16;

struct CoordGlobals {
    base_url: Mutex<Option<String>>,
    insecure_tls: AtomicBool,
    endpoints: Mutex<Vec<CoordEndpoint>>,
    /// Bootstrap relay we last reserved on (prefer matching circuit for coord register).
    preferred_relay_peer_id: Mutex<Option<String>>,
    heartbeat_stop: AtomicBool,
    heartbeat_join: Mutex<Option<JoinHandle<()>>>,
}


static COORD: OnceLock<CoordGlobals> = OnceLock::new();
/// Serializes challenge + register HTTP (parallel calls cause nonce mismatch on the server).
static COORD_REG_HTTP: OnceLock<Mutex<()>> = OnceLock::new();
static COORD_REG_WORKER_BUSY: AtomicBool = AtomicBool::new(false);
static COORD_REG_PENDING: AtomicBool = AtomicBool::new(false);
static COORD_REGISTERED: AtomicBool = AtomicBool::new(false);
static COORD_LAST_OK_MS: AtomicU64 = AtomicU64::new(0);
static COORD_LAST_REG_ATTEMPT_MS: AtomicU64 = AtomicU64::new(0);
/// Avoid flapping `coord_registered` on one transient mobile-data failure.
static COORD_CONSEC_FAILS: AtomicU64 = AtomicU64::new(0);

/// Backoff repeated coord peer lookups from the daemon/UI RPC path.
/// Key: peer public_key_hex.
static COORD_LOOKUP_BACKOFF: OnceLock<Mutex<HashMap<String, CoordLookupBackoff>>> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
struct CoordLookupBackoff {
    next_allowed_ms: u64,
    step_ms: u64,
}

fn lookup_backoff_map() -> &'static Mutex<HashMap<String, CoordLookupBackoff>> {
    COORD_LOOKUP_BACKOFF.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Min gap between full challenge+register cycles when already registered (reduces ngrok 401 storms).
const MIN_REGISTER_INTERVAL_MS: u64 = 10_000;
/// Retry sooner when never successfully registered.
const MIN_REGISTER_RETRY_MS: u64 = 2_000;
const HEARTBEAT_INTERVAL_SECS: u64 = 25;
/// If no successful heartbeat/self-lookup in this window, force re-register (server TTL ~90s).
const PRESENCE_STALE_MS: u64 = 70_000;

fn coord_reg_http_lock() -> &'static Mutex<()> {
    COORD_REG_HTTP.get_or_init(|| Mutex::new(()))
}

fn coord_globals() -> &'static CoordGlobals {
    COORD.get_or_init(|| CoordGlobals {
        base_url: Mutex::new(None),
        insecure_tls: AtomicBool::new(false),
        endpoints: Mutex::new(Vec::new()),
        preferred_relay_peer_id: Mutex::new(None),
        heartbeat_stop: AtomicBool::new(false),
        heartbeat_join: Mutex::new(None),
    })
}

pub(crate) fn has_coord_endpoints() -> bool {
    coord_globals()
        .endpoints
        .lock()
        .ok()
        .is_some_and(|v| !endpoints_for_coord_register(v.clone()).is_empty())
}

fn suspend_coord_presence_waiting_endpoints() {
    COORD_REGISTERED.store(false, Ordering::Relaxed);
    COORD_LAST_OK_MS.store(0, Ordering::Relaxed);
    // Stop heartbeats so stale coord presence naturally expires while waiting for
    // a relay/public listen endpoint.
    coord_globals()
        .heartbeat_stop
        .store(true, Ordering::Relaxed);
}

/// Called when libp2p accepts a relay reservation — register the matching circuit on coord.
pub fn coord_note_relay_reservation(relay_peer_id: libp2p::PeerId) {
    if !coord_is_configured() {
        return;
    }
    let relay = relay_peer_id.to_string();
    if let Ok(mut g) = coord_globals().preferred_relay_peer_id.lock() {
        *g = Some(relay);
    }
}

/// Coord URL is set — try server lookup first; DHT/mDNS still used when coord misses.
/// Public DHT bootnodes are always dialed so relay circuits can be reserved and registered.
pub fn wan_discovery_via_coord_only() -> bool {
    coord_is_configured()
}

/// Immediate coord re-register (e.g. right after relay reservation accepted).
pub fn schedule_register_presence_force() {
    spawn_register_presence_inner(true);
}

/// Drop coord registration state after a network handover (stale relay/LAN endpoints).
pub fn coord_invalidate_presence_on_network_change() {
    suspend_coord_presence_waiting_endpoints();
    if let Ok(mut v) = coord_globals().endpoints.lock() {
        v.clear();
    }
    if let Ok(mut g) = coord_globals().preferred_relay_peer_id.lock() {
        *g = None;
    }
}

/// True when a coordination server URL was configured (LAN/WAN endpoint lookup).
pub fn coord_is_configured() -> bool {
    coord_globals()
        .base_url
        .lock()
        .ok()
        .and_then(|u| u.as_ref().cloned())
        .is_some_and(|s| !s.is_empty())
}

/// True after a successful `POST /v1/peers/register` for this session.
pub fn coord_is_registered() -> bool {
    COORD_REGISTERED.load(Ordering::Relaxed)
}

/// Recent successful coord register, heartbeat, or self-lookup (internet likely up).
pub fn coord_link_recently_ok() -> bool {
    if !coord_is_configured() {
        return true;
    }
    if !COORD_REGISTERED.load(Ordering::Relaxed) {
        return false;
    }
    let last = COORD_LAST_OK_MS.load(Ordering::Relaxed);
    last > 0 && unix_ms_now().saturating_sub(last) < PRESENCE_STALE_MS
}

pub fn set_coord_base_url(url: &str, insecure_tls: bool) {
    let g = coord_globals();
    if let Ok(mut u) = g.base_url.lock() {
        *u = Some(url.trim().trim_end_matches('/').to_string());
    }
    g.insecure_tls.store(insecure_tls, Ordering::Relaxed);
    COORD_REGISTERED.store(false, Ordering::Relaxed);
    COORD_LAST_OK_MS.store(0, Ordering::Relaxed);
    crate::flow_log::info("coord", format!("base_url set to {}", url.trim()));
    schedule_register_presence();
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn client() -> Result<CoordHttpClient, String> {
    let g = coord_globals();
    let base = g
        .base_url
        .lock()
        .map_err(|_| "coord mutex poisoned")?
        .clone()
        .ok_or("coord base url not set")?;
    let tls = g.insecure_tls.load(Ordering::Relaxed);
    CoordHttpClient::new(&base, tls)
}

/// Host field for coord TCP endpoints — public routable IPv4 only (LAN uses mDNS, not coord).
fn is_coord_publishable_host(host: &str) -> bool {
    #[cfg(test)]
    if host.starts_with("127.") {
        return true;
    }
    if host.starts_with("127.") || host == "0.0.0.0" || host == "::" {
        return false;
    }
    if host.starts_with("169.254.") || host.starts_with("fe80:") {
        return false;
    }
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return crate::p2p::dht_bootstrap::is_public_bootstrap_ipv4(ip);
    }
    false
}

fn endpoint_key(ep: &CoordEndpoint) -> String {
    format!("{}:{}:{}", ep.scheme, ep.host, ep.port)
}

fn ipv4_field_rank(host: &str) -> u8 {
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        if ip.is_private() {
            if ip.octets()[0] == 10 || (ip.octets()[0] == 192 && ip.octets()[1] == 168) {
                return 3;
            }
            return 2;
        }
        return 0;
    }
    if host.starts_with("192.168.") || host.starts_with("10.") {
        3
    } else {
        1
    }
}

fn is_coord_lan_tcp_fallback(ma: &Multiaddr) -> bool {
    is_coord_presence_tcp_fallback(ma)
        && crate::p2p::dht_bootstrap::ipv4_from_ma_str(&ma.to_string())
            .is_some_and(|ip| ip.is_private() && !crate::p2p::dht_bootstrap::is_cgnat_ipv4(ip))
}

/// Any non-loopback DM TCP listen (RFC1918, CGNAT, or public) — coord presence when WAN TCP is absent.
fn is_coord_presence_tcp_fallback(ma: &Multiaddr) -> bool {
    if crate::p2p::dht_bootstrap::is_relay_circuit_multiaddr(ma) {
        return false;
    }
    let Some(ip) = crate::p2p::dht_bootstrap::ipv4_from_ma_str(&ma.to_string()) else {
        return false;
    };
    !ip.is_loopback() && crate::p2p::dht_bootstrap::is_dm_dial_multiaddr(ma)
}

fn listen_addrs_to_coord_endpoints(addrs: &[Multiaddr]) -> Vec<CoordEndpoint> {
    let has_relay = addrs
        .iter()
        .any(crate::p2p::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr);
    let mut eps = Vec::new();
    for ma in addrs {
        eps.extend(multiaddr_to_coord_endpoints(ma));
    }
    // With coord, only relay (or public TCP) is WAN-dialable — never CGNAT-only presence.
    if eps.is_empty() && !has_relay && !coord_is_configured() {
        for ma in addrs {
            if !is_coord_presence_tcp_fallback(ma) {
                continue;
            }
            // RFC1918 is LAN-only — mDNS, not coord (misleading for WAN peers).
            if let Some(ip) = crate::p2p::dht_bootstrap::ipv4_from_ma_str(&ma.to_string()) {
                if ip.is_private() && !crate::p2p::dht_bootstrap::is_cgnat_ipv4(ip) {
                    continue;
                }
            }
            if let Some(dm) = DmDialAddr::parse(&ma.to_string()) {
                eps.push(CoordEndpoint {
                    scheme: "tcp".into(),
                    host: dm.host,
                    port: dm.port,
                });
            }
        }
    }
    if has_relay {
        eps.retain(|e| {
            e.scheme == "libp2p"
                || (e.scheme == "tcp" && is_coord_publishable_host(&e.host))
        });
    }
    eps.sort_by_key(|e| {
        if e.scheme == "libp2p" {
            0u8
        } else {
            ipv4_field_rank(&e.host) + 1
        }
    });
    let mut seen = HashSet::new();
    eps.retain(|e| seen.insert(endpoint_key(e)));
    if eps.len() > MAX_COORD_ENDPOINTS {
        eps.truncate(MAX_COORD_ENDPOINTS);
    }
    eps
}

fn should_throttle_register() -> bool {
    let now = unix_ms_now();
    let last = COORD_LAST_REG_ATTEMPT_MS.load(Ordering::Relaxed);
    let min_gap = if COORD_REGISTERED.load(Ordering::Relaxed) {
        MIN_REGISTER_INTERVAL_MS
    } else {
        MIN_REGISTER_RETRY_MS
    };
    now.saturating_sub(last) < min_gap
}

fn spawn_register_presence() {
    spawn_register_presence_inner(false);
}

/// `force` bypasses throttle (endpoint set changed while already registered).
fn spawn_register_presence_inner(force: bool) {
    if !force && should_throttle_register() {
        if !COORD_REGISTERED.load(Ordering::Relaxed) {
            COORD_REG_PENDING.store(true, Ordering::Release);
        }
        return;
    }
    if COORD_REG_WORKER_BUSY.load(Ordering::Acquire) {
        COORD_REG_PENDING.store(true, Ordering::Release);
        return;
    }
    if COORD_REG_WORKER_BUSY
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        COORD_REG_PENDING.store(true, Ordering::Release);
        return;
    }
    COORD_LAST_REG_ATTEMPT_MS.store(unix_ms_now(), Ordering::Relaxed);
    if std::thread::Builder::new()
        .name("ghalbol-coord-reg".into())
        .spawn(|| {
            struct BusyGuard;
            impl Drop for BusyGuard {
                fn drop(&mut self) {
                    COORD_REG_WORKER_BUSY.store(false, Ordering::Release);
                    if COORD_REG_PENDING.swap(false, Ordering::AcqRel)
                        && !COORD_REGISTERED.load(Ordering::Relaxed)
                    {
                        spawn_register_presence_inner(false);
                    }
                }
            }
            let _busy = BusyGuard;
            if let Err(e) = try_register_presence() {
                crate::flow_log::warn("coord", format!("register: {e}"));
            }
        })
        .is_err()
    {
        COORD_REG_WORKER_BUSY.store(false, Ordering::Release);
    }
}

/// Safe from the tokio P2P task (blocking HTTP on a std thread).
pub fn schedule_register_presence() {
    spawn_register_presence();
}

fn multiaddr_to_coord_endpoints(ma: &Multiaddr) -> Vec<CoordEndpoint> {
    if crate::p2p::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr(ma) {
        return vec![CoordEndpoint {
            scheme: "libp2p".into(),
            host: ma.to_string(),
            port: 0,
        }];
    }
    if !crate::p2p::dht_bootstrap::is_coord_register_tcp_multiaddr(ma) {
        return Vec::new();
    }
    if let Some(dm) = DmDialAddr::parse(&ma.to_string()) {
        return vec![CoordEndpoint {
            scheme: "tcp".into(),
            host: dm.host,
            port: dm.port,
        }];
    }
    Vec::new()
}

/// Rebuild coord registration from the current libp2p publishable listen set (relay + TCP).
/// Call with [`SessionState::published_listen_snapshot`] after listen addrs change.
pub fn rebuild_coord_endpoints_from_listen(addrs: &[Multiaddr]) {
    if !coord_is_configured() {
        return;
    }
    let has_relay = addrs
        .iter()
        .any(crate::p2p::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr);
    let publishable: Vec<Multiaddr> = addrs
        .iter()
        .filter(|ma| {
            if is_coord_lan_tcp_fallback(ma) {
                return false;
            }
            if has_relay || coord_is_configured() {
                return crate::p2p::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr(ma)
                    || crate::p2p::dht_bootstrap::is_coord_register_tcp_multiaddr(ma);
            }
            crate::p2p::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr(ma)
                || crate::p2p::dht_bootstrap::is_coord_register_tcp_multiaddr(ma)
                || is_coord_presence_tcp_fallback(ma)
        })
        .cloned()
        .collect();
    let eps = listen_addrs_to_coord_endpoints(if publishable.is_empty() {
        addrs
    } else {
        &publishable
    });
    if eps.is_empty() {
        let cgnat_only = !has_relay
            && addrs.iter().any(|ma| {
                crate::p2p::dht_bootstrap::ipv4_from_ma_str(&ma.to_string())
                    .is_some_and(crate::p2p::dht_bootstrap::is_cgnat_ipv4)
            });
        let g = coord_globals();
        let mut cleared = false;
        if let Ok(mut v) = g.endpoints.lock() {
            if !v.is_empty() {
                v.clear();
                cleared = true;
            }
        }
        let was_registered = COORD_REGISTERED.load(Ordering::Relaxed);
        if cleared || was_registered {
            suspend_coord_presence_waiting_endpoints();
            if cgnat_only {
                crate::flow_log::info(
                    "coord",
                    "CGNAT listen addr only — not WAN-dialable; waiting for libp2p relay circuit",
                );
            } else {
                crate::flow_log::warn(
                    "coord",
                    "waiting for relay/public listen endpoint before coord register",
                );
            }
        }
        return;
    }
    let g = coord_globals();
    let Ok(mut v) = g.endpoints.lock() else {
        return;
    };
    let mut old_keys: Vec<String> = v.iter().map(endpoint_key).collect();
    let mut new_keys: Vec<String> = eps.iter().map(endpoint_key).collect();
    old_keys.sort();
    new_keys.sort();
    if old_keys == new_keys {
        if !COORD_REGISTERED.load(Ordering::Relaxed) {
            spawn_register_presence();
        }
        return;
    }
    *v = eps;
    // Keep heartbeats running until the new register succeeds (avoids GET 404 gaps).
    spawn_register_presence_inner(true);
}

fn presence_is_stale() -> bool {
    if !COORD_REGISTERED.load(Ordering::Relaxed) {
        return false;
    }
    let last = COORD_LAST_OK_MS.load(Ordering::Relaxed);
    last == 0 || unix_ms_now().saturating_sub(last) >= PRESENCE_STALE_MS
}

/// Retry coord register from libp2p listen snapshot until HTTP register + lookup succeed.
pub fn coord_register_tick(listen_snapshot: &[Multiaddr]) {
    if !coord_is_configured() {
        return;
    }
    if presence_is_stale() {
        crate::flow_log::warn("coord", "presence stale (no recent heartbeat) — re-registering");
        COORD_REGISTERED.store(false, Ordering::Relaxed);
    }
    rebuild_coord_endpoints_from_listen(listen_snapshot);
    if !has_coord_endpoints() {
        return;
    }
    if !COORD_REGISTERED.load(Ordering::Relaxed) {
        spawn_register_presence_inner(false);
    }
}

/// Push libp2p listen addresses into coord registration (full snapshot preferred).
pub fn sync_published_listen_addrs(addrs: &[Multiaddr]) {
    rebuild_coord_endpoints_from_listen(addrs);
}

fn dial_addr_to_endpoints(addr: &DmDialAddr) -> Vec<CoordEndpoint> {
    if !is_coord_publishable_host(addr.host.trim()) {
        return Vec::new();
    }
    vec![CoordEndpoint {
        scheme: "tcp".into(),
        host: addr.host.clone(),
        port: addr.port,
    }]
}

/// Legacy DM poll listen hook — merges one addr then rebuilds if the set changed.
pub fn on_listen_dm_addr(addr: &DmDialAddr) {
    let eps = dial_addr_to_endpoints(addr);
    if eps.is_empty() {
        return;
    }
    let g = coord_globals();
    let Ok(mut v) = g.endpoints.lock() else {
        return;
    };
    let before = v.len();
    for ep in eps {
        let key = endpoint_key(&ep);
        if v.iter().any(|e| endpoint_key(e) == key) {
            continue;
        }
        v.push(ep);
    }
    if v.len() > before {
        spawn_register_presence();
    }
}

fn coord_endpoints_to_dial_multiaddrs(endpoints: &[CoordEndpoint]) -> Vec<Multiaddr> {
    let mut out = Vec::new();
    // Relay circuits first (WAN/NAT); then direct TCP (LAN or UPnP).
    for ep in endpoints {
        if ep.scheme != "libp2p" {
            continue;
        }
        if let Ok(ma) = ep.host.trim().parse::<Multiaddr>() {
            if crate::p2p::dht_bootstrap::is_coord_relay_tcp_circuit_multiaddr(&ma) {
                out.push(ma);
            }
        }
    }
    for ep in endpoints {
        let scheme = ep.scheme.as_str();
        #[cfg(target_os = "android")]
        if scheme == "quic" {
            continue;
        }
        if scheme != "tcp" && scheme != "quic" {
            continue;
        }
        if ep.port == 0 || ep.host.trim().is_empty() {
            continue;
        }
        if ep.port == 4001 {
            continue;
        }
        let dm = DmDialAddr::new(ep.host.clone(), ep.port);
        if let Ok(ma) = dm.to_multiaddr_string().parse::<Multiaddr>() {
            if crate::p2p::dht_bootstrap::is_dm_dial_multiaddr(&ma) {
                out.push(ma);
            }
        }
    }
    let out = crate::p2p::dht_bootstrap::filter_coord_dial_addrs(out);
    crate::p2p::dht_bootstrap::filter_coord_relay_dial_platform(out)
}

fn pick_coord_libp2p_endpoints(eps: Vec<CoordEndpoint>) -> Vec<CoordEndpoint> {
    let preferred = coord_globals()
        .preferred_relay_peer_id
        .lock()
        .ok()
        .and_then(|g| g.clone());
    let mut libp2p: Vec<CoordEndpoint> = eps
        .into_iter()
        .filter(|e| e.scheme == "libp2p")
        .collect();
    libp2p.sort_by_key(|e| {
        let h = e.host.as_str();
        let rank = if h.contains("/ip4/") && h.contains("/tcp/") && h.contains("/p2p-circuit") && !h.contains("/quic")
        {
            0u8
        } else {
            2u8
        };
        let pref = preferred
            .as_ref()
            .map(|relay| h.contains(relay.as_str()))
            .unwrap_or(false);
        (if pref { 0 } else { 1 }, rank)
    });
    if libp2p.is_empty() {
        return libp2p;
    }
    vec![libp2p.remove(0)]
}

/// When a relay circuit is available, register only `libp2p` endpoints (not CGNAT/LAN TCP).
fn endpoints_for_coord_register(eps: Vec<CoordEndpoint>) -> Vec<CoordEndpoint> {
    if eps.iter().any(|e| e.scheme == "libp2p") {
        pick_coord_libp2p_endpoints(eps)
    } else if coord_is_configured() {
        // CGNAT/LAN TCP is not dialable over WAN; wait for relay reservation.
        Vec::new()
    } else {
        eps
    }
}

fn try_register_presence() -> Result<(), String> {
    let _http = coord_reg_http_lock()
        .lock()
        .map_err(|_| "coord register http mutex poisoned")?;
    let ident = match crate::session_runtime::unlocked_identity_clone() {
        Ok(i) => i,
        Err(_) => return Err("identity not unlocked".into()),
    };
    let pk = ident.public_key_hex();
    let secret = ident.secp256k1_secret().clone();
    // Re-read under the register lock so a slow thread cannot publish stale CGNAT after relay.
    let endpoints = endpoints_for_coord_register(
        coord_globals()
            .endpoints
            .lock()
            .map_err(|_| "coord mutex poisoned")?
            .clone(),
    );
    if endpoints.is_empty() {
        return Err("no listen endpoints for coord register yet".into());
    }
    let ipv4 = endpoints
        .iter()
        .filter(|e| e.scheme == "tcp" && !e.host.contains(':'))
        .min_by_key(|e| ipv4_field_rank(&e.host))
        .map(|e| e.host.as_str())
        .map(str::to_string);
    let ipv6 = endpoints
        .iter()
        .filter(|e| e.scheme == "tcp" && e.host.contains(':'))
        .min_by_key(|e| ipv4_field_rank(&e.host))
        .map(|e| e.host.as_str())
        .map(str::to_string);

    let client = client()?;
    client.register(
        &secret,
        &pk,
        &endpoints,
        ipv4.as_deref(),
        ipv6.as_deref(),
    )?;
    client
        .lookup(&pk)
        .map_err(|e| format!("register HTTP ok but GET /v1/peers failed: {e}"))?;
    let ep_summary: String = endpoints
        .iter()
        .map(|e| {
            if e.scheme == "libp2p" {
                format!("libp2p:{}", &e.host[..e.host.len().min(48)])
            } else {
                format!("{}://{}:{}", e.scheme, e.host, e.port)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    crate::flow_log::info(
        "coord",
        format!(
            "registered {} endpoint(s) for {} [{}]",
            endpoints.len(),
            &pk[..8.min(pk.len())],
            ep_summary
        ),
    );
    COORD_REGISTERED.store(true, Ordering::Relaxed);
    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
    COORD_LAST_OK_MS.store(unix_ms_now(), Ordering::Relaxed);
    start_heartbeat_loop(pk);
    Ok(())
}

fn start_heartbeat_loop(public_key_hex: String) {
    let g = coord_globals();
    g.heartbeat_stop.store(false, Ordering::Relaxed);
    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
    if let Ok(mut j) = g.heartbeat_join.lock() {
        if let Some(h) = j.take() {
            let _ = h.join();
        }
    }
    let stop = &g.heartbeat_stop;
    let handle = std::thread::Builder::new()
        .name("ghalbol-coord-heartbeat".into())
        .spawn(move || {
            loop {
                for _ in 0..HEARTBEAT_INTERVAL_SECS {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                let Ok(c) = client() else {
                    continue;
                };
                if !COORD_REGISTERED.load(Ordering::Relaxed) {
                    spawn_register_presence_inner(true);
                    continue;
                }
                if c.heartbeat(&public_key_hex).is_err() {
                    crate::flow_log::warn(
                        "coord",
                        format!(
                            "heartbeat failed for {} — scheduling re-register",
                            &public_key_hex[..8.min(public_key_hex.len())]
                        ),
                    );
                    let fails = COORD_CONSEC_FAILS.fetch_add(1, Ordering::Relaxed) + 1;
                    // Keep `coord_registered=true` through brief cellular route changes.
                    // Clearing it immediately makes the peer vanish and leads to 404 lookups.
                    if fails >= 3 {
                        COORD_REGISTERED.store(false, Ordering::Relaxed);
                    }
                    spawn_register_presence_inner(true);
                } else if c.lookup(&public_key_hex).is_err() {
                    crate::flow_log::warn(
                        "coord",
                        format!(
                            "GET /v1/peers/{} failed after heartbeat — re-registering",
                            &public_key_hex[..8.min(public_key_hex.len())]
                        ),
                    );
                    let fails = COORD_CONSEC_FAILS.fetch_add(1, Ordering::Relaxed) + 1;
                    if fails >= 3 {
                        COORD_REGISTERED.store(false, Ordering::Relaxed);
                    }
                    spawn_register_presence_inner(true);
                } else {
                    COORD_LAST_OK_MS.store(unix_ms_now(), Ordering::Relaxed);
                    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
                }
            }
        })
        .ok();
    if let Ok(mut j) = g.heartbeat_join.lock() {
        *j = handle;
    }
}

pub fn stop_coord_presence() {
    COORD_REGISTERED.store(false, Ordering::Relaxed);
    COORD_LAST_OK_MS.store(0, Ordering::Relaxed);
    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
    let g = coord_globals();
    g.heartbeat_stop.store(true, Ordering::Relaxed);
    if let Ok(mut j) = g.heartbeat_join.lock() {
        if let Some(h) = j.take() {
            let _ = h.join();
        }
    }
}

pub fn lookup_bootstrap_multiaddrs(public_key_hex: &str) -> Result<Vec<String>, String> {
    let client = client()?;
    let record = client.lookup(public_key_hex)?;
    let mut addrs = endpoints_to_dial_multiaddr_strings(&record.endpoints);
    for ma in coord_endpoints_to_dial_multiaddrs(&record.endpoints) {
        addrs.push(ma.to_string());
    }
    let mut seen = HashSet::new();
    addrs.retain(|a| seen.insert(a.clone()));
    Ok(addrs)
}

pub fn coord_lookup_peer_json(public_key_hex: &str) -> serde_json::Value {
    let pk = public_key_hex.trim();
    let now = unix_ms_now();
    if let Ok(m) = lookup_backoff_map().lock() {
        if let Some(b) = m.get(pk) {
            if now < b.next_allowed_ms {
                return serde_json::json!({
                    "ok": false,
                    "error": format!("lookup backoff (retry in {}ms)", b.next_allowed_ms.saturating_sub(now)),
                    "backoff_ms": b.next_allowed_ms.saturating_sub(now),
                });
            }
        }
    }
    match lookup_bootstrap_multiaddrs(pk) {
        Ok(addrs) => {
            if let Ok(mut m) = lookup_backoff_map().lock() {
                m.remove(pk);
            }
            serde_json::json!({
                "ok": true,
                "bootstrap_peers": addrs,
            })
        }
        Err(e) => {
            // Exponential backoff on 404/not-found; shorter backoff on transient reachability errors.
            let es = e.to_string();
            let mut step = if es.contains("404") || es.contains("peer_not_on_server") {
                2_000u64
            } else {
                5_000u64
            };
            let mut next = now.saturating_add(step);
            if let Ok(mut m) = lookup_backoff_map().lock() {
                if let Some(prev) = m.get(pk).copied() {
                    step = (prev.step_ms.saturating_mul(2)).min(30_000);
                    next = now.saturating_add(step);
                }
                m.insert(
                    pk.to_string(),
                    CoordLookupBackoff {
                        next_allowed_ms: next,
                        step_ms: step,
                    },
                );
            }
            serde_json::json!({ "ok": false, "error": e })
        }
    }
}

pub fn coord_set_base_url_json(url: &str, insecure_tls: bool) -> serde_json::Value {
    if url.trim().is_empty() {
        return serde_json::json!({ "ok": false, "error": "url empty" });
    }
    set_coord_base_url(url, insecure_tls);
    let cfg = crate::app_paths::storage_config_for_namespace(crate::ANDROID_LIBRARY_NAMESPACE);
    if let Err(e) = crate::preferences_v1::coord_settings_set(&cfg, url, insecure_tls) {
        return serde_json::json!({ "ok": false, "error": format!("{e}") });
    }
    serde_json::json!({ "ok": true, "base_url": url.trim() })
}

/// Non-blocking: schedules HTTP register on a background thread. Never call
/// [`try_register_presence`] from the daemon/UI RPC socket thread (blocks ~seconds).
pub fn coord_register_now_json() -> serde_json::Value {
    if !coord_is_configured() {
        return serde_json::json!({ "ok": false, "error": "coord base url not set" });
    }
    if crate::session_runtime::unlocked_identity_clone().is_err() {
        return serde_json::json!({ "ok": false, "error": "identity not unlocked" });
    }
    let endpoints = match coord_globals().endpoints.lock() {
        Ok(v) => v.clone(),
        Err(_) => {
            return serde_json::json!({ "ok": false, "error": "coord mutex poisoned" });
        }
    };
    if endpoints.is_empty() {
        return serde_json::json!({
            "ok": false,
            "error": "no listen endpoints for coord register yet",
        });
    }
    if COORD_REGISTERED.load(Ordering::Relaxed) {
        return serde_json::json!({ "ok": true, "already_registered": true });
    }
    schedule_register_presence();
    serde_json::json!({ "ok": true, "scheduled": true })
}

pub fn coord_client_for_lookup() -> Result<CoordHttpClient, String> {
    client()
}

pub fn lookup_dial_addrs_for_public_key(public_key_hex: &str) -> Result<Vec<DmDialAddr>, String> {
    let client = client()?;
    let record = client.lookup(public_key_hex)?;
    Ok(coord_endpoints_to_dial_addrs(&record.endpoints))
}

/// Coord lookup → ranked/filtered libp2p multiaddrs (relay + public TCP before RFC1918).
pub fn lookup_dial_multiaddrs_for_public_key(public_key_hex: &str) -> Result<Vec<Multiaddr>, String> {
    let client = client()?;
    let record = client.lookup(public_key_hex)?;
    Ok(coord_endpoints_to_dial_multiaddrs(&record.endpoints))
}

/// [`lookup_dial_multiaddrs_for_public_key`] on the blocking thread pool (safe from tokio tasks).
pub async fn lookup_dial_multiaddrs_for_public_key_async(
    public_key_hex: &str,
) -> Result<Vec<Multiaddr>, String> {
    let pk = public_key_hex.trim().to_string();
    tokio::task::spawn_blocking(move || lookup_dial_multiaddrs_for_public_key(&pk))
        .await
        .map_err(|e| format!("coord lookup join: {e}"))?
}

/// [`lookup_dial_addrs_for_public_key`] on the blocking thread pool (safe from tokio tasks).
pub async fn lookup_dial_addrs_for_public_key_async(
    public_key_hex: &str,
) -> Result<Vec<DmDialAddr>, String> {
    let pk = public_key_hex.trim().to_string();
    tokio::task::spawn_blocking(move || lookup_dial_addrs_for_public_key(&pk))
        .await
        .map_err(|e| format!("coord lookup join: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static COORD_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    /// Serializes tests that touch process-wide coord globals.
    fn coord_test_setup() -> std::sync::MutexGuard<'static, ()> {
        let guard = COORD_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_coord_globals_for_test();
        guard
    }

    fn reset_coord_globals_for_test() {
        let g = coord_globals();
        if let Ok(mut u) = g.base_url.lock() {
            *u = None;
        }
        g.insecure_tls.store(false, Ordering::Relaxed);
        if let Ok(mut e) = g.endpoints.lock() {
            e.clear();
        }
        if let Ok(mut r) = g.preferred_relay_peer_id.lock() {
            *r = None;
        }
        COORD_REGISTERED.store(false, Ordering::Relaxed);
        COORD_LAST_OK_MS.store(0, Ordering::Relaxed);
        COORD_LAST_REG_ATTEMPT_MS.store(0, Ordering::Relaxed);
        COORD_REG_PENDING.store(false, Ordering::Relaxed);
        COORD_REG_WORKER_BUSY.store(false, Ordering::Relaxed);
    }

    #[test]
    fn wan_filter_skips_lan_when_public_present() {
        let lan: Multiaddr = "/ip4/192.168.1.2/tcp/4001".parse().unwrap();
        let wan: Multiaddr = "/ip4/8.8.8.8/tcp/4001".parse().unwrap();
        let out = crate::p2p::dht_bootstrap::filter_wan_preferred_dm_dial_addrs(vec![lan.clone(), wan.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], wan);
    }

    #[test]
    fn coord_register_skips_rfc1918_tcp() {
        let lan: Multiaddr = "/ip4/192.168.1.38/tcp/38505".parse().unwrap();
        assert!(!crate::p2p::dht_bootstrap::is_coord_register_tcp_multiaddr(&lan));
        let out = crate::p2p::dht_bootstrap::filter_coord_dial_addrs(vec![lan]);
        assert!(out.is_empty());
    }

    #[test]
    fn coord_dial_skips_lan_and_cgnat() {
        let lan: Multiaddr = "/ip4/192.168.1.38/tcp/38505".parse().unwrap();
        let cgnat: Multiaddr = "/ip4/100.73.97.100/tcp/33881".parse().unwrap();
        let out = crate::p2p::dht_bootstrap::filter_coord_dial_addrs(vec![lan, cgnat]);
        assert!(out.is_empty());
    }

    #[test]
    fn coord_skips_cgnat_only_when_coord_url_set() {
        let _guard = coord_test_setup();
        set_coord_base_url("https://example.test", false);
        let cgnat: Multiaddr = "/ip4/100.104.255.165/tcp/40993".parse().unwrap();
        let eps = listen_addrs_to_coord_endpoints(&[cgnat]);
        assert!(eps.is_empty());
        let reg = endpoints_for_coord_register(vec![CoordEndpoint {
            scheme: "tcp".into(),
            host: "100.104.255.165".into(),
            port: 40993,
        }]);
        assert!(reg.is_empty());
    }

    #[test]
    fn pick_coord_prefers_active_relay_reservation() {
        let _guard = coord_test_setup();
        let relay_a = "QmRelayA";
        let relay_b = "QmRelayB";
        if let Ok(mut g) = coord_globals().preferred_relay_peer_id.lock() {
            *g = Some(relay_a.to_string());
        }
        let eps = pick_coord_libp2p_endpoints(vec![
            CoordEndpoint {
                scheme: "libp2p".into(),
                host: format!(
                    "/ip4/54.38.47.166/tcp/4001/p2p/{relay_b}/p2p-circuit/p2p/16Uiu2HAm699T"
                ),
                port: 0,
            },
            CoordEndpoint {
                scheme: "libp2p".into(),
                host: format!(
                    "/ip4/51.81.93.51/tcp/4001/p2p/{relay_a}/p2p-circuit/p2p/16Uiu2HAm699T"
                ),
                port: 0,
            },
        ]);
        assert_eq!(eps.len(), 1);
        assert!(eps[0].host.contains(relay_a));
    }

    #[test]
    fn register_payload_keeps_one_ipv4_libp2p() {
        let _guard = coord_test_setup();
        let eps = endpoints_for_coord_register(vec![
            CoordEndpoint {
                scheme: "tcp".into(),
                host: "100.104.255.165".into(),
                port: 40569,
            },
            CoordEndpoint {
                scheme: "libp2p".into(),
                host: "/ip6/2001:41d0:203:2ca6::/tcp/4001/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb/p2p-circuit/p2p/16Uiu2HAm699TtKnm9LHXoS6MbVp8ehX7U8hyomVhivz9KuVKsYis".into(),
                port: 0,
            },
            CoordEndpoint {
                scheme: "libp2p".into(),
                host: "/ip4/54.38.47.166/tcp/4001/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb/p2p-circuit/p2p/16Uiu2HAm699TtKnm9LHXoS6MbVp8ehX7U8hyomVhivz9KuVKsYis".into(),
                port: 0,
            },
        ]);
        assert_eq!(eps.len(), 1);
        assert!(eps[0].host.contains("/ip4/54.38.47.166"));
    }

    #[test]
    fn coord_register_prefers_relay_tcp_over_lan() {
        let _guard = coord_test_setup();
        let lan: Multiaddr = "/ip4/192.168.1.38/tcp/38505".parse().unwrap();
        let relay: Multiaddr = "/ip4/51.81.93.51/tcp/4001/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa/p2p-circuit/p2p/16Uiu2HAm699TtKnm9LHXoS6MbVp8ehX7U8hyomVhivz9KuVKsYis"
            .parse()
            .unwrap();
        let eps = listen_addrs_to_coord_endpoints(&[lan, relay.clone()]);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].scheme, "libp2p");
        assert_eq!(eps[0].host, relay.to_string());
    }

    #[test]
    fn coord_presence_includes_cgnat_tcp() {
        let _guard = coord_test_setup();
        let cgnat: Multiaddr = "/ip4/100.64.1.2/tcp/38505".parse().unwrap();
        assert!(is_coord_presence_tcp_fallback(&cgnat));
        let eps = listen_addrs_to_coord_endpoints(&[cgnat]);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].host, "100.64.1.2");
        assert_eq!(eps[0].port, 38505);
    }

    #[test]
    fn rebuild_clears_stale_coord_presence_when_no_registerable_endpoints() {
        let _guard = coord_test_setup();
        let g = coord_globals();
        if let Ok(mut u) = g.base_url.lock() {
            *u = Some("https://example.test".into());
        }
        if let Ok(mut e) = g.endpoints.lock() {
            e.push(CoordEndpoint {
                scheme: "libp2p".into(),
                host: "/ip4/51.81.93.51/tcp/4001/p2p/QmRelay/p2p-circuit/p2p/16Uiu2HAmTest"
                    .into(),
                port: 0,
            });
        }
        COORD_REGISTERED.store(true, Ordering::Relaxed);
        g.heartbeat_stop.store(false, Ordering::Relaxed);

        let cgnat: Multiaddr = "/ip4/100.104.255.165/tcp/40993".parse().unwrap();
        rebuild_coord_endpoints_from_listen(&[cgnat]);

        let endpoints = g.endpoints.lock().unwrap().clone();
        assert!(endpoints.is_empty());
        assert!(!COORD_REGISTERED.load(Ordering::Relaxed));
        assert!(g.heartbeat_stop.load(Ordering::Relaxed));
    }
}