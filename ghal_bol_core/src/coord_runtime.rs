//! Process-wide coordination server URL, registration, and heartbeat loop.

use crate::coord::{CoordEndpoint, CoordHttpClient};
use crate::dm_transport::{DmDialAddr, coord_endpoints_to_dial_addrs};
use crate::multiaddr_local::Multiaddr;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

const MAX_COORD_ENDPOINTS: usize = 16;
/// Max coord/relay servers in the configured list (TRANSPORT.md).
const MAX_COORD_SERVERS: usize = 8;

struct CoordGlobals {
    base_urls: Mutex<Vec<String>>,
    insecure_tls: AtomicBool,
    endpoints: Mutex<Vec<CoordEndpoint>>,
    /// Coord servers where the last register+heartbeat cycle succeeded.
    registered_on: Mutex<HashSet<String>>,
    /// Bootstrap relay we last reserved on (prefer matching circuit for coord register).
    preferred_relay_peer_id: Mutex<Option<String>>,
    /// libp2p local peer id — coord POST TCP must terminate on this id.
    local_peer_id: Mutex<Option<String>>,
    /// `host:port` keys from `GET /v1/relay` — never POST these as peer TCP.
    relay_bootstrap_tcp: Mutex<HashSet<String>>,
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
#[cfg(not(test))]
static RELAY_PRESENCE_CHECK_PENDING: AtomicBool = AtomicBool::new(false);
#[cfg(not(test))]
static RELAY_PRESENCE_POLL_LAST_END_MS: AtomicU64 = AtomicU64::new(0);

/// Fast poll after reservation (coord HTTP flap ~seconds); slow poll until mirror or circuit down.
#[cfg(not(test))]
const RELAY_PRESENCE_POLL_FAST_MS: u64 = 500;
#[cfg(not(test))]
const RELAY_PRESENCE_POLL_FAST_ATTEMPTS: u32 = 12;
#[cfg(not(test))]
const RELAY_PRESENCE_POLL_SLOW_MS: u64 = 2_000;
#[cfg(not(test))]
const RELAY_PRESENCE_POLL_SLOW_ATTEMPTS: u32 = 30;
const RELAY_PRESENCE_RESCHEDULE_MIN_MS: u64 = 5_000;
/// When already registered, relay self-poll is guardrail-only — do not spawn every coord_tick.
const RELAY_PRESENCE_RESCHEDULE_REGISTERED_MS: u64 = 30_000;
/// Cap recover_coord / degraded-tick relay polls during coord HTTP flap (TRANSPORT.md § storm throttle).
const COORD_RECOVERY_MIN_MS: u64 = 15_000;
/// Throttle repeated heartbeat-failure warnings when many threads overlap during recovery.
const COORD_HEARTBEAT_FAIL_LOG_MIN_MS: u64 = 30_000;

static COORD_RECOVERY_LAST_MS: AtomicU64 = AtomicU64::new(0);
static COORD_HEARTBEAT_FAIL_LOG_LAST_MS: AtomicU64 = AtomicU64::new(0);
static RELAY_PRESENCE_VISIBLE_LOG_LAST_MS: AtomicU64 = AtomicU64::new(0);

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

/// Keep FFI/Dart `coord_lookup_peer` backoff aligned with `:p2p` session backoff.
pub fn clear_coord_lookup_backoff_for_pk(public_key_hex: &str) {
    let pk = public_key_hex.trim();
    if !crate::contacts_v1::is_valid_public_key_hex(pk) {
        return;
    }
    if let Ok(mut m) = lookup_backoff_map().lock() {
        m.remove(pk);
    }
}

/// Mirror `:p2p` `note_coord_lookup_not_found` so duplicate Dart lookups do not fight native upkeep.
pub fn sync_coord_lookup_peer_not_found(public_key_hex: &str, step_ms: u64, now_ms: i64) {
    let pk = public_key_hex.trim();
    if !crate::contacts_v1::is_valid_public_key_hex(pk) {
        return;
    }
    let step = step_ms.clamp(500, 5_000);
    let next = (now_ms as u64).saturating_add(step);
    if let Ok(mut m) = lookup_backoff_map().lock() {
        m.insert(
            pk.to_string(),
            CoordLookupBackoff {
                next_allowed_ms: next,
                step_ms: step,
            },
        );
    }
}

/// Min gap between full challenge+register cycles when already registered (reduces coord HTTP 401 storms).
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
        base_urls: Mutex::new(Vec::new()),
        insecure_tls: AtomicBool::new(false),
        endpoints: Mutex::new(Vec::new()),
        registered_on: Mutex::new(HashSet::new()),
        preferred_relay_peer_id: Mutex::new(None),
        local_peer_id: Mutex::new(None),
        relay_bootstrap_tcp: Mutex::new(HashSet::new()),
        heartbeat_stop: AtomicBool::new(false),
        heartbeat_join: Mutex::new(None),
    })
}

/// Legacy no-op — native connect uses identity wire, not libp2p PeerId.
pub fn coord_set_local_peer_id(peer: String) {
    if let Ok(mut g) = coord_globals().local_peer_id.lock() {
        *g = Some(peer);
    }
}

fn coord_local_peer_id() -> Option<String> {
    coord_globals().local_peer_id.lock().ok().and_then(|g| g.clone())
}

