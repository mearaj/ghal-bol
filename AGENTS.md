# AI agent guide — Ghal Bol workspace

**Read this file first** in a new session. Then **`docs/DESIGN.md`** before changing P2P, acks, invites, or persistence. Transport (libp2p): **`docs/TRANSPORT.md`** — especially § **Connectivity lifecycle**, § **Network truth**, § **Asymmetric LAN↔WAN mux recovery**, § **Post-mortem 2026-06-24**, § **Post-mortem 2026-06-25** (mandatory before dial/upkeep/coord lookup changes).

**Connectivity policy (agents):** `docs/TRANSPORT.md` § **Connectivity lifecycle**, § **Network truth**, § **Parallel LAN + WAN transport**. Do **not** throttle relay reservation, coord lookup, or WAN recovery ticks because of informal “don’t flood” notes — throttle **storms** only (repeated `listen_on`, redundant dials). Register when **publishable endpoints change**, not every tick.

| Misread | Wrong agent behaviour (breaks WAN) | Correct meaning |
|---------|----------------------------------|-----------------|
| “Don’t register again and again” | Skip relay reservation, coord lookup, or WAN recovery ticks | Throttle redundant **`POST /v1/register`** when endpoints unchanged (`should_throttle_register`); **force** register on endpoint change, failed register, handover, relay accepted |
| “Full eye on the network” | Register/coord HTTP on every tick; or tear down steady links | Continuous profile watch; register when **publishable endpoint changes** — not spam |
| “Steady, reliable, don’t flood” | One relay only, no parallel coord relays, no bootstrap dials | Throttle **storms** (repeated `listen_on`, redundant bootstrap `swarm.dial`) — **not** required reservation + happy-eyeballs dial (TRANSPORT.md § CGNAT) |
| Old “relay disk cache for boot” | Reintroduce `ghalbol_relay.json` or boot from stale bore port | **TRANSPORT.md § “Caching policy (canonical)”.** Relay/bootstrap/coord dial addrs are **live HTTP only** — no disk cache; legacy files purged on start |
| “`if_addrs` shows Wi‑Fi / rmnet” | `profile=lan` minutes after mobile-data switch; wrong dial path | **OS default route** (`os=cell` / `os=wifi` in `Native/flow`) — TRANSPORT.md § **Network truth** |
| “`conn=true,stream=true` means chat works” | Ticks/outbox stuck on zombie LAN mux while peer on WAN | § **Asymmetric LAN↔WAN mux recovery** — `close direct … relay kept`, not full disconnect loop |
| “Connected peer needs no coord lookup” | Foreground/outbox peer skipped during LAN handover while remote on mobile-data → one-way WAN | § **Post-mortem 2026-06-25** — `coord_lookup_upkeep_satisfied` false when `lan_listen_rediscovery_requested` + intent or `peer_wan_asymmetric_mux_likely` |
| “Bursty delivery = broken WAN” | Revert asymmetric mux fix or add Flutter `NetworkHelper` connect gating | Multi-second stalls then `burst resync` during handover is **known trade-off** (5s reconcile + burst drain); both directions eventually work — § **Known symptom — bursty delivery** |
| “Urgent reconnect should beat all throttles” | Clear `circuit_dial_in_flight` + `disconnect_peer_id` during relay handshake | Urgent beats **404 backoff**, not **in-flight guard** — wait for handshake or expiry (TRANSPORT.md § Post-mortem 2026-06-24) |
| “No relay hop = chat link unstable” | `dm_peer_chat_link_stable=false` every tick while LAN stream healthy → mux churn | Use **`needs_additive_relay_dial`** for background WAN; **`dm_peer_stream_up` → upkeep noop** (TRANSPORT.md § Post-mortem 2026-06-24) |
| “Stream stable = skip all coord work” | Additive relay never dialed on Wi‑Fi; WAN handover dead | Still coord lookup + `dial_additive_dm_addr` when relay hop missing (TRANSPORT.md § Both links active) |

## Golden rules

0. **Prime directive — instant connect at any roster size.** Whenever two peers have *any*
   technically reachable path (LAN, relay circuit, future transport), they must **connect within a
   few seconds** and a message flows. A user may have **thousands of stale contacts** (offline /
   never-registered / `404`); **handling them must never delay a reachable peer** or flood coord.
   Lookups are split urgent/priority (active intent — uncapped) vs LRU background sweep (capped:
   `COORD_BACKGROUND_LOOKUPS_PER_TICK`) in `run_dm_coord_lookup_pass`. If any throttle/backoff/grace
   is ever in tension with this, **the directive wins.** Canonical: [TRANSPORT.md](docs/TRANSPORT.md)
   § “The prime directive — instant connect at any roster size” (+ acceptance criteria).
