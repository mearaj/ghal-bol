# AI agent guide — Ghal Bol workspace

**Read this file first** in a new session. Then **`docs/DESIGN.md`** before changing P2P, acks, invites, or persistence. Transport (libp2p): **`docs/TRANSPORT.md`**. **`docs/STORY.md`** is **human-authored** connectivity / discovery policy (agents: **read only** — it overrides conflicting guidance in other docs; **never edit, revert, or `git checkout` it**).

### STORY.md — do not misread the first sections

`STORY.md` opens with human backlog (`## Current issues to resolve`, `# Now`, `# Next`). **Those are not agent task lists or implementation specs.** Agents must **not** implement from them, throttle relay/WAN recovery because of “don’t flood”, or treat “Now your job is to fix…” as permission for log-driven patchwork.

**Binding connectivity policy for agents starts at `# Story`** (the section that says *anything in the docs that violates this story should be overridden*). For **how** relay reservation, bootstrap dial, and coord register work mechanically, **`docs/TRANSPORT.md`** (§ Client, § CGNAT) and **`coord_runtime.rs`** are canonical — not the opening paragraphs of STORY.

| Misread from STORY top | Wrong agent behaviour (breaks WAN) | Correct meaning |
|------------------------|----------------------------------|-----------------|
| “Don’t register again and again” | Skip relay reservation, coord lookup, or WAN recovery ticks | Throttle redundant **`POST /v1/register`** when endpoints unchanged (`should_throttle_register`); **force** register on endpoint change, failed register, handover, relay accepted |
| “Full eye on the network” | Register/coord HTTP on every tick; or tear down steady links | Continuous profile watch; register when **publishable endpoint changes** — not spam |
| “Steady, reliable, don’t flood” | One relay only, no parallel coord relays, no bootstrap dials | Throttle **storms** (repeated `listen_on`, redundant bootstrap `swarm.dial`) — **not** required reservation + happy-eyeballs dial (TRANSPORT.md § CGNAT) |

## Golden rules

1. **`ghal_bol` (Rust) owns all product logic** — crypto, keystore, libp2p, outbox, **ack send/retry**, contacts, transcripts, invite codec, call signaling. Implement behaviour here and expose **`ghal_bol_ffi_*`** (or daemon JSON-RPC on Linux/Android `:p2p`).
2. **`ghal_bol_ui` (Flutter) is a thin shell** — screens, navigation, hub layout, QR scan/share UI, composer, rendering delivery ticks from native state. **Do not re-implement ack policy, outbox, or transcript merge in Dart.** Session signals: **`GhalBolUiSession` only** (`setVisible` + `setRoom` → `p2p_sync_ui_session`) — never deprecated `setAppAckReadEnabled` / `setForegroundPeer` / `setAppUiVisible` from product code. **Do not** HTTP coord lookup, coord register ticks, or `dial_bootstrap_peers` from Dart — `sync_contacts` / `register_dm_peer` only; WAN recovery, coord register/lookup/dial, and LAN-vs-WAN routing run in **`chat_server.rs`** / **`coord_runtime.rs`** (see override rules below).
3. **`docs/DESIGN.md` is canonical** for architecture; **`docs/TRANSPORT.md`** for libp2p transport. Wire detail: `docs/GHAL_BOL_DM_MSG_V1.md`. Invites: `docs/GHAL_BOL_URI_SCHEME.md`. If code and agent-editable docs disagree, **fix both in the same change**. **`docs/STORY.md` is not agent-editable** — when behaviour must match human story, change **code + DESIGN.md / TRANSPORT.md**, not STORY.md.
4. **Never touch `docs/STORY.md`** — no edits, no reverts, no `git checkout -- docs/STORY.md`, no “restore canonical STORY”. Humans own that file.
5. **Guest scans host QR** — guest stores host `public_key_hex` and dials. **Host may have zero contacts** until first inbound. **Never** require mutual QR or “both sides need each other’s key from QR”.
6. **Do not `p2p_stop` / restart libp2p on every contact change** — use `register_dm_peer` / `sync_contacts` hot-register only.
7. **Do not run `scripts/sync_ghal_bol_native_for_flutter.sh` while the Linux app is running** — it stops `ghal_bol_daemon` and causes `Broken pipe` on the UI socket. **Android:** rebuild with `pack_android_workspace_jni_libs.sh` — **default must ship all four standard ABIs** (`armeabi-v7a`, `arm64-v8a`, `x86`, `x86_64`) for Play, emulators, and 32-bit ARM devices; `PACK_ANDROID_ARM64_ONLY=1` is a dev fast-path only (host `cargo-ndk`; no adb).
8. **E2E for all peer-key traffic** — Any product communication between two contacts must use **end-to-end** crypto tied to the device **secp256k1 private key** and the peer’s **66-hex public key** (same identity as chat). Includes: DM text (`secp256k1_seal`), call signaling (`ghal_bol_call_v1`), call **audio and video** media (FrameCryptor + `derive_call_media_keys_from_identity`). Do **not** ship peer-facing plaintext payloads or disable media/signaling E2E for “performance” without an explicit product decision. Connect/setup may fall back to transport-only (e.g. DTLS-SRTP) only when identity E2EE setup fails — never by default.
9. **Avoid P2P-breaking caches** — Do not cache dial targets, coord lookup addrs, or mDNS LAN addrs if staleness could break or degrade connectivity. Prefer live mDNS events + coord HTTP lookup. **`dm_upkeep` must not re-dial LAN from `peer_mdns_lan_candidate_addrs`** — LAN is event-driven; upkeep is coord/WAN only. Do not rank ephemeral LAN ports (highest port, preferred addr, TTL). The only permitted on-disk transport cache is `ghalbol_relay.json` (invalidate on relay TCP failure). See [TRANSPORT.md](docs/TRANSPORT.md) § “Caching policy (P2P)”, § “Ephemeral LAN TCP ports”.
10. **Avoid assumed timers for async P2P work** — General rule (not dial-only): when policy (A) depends on work with unknown duration, a worker (B) owns it until the stack reports an outcome; B **notifies subscribers** and A reacts **instantly** — never “wait N seconds then retry.” Applies to connect, handover, coord lookup, relay reserve, stream open, register, etc. Timers only for guardrails (in-flight observation, storm throttles, keepalive, register dedupe). See [TRANSPORT.md](docs/TRANSPORT.md) § “Event-driven async — avoid assumed timers”. **Do not** reintroduce grace-window coord blackout or tick-polled recovery without a new event.

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