fn coord_relay_bootstrap_tcp_keys() -> HashSet<String> {
    coord_globals()
        .relay_bootstrap_tcp
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Remember relay bootstrap host:ports from `GET /v1/relay` (never POST as peer TCP).
pub fn coord_note_relay_bootstrap_addrs(addrs: &[String]) {
    let keys = crate::p2p::network_transport::relay_bootstrap_tcp_keys(addrs);
    if keys.is_empty() {
        return;
    }
    if let Ok(mut g) = coord_globals().relay_bootstrap_tcp.lock() {
        g.extend(keys);
    }
}

fn normalize_coord_url(url: &str) -> Option<String> {
    let t = url.trim().trim_end_matches('/');
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn is_coord_url_delimiter(c: char) -> bool {
    c == ',' || c == ';' || c.is_ascii_whitespace()
}

/// Parse one URL, JSON array, or a delimiter-separated list.
///
/// Delimiters may be comma, semicolon, tab, space, newline, or any mix.
pub fn parse_coord_urls(input: &str) -> Vec<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with('[') {
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
            return arr
                .into_iter()
                .filter_map(|s| normalize_coord_url(&s))
                .take(MAX_COORD_SERVERS)
                .collect();
        }
    }
    trimmed
        .split(is_coord_url_delimiter)
        .filter_map(|s| normalize_coord_url(s))
        .take(MAX_COORD_SERVERS)
        .collect()
}

/// Parse coord URL(s) from daemon RPC / FFI / `p2p_start` JSON (new + legacy keys).
pub fn coord_urls_from_json_value(v: &serde_json::Value) -> Vec<String> {
    for key in ["base_urls", "coord_base_urls"] {
        if let Some(arr) = v.get(key).and_then(|x| x.as_array()) {
            let urls: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str())
                .filter_map(|s| normalize_coord_url(s))
                .take(MAX_COORD_SERVERS)
                .collect();
            if !urls.is_empty() {
                return urls;
            }
        }
    }
    for key in ["base_url", "coord_base_url", "url"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            if let Some(u) = normalize_coord_url(s) {
                return vec![u];
            }
        }
    }
    Vec::new()
}

/// Configured coord/relay server base URLs (register on all; lookup in order).
pub fn coord_base_urls() -> Vec<String> {
    coord_globals()
        .base_urls
        .lock()
        .ok()
        .map(|v| v.clone())
        .unwrap_or_default()
}

fn client_for(base: &str) -> Result<CoordHttpClient, String> {
    CoordHttpClient::new(base, coord_globals().insecure_tls.load(Ordering::Relaxed))
        .map_err(|e| e.to_string())
}

pub(crate) fn has_coord_endpoints() -> bool {
    has_public_coord_register_endpoints()
}

pub(crate) fn has_public_coord_register_endpoints() -> bool {
    coord_globals()
        .endpoints
        .lock()
        .ok()
        .is_some_and(|v| endpoints_are_registerable(&v))
}

fn endpoints_are_registerable(eps: &[CoordEndpoint]) -> bool {
    !endpoints_for_coord_register(eps.to_vec()).is_empty()
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

/// Legacy no-op — relay reservations removed with libp2p.
pub fn coord_note_relay_reservation(_relay_peer_id: String) {}

/// Coord URL is set — WAN peer discovery requires coord/relay lookup (TRANSPORT.md).
/// mDNS/LAN still works without coord; coord HTTP retries continue in the background.
pub fn wan_discovery_via_coord_only() -> bool {
    coord_is_configured()
}

/// Immediate coord re-register — only after publishable endpoint set change (rebuild_coord).
pub fn schedule_register_presence_force() {
    spawn_register_presence_inner(true);
}

/// Relay reservation accepted or circuit listen up — server registers `/p2p-circuit` on coord.
pub fn schedule_coord_presence_after_relay() {
    if !coord_is_configured() {
        return;
    }
    if has_public_coord_register_endpoints() {
        spawn_register_presence_inner(true);
    }
    schedule_check_relay_presence_once();
}

/// Relay circuit listener is up — publish on coord and poll server row (do not set registered until HTTP confirms).
pub fn coord_note_relay_circuit_ready() {
    schedule_coord_presence_after_relay();
}

fn relay_presence_reschedule_min_ms() -> u64 {
    if COORD_REGISTERED.load(Ordering::Relaxed) {
        RELAY_PRESENCE_RESCHEDULE_REGISTERED_MS
    } else {
        RELAY_PRESENCE_RESCHEDULE_MIN_MS
    }
}

fn coord_recovery_throttled(now: u64) -> bool {
    let last = COORD_RECOVERY_LAST_MS.load(Ordering::Relaxed);
    last > 0 && now.saturating_sub(last) < COORD_RECOVERY_MIN_MS
}

fn note_coord_recovery_attempt(now: u64) {
    COORD_RECOVERY_LAST_MS.store(now, Ordering::Relaxed);
}

fn should_log_coord_heartbeat_fail(now: u64) -> bool {
    let last = COORD_HEARTBEAT_FAIL_LOG_LAST_MS.load(Ordering::Relaxed);
    if last > 0 && now.saturating_sub(last) < COORD_HEARTBEAT_FAIL_LOG_MIN_MS {
        return false;
    }
    COORD_HEARTBEAT_FAIL_LOG_LAST_MS.store(now, Ordering::Relaxed);
    true
}

fn schedule_check_relay_presence_once() {
    #[cfg(test)]
    {
        return;
    }
    #[cfg(not(test))]
    schedule_check_relay_presence_once_impl();
}

#[cfg(not(test))]
fn schedule_check_relay_presence_once_impl() {
    if RELAY_PRESENCE_CHECK_PENDING.load(Ordering::Acquire) {
        return;
    }
    let now = unix_ms_now();
    let last_end = RELAY_PRESENCE_POLL_LAST_END_MS.load(Ordering::Relaxed);
    if last_end > 0 && now.saturating_sub(last_end) < relay_presence_reschedule_min_ms() {
        return;
    }
    if RELAY_PRESENCE_CHECK_PENDING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    std::thread::Builder::new()
        .name("ghal_bol-coord-relay-poll".into())
        .spawn(|| {
            let mut attempt: u32 = 0;
            let poll_once = |attempt: u32| -> bool {
                if !coord_is_configured() {
                    return true;
                }
                let Ok(ident) = crate::session_runtime::unlocked_identity_clone() else {
                    return true;
                };
                if refresh_relay_presence_from_coord(&ident.identity_wire()) {
                    return true;
                }
                if crate::wan_coord::local_relay_circuit_listening()
                    && (attempt == 2 || attempt == 7 || attempt % 15 == 0)
                {
                    crate::p2p::notify_relay_refresh();
                    if has_public_coord_register_endpoints() {
                        spawn_register_presence_inner(false);
                    }
                }
                false
            };
            while attempt < RELAY_PRESENCE_POLL_FAST_ATTEMPTS {
                if attempt > 0 {
                    std::thread::sleep(Duration::from_millis(RELAY_PRESENCE_POLL_FAST_MS));
                }
                if poll_once(attempt) {
                    break;
                }
                attempt += 1;
            }
            while attempt < RELAY_PRESENCE_POLL_FAST_ATTEMPTS + RELAY_PRESENCE_POLL_SLOW_ATTEMPTS {
                if !crate::wan_coord::local_relay_circuit_listening() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(RELAY_PRESENCE_POLL_SLOW_MS));
                if poll_once(attempt) {
                    break;
                }
                attempt += 1;
            }
            RELAY_PRESENCE_POLL_LAST_END_MS.store(unix_ms_now(), Ordering::Relaxed);
            RELAY_PRESENCE_CHECK_PENDING.store(false, Ordering::Release);
        })
        .ok();
}

