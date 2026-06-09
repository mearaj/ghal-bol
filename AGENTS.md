# AI agent guide — Ghal Bol workspace

**Read this file first** in a new session. Then read **`docs/STORY.md`** (connectivity / discovery — overrides conflicting docs), **`docs/DESIGN.md`** before changing P2P, acks, invites, or persistence. Transport (libp2p): **`docs/TRANSPORT.md`**.

## Golden rules

1. **`ghal_bol` (Rust) owns all product logic** — crypto, keystore, libp2p, outbox, **ack send/retry**, contacts, transcripts, invite codec, call signaling. Implement behaviour here and expose **`ghal_bol_ffi_*`** (or daemon JSON-RPC on Linux/Android `:p2p`).
2. **`ghal_bol_ui` (Flutter) is a thin shell** — screens, navigation, hub layout, QR scan/share UI, composer, rendering delivery ticks from native state. **Do not re-implement ack policy, outbox, or transcript merge in Dart.** **Do not** HTTP coord lookup, coord register ticks, or `dial_bootstrap_peers` from Dart — `sync_contacts` / `register_dm_peer` only; WAN recovery, coord register/lookup/dial, and LAN-vs-WAN routing run in **`chat_server.rs`** / **`coord_runtime.rs`** (see override rules below).
3. **`docs/DESIGN.md` is canonical** for architecture; **`docs/STORY.md`** for connectivity / discovery (overrides conflicting guidance). Wire detail: `docs/GHAL_BOL_DM_MSG_V1.md`. Invites: `docs/GHAL_BOL_URI_SCHEME.md`. If code and docs disagree, **fix both in the same change**.
4. **Guest scans host QR** — guest stores host `public_key_hex` and dials. **Host may have zero contacts** until first inbound. **Never** require mutual QR or “both sides need each other’s key from QR”.
5. **Do not `p2p_stop` / restart libp2p on every contact change** — use `register_dm_peer` / `sync_contacts` hot-register only.
6. **Do not run `scripts/sync_ghal_bol_native_for_flutter.sh` while the Linux app is running** — it stops `ghal_bol_daemon` and causes `Broken pipe` on the UI socket. **Android:** rebuild with `pack_android_workspace_jni_libs.sh` — **default must ship all four standard ABIs** (`armeabi-v7a`, `arm64-v8a`, `x86`, `x86_64`) for Play, emulators, and 32-bit ARM devices; `PACK_ANDROID_ARM64_ONLY=1` is a dev fast-path only (host `cargo-ndk`; no adb).
7. **E2E for all peer-key traffic** — Any product communication between two contacts must use **end-to-end** crypto tied to the device **secp256k1 private key** and the peer’s **66-hex public key** (same identity as chat). Includes: DM text (`secp256k1_seal`), call signaling (`ghal_bol_call_v1`), call **audio and video** media (FrameCryptor + `derive_call_media_keys_from_identity`). Do **not** ship peer-facing plaintext payloads or disable media/signaling E2E for “performance” without an explicit product decision. Connect/setup may fall back to transport-only (e.g. DTLS-SRTP) only when identity E2EE setup fails — never by default.

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

## Message state (do not break this)

Read **`docs/DESIGN.md`** in full before touching acks, ticks, foreground, or transcript — especially § **“Truthful status in the UI”**, § **“Leave / backlog”**, and § **“Transcript threads and conversation keys”**.