1. **`ghal_bol` (Rust) owns all product logic** — crypto, keystore, libp2p, outbox, **ack send/retry**, contacts, transcripts, invite codec, call signaling. Implement behaviour here and expose **`ghal_bol_ffi_*`** (or daemon JSON-RPC on Linux/Android `:p2p`).
2. **`ghal_bol_ui` (Flutter) is a thin shell** — screens, navigation, hub layout, QR scan/share UI, composer, rendering delivery ticks from native state. **Do not re-implement ack policy, outbox, or transcript merge in Dart.** Session signals: **`GhalBolUiSession` only** (`setVisible` + `setRoom` → `p2p_sync_ui_session`) — never deprecated `setAppAckReadEnabled` / `setForegroundPeer` / `setAppUiVisible` from product code. **Do not** HTTP coord lookup, coord register ticks, or `dial_bootstrap_peers` from Dart — `sync_contacts` / `register_dm_peer` only; WAN recovery, coord register/lookup/dial, and LAN-vs-WAN routing run in **`chat_server.rs`** / **`coord_runtime.rs`** (see override rules below).
3. **`docs/DESIGN.md` is canonical** for architecture; **`docs/TRANSPORT.md`** for libp2p transport and connectivity policy. Wire detail: `docs/GHAL_BOL_DM_MSG_V1.md`. Invites: `docs/GHAL_BOL_URI_SCHEME.md`. If code and agent-editable docs disagree, **fix both in the same change**.
4. **Guest scans host QR** — guest stores host `public_key_hex` and dials. **Host may have zero contacts** until first inbound. **Never** require mutual QR or “both sides need each other’s key from QR”.
5. **Do not `p2p_stop` / restart libp2p on every contact change** — use `register_dm_peer` / `sync_contacts` hot-register only.
6. **Do not run `scripts/sync_ghal_bol_native_for_flutter.sh` while the Linux app is running** — it stops `ghal_bol_daemon` and causes `Broken pipe` on the UI socket. **Android:** rebuild with `pack_android_workspace_jni_libs.sh` — **default must ship all four standard ABIs** (`armeabi-v7a`, `arm64-v8a`, `x86`, `x86_64`) for Play, emulators, and 32-bit ARM devices; `PACK_ANDROID_ARM64_ONLY=1` is a dev fast-path only (host `cargo-ndk`; no adb).
7. **E2E for all peer-key traffic** — Any product communication between two contacts must use **end-to-end** crypto tied to the device **secp256k1 private key** and the peer’s **66-hex public key** (same identity as chat). Includes: DM text (`secp256k1_seal`), call signaling (`ghal_bol_call_v1`), call **audio and video** media (`derive_call_media_keys_from_identity` + per-frame AES-GCM seal on `/ghal-bol/call/*` substreams). Do **not** ship peer-facing plaintext payloads or disable media/signaling E2E for “performance” without an explicit product decision.
8. **Caching — immutable only on disk** — Persist to disk only data that is **user-owned and does not change meaningfully without user action** (keystore, contacts, transcript, preferences). If a value **can change** (relay port, coord presence, mDNS LAN port, bootstrap multiaddr) and relying on a stale copy **could break chat or WAN**, **do not cache it** — refetch live (`GET /v1/relay`, `GET /v1/peers/…`, mDNS events). In-memory session mirrors and short storm throttles are OK when cleared on failure. New cache only with an explicit documented exception in TRANSPORT.md § “Caching policy”. See also golden rule on **`dm_upkeep` LAN** (event-driven only).
9. **Avoid assumed timers for async P2P work** — General rule (not dial-only): when policy (A) depends on work with unknown duration, a worker (B) owns it until the stack reports an outcome; B **notifies subscribers** and A reacts **instantly** — never “wait N seconds then retry.” Applies to connect, handover, coord lookup, relay reserve, stream open, register, etc. Timers only for guardrails (in-flight observation, storm throttles, keepalive, register dedupe). See [TRANSPORT.md](docs/TRANSPORT.md) § “Event-driven async — avoid assumed timers”. **Do not** reintroduce grace-window coord blackout or tick-polled recovery without a new event.

## Repository layout

| Path | Role |
|------|------|
| `ghal_bol/` | Rust crate: `rlib` + `cdylib`, optional `ghal_bol_daemon` binary |
| `ghal_bol_ui/` | Flutter app (`com.ghalbol`) |
| `docs/` | Design + wire specs (source of truth with code) |
| `scripts/` | Native build/sync for Flutter |

**Workspace root** = directory containing root `README.md` and `Cargo.toml`.

## Architecture (one picture)

```text
┌──────────────────────────────────────────────────────────────┐
│  ghal_bol_ui — Flutter (main process on Android)              │
│  Unlock UI, ChatHub, ChatScreen, scan/share, poll for events  │
│  Sets foreground peer + app_ack_read_enabled via RPC/FFI      │
│  MUST NOT: send acks, own outbox, merge dm_message stores     │
└────────────────────────────┬─────────────────────────────────┘
                             │ dart:ffi  OR  Unix socket RPC
┌────────────────────────────▼─────────────────────────────────┐
│  ghal_bol — Rust                                              │
│  chat_server.rs   libp2p streams, outbox, ack_received/read   │
│  dm_event_handler.rs   poll path → contacts + transcript      │
│  contacts_v1.rs, dm_transcript_store.rs, connect_invite_v1    │
│  p2p_runtime.rs, p2p_ffi.rs, daemon/server.rs (RPC)         │
└──────────────────────────────────────────────────────────────┘

Android :p2p process     Linux ghal_bol_daemon
(GhalBolP2pService)      (bundled libexec/)
     ↑ same Rust node, separate from UI process
```

## Where to put new code

| Concern | Owner | Primary files |
|---------|--------|----------------|
| Send/recv acks, outbox, stream I/O | **Rust** | `ghal_bol/src/p2p/chat_server.rs` |
| Apply events → JSON stores | **Rust** | `ghal_bol/src/dm_event_handler.rs` |
| Contacts / unread / preview / **`is_known` / `is_blocked`** | **Rust** (`contacts_v1.rs`); trust banner + **Unknown** chip **Flutter** — `docs/DESIGN.md` § Contact trust |
| Transcript lines, `delivery` | **Rust** | `ghal_bol/src/dm_transcript_v1.rs`, `dm_transcript_store.rs` |
| Invites format 2 | **Rust** + thin Dart codec | `connect_invite_v1.rs`, `invite_uri_codec.dart` |
| Hub, roster, which room is open | **Flutter** | `chat_hub_screen.dart` |
| Display ticks (no policy) | **Flutter** | `chat_screen.dart`, `dm_delivery_sync.dart` (comments only) |
| Start P2P / dm_peers list | **Flutter** | `p2p_network_coordinator.dart` |
| Poll loop (UI refresh only) | **Flutter** | `p2p_event_bridge.dart` |
| OS network hints (Wi‑Fi link, default route) | **Rust** | `android_network.rs`, `linux_network.rs`, `network_transport.rs` (`OsNetworkSnapshot`), `chat_server.rs` `network_tick` |

## Message state (do not break this)

Read **`docs/DESIGN.md`** in full before touching acks, ticks, foreground, or transcript — especially § **“Truthful status in the UI”**, § **“Leave / backlog”**, and § **“Transcript threads and conversation keys”**.