fn peer_record_has_relay_circuit(record: &crate::coord::CoordPeerRecord) -> bool {
    record.endpoints.iter().any(|e| {
        e.scheme == "libp2p" && e.host.contains("/p2p-circuit")
    })
}

/// Native-connect or legacy row visible on coord (client TCP register or relay circuit).
fn peer_record_has_coord_presence(record: &crate::coord::CoordPeerRecord) -> bool {
    if peer_record_has_relay_circuit(record) {
        return true;
    }
    record.endpoints.iter().any(|e| {
        e.scheme == "tcp" && !e.host.trim().is_empty() && e.port != 0
    })
}

/// During LAN↔WAN handover coord HTTP may flap while the relay server still has our circuit.
/// Blocking HTTP — call only from a std thread (heartbeat, relay-presence poll, spawn_blocking).
/// Do **not** call from the libp2p tokio task (reqwest::blocking runtime drop panics).
/// Blocking HTTP — call only from a std thread (heartbeat, relay-presence poll, spawn_blocking).
/// Do **not** call from the libp2p tokio task (reqwest::blocking runtime drop panics).
pub fn try_restore_relay_presence_from_coord() -> bool {
    let Ok(ident) = crate::session_runtime::unlocked_identity_clone() else {
        return false;
    };
    refresh_relay_presence_from_coord(&ident.identity_wire())
}

/// GET /v1/peers/self — authoritative whether the relay server still has our circuit.
fn refresh_relay_presence_from_coord(pk: &str) -> bool {
    let urls = coord_base_urls();
    if urls.is_empty() {
        return false;
    }
    let mut any_visible = false;
    let mut any_http_ok = false;
    let mut registered = HashSet::new();
    for base in &urls {
        let Ok(client) = client_for(base) else {
            continue;
        };
        match client.lookup(pk) {
            Ok(rec) if peer_record_has_coord_presence(&rec) => {
                any_http_ok = true;
                any_visible = true;
                registered.insert(base.clone());
            }
            Ok(_) => {
                any_http_ok = true;
            }
            Err(e) => {
                if coord_lookup_err_means_http_reachable(&e) {
                    any_http_ok = true;
                }
                crate::flow_log::debug(
                    "coord",
                    format!(
                        "relay presence poll on {base} for {}: {e}",
                        &pk[..8.min(pk.len())]
                    ),
                );
            }
        }
    }
    if any_http_ok {
        note_coord_transport_ok();
    }
    if any_http_ok && !any_visible {
        // HTTP up but no circuit row — re-publish; do not flip coord_registered=false on a
        // transient coord purge while our local relay reservation may still be live.
        if has_public_coord_register_endpoints() {
            spawn_register_presence_inner(false);
        } else {
            let now = unix_ms_now();
            if !coord_recovery_throttled(now) {
                note_coord_recovery_attempt(now);
                schedule_check_relay_presence_once();
            }
        }
        return false;
    }
    if !any_visible {
        return false;
    }
    // Idempotent: visible on coord while already registered — refresh transport ok only.
    if COORD_REGISTERED.load(Ordering::Relaxed) {
        if any_http_ok {
            note_coord_transport_ok();
        }
        return true;
    }
    if let Ok(mut r) = coord_globals().registered_on.lock() {
        *r = registered;
    }
    COORD_REGISTERED.store(true, Ordering::Relaxed);
    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
    COORD_LAST_OK_MS.store(unix_ms_now(), Ordering::Relaxed);
    let now = unix_ms_now();
    let last_log = RELAY_PRESENCE_VISIBLE_LOG_LAST_MS.load(Ordering::Relaxed);
    if last_log == 0 || now.saturating_sub(last_log) >= RELAY_PRESENCE_RESCHEDULE_REGISTERED_MS {
        RELAY_PRESENCE_VISIBLE_LOG_LAST_MS.store(now, Ordering::Relaxed);
        crate::flow_log::info(
            "coord",
            format!(
                "relay presence visible on coord for {} — circuit registered by relay server",
                &pk[..8.min(pk.len())]
            ),
        );
    }
    start_heartbeat_loop(pk.to_string());
    crate::p2p::notify_dm_presence_wake();
    true
}

