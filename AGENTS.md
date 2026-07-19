# AI agent guide — Ghal Bol workspace

**Read this file first** in a new session. Then **`docs/DESIGN.md`** before changing P2P, acks, invites, or persistence. Transport (native connect): **`docs/TRANSPORT.md`** — especially § **Connectivity lifecycle**, § **Network truth**, § **Asymmetric LAN↔WAN mux recovery**.

**Connectivity policy (agents):** `docs/TRANSPORT.md` § **Connectivity lifecycle**, § **Network truth**, § **Parallel LAN + WAN transport**. Do **not** throttle coord bridge lookup or WAN recovery ticks because of informal “don’t flood” notes — throttle **storms** only. Register when **publishable endpoints change**, not every tick.

| Misread | Wrong agent behaviour (breaks WAN) | Correct meaning |
|---------|----------------------------------|-----------------|
| “Don’t register again and again” | Skip coord lookup or WAN recovery ticks | Throttle redundant **`POST /v1/register`** when endpoints unchanged (`should_throttle_register`); **force** register on endpoint change, failed register, handover, relay accepted |
| “Full eye on the network” | Register/coord HTTP on every tick; or tear down steady links | Continuous profile watch; register when **publishable endpoint changes** — not spam |
| “`if_addrs` shows Wi‑Fi / rmnet” | `profile=lan` minutes after mobile-data switch; wrong dial path | **OS default route** (`os=cell` / `os=wifi` in `Native/flow`) — TRANSPORT.md § **Network truth** |

## Golden rules

0. **Prime directive — instant connect at any roster size (calls + LAN text).** Whenever two peers have *any*
 technically reachable path (LAN, coord bridge for **calls**), they must **connect within a
 few seconds**. **WAN text** uses `ghal_bol_delivery`. Coord/bridge upkeep serves **calls** and LAN paths, not WAN DM text when `GHAL_BOL_DELIVERY_URL` is set. Note: It is no longer pure WAN p2p, but it is LAN/Voice/Video pure p2p where possible else p2p with the help of relay server. See `text_transport.rs` and [DESIGN.md](docs/DESIGN.md) § “Why pure P2P WAN text was dropped”.
1. **`ghal_bol_core` (Rust) owns all product logic** — crypto, keystore, **WAN text** (`delivery_runtime`), **LAN text + calls** (native connect), contacts, transcripts, invite codec. WAN text: delivery upload/acks; LAN text: outbox/acks; calls: signaling + media. Expose **`ghal_bol_core_ffi_*`** (or daemon JSON-RPC on Linux/Android `:p2p`).
2. **`ghal_bol_ui` (Flutter) is a thin shell** — screens, navigation, hub layout, QR scan/share UI, composer, rendering delivery ticks from native state. **Do not re-implement ack policy, outbox, or transcript merge in Dart.** Session signals: **`GhalBolUiSession` only** (`setVisible` + `setRoom` → `p2p_sync_ui_session`) — never deprecated `setAppAckReadEnabled` / `setForegroundPeer` / `setAppUiVisible` from product code. **Do not** HTTP coord lookup, coord register ticks from Dart — `sync_contacts` / `register_dm_peer` only; WAN recovery, coord register/lookup/dial, and LAN-vs-WAN routing run in native connect. **Daemon ↔ UI:** precompiled `ghal_bol_core_daemon` + contract in **`ghal_bol_core::daemon`** (Rust) and **`ghal_bol_ui/lib/daemon_client_api.dart`** (Dart mirror) — declare new RPCs in `client_api.rs` + Dart mirror only; run `./scripts/check_daemon_sdk_parity.sh`; see **`docs/DAEMON_INTEGRATOR.md`**.
3. **`docs/DESIGN.md` is canonical** for architecture; **`docs/TRANSPORT.md`** for transport and connectivity policy. Wire detail: `docs/GHAL_BOL_DM_MSG_V1.md`. Invites: `docs/GHAL_BOL_URI_SCHEME.md`. If code and agent-editable docs disagree, **fix both in the same change**.
4. **Guest scans host QR** — guest stores host `public_key_hex` and dials. **Host may have zero contacts** until first inbound. **Never** require mutual QR or “both sides need each other’s key from QR”.
5. **Do not stop / restart native connect on every contact change** — use `register_dm_peer` / `sync_contacts` hot-register only.
6. **Do not run `scripts/sync_ghal_bol_native_for_flutter.sh` while the Linux app is running** — it stops `ghal_bol_core_daemon` and causes `Broken pipe` on the UI socket. **Android:** rebuild with `pack_android_workspace_jni_libs.sh` — **default must ship all four standard ABIs** (`armeabi-v7a`, `arm64-v8a`, `x86`, `x86_64`) for Play, emulators, and 32-bit ARM devices; `PACK_ANDROID_ARM64_ONLY=1` is a dev fast-path only (host `cargo-ndk`; no adb).
7. **E2E for all peer-key traffic** — Any product communication between two contacts must use **end-to-end** crypto tied to the device identity key and the peer’s contact identity wire (same identity as chat). Includes: DM text (**transport KEM v2** after `TransportKemHello`), call signaling (`ghal_bol_call_v1` + **transport KEM** `CALL_CIPHER_TRANSPORT_V2`), call **audio and video** media (`derive_call_media_keys_from_transport` + per-frame AES-GCM seal on `/ghal-bol/call/*` substreams). Offline auxiliary FFI seal (`offline_seal_v1`, secp256k1 recipient only) is not used by product chat/call paths. Do **not** ship peer-facing plaintext payloads or disable media/signaling E2E for “performance” without an explicit product decision.
8. **Caching — immutable only on disk** — Persist to disk only data that is **user-owned and does not change meaningfully without user action** (keystore, contacts, transcript, preferences). If a value **can change** (relay port, coord presence, mDNS LAN port, bootstrap multiaddr) and relying on a stale copy **could break chat or WAN**, **do not cache it** — refetch live (`GET /v1/relay`, `GET /v1/peers/…`, mDNS events). In-memory session mirrors and short storm throttles are OK when cleared on failure. New cache only with an explicit documented exception in TRANSPORT.md § “Caching policy”. See also golden rule on **`dm_upkeep` LAN** (event-driven only).
9. **Avoid assumed timers for async P2P work** — General rule (not dial-only): when policy (A) depends on work with unknown duration, a worker (B) owns it until the stack reports an outcome; B **notifies subscribers** and A reacts **instantly** — never “wait N seconds then retry.” Applies to connect, handover, coord lookup, relay reserve, stream open, register, etc. Timers only for guardrails (in-flight observation, storm throttles, keepalive, register dedupe). See [TRANSPORT.md](docs/TRANSPORT.md) § “Event-driven async — avoid assumed timers”. **Do not** reintroduce grace-window coord blackout or tick-polled recovery without a new event.