- **Recipient decides** delivery/read; sender never invents ticks or sends `ack_request`.
- **Truthful UI only** — show `delivery` / read ticks after native transcript patch on poll; never optimistic or Dart-invented state.
- **No sync between devices** — each side has its own transcript; disagreement is normal until acks arrive.
- **Delivery always** (`ack_received` from `:p2p`); **read only** when hub has the room open for **new** inbound.
- **After leave:** no **new** `ack_read` for **new** mail; **must keep** retrying `ack_read` for messages seen while the room was open until sender confirms.
- **Hub room close:** `setForegroundConversation(null)` via `syncUiSession` — bridge applies leave drain atomically.
- **Hub room open:** `setUiVisible(true)` when resumed + `setForegroundConversation(peer)` — native opens read gate then foreground.
- **Android inactive:** `setUiVisible(false)` — no new read receipts; do not leave read gate on “because room is still open”.
- **Linux desktop inactive:** **do not** `setUiVisible(false)` — GTK window drag/focus flicker must not set `:p2p` `read=false` while chat pane is open (DESIGN.md § “Fixed 2026-06-15 — Linux desktop read ticks”). Room open sync: **`setVisible(true)` then `setRoom(pk)`**.
- **Splitting UI session RPCs** — use `p2p_sync_ui_session` only; separate `set_app_ack_read_enabled` / foreground calls drift after recover/node_ready.
- **Flutter poll refreshes UI only** — never sends acks.
- **Read acks are near-single-shot:** one immediate `ack_read` per text id in-room, then ~1 s retries **only until** peer `ack_received` confirms — see **`docs/DESIGN.md`** § “Read receipts — wire volume, confirm loop, poll”. Duplicate `ack_read` floods are **bugs**, not normal.
- **Transcript load:** merge **peer id + public key** conversation keys so chat is not empty while roster preview exists.

## P2P processes

| Platform | libp2p runs in | UI talks via |
|----------|----------------|--------------|
| **Android** | Process **`:p2p`** — `GhalBolP2pService` | Unix socket `files/.../ghalbol/p2p.sock` JSON-RPC |
| **Linux desktop** | **`ghal_bol_daemon`** in bundle `libexec/` | `$XDG_RUNTIME_DIR/ghalbol/p2p.sock` |
| **In-process** | Same process as FFI (no daemon) | Direct `ghal_bol_ffi_p2p_*` when daemon not used |

**Unlock on daemon platforms:** daemon `unlock` + in-app FFI unlock must match same identity / data dir.

**Poll:** `p2p_poll` on the **state RPC socket** → `apply_p2p_event_json` in the P2P process writes disk → Flutter reloads via FFI. **Ack transmission does not depend on poll.**

**Unlock:** daemon `unlock` + FFI unlock must share identity and data dir. **`p2p_start` with `already_running`** must refresh `set_p2p_handler_context` and re-register `dm_peers`.

## Build & run

```bash
# From repo root — quit flutter first
# Linux desktop: sync copies lib + ghal_bol_daemon (stops stale daemon)
./scripts/sync_ghal_bol_native_for_flutter.sh
# Android phone: pack only (cargo-ndk → build/android-native-ndk/; no adb)
./scripts/pack_android_workspace_jni_libs.sh

cd ghal_bol_ui && flutter run   # Android: com.ghalbol.debug; Linux debug: ~/.local/share/com.ghalbol.debug/ (release: ~/.local/share/com.ghalbol/)

# Checks
cargo test -p ghal_bol
cd ghal_bol_ui && dart analyze && flutter test
```

## Anti-patterns (do not reintroduce)