fn heartbeat_err_peer_not_on_coord(err: &str) -> bool {
    err.contains("404") || err.contains("not registered")
}

fn recover_coord_presence_after_server_drop(pk: &str) {
    let now = unix_ms_now();
    if coord_recovery_throttled(now) {
        return;
    }
    note_coord_recovery_attempt(now);
    if refresh_relay_presence_from_coord(pk) {
        return;
    }
    if has_public_coord_register_endpoints() {
        spawn_register_presence_inner(false);
    } else {
        schedule_check_relay_presence_once();
    }
}

/// Bootstrap HOP to coord relay dropped — reconcile WAN presence without tearing down
/// parallel LAN+WAN when local relay circuit listen is still up.
pub fn mark_coord_relay_hop_lost() {
    if !crate::wan_coord::local_relay_circuit_listening() {
        COORD_REGISTERED.store(false, Ordering::Relaxed);
    }
    schedule_check_relay_presence_once();
}

/// Clear stale endpoint snapshot after a full WAN network handover.
pub fn coord_invalidate_presence_on_network_change() {
    if let Ok(mut v) = coord_globals().endpoints.lock() {
        v.clear();
    }
    if let Ok(mut g) = coord_globals().preferred_relay_peer_id.lock() {
        *g = None;
    }
}


/// Relay removed — returns empty.
pub fn fetch_all_ghal_bol_relays(_remap: bool) -> Vec<(String, Vec<String>)> {
    Vec::new()
}

/// Delete legacy `ghalbol_relay*.json` files from the identity data dir (`:p2p` start).
pub fn purge_legacy_relay_cache_files(data_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let legacy = name.as_ref() == "ghalbol_relay.json"
            || (name.starts_with("ghalbol_relay_") && name.ends_with(".json"));
        if legacy {
            if std::fs::remove_file(entry.path()).is_ok() {
                crate::flow_log::info("relay", format!("purged legacy relay cache {name}"));
            }
        }
    }
}

/// Relay TCP failed — purge any leftover legacy cache files in `data_dir`.
pub fn invalidate_cached_ghalbol_relay(data_dir: Option<&std::path::Path>) {
    if let Some(dir) = data_dir {
        purge_legacy_relay_cache_files(dir);
    }
}

/// True when at least one coordination server URL was configured.
pub fn coord_is_configured() -> bool {
    !coord_base_urls().is_empty()
}