## Repository layout

| Path | Role |
|------|------|
| `ghal_bol_core/` | Rust crate: `rlib` + `cdylib`, optional `ghal_bol_core_daemon` binary |
| `ghal_bol_coord/` | Coordination + relay server |
| `ghal_bol_delivery/` | Delivery server (WAN text mailbox) |
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
│  ghal_bol_core — Rust                                         │
│  delivery_runtime — WAN text (E2E mailbox)                    │
│  native connect — LAN text + calls                            │
│  dm_event_handler, contacts, transcripts, invites             │
│  p2p_runtime, p2p_ffi, daemon/server.rs (RPC)                 │
└──────────────────────────────────────────────────────────────┘

Android :p2p process     Linux ghal_bol_core_daemon
(GhalBolP2pService)      (bundled libexec/)
     ↑ same Rust node, separate from UI process
```

## Where to put new code

| Concern | Owner | Primary files |
|---------|--------|----------------|
| WAN text send/recv, delivery acks | **Rust** | `ghal_bol_core/src/delivery_runtime.rs`, `delivery_client.rs`, `delivery_read_acks.rs` |
| LAN text send/recv acks, outbox, stream I/O | **Rust** | `ghal_bol_core/src/connect/` |
| Apply events → JSON stores | **Rust** | `ghal_bol_core/src/dm_event_handler.rs` |
| Contacts / unread / preview / **`is_known` / `is_blocked`** | **Rust** (`contacts_v1.rs`); trust banner + **Unknown** chip **Flutter** — `docs/DESIGN.md` § Contact trust |
| Transcript lines, `delivery` | **Rust** | `ghal_bol_core/src/dm_transcript_v1.rs`, `dm_transcript_store.rs` |
| Invites format 2 | **Rust** + thin Dart codec | `connect_invite_v1.rs`, `invite_uri_codec.dart` |
| Hub, roster, which room is open | **Flutter** | `chat_hub_screen.dart` |
| Display ticks (no policy) | **Flutter** | `chat_screen.dart`, `dm_delivery_sync.dart` (comments only) |
| Start P2P / dm_peers list | **Flutter** | `p2p_network_coordinator.dart` |
| Poll loop (UI refresh only) | **Flutter** | `p2p_event_bridge.dart` |
| Android background permissions / OEM onboarding (screen off) | **Flutter** (sequential UI) + **Kotlin** (OS checks, settings intents) | `android_background_readiness.dart`, `BackgroundReadiness.kt`, `embedder_storage.dart`, `chat_hub_screen.dart` |
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

| Platform | native connect runs in | UI talks via |
|----------|----------------|--------------|
| **Android** | Process **`:p2p`** — `GhalBolP2pService` | Unix socket `files/.../ghal_bol/p2p.sock` JSON-RPC |
| **Linux desktop** | **`ghal_bol_core_daemon`** in bundle `libexec/` | `$XDG_RUNTIME_DIR/ghal_bol/p2p.sock` |
| **In-process** | Same process as FFI (no daemon) | Direct `ghal_bol_core_ffi_p2p_*` when daemon not used |

**Unlock on daemon platforms:** daemon `unlock` + in-app FFI unlock must match same identity / data dir.

**Poll:** `p2p_poll` on the **state RPC socket** → `apply_p2p_event_json` in the P2P process writes disk → Flutter reloads via FFI. **Ack transmission does not depend on poll.**

**Unlock:** daemon `unlock` + FFI unlock must share identity and data dir. **`p2p_start` with `already_running`** must refresh `set_p2p_handler_context` and re-register `dm_peers`.

## Build & run

```bash
# From repo root — quit flutter first
# Linux desktop: sync copies lib + ghal_bol_core_daemon (stops stale daemon)
./scripts/sync_ghal_bol_native_for_flutter.sh
# Android phone: pack only (cargo-ndk → build/android-native-ndk/; no adb)
./scripts/pack_android_workspace_jni_libs.sh