- **Rust warning suppressions** — no `#[allow(dead_code)]` / `RUSTFLAGS=-A warnings` to hide pack/sync build warnings; delete unused code or wire it up.
- Mutual-QR requirement or “both need each other’s public key from QR”.
- Dart-side inbound message filtering that drops events when FFI not rebuilt.
- Hub `stores_updated` → full roster reload storm (preview debounce on inbound text; **roster** bump on `peer_identified` only — not every text).
- Blocking `p2p_poll` behind `send_text_dm` on the main RPC socket (use state socket for poll + foreground).
- Multiple concurrent `ensureDaemonRunning()` without serialization (double daemon).
- `setState` after dispose in async P2P callbacks (`mounted` check).
- Restarting full libp2p on each contact upsert.
- Sender sending `ack_request`.
- In-room **only** `ack_read` with **no** `ack_received` when `:p2p` may outlive the UI.
- Read-ack **burst/upkeep storms** (128×512 bursts, retry every poll tick, emit every wire ack to UI, `stores_updated` on no-op patch) — see DESIGN.md § “Read receipts — wire volume”.
- **Outbox burst double-send** — `resync_outbox_burst_for_peer` re-sending rows the ~1s periodic `resync_pending_outbox` just put on the wire (ignoring `OUTBOX_RESEND_INTERVAL_MS`) → peer emits **duplicate `ack_received`** per duplicate text (looks like delayed/storming acks). Burst must skip rows sent within `OUTBOX_RESEND_INTERVAL_MS`; backlog/new rows still drain instantly on stream-open. TRANSPORT.md § **Post-mortem 2026-06-25 → Follow-on fix — outbox burst double-send**.
- Hub **double transcript merge** on `previewChangeCount` while open chat already uses `ingestP2pEvent`.
- **Dual transcript writers** — UI `transcript_save` / FFI append racing `:p2p` poll on daemon, or Rust code opening `chat_transcript_v1.json` outside `dm_transcript_store` (append + `read_ack_sent` patch clobber → rows vanish). See DESIGN.md § “On-disk store ownership”.
- **Empty native reload wiping chat** — `force: true` reload that clears persisted lines when `transcriptLoadMerged` returns 0 rows during same-room refresh.
- Contact trust UI that changes **ack policy**, **foreground order**, or blocks **`ack_received`** for `is_known: false` peers — see `docs/DESIGN.md` § Contact trust (additive only).
- A second block store in preferences instead of **`is_blocked`** on `contacts_v1.json`.
- **Read-ack seed without `received_at_ms` or cutoff** — false blue ticks and ack storms; use **`dispatch_read_ack_pass`** eligibility (DESIGN.md § “Inbound `received_at_ms`”).
- **Clearing read-ack queue on leave** or clearing foreground inside `set_app_ack_read_enabled(false)` before leave drain.
- **Wrong hub close order** — `setAppAckReadEnabled(false)` before `setForegroundConversation(null)`.
- **Single conversation key** for transcript load when history uses both peer id and public key buckets.
- **Hub transcript keyed on `activeContact` alone** — roster reload after send/poll can set `activeContact` to null for a frame; `didUpdateWidget` then treats it as a room switch, reloads `conv=solo`, and **other chats look wiped**. Hub must pass stable **`hubThreadKey`** (`_selectedConversationKey`); see DESIGN.md § “Hub chat — stable thread id”.
- **Weakening E2E** — skipping video encrypt, or plaintext call/chat payloads when the peer’s secp256k1 keys are the trust anchor (see golden rule 7).
- **`redial_public_dht_bootnodes` while bootstrap connected** during WAN recovery — disconnects all bootstrap TCP and stalls relay/coord for minutes; see `docs/TRANSPORT.md` § “WAN recovery — relay reservation and bootstrap redial”. Log: `forcing bootstrap redial` with `bootstrap_ok=true`.
- **Re-issuing relay `listen_on` every tick (1s storm)** — the storm is repeating `listen_on` for a relay faster than `RELAY_RESERVE_THROTTLE_MS`, **not** covering all relays once. Reserve on **all** eligible bootstraps in parallel via `try_relay_reservations` (per-relay throttle prevents the storm). Do **not** serialize one-relay-at-a-time — that lets one pending-but-never-accepted reservation block the others and stalls WAN for minutes. **2026-06-29:** recurring `run_wan_recovery_pass` must **not** pass `force=true` to `ensure_wan_relay_circuit` — that bypasses the throttle every tick and cancels in-flight reservations after `left LAN` (TRANSPORT.md § Post-mortem 2026-06-29).
- **Bootstrap relay dial storm on CGNAT/mobile** — uncordinated `swarm.dial` to the coord relay from refetch + WAN recovery + redial (many `coord relay dial` lines per second, never `bootstrap connection` / `reservation accepted` on the phone). Must use `issue_bootstrap_dials` / `should_issue_bootstrap_dial` and CGNAT probe `listen_on` — see `docs/TRANSPORT.md` § “CGNAT / mobile-data relay reservation”.
- **Removing CGNAT probe reservation** — `try_ghalbol_probe_style_circuit_listen` at startup and in `retry_stalled_relay_reservations` when `!any_bootstrap_connected` is required for mobile-data; Wi‑Fi-only testing hides this regression.
- **One-sided relay OK** — desktop `reservation accepted` + phone stuck on `CGNAT listen addr only` → coord 404 for phone forever, no chat. Fix the phone side, not coord HTTP.
- **Blocking peer relay dials until own circuit listens** — gating `dial_dm_peer_addr` on `!relay_circuit_listening` for CGNAT logs `skip relay dial … self relay circuit not ready yet` after `coord_lookup_peer ok` and stalls WAN ~40s. Outbound peer circuit dials only need coord relay bootstrap TCP; throttle with `should_routed_dial`, not own-reservation gate. See TRANSPORT.md § “Outbound peer relay dials vs own reservation”.
- **404 coord backoff during urgent DM reconnect** — after `dm connection closed`, coord lookup must not wait exponential backoff; see `mark_dm_reconnect_urgent` + `is_pk_reconnect_urgent`.
- **Blocking `node_ready` on full WAN** (45s relay wait) — emit `node_ready` after brief relay dial; recovery continues on `coord_tick`.
- **Kademlia / public-bootstrap WAN peer discovery** when coord is down — forbidden; WAN requires coord/relay; LAN (mDNS) still works ([TRANSPORT.md](docs/TRANSPORT.md) § Connectivity lifecycle).
- **Slow WAN fallback after LAN loss** — mDNS `Expired` must re-kick coord/relay lookup immediately; do not wait on LAN TTL.
- **Orphan active call when UI is gone** — `:p2p` / daemon outliving Flutter is **not** permission to keep media up. Last UI socket EOF must run **`p2p_force_end_active_call`**; GTK X / call-screen pop / `detached` must also hang up. See `docs/DESIGN.md` § “Call UI lifecycle and privacy”.
- **Fixing call restore / notification without force-end** — changes to `call_active`, `CallController.syncActiveCallFromNative`, or incoming-call notify must not skip UI-session teardown (regression checklist in DESIGN.md).
- **Releasing call video textures on `NativeCallVideoView.dispose`** — use `CallVideoTexturePool.releaseCall` on hangup only; dispose-time release caused Linux SIGSEGV during video (DESIGN.md § “Call UI lifecycle”, GHAL_BOL_VIDEO_NATIVE_V1.md).
- **Promoting delivery/read ticks in Flutter without native transcript patch** — ticks are recipient-authority only (DESIGN.md § “Truthful status”).
- **Blocking LAN upkeep during WAN recovery** — `lan_handover_upkeep_if_needed` must run **in parallel** with WAN relay reserve (no early return). `relay_lost_on_lan` must not re-kick full handover every 5s while `wan_recovery_active` (symptom: endless `mdns restarted after LAN handover`, zero `mdns discovered`, coord down). TRANSPORT.md § “Parallel LAN + WAN transport”.
- **Closing relay links when direct LAN connects** — parallel transport keeps **both links active**; tearing down relay on LAN upgrade breaks WAN handover. See TRANSPORT.md § “Parallel LAN + WAN transport”, § “Both links active”.
- **dm_upkeep skipping coord lookup when connected** — must still additive-dial relay while direct up; use `coord_lookup_upkeep_satisfied` (stable mux + relay), not `swarm.is_connected` alone. TRANSPORT.md § “Both links active”.
- **Separate LAN/WAN message or ack stores** — all state in Rust `dm_transcript_store` / outbox; monotonic delivery merge (`read` ⊃ `delivered`). See DESIGN.md § “Unified message state (E)”.
- **Racing coord relay dials against mDNS LAN** — **superseded 2026-06-17:** parallel LAN+WAN is intentional; regressions are **uncoordinated dial spam** (many dials/s) and **stale LAN port re-dial from upkeep**, not parallel `mdns dialing` + `coord dialing` on Wi‑Fi. See TRANSPORT.md § “Parallel LAN + WAN transport”.
- **Unbounded full-roster coord lookup** — iterating **all** `dm_public_keys()` with sequential `await` per upkeep tick / handover burst (old `coord_lookup_dm_peers`). At thousands of stale contacts this floods coord and blocks the swarm loop, delaying reachable peers — violates the **prime directive**. Use `run_dm_coord_lookup_pass`: urgent/priority uncapped, idle contacts LRU-swept under `COORD_BACKGROUND_LOOKUPS_PER_TICK`. Do **not** let a global wake bypass the background cap, and do **not** cap/back-off urgent or priority (outbox/foreground) peers. See [TRANSPORT.md](docs/TRANSPORT.md) § “The prime directive”.
- **Silent early returns on the connect path** — skip/defer/return in the connect/lookup/upkeep flow with no log, so a stuck peer is undebuggable. Log the decision (intent-gated + throttled to avoid roster spam). And never log “dialing/ok” on a no-op. See [TRANSPORT.md](docs/TRANSPORT.md) § “Logging — see the precise flow”.
- **Competing dial policies instead of stream-first symmetric connect** — a third path calling `swarm.dial` for the same peer alongside the two legitimate owners. **Dial ownership is per-transport and event-driven:** LAN dials are owned by the **mDNS `Discovered` handler**, WAN dials by **coord lookup** (upkeep tick / `notify_coord_lookup`). `register_dm_peer` / `send_text` / **identify** (when coord configured) must **signal only** (`notify_coord_lookup` / `notify_dm_presence_wake`), never dial. "One upkeep owner" means one owner of the **stream/connection lifecycle**, not one function issuing every dial — and it does **not** justify routing LAN dials through the 1s tick (that breaks TRANSPORT.md § “Ephemeral LAN TCP ports” event-driven LAN). The shared in-flight guards are **required guardrails**, not patches to strip. See [DESIGN.md](docs/DESIGN.md) § “Stream-first symmetric connect”.
- **P2P transport caching** — any dial/lookup/addr cache that staleness could break. Canonical: TRANSPORT.md § “Caching policy”. Disk = immutable user data only.
- **Timer-based async policy** — grace windows, tick-polled recovery, or tuning `N`-second constants instead of worker→subscriber events. Applies to connect/handover and any P2P path with unknown duration. See TRANSPORT.md § “Event-driven async”.
- **Flutter network-change RPCs** — no `p2p_notify_network_change` / resume connectivity hints from Dart; Android `:p2p` registers `ConnectivityManager` callbacks; Linux uses `linux_network.rs` operstate on `network_tick`; Rust owns Wi‑Fi handover recovery.
- **Soft mDNS-only Wi‑Fi switch recovery** — `lan_handover_upkeep` must call full `kick_lan_dm_rediscovery_after_handover` (fresh listen + force mDNS), not `restart_mdns_behaviour` alone; symptom: repeating `LAN upkeep — nudge mDNS` with zero `mdns discovered`. See TRANSPORT.md § “LAN stability — cold start and Wi‑Fi toggle”.
- **`libp2p::mdns::Config::default()` for the chat node** — its `query_interval` is **5 minutes**, so after a LAN link drops the peer is not re-discovered for minutes (`LAN soft rediscovery — link down, no mDNS candidate yet`, `active_links=0`) — LAN looks broken whenever WAN/relay is also down. Always build mDNS via `ghal_bol_mdns_config()` (`query_interval=5s`); fast LAN re-discovery must come from the **query interval**, not from rebinding the TCP port or restarting mDNS on a tick (that is the churn storm). TRANSPORT.md § **LAN re-discovery cadence — mDNS query interval**.
- **Recovery throttle double-consume** — do not call `should_run_lan_recovery` then only soft-restart mDNS; `kick_lan` owns the 5s throttle.
- **Forbidden 2026-06-15 hub UI session patch** — `lastApplySucceeded` / `uiSessionLastApplyOk`, `_invalidateNativeForegroundSync`, per-frame session retry from `build()`, hub `node_ready`/`_attachHubChat`/`resume`/`call end` session reapply storms. **Reverted:** stopped P2P chat (`stream_ready_count=0`, leave-drain bursts), UI looked wiped (`conv=solo`), users lost identity indirectly. **Do not** use this to fix Linux read ticks — use Linux **`inactive`** rule + low-volume `GhalBolUiSession.nudge()` instead (DESIGN.md § “Fixed 2026-06-15”). § “FORBIDDEN — reverted 2026-06-15”.
- **Linux desktop `inactive` → setVisible(false)** — regression; restores “resize fixes ticks” bug (`ui_session_applied read=false` with room open).
- **Port guessing / ranking for LAN** — highest-port-wins, preferred mDNS addr, probing with `nc` instead of mDNS `Discovered`/`Expired` + `Native/flow` listen_addrs. Ephemeral ports change every restart; see TRANSPORT.md § “Ephemeral LAN TCP ports”.
- **`if_addrs`-only `profile=lan|mobile-data`** — must use OS default route (`OsNetworkSnapshot`); `rmnet` visible ≠ mobile-data default. TRANSPORT.md § **Network truth**.
- **Full `disconnect_peer_id` when relay link exists** on mux reconcile / stream open timeout — use `close_direct_dm_connections` + reopen on relay. TRANSPORT.md § **Asymmetric LAN↔WAN mux recovery**.
- **Skipping coord lookup for connected foreground peer during LAN handover** — `coord_lookup_upkeep_satisfied` must be false when `lan_listen_rediscovery_requested` + foreground/outbox, or `peer_wan_asymmetric_mux_likely`. Symptom: one-way WAN (inbound ok, outbound no `ack_received`). TRANSPORT.md § **Post-mortem 2026-06-25**.
- **Trusting mDNS/TTL alone when outbound stuck** — `peer_has_stale_direct_lan_conn` must treat lingering mDNS + stuck outbox as stale after remote leaves LAN. TRANSPORT.md § **Post-mortem 2026-06-25**.
- **Reverting asymmetric mux fix for bursty delivery** — multi-second batch delivery during handover is expected (reconcile throttle + burst resync); do not move connect policy to Flutter `NetworkHelper`. TRANSPORT.md § **Known symptom — bursty delivery**.
- **Urgent reconnect clears in-flight relay dial** — never `clear_circuit_dial_in_flight` + `disconnect_peer_id` during relay handshake; causes `Pending connection attempt has been aborted` and relay `ACCEPTED→closed` loops. TRANSPORT.md § **Post-mortem 2026-06-24**.
- **`dm_peer_chat_link_stable=false` when relay hop missing** — on Wi‑Fi with healthy LAN stream this forces coord lookup/dial every upkeep tick and kills steady chat after backlog drain. Use `needs_additive_relay_dial` instead. TRANSPORT.md § **Post-mortem 2026-06-24**.
- **Coord lookup early return when additive relay needed** — after coord HTTP when stream stable but `!peer_has_relay_connection`, must still `coord_dial_from_lookup_addrs` / `dial_additive_dm_addr`. TRANSPORT.md § **Post-mortem 2026-06-24**.
- **Relay `ConnectionClosed` with other path open — no stream reopen** — zombie mux (`conn=true,stream=true`, repeating `resync N pending`). Event-driven `request_dm_stream_reopen` + `notify_coord_lookup` in `swarm_events`; ack/outbox send fail → same. TRANSPORT.md § **Post-mortem 2026-06-24**.
- **Blocking coord HTTP on tokio swarm loop** — `try_restore_relay_presence_from_coord` / `reqwest::blocking` in `run_wan_recovery_pass` panics and kills `:p2p`. Background std thread only (`coord_register_tick`). TRANSPORT.md § **Post-mortem 2026-06-24**.
- **LAN TCP fail clears circuit in-flight** — parallel LAN+WAN violation; LAN `OutgoingConnectionError` must not touch `circuit_dial_in_flight`. TRANSPORT.md § **Post-mortem 2026-06-24**.
- **Coord lookup `.await` on the swarm loop** — `coord_lookup_dm_peer` / `run_dm_coord_lookup_pass` must **never** await coord HTTP inside the `tokio::select!` arm holding `&mut swarm` (`spawn_blocking` + awaiting its handle still starves libp2p → inbound relay `STOP` times out → `relay-circuit dial timed out`, WAN dead, LAN flaky). Use `request_coord_lookup` (off-loop `tokio::spawn`) + `apply_coord_lookup_result` / `drain_ready_coord_lookups` (sync, on-loop). TRANSPORT.md § **Post-mortem 2026-06-25 (coord lookup `.await` froze the swarm loop)**.