- **Recipient decides** delivery/read; sender never invents ticks or sends `ack_request`.
- **Truthful UI only** — show `delivery` / read ticks after native transcript patch on poll; never optimistic or Dart-invented state.
- **No sync between devices** — each side has its own transcript; disagreement is normal until acks arrive.
- **Delivery always** (`ack_received` from `:p2p`); **read only** when hub has the room open for **new** inbound.
- **After leave:** no **new** `ack_read` for **new** mail; **must keep** retrying `ack_read` for messages seen while the room was open until sender confirms.
- **Hub room close:** `setForegroundConversation(null)` **then** `setAppAckReadEnabled(false)` — never reverse.
- **Hub room open:** `setAppAckReadEnabled(true)` **then** `setForegroundConversation(peer)`.
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
- Hub **double transcript merge** on `previewChangeCount` while open chat already uses `ingestP2pEvent`.
- Contact trust UI that changes **ack policy**, **foreground order**, or blocks **`ack_received`** for `is_known: false` peers — see `docs/DESIGN.md` § Contact trust (additive only).
- A second block store in preferences instead of **`is_blocked`** on `contacts_v1.json`.
- **Fake ticks** — Flutter showing delivered/read without transcript patch from poll.
- **Clearing read-ack queue on leave** or clearing foreground inside `set_app_ack_read_enabled(false)` before leave drain.
- **Wrong hub close order** — `setAppAckReadEnabled(false)` before `setForegroundConversation(null)`.
- **Single conversation key** for transcript load when history uses both peer id and public key buckets.
- **Weakening E2E** — audio-only FrameCryptor, skipping video encrypt, or plaintext call/chat payloads when the peer’s secp256k1 keys are the trust anchor (see golden rule 7).
- **`redial_public_dht_bootnodes` while bootstrap connected** during WAN recovery — disconnects all bootstrap TCP and stalls relay/coord for minutes; see `docs/TRANSPORT.md` § “WAN recovery — relay reservation and bootstrap redial”. Log: `forcing bootstrap redial` with `bootstrap_ok=true`.
- **Re-issuing relay `listen_on` every tick (1s storm)** — the storm is repeating `listen_on` for a relay faster than `RELAY_RESERVE_THROTTLE_MS`, **not** covering all relays once. Reserve on **all** eligible bootstraps in parallel via `try_relay_reservations` (per-relay throttle prevents the storm). Do **not** serialize one-relay-at-a-time — that lets one pending-but-never-accepted reservation block the others and stalls WAN for minutes.
- **404 coord backoff during urgent DM reconnect** — after `dm connection closed`, coord lookup must not wait exponential backoff; see `mark_dm_reconnect_urgent` + `is_pk_reconnect_urgent`.
- **Blocking `node_ready` on full WAN** (45s relay wait) — emit `node_ready` after brief relay dial; recovery continues on `coord_tick`.
- **Kademlia / public-bootstrap WAN peer discovery** when coord is down — forbidden; WAN requires coord/relay; LAN (mDNS) still works ([STORY.md](docs/STORY.md)). **libp2p remains** for transport (relay circuits, DCUtR, mDNS, streams, Noise) — only the old “coord down → find peers via libp2p DHT” fallback is removed.
- **Slow WAN fallback after LAN loss** — mDNS `Expired` must re-kick coord/relay lookup immediately; do not wait on LAN TTL.

## Debugging checklist (one message, two devices)

**Logs:** In-app App log shows `Native/flow` connectivity snapshots every ~30s, `Native/kad|coord|dial|swarm|listen|mdns`, and numbered `P2P` `step=` journey lines. Full libp2p detail on stderr/logcat: `grep ghal_bol` (all levels). Optional: `GHAL_BOL_VERBOSE_LOG=1` before start to forward Rust `debug` lines into the App log too.