cd ghal_bol_ui && flutter run   # Android: com.ghalbol.debug; Linux debug: ~/.local/share/com.ghalbol.debug/ (release: ~/.local/share/com.ghalbol/)

# Checks
cargo test -p ghal_bol_core
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
- Restarting full native connect on each contact upsert.
- Sender sending `ack_request`.
- In-room **only** `ack_read` with **no** `ack_received` when `:p2p` may outlive the UI.
- Read-ack **burst/upkeep storms** (128×512 bursts, retry every poll tick, emit every wire ack to UI, `stores_updated` on no-op patch) — see DESIGN.md § “Read receipts — wire volume”.
- **Outbox burst double-send** — `resync_outbox_burst_for_peer` re-sending rows the ~1s periodic `resync_pending_outbox` just put on the wire (ignoring `OUTBOX_RESEND_INTERVAL_MS`) → peer emits **duplicate `ack_received`** per duplicate text (looks like delayed/storming acks). Burst must skip rows sent within `OUTBOX_RESEND_INTERVAL_MS`; backlog/new rows still drain instantly on stream-open. TRANSPORT.md § ** → Follow-on fix — outbox burst double-send**.
- Hub **double transcript merge** on `previewChangeCount` while open chat already uses `ingestP2pEvent`.
- **Dual transcript writers** — UI `transcript_save` / FFI append racing `:p2p` poll on daemon, or Rust code opening `chat_transcript_v1.json` outside `dm_transcript_store` (append + `read_ack_sent` patch clobber → rows vanish). See DESIGN.md § “On-disk store ownership”.
- **Empty native reload wiping chat** — `force: true` reload that clears persisted lines when `transcriptLoadMerged` returns 0 rows during same-room refresh.
- Contact trust UI that changes **ack policy**, **foreground order**, or blocks **`ack_received`** for `is_known: false` peers — see `docs/DESIGN.md` § Contact trust (additive only).

## Debugging checklist (one message, two devices)