- **Editing or reverting `docs/STORY.md`** — human-owned; agents align code / DESIGN.md / TRANSPORT.md instead; never `git checkout -- docs/STORY.md`.
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
- **Dual transcript writers** — UI `transcript_save` / FFI append racing `:p2p` poll on daemon, or Rust code opening `chat_transcript_v1.json` outside `dm_transcript_store` (append + `read_ack_sent` patch clobber → rows vanish). See DESIGN.md § “On-disk store ownership”.
- **Empty native reload wiping chat** — `force: true` reload that clears persisted lines when `transcriptLoadMerged` returns 0 rows during same-room refresh.
- Contact trust UI that changes **ack policy**, **foreground order**, or blocks **`ack_received`** for `is_known: false` peers — see `docs/DESIGN.md` § Contact trust (additive only).
- A second block store in preferences instead of **`is_blocked`** on `contacts_v1.json`.
- **Fake ticks** — Flutter showing delivered/read without transcript patch from poll.
- **Clearing read-ack queue on leave** or clearing foreground inside `set_app_ack_read_enabled(false)` before leave drain.
- **Wrong hub close order** — `setAppAckReadEnabled(false)` before `setForegroundConversation(null)`.
- **Single conversation key** for transcript load when history uses both peer id and public key buckets.
- **Hub transcript keyed on `activeContact` alone** — roster reload after send/poll can set `activeContact` to null for a frame; `didUpdateWidget` then treats it as a room switch, reloads `conv=solo`, and **other chats look wiped**. Hub must pass stable **`hubThreadKey`** (`_selectedConversationKey`); see DESIGN.md § “Hub chat — stable thread id”.
- **Weakening E2E** — audio-only FrameCryptor, skipping video encrypt, or plaintext call/chat payloads when the peer’s secp256k1 keys are the trust anchor (see golden rule 7).
- **`redial_public_dht_bootnodes` while bootstrap connected** during WAN recovery — disconnects all bootstrap TCP and stalls relay/coord for minutes; see `docs/TRANSPORT.md` § “WAN recovery — relay reservation and bootstrap redial”. Log: `forcing bootstrap redial` with `bootstrap_ok=true`.
- **Re-issuing relay `listen_on` every tick (1s storm)** — the storm is repeating `listen_on` for a relay faster than `RELAY_RESERVE_THROTTLE_MS`, **not** covering all relays once. Reserve on **all** eligible bootstraps in parallel via `try_relay_reservations` (per-relay throttle prevents the storm). Do **not** serialize one-relay-at-a-time — that lets one pending-but-never-accepted reservation block the others and stalls WAN for minutes.
- **Bootstrap relay dial storm on CGNAT/mobile** — uncordinated `swarm.dial` to the coord relay from refetch + WAN recovery + redial (many `coord relay dial` lines per second, never `bootstrap connection` / `reservation accepted` on the phone). Must use `issue_bootstrap_dials` / `should_issue_bootstrap_dial` and CGNAT probe `listen_on` — see `docs/TRANSPORT.md` § “CGNAT / mobile-data relay reservation”.
- **Removing CGNAT probe reservation** — `try_ghalbol_probe_style_circuit_listen` at startup and in `retry_stalled_relay_reservations` when `!any_bootstrap_connected` is required for mobile-data; Wi‑Fi-only testing hides this regression.
- **One-sided relay OK** — desktop `reservation accepted` + phone stuck on `CGNAT listen addr only` → coord 404 for phone forever, no chat. Fix the phone side, not coord HTTP.
- **Blocking peer relay dials until own circuit listens** — gating `dial_dm_peer_addr` on `!relay_circuit_listening` for CGNAT logs `skip relay dial … self relay circuit not ready yet` after `coord_lookup_peer ok` and stalls WAN ~40s. Outbound peer circuit dials only need coord relay bootstrap TCP; throttle with `should_routed_dial`, not own-reservation gate. See TRANSPORT.md § “Outbound peer relay dials vs own reservation”.
- **404 coord backoff during urgent DM reconnect** — after `dm connection closed`, coord lookup must not wait exponential backoff; see `mark_dm_reconnect_urgent` + `is_pk_reconnect_urgent`.
- **Blocking `node_ready` on full WAN** (45s relay wait) — emit `node_ready` after brief relay dial; recovery continues on `coord_tick`.
- **Kademlia / public-bootstrap WAN peer discovery** when coord is down — forbidden; WAN requires coord/relay; LAN (mDNS) still works ([STORY.md](docs/STORY.md)). **libp2p remains** for transport (relay circuits, DCUtR, mDNS, streams, Noise) — only the old “coord down → find peers via libp2p DHT” fallback is removed. Log lines saying `bootstrap_*` mean the **coord relay** from `GET /v1/relay`, not IPFS bootstrap peers — see [TRANSPORT.md](docs/TRANSPORT.md) § “Naming”.
- **Slow WAN fallback after LAN loss** — mDNS `Expired` must re-kick coord/relay lookup immediately; do not wait on LAN TTL.
- **Orphan active call when UI is gone** — `:p2p` / daemon outliving Flutter is **not** permission to keep media up. Last UI socket EOF must run **`p2p_force_end_active_call`**; GTK X / call-screen pop / `detached` must also hang up. See `docs/DESIGN.md` § “Call UI lifecycle and privacy”.
- **Fixing call restore / notification without force-end** — changes to `call_active`, `CallController.syncActiveCallFromNative`, or incoming-call notify must not skip UI-session teardown (regression checklist in DESIGN.md).
- **Releasing call video textures on `NativeCallVideoView.dispose`** — use `CallVideoTexturePool.releaseCall` on hangup only; dispose-time release caused Linux SIGSEGV during video (DESIGN.md § “Call UI lifecycle”, GHAL_BOL_VIDEO_NATIVE_V1.md).
- **Promoting delivery/read ticks in Flutter without native transcript patch** — ticks are recipient-authority only (DESIGN.md § “Truthful status”).
- **Racing coord relay dials against mDNS LAN on Wi‑Fi before first connect** — defer relay while **LAN dial is in flight** (stream-first: one route at a time). Symptom: `mdns dialing` + `coord_lookup_peer ok — dialing … via relay circuit` within ~2s, never `peer_connected`. See [TRANSPORT.md](docs/TRANSPORT.md) § “LAN relay vs mDNS race”, § “Stream-first symmetric connect”.
- **Competing dial policies instead of stream-first symmetric connect** — multiple paths (`kick_dm_peer_discovery`, `register_dm_peer`, `coord_tick`, mDNS handler) each calling `swarm.dial` for the same peer in the same second. One stream per contact, one upkeep owner (~1s). See [DESIGN.md](docs/DESIGN.md) § “Stream-first symmetric connect”.
- **P2P dial/lookup caches** — coord lookup addr cache, frozen mDNS LAN addr, **`dm_upkeep` LAN re-dials from candidate set**, or Dart routing cache. If staleness could break P2P, do not cache. See TRANSPORT.md § “Caching policy (P2P)”, § “Ephemeral LAN TCP ports”.
- **Timer-based async policy** — grace windows, tick-polled recovery, or tuning `N`-second constants instead of worker→subscriber events. Applies to connect/handover and any P2P path with unknown duration. See TRANSPORT.md § “Event-driven async”.
- **Flutter network-change RPCs** — no `p2p_notify_network_change` / resume connectivity hints from Dart; Android `:p2p` registers `ConnectivityManager` callbacks and Rust (`android_network.rs` + `network_tick`) owns Wi‑Fi handover recovery.
- **Forbidden 2026-06-15 hub UI session patch** — `lastApplySucceeded` / `uiSessionLastApplyOk`, `_invalidateNativeForegroundSync`, per-frame session retry from `build()`, hub `node_ready`/`_attachHubChat`/`resume`/`call end` session reapply storms. **Reverted:** stopped P2P chat (`stream_ready_count=0`, leave-drain bursts), UI looked wiped (`conv=solo`), users lost identity indirectly. **Do not** use this to fix Linux read ticks — use Linux **`inactive`** rule + low-volume `GhalBolUiSession.nudge()` instead (DESIGN.md § “Fixed 2026-06-15”). § “FORBIDDEN — reverted 2026-06-15”.
- **Linux desktop `inactive` → setVisible(false)** — regression; restores “resize fixes ticks” bug (`ui_session_applied read=false` with room open).
- **Port guessing / ranking for LAN** — highest-port-wins, preferred mDNS addr, probing with `nc` instead of mDNS `Discovered`/`Expired` + `Native/flow` listen_addrs. Ephemeral ports change every restart; see TRANSPORT.md § “Ephemeral LAN TCP ports”.

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
| Blue tick missing | Sender not getting `ack_read`, or poll not applying; room open + visible + `may_send_in_room_read_ack`? |
| Blue tick when app inactive/background | **Android:** read gate off on `inactive` — expected. **Linux desktop:** chat visible + `read=false` after `inactive` — regression (DESIGN.md § Fixed 2026-06-15). |
| Read tick missing after user left room | Leave drain must run: log `chat room leave … drain ack_read`; hub close order; queue not cleared |
| `chat room enter … skipped — app not visible` | Gate off before foreground cmd; fix hub open order or `RunReadAckCatchup` |
| Hub preview OK, chat empty | Transcript key split; use merged load (peer id + public key) — DESIGN.md § “Transcript threads” |
| Ticks appear without peer ack | Fake state — Flutter must not promote; check poll + transcript patch only |
| Wire OK, empty roster | `handler context not set` on daemon poll; unlock + `p2p_start` with `app_namespace` |
| Host no contact after scan | `peer_identified` or inbound text `stores_updated` → roster bump + `merge_discovered_peer_id` |
| LAN chat broken on Wi‑Fi (mDNS shows peer, no connect) | `mdns dialing` then relay dial to same peer within ~2s? Never `dm connection established`? **Regression:** relay racing mDNS before first connect — fix `should_defer_coord_relay_for_lan` (defer without `connected` gate). TRANSPORT.md § “LAN relay vs mDNS race”. Also check stale contact pk (desktop dials old key while mDNS discovers new peer id). |
| Same `mdns dialing …/tcp/PORT` every ~20s for minutes; `listen_addrs` / fresh `mdns discovered` shows **different** port | **Stale mDNS candidate cache + upkeep LAN re-dial** — not a port to hardcode. Fix: event-driven LAN only, coord-only upkeep; remove port-ranking heuristics. TRANSPORT.md § “Ephemeral LAN TCP ports”. Full app restart after native rebuild. |
| Chat worked 5–10 min then died on LAN | Linux idle timeout was 300s; listen port may have changed — check `dm peer disconnected` + stale dial loop above. Desktop idle now 120s. |
| WAN chat dead minutes, coord health OK | `forcing bootstrap redial` loop? `wan_recovery=true` + `relay_listen=false` + `bootstrap_ok=true`? Fix `run_wan_recovery_pass` — never disconnect coord relay for relay; rebuild native. TRANSPORT.md § WAN recovery. **Note:** `bootstrap_*` logs = coord relay, not IPFS peers. |
| Coord lookup 404 for peer | Peer not on coord yet — both need `reservation accepted` + `coord registered`; **not** proof coord HTTP is down. If **all** lookups 404 and server shows no `peer registered`, relay TCP is dead (dev: bore stopped / wrong port). **Asymmetric:** Wi‑Fi side registered, phone 404 → phone never got relay circuit — TRANSPORT.md § “CGNAT / mobile-data relay reservation” |
| Phone: many `coord relay dial`/s, no `bootstrap connection`, `CGNAT listen addr only` | Bootstrap **dial storm** or missing CGNAT probe reservation — rebuild native; see TRANSPORT.md § “CGNAT / mobile-data relay reservation” |
| `coord_lookup_peer ok` then `skip relay dial … self relay circuit not ready yet` (no `peer_connected` for ~40s) | **Regression:** peer relay dial gated on own reservation — remove gate; peer dials use `should_routed_dial` only. TRANSPORT.md § “Outbound peer relay dials vs own reservation” |
| Endless `GET /v1/peers/…` 404 on coord server, `GET /v1/relay` 200 | Relay TCP unreachable while HTTP OK (dev: bore dead or stale port in client cache) | `nc -zv` on `/v1/relay` addr; restart `run_server.sh`; restart apps; deploy README § “Regression prevention” |
| `waiting for relay/public listen endpoint before coord register` | No relay circuit — cannot register WAN endpoint | Fix bore/firewall; `coord_registered=false` until `reservation accepted` |
| `relay has no public address advertised` / `advertised=[]` on server | bore did not run at server start | `run_server.sh` must print `Starting bore:`; check bore-skip reason on stderr |
| Slow reconnect after idle | `dm peer disconnected` then 404 backoff + stale CGNAT `Timeout` dials? TRANSPORT.md § Idle chat reconnect — urgent reconnect must not apply 404 backoff; no blind `try_routed_dial` for WAN coord peers |
| Relay reservation never `accepted` | Is the **Ghal Bol relay** (coord-colocated, `GET /v1/relay`) reachable? Check `ghalbol relay … preferred for reservation` at startup + `relay v2 node started` in coord logs. Reserve on **all** configured coord relays (`try_relay_reservations`), per-relay throttled. Do **not** expect public IPFS bootstraps to substitute. See TRANSPORT.md § "Ghal Bol relay" |
| Call still active after Linux X / Ctrl+C / UI kill | UI gone but `:p2p`/daemon still up — must **`force_end_active_call`** on last UI socket EOF. Log: `force_end_active_call reason=ui_session_ended`. Also check Flutter `window_closed_by_user` / `call_screen_dismissed_*`. DESIGN.md § “Call UI lifecycle and privacy” |
| Linux **SIGSEGV** / sudden exit during **video** call | **Regression:** releasing `CallVideoTexturePool` / embedder texture on widget dispose mid-call. Textures must release on **hangup/end** only (`releaseCall` after `callVideoStop`). See DESIGN.md § “Call UI lifecycle”, GHAL_BOL_VIDEO_NATIVE_V1.md § “Flutter video textures and call end” |
| Linux desktop: read acks missing while chat visibly open | **Regression:** Linux `inactive` → `setVisible(false)` while room open (`ui_session_applied read=false`). Fix: DESIGN.md § “Fixed 2026-06-15 — Linux desktop read ticks”. **Never** forbidden `lastApplySucceeded` patch. |
| Chat dead, `stream_ready_count=0`, hub bootstrap `room closed` ×N | Forbidden session-sync patch or foreground storm — not coord-only. DESIGN.md forbidden table |
| UI empty / `conv=solo`, disk has `ghal_bol/*.json` | Session desync — not keystore delete. Do not new identity |
| Linux desktop: single tick, peer did read | Check **recipient** logs for `ack_read sent` — gate may be off on their side; sender transcript `delivery=delivered` is truthful until peer ack arrives |
| Incoming-call notification tap does not show UI | Daemon wrote `incoming_call_wake` but Flutter not polling — check wake poll in `p2p_event_bridge.dart` + D-Bus activate in `incoming_call_notify.rs` |
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
| `docs/STORY.md` | **Human-only** connectivity / discovery story (agents: read, never write) |
| `docs/COORDINATION_SERVER.md` | Run/test coord server, local dev stack, **HTTP log troubleshooting** |
| `docs/TRANSPORT.md` | libp2p transport stack, discovery, invariants, **§ WAN prerequisites** |
| `ghal_bol_server/deploy/README.md` | Dev `run_server.sh`, bore/ngrok, **§ Regression prevention** |
| `docs/WEB_SITE.md` | Static **ghalbol.com** web build, Firebase, Linux download, `/connect/…` handoff |
| `README.md` | Product vision + repo map |
| `ghal_bol_ui/README.md` | Flutter shell scope (native vs `bootstrap_web`) |

## Naming

- Product: **Ghal Bol** — domain **ghalbol.com**, package **`com.ghalbol`**
- Android namespace: `com.ghalbol`
- Rust crate / Dart package: `ghal_bol` / `ghal_bol_ui`