**Dial — WAN first:** coord/relay for remote peers; mDNS/direct TCP **only** when the peer is on local LAN (mDNS discovery). Wi‑Fi + RFC1918 → `profile=lan` but still WAN-first for contacts not on LAN. Cellular/CGNAT without active LAN → relay via coord. Do not treat “cellular iface present” as mobile-data when Wi‑Fi LAN is active.

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
| Blue tick missing | Sender not getting `ack_read`, or poll not applying; room open + `app_ack_read_enabled`? |
| Read tick missing after user left room | Leave drain must run: log `chat room leave … drain ack_read`; hub close order; queue not cleared |
| `chat room enter … skipped — app not visible` | Gate off before foreground cmd; fix hub open order or `RunReadAckCatchup` |
| Hub preview OK, chat empty | Transcript key split; use merged load (peer id + public key) — DESIGN.md § “Transcript threads” |
| Ticks appear without peer ack | Fake state — Flutter must not promote; check poll + transcript patch only |
| Wire OK, empty roster | `handler context not set` on daemon poll; unlock + `p2p_start` with `app_namespace` |
| Host no contact after scan | `peer_identified` or inbound text `stores_updated` → roster bump + `merge_discovered_peer_id` |
| WAN chat dead minutes, coord health OK | `forcing bootstrap redial` loop? `wan_recovery=true` + `relay_listen=false` + `bootstrap_ok=true`? Fix `run_wan_recovery_pass` — never disconnect bootstrap for relay; rebuild native. TRANSPORT.md § WAN recovery |
| Coord lookup 404 for peer | Peer not on coord yet — both need `reservation accepted` + `coord registered`; not proof coord server is down |
| Slow reconnect after idle | `dm peer disconnected` then 404 backoff + stale CGNAT `Timeout` dials? TRANSPORT.md § Idle chat reconnect — urgent reconnect must not apply 404 backoff; no blind `try_routed_dial` for WAN coord peers |
| Relay reservation never `accepted` | Is the **Ghal Bol relay** (coord-colocated, `GET /v1/relay`) reachable? Check `ghalbol relay … preferred for reservation` at startup + `relay v2 node started` in coord logs. Reserve on **all** configured coord relays (`try_relay_reservations`), per-relay throttled. Do **not** expect public IPFS bootstraps to substitute. See TRANSPORT.md § "Ghal Bol relay" |
| Relay conn drops mid-handshake (`Decode(UnexpectedEof)`), `addrs=[]`, `coord_registered=false` on **every real device** | **Relay server missing `secp256k1` libp2p feature.** Clients use secp256k1 device keys; a relay without that feature can't authenticate them in Noise and drops the link. Add `secp256k1` to `ghal_bol_server/Cargo.toml`. An ed25519-only `relay_probe` hides this — test with `PROBE_SECP256K1=1`. **Not** a Kademlia/listener bug. See TRANSPORT.md § "Ghal Bol relay" |
| `node_ready` minutes late | Startup blocked on relay — must emit after ~3s; WAN recovery on coord_tick |

## Doc index

| File | Use |
|------|-----|
| `docs/DESIGN.md` | Layers, truthful ticks, state model, room open/close, leave backlog, transcript keys |
| `docs/GHAL_BOL_DM_MSG_V1.md` | Wire + ack kinds + upkeep |
| `docs/GHAL_BOL_URI_SCHEME.md` | QR / `ghalbol://` invites |
| `docs/GHAL_BOL_VOICE_V1.md` | Call signaling + current WebRTC media (shipping) |
| `docs/GHAL_BOL_CALL_NATIVE_V2.md` | Native Rust voice engine over the P2P link (replaces WebRTC, phased) — voice shipping |
| `docs/GHAL_BOL_VIDEO_NATIVE_V1.md` | Native Rust **video** wire/engine spec (no WebRTC) |
| `docs/STORY.md` | **Connectivity / discovery policy** — overrides conflicting guidance |
| `docs/COORDINATION_SERVER.md` | Run/test coord server, local dev stack |
| `docs/TRANSPORT.md` | libp2p transport stack, discovery, invariants |
| `docs/WEB_SITE.md` | Static **ghalbol.com** web build, Firebase, Linux download, `/connect/…` handoff |
| `README.md` | Product vision + repo map |
| `ghal_bol_ui/README.md` | Flutter shell scope (native vs `bootstrap_web`) |

## Naming

- Product: **Ghal Bol** — domain **ghalbol.com**, package **`com.ghalbol`**
- Android namespace: `com.ghalbol`
- Rust crate / Dart package: `ghal_bol` / `ghal_bol_ui`