**Logs:** In-app App log shows `Native/flow` connectivity snapshots every ~30s, `Native/kad|coord|dial|swarm|listen|mdns`, and numbered `P2P` `step=` journey lines. Full native connect detail on stderr/logcat: `grep ghal_bol` (all levels). Optional: `GHAL_BOL_VERBOSE_LOG=1` before start to forward Rust `debug` lines into the App log too.

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
| Read tick missing after user left room | Leave drain must run: log `chat room leave … cutoff_ms=` + `chat room frozen`; hub close order; queue not cleared |
| `chat room enter … skipped — app not visible` | Gate off before foreground cmd; fix hub open order or `RunReadAckCatchup` |
| Hub preview OK, chat empty | Transcript key split; use merged load (peer id + public key) — DESIGN.md § “Transcript threads” |
| Ticks appear without peer ack | Fake state — Flutter must not promote; check poll + transcript patch only |
| Wire OK, empty roster | `handler context not set` on daemon poll; unlock + `p2p_start` with `app_namespace` |
| Host no contact after scan | `peer_identified` or inbound text `stores_updated` → roster bump + `merge_discovered_peer_id` |
| LAN chat broken on Wi‑Fi (mDNS shows peer, no connect) | Stale LAN port re-dial from upkeep? Same `mdns dialing …/tcp/PORT` while `listen_addrs` shows different port? Check stale contact pk. |
| `profile=lan` but phone on mobile-data; desktop `dm connection … (direct)`; `outbox_pending` high | `os=cell` on phone; |
| `profile=` wrong minutes after toggle; `os=` missing in flow log | Stale native build or `if_addrs`-only profile | Rebuild native; `os=wifi|cell/validated/…` must flip ~1s — TRANSPORT.md § **Network truth** |
| Chat worked 5–10 min then died on LAN | Linux idle timeout was 300s; listen port may have changed. Desktop idle now 120s. |
| Coord lookup 404 for peer | Peer not on coord yet. |
| Call still active after Linux X / Ctrl+C / UI kill | UI gone but `:p2p`/daemon still up — must **`force_end_active_call`** on last UI socket EOF. Log: `force_end_active_call reason=ui_session_ended`. Also check Flutter `window_closed_by_user` / `call_screen_dismissed_*`. DESIGN.md § “Call UI lifecycle and privacy” |
| Chat dead, `stream_ready_count=0`, hub bootstrap `room closed` ×N | Forbidden session-sync patch or foreground storm — not coord-only. DESIGN.md forbidden table |
| UI empty / `conv=solo`, disk has `ghal_bol/*.json` | Session desync — not keystore delete. Do not new identity |
| Linux desktop: single tick, peer did read | Check **recipient** logs for `ack_read sent` — gate may be off on their side; sender transcript `delivery=delivered` is truthful until peer ack arrives |
| Android: read on screen, sender single tick | **`inactive`** gates read off while room stays open; **`resumed`** must log `read gate opened — catch-up ack_read` (`p2p_sync_ui_session` queues catch-up even if foreground unchanged). DESIGN.md § “Fixed 2026-06-29”. |
| Android: no messages after device reboot until app manually opened | **Boot auto-start:** `BootReceiver` starts `GhalBolP2pService` on `BOOT_COMPLETED` when keystore exists. Daemon runs locked → posts high-priority "unlock needed" notification. User taps notification → `IdentityScreen` → password → unlock → `cancelUnlockNotification()`. If no notification: check `RECEIVE_BOOT_COMPLETED` permission, `BootReceiver` in manifest, keystore file exists. DESIGN.md § "Boot auto-start and unlock notification". |
| Android: no messages with screen off / after lock (FGS still running) | **Not** read gate (`ack_received` may work briefly then stop). Check battery optimization (`isBatteryOptimized`), “Pause app activity if unused”, OEM autostart. Hub must run **`AndroidBackgroundReadiness`** before P2P; user should complete prompted steps. Events buffer on aggressive OEMs may show `am_app_frozen` / `fast_freezer` — still require user OEM settings even after stock exemptions. DESIGN.md § “Fixed 2026-07-05 — Android background readiness”. |
| Linux: no messages after reboot/login until app manually opened | **XDG autostart:** `~/.config/autostart/com.ghalbol.daemon.desktop` starts `ghal_bol_core_daemon` on login. After 10 s grace, if still locked and no UI socket, daemon raises the app (`gtk-launch` + D-Bus) and shows unlock notification. User enters password → unlock → P2P starts. Autostart entry includes `GHAL_BOL_APP_NAMESPACE` from the last unlock. DESIGN.md § "Linux desktop — daemon auto-start and unlock notification". |
| Incoming-call notification tap does not show UI | Daemon wrote `incoming_call_wake` but Flutter not polling — check wake poll in `p2p_event_bridge.dart` + D-Bus activate in `incoming_call_notify.rs` |

## Doc index

| File | Use |
|------|-----|
| `docs/DAEMON_INTEGRATOR.md` | Precompiled daemon + SDK integrator model, multi-app isolation, config env vars |
| `docs/DESIGN.md` | Layers, truthful ticks, state model, room open/close, leave backlog, transcript keys, **§ UI integrator contract (daemon-owned)** |
| `docs/IDENTITY.md` | Local identity model (today: secp256k1); link to multi-algo spec |
| `docs/MULTI_ALGO.md` | Multi-algorithm identity wire format, algorithm enum, identity vs transport/E2E, implementation status |
| `docs/GHAL_BOL_DM_MSG_V1.md` | Wire + ack kinds + upkeep |
| `docs/GHAL_BOL_URI_SCHEME.md` | QR / `ghalbol://` invites |
| `docs/GHAL_BOL_VOICE_V1.md` | Call signaling (`ghal_bol_call_v1`) |
| `docs/GHAL_BOL_CALL_NATIVE_V2.md` | Native Rust voice engine over the P2P link (shipping) |
| `docs/GHAL_BOL_VIDEO_NATIVE_V1.md` | Native Rust video wire/engine (shipping) |
| `docs/COORDINATION_SERVER.md` | Run/test coord server, local dev stack, **HTTP log troubleshooting** |
| `docs/TRANSPORT.md` | native transport, **Connectivity lifecycle**, **Network truth**, **Asymmetric mux recovery**, caching policy, LAN stability, WAN/CGNAT |
| `docs/ROADMAP.md` | Human product backlog only — not agent implementation specs |