## Debugging checklist (one message, two devices)

**Logs:** In-app App log shows `Native/flow` connectivity snapshots every ~30s, `Native/kad|coord|dial|swarm|listen|mdns`, and numbered `P2P` `step=` journey lines. Full libp2p detail on stderr/logcat: `grep ghal_bol` (all levels). Optional: `GHAL_BOL_VERBOSE_LOG=1` before start to forward Rust `debug` lines into the App log too.

**Dial — parallel LAN + WAN:** On Wi‑Fi, coord/relay and mDNS/direct TCP **both run** for configured contacts — both links may stay connected (TRANSPORT.md § “Both links active”). mDNS/direct TCP applies when the peer is on local LAN (mDNS discovery); coord/relay always when coord is configured. **`profile=lan` vs `mobile-data` follows OS default route** (`os=` in `Native/flow`) — not “cellular iface visible” in `if_addrs`. Cellular/CGNAT without active LAN → relay via coord only.

Trace the **native chain** in [DESIGN.md](docs/DESIGN.md) — do not blame Flutter for ack send.

| Step | Sender device | Receiver device (`:p2p` logs) |
|------|---------------|----------------------------------|
| 1 | `send_text_dm queued` | — |
| 2 | `peer_connected` / `chat_ready` on poll | same |
| 3 | — | inbound text + `delivery_ack: ack_received sent` |
| 4 | poll: `dm_message` `ack_received` → outbound `delivered` | `stores_updated` on poll |