/// Insecure TLS flag for the configured coord URL list (always false for all-HTTPS).
pub fn coord_insecure_tls() -> bool {
    coord_globals().insecure_tls.load(Ordering::Relaxed)
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

/// Coord URL is set but coord HTTP transport has not succeeded recently — LAN/mDNS still works.
/// Self not yet on coord (`awaiting_coord_mirror`) is **not** transport degradation when HTTP
/// recently returned 200/404 (TRANSPORT.md § WAN phases — coord HTTP flap recovery).
pub fn coord_http_degraded() -> bool {
    if !coord_is_configured() {
        return false;
    }
    // One transient mobile/coord HTTP flake must not flip WAN to degraded for minutes.
    if COORD_CONSEC_FAILS.load(Ordering::Relaxed) >= 2 {
        return true;
    }
    let last = COORD_LAST_OK_MS.load(Ordering::Relaxed);
    last == 0 || unix_ms_now().saturating_sub(last) >= PRESENCE_STALE_MS
}

fn coord_lookup_err_means_http_reachable(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("404")
        || e.contains("peer_not_on")
        || e.contains("not registered")
        || e.contains("no dialable endpoints")
}

/// Record a coord HTTP transport failure (lookup/register/heartbeat) — flips degraded quickly.
pub(crate) fn note_coord_transport_failure() {
    COORD_CONSEC_FAILS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_coord_transport_ok() {
    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
    COORD_LAST_OK_MS.store(unix_ms_now(), Ordering::Relaxed);
}

/// Single-server helper (integration tests + legacy callers).
pub fn set_coord_base_url(url: &str, insecure_tls: bool) {
    set_coord_base_urls(&[url.to_string()], insecure_tls);
}

/// HTTPS coord URLs always use verified TLS in Rust — stale `insecure_tls` prefs from self-signed
/// dev must not stick after switching to Let's Encrypt (AGENTS.md: product logic in `ghal_bol`).
fn resolve_coord_insecure_tls(urls: &[String], requested: bool) -> bool {
    if urls.is_empty() {
        return false;
    }
    if urls
        .iter()
        .all(|u| u.trim().to_ascii_lowercase().starts_with("https://"))
    {
        return false;
    }
    requested
}

pub fn set_coord_base_urls(urls: &[String], insecure_tls: bool) {
    let insecure_tls = resolve_coord_insecure_tls(urls, insecure_tls);
    let urls: Vec<String> = urls
        .iter()
        .filter_map(|u| normalize_coord_url(u))
        .take(MAX_COORD_SERVERS)
        .collect();
    let g = coord_globals();
    let unchanged = g.base_urls.lock().ok().is_some_and(|cur| *cur == urls)
        && g.insecure_tls.load(Ordering::Relaxed) == insecure_tls;
    if unchanged {
        // `p2p_start already_running` and hub resume call this every session — do not wipe
        // coord presence or force a WAN rediscovery gap when URLs are unchanged.
        if !COORD_REGISTERED.load(Ordering::Relaxed) {
            schedule_check_relay_presence_once();
        }
        return;
    }
    if let Ok(mut u) = g.base_urls.lock() {
        *u = urls.clone();
    }
    if let Ok(mut r) = g.registered_on.lock() {
        r.clear();
    }
    g.insecure_tls.store(insecure_tls, Ordering::Relaxed);
    COORD_REGISTERED.store(false, Ordering::Relaxed);
    COORD_LAST_OK_MS.store(0, Ordering::Relaxed);
    crate::flow_log::info(
        "coord",
        format!("base_urls set ({} server(s)): {urls:?}", urls.len()),
    );
    schedule_register_presence();
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
        return crate::p2p::network_transport::is_public_bootstrap_ipv4(ip);
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

/// Any non-loopback DM TCP listen — legacy no-coord path only; coord mode uses public TCP + relay.
fn is_coord_presence_tcp_fallback(ma: &Multiaddr) -> bool {
    if crate::p2p::network_transport::is_relay_circuit_multiaddr(ma) {
        return false;
    }
    let Some(ip) = crate::p2p::network_transport::ipv4_from_ma_str(&ma.to_string()) else {
        return false;
    };
    !ip.is_loopback() && crate::p2p::network_transport::is_dm_dial_multiaddr(ma)
}

fn listen_addrs_to_coord_endpoints(addrs: &[Multiaddr]) -> Vec<CoordEndpoint> {
    let has_relay = addrs
        .iter()
        .any(crate::p2p::network_transport::is_coord_relay_tcp_circuit_multiaddr);
    let mut eps = Vec::new();
    for ma in addrs {
        eps.extend(multiaddr_to_coord_endpoints(ma));
    }
    // With coord, only relay (or public TCP) is WAN-dialable — never CGNAT/LAN via POST.
    if eps.is_empty() && !has_relay {
        for ma in addrs {
            if coord_is_configured() {
                continue;
            }
            if !is_coord_presence_tcp_fallback(ma) {
                continue;
            } else if let Some(ip) =
                crate::p2p::network_transport::ipv4_from_ma_str(&ma.to_string())
            {
                // RFC1918 is LAN-only — mDNS, not coord (misleading for WAN peers).
                if ip.is_private() && !crate::p2p::network_transport::is_cgnat_ipv4(ip) {
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
        let bootstraps = coord_relay_bootstrap_tcp_keys();
        eps.retain(|e| {
            e.scheme == "libp2p"
                || (e.scheme == "tcp"
                    && is_coord_publishable_host(&e.host)
                    && !crate::p2p::network_transport::is_relay_bootstrap_tcp(
                        &e.host, e.port, &bootstraps,
                    ))
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
    #[cfg(test)]
    if coord_base_urls()
        .iter()
        .any(|b| b.contains("example.test"))
    {
        return;
    }
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
        .name("ghal_bol-coord-reg".into())
        // Blocking HTTP + TLS on the default 2 MiB thread stack overflowed on CI runners.
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            struct BusyGuard;
            impl Drop for BusyGuard {
                fn drop(&mut self) {
                    COORD_REG_WORKER_BUSY.store(false, Ordering::Release);
                    // `COORD_REG_PENDING` is drained on the next `coord_register_tick` (libp2p
                    // 1s loop). Do not call `spawn_register_presence_inner` from `Drop` — that
                    // chained synchronous spawns on a shallow exiting stack and caused SIGABRT on
                    // GitHub Actions integration tests.
                }
            }
            let _busy = BusyGuard;
            if let Err(e) = try_register_presence() {
                let es = e.to_string();
                if es.contains("relay-only") || es.contains("no listen endpoints") {
                    crate::flow_log::debug("coord", format!("register: {e}"));
                } else {
                    note_coord_transport_failure();
                    crate::flow_log::warn("coord", format!("register: {e}"));
                }
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
    if crate::p2p::network_transport::is_coord_relay_tcp_circuit_multiaddr(ma) {
        return vec![CoordEndpoint {
            scheme: "libp2p".into(),
            host: ma.to_string(),
            port: 0,
        }];
    }
    let preferred_relay = coord_globals()
        .preferred_relay_peer_id
        .lock()
        .ok()
        .and_then(|g| g.clone());
    let local = match coord_local_peer_id() {
        Some(p) => p,
        None => return Vec::new(),
    };
    if !crate::p2p::network_transport::is_peer_own_coord_register_tcp(
        ma,
        &local,
        &coord_relay_bootstrap_tcp_keys(),
        preferred_relay.as_deref(),
    ) {
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
/// Native connect passes an empty snapshot — endpoints come from [`on_listen_dm_addr`].
pub fn rebuild_coord_endpoints_from_listen(addrs: &[Multiaddr]) {
    if !coord_is_configured() {
        return;
    }
    if addrs.is_empty() {
        if has_coord_endpoints() && !COORD_REGISTERED.load(Ordering::Relaxed) {
            spawn_register_presence_inner(false);
        }
        return;
    }
    let has_relay = addrs
        .iter()
        .any(crate::p2p::network_transport::is_coord_ipv4_relay_listen);
    let publishable: Vec<Multiaddr> = addrs
        .iter()
        .filter(|ma| {
            crate::p2p::network_transport::is_coord_ipv4_relay_listen(ma)
                || {
                    let local = coord_local_peer_id();
                    let preferred = coord_globals()
                        .preferred_relay_peer_id
                        .lock()
                        .ok()
                        .and_then(|g| g.clone());
                    local.is_some_and(|local| {
                        crate::p2p::network_transport::is_peer_own_coord_register_tcp(
                            ma,
                            &local,
                            &coord_relay_bootstrap_tcp_keys(),
                            preferred.as_deref(),
                        )
                    })
                }
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
                crate::p2p::network_transport::ipv4_from_ma_str(&ma.to_string())
                    .is_some_and(crate::p2p::network_transport::is_cgnat_ipv4)
            });
        let was_registered = COORD_REGISTERED.load(Ordering::Relaxed);
        if was_registered {
            // CGNAT-only local listen snapshot — server still owns relay circuit on coord.
            schedule_check_relay_presence_once();
            return;
        }
        let g = coord_globals();
        if let Ok(mut v) = g.endpoints.lock() {
            if !v.is_empty() {
                v.clear();
            }
        }
        if !was_registered {
            suspend_coord_presence_waiting_endpoints();
            if cgnat_only {
                crate::flow_log::debug(
                    "coord",
                    "CGNAT — no public TCP for coord register; WAN text uses delivery, LAN uses mDNS",
                );
            } else if !crate::wan_coord::local_relay_circuit_listening() {
                crate::flow_log::debug(
                    "coord",
                    "no public TCP endpoint yet — WAN text uses delivery; LAN uses mDNS",
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
            schedule_check_relay_presence_once();
        }
        return;
    }
    *v = eps;
    if endpoints_are_registerable(&v) {
        spawn_register_presence_inner(true);
    } else {
        schedule_check_relay_presence_once();
    }
}

/// Remote peer circuit dial failed — coord row may be stale; clear lookup backoff for urgent retry.
pub fn note_remote_peer_circuit_stale(public_key_hex: &str) {
    clear_coord_lookup_backoff_for_pk(public_key_hex);
}

fn coord_heartbeat_running() -> bool {
    coord_globals()
        .heartbeat_join
        .lock()
        .ok()
        .and_then(|j| j.as_ref().map(|h| !h.is_finished()))
        .unwrap_or(false)
}

/// Start the coord heartbeat/presence thread while awaiting phase D (circuit mirror on coord).
pub fn ensure_coord_presence_polling() {
    if !coord_is_configured() || COORD_REGISTERED.load(Ordering::Relaxed) {
        return;
    }
    if !crate::wan_coord::local_relay_circuit_listening() {
        return;
    }
    if coord_heartbeat_running() {
        return;
    }
    let Ok(ident) = crate::session_runtime::unlocked_identity_clone() else {
        return;
    };
    start_heartbeat_loop(ident.identity_wire());
}

/// Drain pending register work and sync endpoint snapshot.
pub fn coord_register_tick(listen_snapshot: &[Multiaddr]) {
    if !coord_is_configured() {
        return;
    }
    rebuild_coord_endpoints_from_listen(listen_snapshot);
    if !COORD_REGISTERED.load(Ordering::Relaxed) {
        if has_coord_endpoints() {
            spawn_register_presence_inner(false);
        }
        if crate::wan_coord::local_relay_circuit_listening() {
            ensure_coord_presence_polling();
            schedule_check_relay_presence_once();
        }
    } else if COORD_REG_PENDING.swap(false, Ordering::AcqRel) {
        spawn_register_presence_inner(false);
    } else if !coord_link_recently_ok() && COORD_CONSEC_FAILS.load(Ordering::Relaxed) >= 1 {
        let now = unix_ms_now();
        if !coord_recovery_throttled(now) {
            note_coord_recovery_attempt(now);
            schedule_check_relay_presence_once();
        }
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
            if crate::p2p::network_transport::is_coord_relay_tcp_circuit_multiaddr(&ma) {
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
            if crate::p2p::network_transport::is_dm_dial_multiaddr(&ma) {
                out.push(ma);
            }
        }
    }
    let out = crate::p2p::network_transport::filter_coord_dial_addrs(out);
    crate::p2p::network_transport::filter_coord_relay_dial_platform(out)
}

/// Client POST /v1/register: public routable TCP only. Relay `/p2p-circuit` is server-registered.
fn endpoints_for_coord_register(eps: Vec<CoordEndpoint>) -> Vec<CoordEndpoint> {
    let tcp: Vec<CoordEndpoint> = eps
        .into_iter()
        .filter(|e| e.scheme == "tcp")
        .collect();
    if !coord_is_configured() {
        return tcp;
    }
    let bootstraps = coord_relay_bootstrap_tcp_keys();
    tcp.into_iter()
        .filter(|e| is_coord_publishable_host(&e.host))
        .filter(|e| {
            !crate::p2p::network_transport::is_relay_bootstrap_tcp(&e.host, e.port, &bootstraps)
        })
        .collect()
}

fn try_register_on_server(
    base: &str,
    ident: &crate::DecryptedIdentity,
    endpoints: &[CoordEndpoint],
) -> Result<(), String> {
    let wire = ident.identity_wire();
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
    let client = client_for(base)?;
    client.register(ident, endpoints, ipv4.as_deref(), ipv6.as_deref())?;
    client
        .lookup(&wire)
        .map_err(|e| format!("register HTTP ok but GET /v1/peers failed: {e}"))?;
    Ok(())
}

fn try_register_presence() -> Result<(), String> {
    let _http = coord_reg_http_lock()
        .lock()
        .map_err(|_| "coord register http mutex poisoned")?;
    let ident = match crate::session_runtime::unlocked_identity_clone() {
        Ok(i) => i,
        Err(_) => return Err("identity not unlocked".into()),
    };
    let wire = ident.identity_wire();
    let endpoints = endpoints_for_coord_register(
        coord_globals()
            .endpoints
            .lock()
            .map_err(|_| "coord mutex poisoned")?
            .clone(),
    );
    if endpoints.is_empty() {
        schedule_check_relay_presence_once();
        return Err("relay-only — coord register skipped".into());
    }
    let urls = coord_base_urls();
    if urls.is_empty() {
        return Err("coord base url not set".into());
    }
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
    let mut any_ok = false;
    let mut registered = HashSet::new();
    let mut last_err = String::new();
    for base in &urls {
        match try_register_on_server(base, &ident, &endpoints) {
            Ok(()) => {
                any_ok = true;
                registered.insert(base.clone());
                crate::flow_log::info(
                    "coord",
                    format!(
                        "registered on {base} — {} endpoint(s) for {} [{}]",
                        endpoints.len(),
                        &wire[..wire.len().min(16)],
                        ep_summary
                    ),
                );
            }
            Err(e) => {
                last_err = e.clone();
                let has_relay = endpoints.iter().any(|ep| ep.scheme == "libp2p");
                let (reason, action) =
                    crate::p2p::connectivity_diag::explain_coord_register_failure(&e, has_relay);
                crate::flow_log::warn(
                    "coord",
                    format!(
                        "register on {base} — reason={reason} | next={action} | http={last_err}"
                    ),
                );
            }
        }
    }
    if !any_ok {
        return Err(if last_err.is_empty() {
            "coord register failed on all servers".into()
        } else {
            last_err
        });
    }
    if let Ok(mut r) = coord_globals().registered_on.lock() {
        *r = registered;
    }
    COORD_REGISTERED.store(true, Ordering::Relaxed);
    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
    COORD_LAST_OK_MS.store(unix_ms_now(), Ordering::Relaxed);
    start_heartbeat_loop(wire);
    // We are visible on coord — known peers should lookup/dial us; we lookup them on next upkeep.
    crate::p2p::notify_dm_presence_wake();
    Ok(())
}

fn start_heartbeat_loop(public_key_hex: String) {
    // One heartbeat thread only. Restarting without joining spawned duplicates that never saw
    // `heartbeat_stop` — that flooded coord HTTP and starved the poll bridge (2026-06-28).
    if coord_heartbeat_running() {
        return;
    }
    let g = coord_globals();
    g.heartbeat_stop.store(false, Ordering::Relaxed);
    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
    let stop = &g.heartbeat_stop;
    let handle = std::thread::Builder::new()
        .name("ghal_bol-coord-heartbeat".into())
        .spawn(move || {
            loop {
                for _ in 0..HEARTBEAT_INTERVAL_SECS {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                let servers: Vec<String> = coord_globals()
                    .registered_on
                    .lock()
                    .ok()
                    .map(|g| g.iter().cloned().collect())
                    .unwrap_or_else(coord_base_urls);
                if servers.is_empty() {
                    continue;
                }
                if !COORD_REGISTERED.load(Ordering::Relaxed) {
                    let _ = refresh_relay_presence_from_coord(&public_key_hex);
                    continue;
                }
                let mut any_ok = false;
                let mut peer_gone = false;
                for base in &servers {
                    let Ok(c) = client_for(base) else {
                        continue;
                    };
                    match c.heartbeat(&public_key_hex) {
                        Ok(_) => {
                            any_ok = true;
                            note_coord_transport_ok();
                            break;
                        }
                        Err(e) if heartbeat_err_peer_not_on_coord(&e) => {
                            peer_gone = true;
                            COORD_REGISTERED.store(false, Ordering::Relaxed);
                            crate::flow_log::warn(
                                "coord",
                                format!(
                                    "heartbeat: peer not on coord ({base}) for {} — recovering",
                                    &public_key_hex[..8.min(public_key_hex.len())]
                                ),
                            );
                            break;
                        }
                        Err(e) => {
                            if coord_lookup_err_means_http_reachable(&e) {
                                note_coord_transport_ok();
                            } else {
                                note_coord_transport_failure();
                            }
                            let now = unix_ms_now();
                            if should_log_coord_heartbeat_fail(now) {
                                crate::flow_log::warn(
                                    "coord",
                                    format!(
                                        "heartbeat failed on {base} for {}: {e}",
                                        &public_key_hex[..8.min(public_key_hex.len())]
                                    ),
                                );
                            }
                        }
                    }
                }
                if any_ok {
                    COORD_LAST_OK_MS.store(unix_ms_now(), Ordering::Relaxed);
                    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
                } else if peer_gone {
                    recover_coord_presence_after_server_drop(&public_key_hex);
                    COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
                } else {
                    let fails = COORD_CONSEC_FAILS.fetch_add(1, Ordering::Relaxed) + 1;
                    if fails >= 2 {
                        if refresh_relay_presence_from_coord(&public_key_hex) {
                            COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
                            COORD_LAST_OK_MS.store(unix_ms_now(), Ordering::Relaxed);
                        } else if fails >= 3 {
                            recover_coord_presence_after_server_drop(&public_key_hex);
                            COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
                        }
                    }
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
    let urls = coord_base_urls();
    if urls.is_empty() {
        return Err("coord base url not set".into());
    }
    let mut last_err = "coord lookup failed".to_string();
    for base in &urls {
        let Ok(client) = client_for(base) else {
            continue;
        };
        match client.lookup(public_key_hex) {
            Ok(record) => {
                let addrs: Vec<String> = coord_endpoints_to_dial_multiaddrs(&record.endpoints)
                    .into_iter()
                    .map(|ma| ma.to_string())
                    .collect();
                if !addrs.is_empty() {
                    return Ok(addrs);
                }
                last_err = format!(
                    "coord lookup ok but peer has no dialable endpoints ({} on record)",
                    record.endpoints.len()
                );
            }
            Err(e) => {
                if coord_lookup_err_means_http_reachable(&e) {
                    note_coord_transport_ok();
                }
                last_err = e.to_string();
            }
        }
    }
    Err(last_err)
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
                800u64
            } else {
                5_000u64
            };
            let mut next = now.saturating_add(step);
            if let Ok(mut m) = lookup_backoff_map().lock() {
                if let Some(prev) = m.get(pk).copied() {
                    step = (prev.step_ms.saturating_mul(2)).min(5_000);
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

/// Legacy single-URL FFI/daemon response shape.
pub fn coord_set_base_url_json(url: &str, insecure_tls: bool) -> serde_json::Value {
    coord_set_base_urls_json(&[url.to_string()], insecure_tls)
}

/// Where coord URL prefs are persisted — Android path is unchanged (always release namespace).
fn coord_prefs_storage_config() -> crate::storage::StorageConfig {
    #[cfg(target_os = "android")]
    {
        crate::app_paths::storage_config_for_namespace(crate::ANDROID_LIBRARY_NAMESPACE)
    }
    #[cfg(not(target_os = "android"))]
    {
        let ns = crate::dm_event_handler::active_app_namespace()
            .unwrap_or_else(|| crate::ANDROID_LIBRARY_NAMESPACE.to_string());
        crate::app_paths::storage_config_for_namespace(&ns)
    }
}

pub fn coord_set_base_urls_json(urls: &[String], insecure_tls: bool) -> serde_json::Value {
    if urls.is_empty() {
        return serde_json::json!({ "ok": false, "error": "url empty" });
    }
    let insecure_tls = resolve_coord_insecure_tls(urls, insecure_tls);
    set_coord_base_urls(urls, insecure_tls);
    let cfg = coord_prefs_storage_config();
    let joined = urls.join(",");
    if let Err(e) = crate::preferences_v1::coord_settings_set(&cfg, &joined, insecure_tls) {
        return serde_json::json!({ "ok": false, "error": format!("{e}") });
    }
    serde_json::json!({
        "ok": true,
        "base_urls": urls,
    })
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

pub fn lookup_dial_addrs_for_public_key(public_key_hex: &str) -> Result<Vec<DmDialAddr>, String> {
    let urls = coord_base_urls();
    if urls.is_empty() {
        return Err("coord base url not set".into());
    }
    let mut last_err = "coord lookup failed".to_string();
    for base in &urls {
        let Ok(client) = client_for(base) else {
            continue;
        };
        match client.lookup(public_key_hex) {
            Ok(record) => {
                let addrs = coord_endpoints_to_dial_addrs(&record.endpoints);
                if !addrs.is_empty() {
                    return Ok(addrs);
                }
                last_err = format!(
                    "coord lookup ok but peer has no dialable endpoints ({} on record)",
                    record.endpoints.len()
                );
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(last_err)
}

/// Coord lookup → ranked/filtered libp2p multiaddrs (relay + public TCP before RFC1918).
/// Tries configured servers in order; stops on first success (TRANSPORT.md).
pub fn lookup_dial_multiaddrs_for_public_key(
    public_key_hex: &str,
) -> Result<Vec<Multiaddr>, String> {
    let urls = coord_base_urls();
    if urls.is_empty() {
        return Err("coord base url not set".into());
    }
    let mut last_err = "coord lookup failed".to_string();
    for base in &urls {
        let Ok(client) = client_for(base) else {
            continue;
        };
        match client.lookup(public_key_hex) {
            Ok(record) => {
                note_coord_transport_ok();
                let addrs = coord_endpoints_to_dial_multiaddrs(&record.endpoints);
                if !addrs.is_empty() {
                    return Ok(addrs);
                }
                last_err = format!(
                    "coord lookup ok but peer has no dialable endpoints ({} on record)",
                    record.endpoints.len()
                );
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(last_err)
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

    #[test]
    fn coord_http_degraded_false_after_recent_ok_even_unregistered() {
        COORD_REGISTERED.store(false, Ordering::Relaxed);
        COORD_CONSEC_FAILS.store(0, Ordering::Relaxed);
        note_coord_transport_ok();
        assert!(!coord_http_degraded());
    }

    #[test]
    fn coord_lookup_404_counts_as_http_reachable() {
        assert!(coord_lookup_err_means_http_reachable("HTTP 404 peer_not_on_server"));
        assert!(!coord_lookup_err_means_http_reachable("error sending request for url"));
    }

    #[test]
    fn https_coord_urls_force_secure_tls() {
        let https = vec!["https://coord.ghalbol.com".to_string()];
        assert!(!resolve_coord_insecure_tls(&https, true));
        assert!(!resolve_coord_insecure_tls(&https, false));
        let http_dev = vec!["http://127.0.0.1:8765".to_string()];
        assert!(resolve_coord_insecure_tls(&http_dev, true));
        assert!(!resolve_coord_insecure_tls(&http_dev, false));
    }

    #[test]
    fn coord_recovery_throttle_blocks_rapid_retries() {
        COORD_RECOVERY_LAST_MS.store(0, Ordering::Relaxed);
        let t0 = 1_000_000u64;
        assert!(!coord_recovery_throttled(t0));
        note_coord_recovery_attempt(t0);
        assert!(coord_recovery_throttled(t0 + 1_000));
        assert!(!coord_recovery_throttled(t0 + COORD_RECOVERY_MIN_MS));
    }

    #[test]
    fn relay_presence_reschedule_stricter_when_registered() {
        COORD_REGISTERED.store(true, Ordering::Relaxed);
        assert_eq!(
            relay_presence_reschedule_min_ms(),
            RELAY_PRESENCE_RESCHEDULE_REGISTERED_MS
        );
        COORD_REGISTERED.store(false, Ordering::Relaxed);
        assert_eq!(
            relay_presence_reschedule_min_ms(),
            RELAY_PRESENCE_RESCHEDULE_MIN_MS
        );
    }
}