| Symptom | Check |
|---------|--------|
| `queued` forever, no `chat_ready` | `dm_peers` registered? guest has host `public_key_hex`? `p2p_start` / `already_running` path? |
| Receiver gets text, sender no tick | `delivery_ack` in `:p2p`? stale foreground suppressing delivery? |
| Many `ack_read` same `ref=` / poll saturated | Broken confirm loop or retry throttle — DESIGN.md § “Read receipts”; not “add dedupe” |
| Blue tick missing | Sender not getting `ack_read`, or poll not applying; room open + visible + `may_send_in_room_read_ack`? Logs: `seeded N … cutoff_ms=`, `ack_read sent`, `patch outbound read` |
| False blue tick (peer never got read) | Read-ack seed without **`received_at_ms`** or past **`chat_room_exit_at_ms`** cutoff — DESIGN.md § “Inbound `received_at_ms`” |
| `GET /v1/relay failed (relay HTTP 400 Bad Request)` loop | Client sent `remap=1`; server expects **`remap=true`**. Fix `coord.rs` `get_relay_remap`; rebuild native |
| Blue tick when app inactive/background | **Android:** read gate off on `inactive` — expected. **Linux desktop:** chat visible + `read=false` after `inactive` — regression (DESIGN.md § Fixed 2026-06-15). |
| Read tick missing after user left room | Leave drain must run: log `chat room leave … cutoff_ms=` + `chat room frozen`; hub close order; queue not cleared |
| `chat room enter … skipped — app not visible` | Gate off before foreground cmd; fix hub open order or `RunReadAckCatchup` |
| Hub preview OK, chat empty | Transcript key split; use merged load (peer id + public key) — DESIGN.md § “Transcript threads” |
| Ticks appear without peer ack | Fake state — Flutter must not promote; check poll + transcript patch only |
| Wire OK, empty roster | `handler context not set` on daemon poll; unlock + `p2p_start` with `app_namespace` |
| Host no contact after scan | `peer_identified` or inbound text `stores_updated` → roster bump + `merge_discovered_peer_id` |
| LAN chat broken on Wi‑Fi (mDNS shows peer, no connect) | Stale LAN port re-dial from upkeep? Same `mdns dialing …/tcp/PORT` while `listen_addrs` shows different port? **Not** parallel coord+mdns dial (that's OK). Check stale contact pk. TRANSPORT.md § “Ephemeral LAN TCP ports”. |
| Wi‑Fi toggle: LAN dead after switch | Repeating `LAN soft rediscovery` with **no** `deferred full kick` / `fresh ephemeral TCP listen`? Pending full kick stuck — TRANSPORT.md § “Deferred full LAN kick”. Expect `LAN DM rediscovery — deferred full kick` then `fresh ephemeral TCP listen`. Full kick on connectivity notify is OK too. |
| Same `mdns dialing …/tcp/PORT` every ~20s for minutes; `listen_addrs` / fresh `mdns discovered` shows **different** port | **Stale mDNS candidate cache + upkeep LAN re-dial** — not a port to hardcode. Fix: event-driven LAN from mDNS `Discovered`; no timer re-dial from candidate set; parallel coord+WAN OK. TRANSPORT.md § “Ephemeral LAN TCP ports”. Full app restart after native rebuild. |
| `profile=lan` but phone on mobile-data; desktop `dm connection … (direct)`; `outbox_pending` high | **Asymmetric mux** — stream on dead LAN path | `os=cell` on phone; `close direct DM link … relay kept`; TRANSPORT.md § **Asymmetric LAN↔WAN mux recovery** |
| One-way WAN after handover: inbound ok, outbound no ticks; `lookup pass: urgent=0` with foreground peer stuck outbox | **2026-06-25 coord skip during LAN rediscovery** | `peer_wan_asymmetric_mux_likely`; coord lookup for active peer; TRANSPORT.md § **Post-mortem 2026-06-25** |
| Messages/acks stall seconds then all arrive at once; both directions eventually work | **Bursty WAN recovery (known)** — not Flutter poll or NetworkHelper | `burst resync`, 5s reconcile throttle; do not revert asymmetric fix; TRANSPORT.md § **Known symptom — bursty delivery** |
| `reopen … peer off LAN` every 1s; server relay ACCEPTED/closed loop | **Mux reconcile storm** + full `disconnect_peer_id` on relay | `dm_direct_conn_ids`; throttle reconcile; keep relay on link reset — TRANSPORT.md § **Network truth** regressions table |
| Backlog delivered then new mail/acks stop; `conn=true,stream=true` | **2026-06-24 zombie mux / dial churn** — relay path closed, stream flag stale, or upkeep declared mux unstable for missing relay | TRANSPORT.md § **Post-mortem 2026-06-24**: `relay circuit in-flight cleared … urgent`, repeating `resync N pending`, `(other path still open)` without stream reopen |
| `Pending connection attempt has been aborted`; relay `ACCEPTED→closed` ~1s | **Urgent dial abort loop** — in-flight guard cleared during handshake | Never clear `circuit_dial_in_flight` on urgent; TRANSPORT.md § Post-mortem 2026-06-24 |
| `:p2p` panic `Cannot drop a runtime … blocking` during WAN recovery | **Blocking coord HTTP on swarm thread** | Remove sync HTTP from tokio loop; TRANSPORT.md § Post-mortem 2026-06-24 |
| `profile=` wrong minutes after toggle; `os=` missing in flow log | Stale native build or `if_addrs`-only profile | Rebuild native; `os=wifi|cell/validated/…` must flip ~1s — TRANSPORT.md § **Network truth** |
| `mdns restarted after LAN handover` every ~5–12s, no `mdns discovered`, WAN stuck (`wan_recovery=true`, coord down) | **LAN blocked by WAN recovery loop** — `relay_lost_on_lan` or early return in `lan_handover_upkeep`. Fix parallel upkeep; fix coord/relay on VM. TRANSPORT.md § “Parallel LAN + WAN transport”. |
| Chat worked 5–10 min then died on LAN | Linux idle timeout was 300s; listen port may have changed — check `dm peer disconnected` + stale dial loop above. Desktop idle now 120s. |
| WAN chat dead minutes, coord health OK | `forcing bootstrap redial` loop? `wan_recovery=true` + `relay_listen=false` + `bootstrap_ok=true`? Fix `run_wan_recovery_pass` — never disconnect coord relay for relay; rebuild native. TRANSPORT.md § WAN recovery. **Note:** `bootstrap_*` logs = coord relay, not IPFS peers. |
| After `left LAN`: `reservation listen_on issued` every ~1s + `Failed to get Reservation`, `relay_listen=false` | **WAN recovery `force` storm** — `run_wan_recovery_pass` must use `ensure_wan_relay_circuit(…, false)`; `force=true` only on handover entry. Grep for `force: true` in recovery path. TRANSPORT.md § **Post-mortem 2026-06-29**. |
| Coord lookup 404 for peer | Peer not on coord yet — both need `reservation accepted` + `coord registered`; **not** proof coord HTTP is down. If **all** lookups 404 and server shows no `peer registered`, relay TCP is dead (dev: bore stopped / wrong port). **Asymmetric:** Wi‑Fi side registered, phone 404 → phone never got relay circuit — TRANSPORT.md § “CGNAT / mobile-data relay reservation” |
| Phone: many `coord relay dial`/s, no `bootstrap connection`, `CGNAT listen addr only` | Bootstrap **dial storm** or missing CGNAT probe reservation — rebuild native; see TRANSPORT.md § “CGNAT / mobile-data relay reservation” |
| `coord_lookup_peer ok` then `skip relay dial … self relay circuit not ready yet` (no `peer_connected` for ~40s) | **Regression:** peer relay dial gated on own reservation — remove gate; peer dials use `should_routed_dial` only. TRANSPORT.md § “Outbound peer relay dials vs own reservation” |
| Endless `GET /v1/peers/…` 404 on coord server, `GET /v1/relay` 200 | Relay TCP unreachable while HTTP OK | GCP: `nc -zv` relay host `:4002`. Home coord1: `./ghal_bol_server/deploy/verify_coord1.sh` (relay `:55002`); restart coord server; apps refetch live `GET /v1/relay` on next coord tick |
| `waiting for relay/public listen endpoint before coord register` | No relay circuit — cannot register WAN endpoint | Fix relay/firewall; `coord_registered=false` until `reservation accepted` |
| `relay has no public address advertised` / `advertised=[]` on server | `GHAL_BOL_RELAY_PUBLIC_HOST` unset or relay disabled | Set public host; restart coord server |
| Slow reconnect after idle | `dm peer disconnected` then 404 backoff + stale CGNAT `Timeout` dials? TRANSPORT.md § Idle chat reconnect — urgent reconnect must not apply 404 backoff; no blind `try_routed_dial` for WAN coord peers |
| Relay reservation never `accepted` | Is the **Ghal Bol relay** (coord-colocated, `GET /v1/relay`) reachable? Check `ghalbol relay … preferred for reservation` at startup + `relay v2 node started` in coord logs. Reserve on **all** configured coord relays (`try_relay_reservations`), per-relay throttled. Do **not** expect public IPFS bootstraps to substitute. See TRANSPORT.md § "Ghal Bol relay" |
| Call still active after Linux X / Ctrl+C / UI kill | UI gone but `:p2p`/daemon still up — must **`force_end_active_call`** on last UI socket EOF. Log: `force_end_active_call reason=ui_session_ended`. Also check Flutter `window_closed_by_user` / `call_screen_dismissed_*`. DESIGN.md § “Call UI lifecycle and privacy” |
| Linux **SIGSEGV** / sudden exit during **video** call | **Regression:** releasing `CallVideoTexturePool` / embedder texture on widget dispose mid-call. Textures must release on **hangup/end** only (`releaseCall` after `callVideoStop`). See DESIGN.md § “Call UI lifecycle”, GHAL_BOL_VIDEO_NATIVE_V1.md § “Flutter video textures and call end” |
| Linux desktop: read acks missing while chat visibly open | **Regression:** Linux `inactive` → `setVisible(false)` while room open (`ui_session_applied read=false`). Fix: DESIGN.md § “Fixed 2026-06-15 — Linux desktop read ticks”. **Never** forbidden `lastApplySucceeded` patch. |
| Chat dead, `stream_ready_count=0`, hub bootstrap `room closed` ×N | Forbidden session-sync patch or foreground storm — not coord-only. DESIGN.md forbidden table |
| UI empty / `conv=solo`, disk has `ghal_bol/*.json` | Session desync — not keystore delete. Do not new identity |
| Linux desktop: single tick, peer did read | Check **recipient** logs for `ack_read sent` — gate may be off on their side; sender transcript `delivery=delivered` is truthful until peer ack arrives |
| Android: read on screen, sender single tick | **`inactive`** gates read off while room stays open; **`resumed`** must log `read gate opened — catch-up ack_read` (`p2p_sync_ui_session` queues catch-up even if foreground unchanged). DESIGN.md § “Fixed 2026-06-29”. |
| Android: display off, sender gets no tick at all (both Wi-Fi and mobile data) | **Battery optimization / app hibernation** throttles `:p2p` despite foreground service. Layer 1: `requestBatteryOptimizationExemption()` (system dialog, one-time). Layer 2: `isUnusedAppPauseEnabled()` fallback prompt. `ack_received` is not gated on UI — the process is throttled. DESIGN.md § "Fixed 2026-07-03". |
| Incoming-call notification tap does not show UI | Daemon wrote `incoming_call_wake` but Flutter not polling — check wake poll in `p2p_event_bridge.dart` + D-Bus activate in `incoming_call_notify.rs` |
| Relay conn drops mid-handshake (`Decode(UnexpectedEof)`), `addrs=[]`, `coord_registered=false` on **every real device** | **Relay server missing `secp256k1` libp2p feature.** Clients use secp256k1 device keys; a relay without that feature can't authenticate them in Noise and drops the link. Add `secp256k1` to `ghal_bol_server/Cargo.toml`. An ed25519-only `relay_probe` hides this — test with `PROBE_SECP256K1=1`. **Not** a Kademlia/listener bug. See TRANSPORT.md § "Ghal Bol relay" |
| `Unexpected peer ID` on relay bootstrap dial | Stale in-memory relay state after server restart — client clears state and refetches `GET /v1/relay` (no disk cache) |
| WAN-only dead (`coord.ghalbol.com`), LAN flaky; `coord_lookup_peer ok` then `relay-circuit dial timed out`; server `circuit ConnectionFailed`; clean `relay_probe`/`circuit_test` work | **Coord lookup `.await` froze the swarm loop** — libp2p not polled during coord HTTP RTT → inbound relay `STOP` times out. Use `request_coord_lookup` (off-loop) + `apply_coord_lookup_result` / `drain_ready_coord_lookups` (sync). Rebuild native. TRANSPORT.md § **Post-mortem 2026-06-25 (coord lookup `.await` froze the swarm loop)** |
| `node_ready` minutes late | Startup blocked on relay — must emit after ~3s; WAN recovery on coord_tick |

## Doc index

| File | Use |
|------|-----|
| `docs/DESIGN.md` | Layers, truthful ticks, state model, room open/close, leave backlog, transcript keys |
| `docs/GHAL_BOL_DM_MSG_V1.md` | Wire + ack kinds + upkeep |
| `docs/GHAL_BOL_URI_SCHEME.md` | QR / `ghalbol://` invites |
| `docs/GHAL_BOL_VOICE_V1.md` | Call signaling (`ghal_bol_call_v1`) |
| `docs/GHAL_BOL_CALL_NATIVE_V2.md` | Native Rust voice engine over the P2P link (shipping) |
| `docs/GHAL_BOL_VIDEO_NATIVE_V1.md` | Native Rust video wire/engine (shipping) |
| `docs/COORDINATION_SERVER.md` | Run/test coord server, local dev stack, **HTTP log troubleshooting** |
| `docs/TRANSPORT.md` | libp2p transport, **Connectivity lifecycle**, **Network truth**, **Asymmetric mux recovery**, **Post-mortem 2026-06-24**, **Post-mortem 2026-06-25**, caching policy, LAN stability, WAN/CGNAT |
| `docs/ROADMAP.md` | Human product backlog only — not agent implementation specs |
| `ghal_bol_server/deploy/README.md` | Home `coord1`, GCP deploy, smoke; **§ Regression prevention** |
| `docs/WEB_SITE.md` | Static **ghalbol.com** web build, Firebase, Linux download, `/connect/…` handoff |
| `README.md` | Product vision + repo map |
| `ghal_bol_ui/README.md` | Flutter shell scope (native vs `bootstrap_web`) |

## Naming

- Product: **Ghal Bol** — domain **ghalbol.com**, package **`com.ghalbol`**
- Android namespace: `com.ghalbol`
- Rust crate / Dart package: `ghal_bol` / `ghal_bol_ui`


## Important Rules that should override the above ones 
After user login for the first time then background service (ghal_bol) should
start running. It should watch the network continuously, should know the status 
of the internet, should be quick to figure out it's global reachable address and also it's 
LAN address and as soon as it's global reachable address is found it should regularly
register itself at the coord server. WAN should always work if internet is active for
both the peers and if coord server is reachable. Now if any peer is found on LAN then only 
for that peer LAN should be used and in case if LAN is lost then again it should repeat the
retular process of WAN and this switch shouldn't impact user experience, he shouldn't see any
weird behavior. Now in case if coord server is unreachable then it should use KAD and libp2p
to figure out the destination peer reachable address but it doesn't means that trying to reach 
coord server at regular interval should be stopped. The ultimate goal is strong, reliable and
smooth interaction between peers. We already have the coord server and libp2p which should be
more than enough for smooth interaction over the WAN/LAN. 
