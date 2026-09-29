# AI agent guide — Ghal Bol workspace

**Read this file first** in a new session. Then **`docs/DESIGN.md`** before changing P2P, acks, invites, or persistence. Transport: **`docs/TRANSPORT.md`** — especially § **Connectivity lifecycle**, § **Network truth**, § **Asymmetric LAN↔WAN mux recovery**. UI shell: **`docs/MAKEPAD_UI.md`**.

**Connectivity policy (agents):** `docs/TRANSPORT.md` § **Connectivity lifecycle**, § **Network truth**, § **Parallel LAN + WAN transport**. Do **not** throttle coord bridge lookup or WAN recovery ticks because of informal “don’t flood” notes — throttle **storms** only. Register when **publishable endpoints change**, not every tick.

| Misread | Wrong agent behaviour (breaks WAN) | Correct meaning |
|---------|----------------------------------|-----------------|
| “Don’t register again and again” | Skip coord lookup or WAN recovery ticks | Throttle redundant **`POST /v1/register`** when endpoints unchanged (`should_throttle_register`); **force** register on endpoint change, failed register, handover, or bridge accepted |
| “Full eye on the network” | Register/coord HTTP on every tick; or tear down steady links | Continuous profile watch; register when **publishable endpoint changes** — not spam |
| “`if_addrs` shows Wi‑Fi / rmnet” | `profile=lan` minutes after mobile-data switch; wrong dial path | **OS default route** (`os=cell` / `os=wifi` in `Native/flow`) — TRANSPORT.md § **Network truth** |

## Golden rules

0. **Prime directive — instant call connect at any roster size.** Whenever two peers have *any*
 technically reachable path (LAN or coord bridge), a **voice/video call** must connect within a
 few seconds. **Text messaging** uses `ghal_bol_delivery` (mailbox), not native-connect P2P.
 Coord upkeep serves **calls** and presence. See `text_transport.rs` and [DESIGN.md](docs/DESIGN.md) § Goals.
1. **`ghal_bol_core` (Rust) owns all product logic** — crypto, keystore, **text** (`delivery_runtime`), **calls** (native connect), contacts, transcripts, invite codec. The app calls **`ghal_bol_core::host`** in-process.
2. **`ghal_bol_app` (Makepad 2) is the only UI.** Screens, navigation, hub, composer, and delivery ticks. **Do not re-implement ack policy, outbox, or transcript merge in the UI.** The open room is **`host::set_open_room`**. App visibility (read gate) is **`host::set_app_visible`**. **Do not** HTTP coord lookup or register from the UI. Splash is `script_mod!`. Where Makepad 2 lacks a widget, fill the gap in Splash. See `docs/MAKEPAD_UI.md`.
3. **`docs/DESIGN.md` is canonical** for architecture; **`docs/TRANSPORT.md`** for transport and connectivity policy. Wire detail: `docs/GHAL_BOL_DM_MSG_V1.md`. Invites: `docs/GHAL_BOL_URI_SCHEME.md`. If code and agent-editable docs disagree, **fix both in the same change**.
4. **Guest scans host QR** — guest stores host `public_key_hex` and dials. **Host may have zero contacts** until first inbound. **Never** require mutual QR or “both sides need each other’s key from QR”.
5. **Do not stop / restart native connect on every contact change** — use `register_dm_peer` / `sync_contacts` hot-register only.
6. **One process.** The Makepad app and native connect share a process. **Lock** hides the hub and leaves the node running. **Log out** stops it. Do not start a second node for the same identity.
7. **E2E for all peer-key traffic** — Any product communication between two contacts must use **end-to-end** crypto tied to the device identity key and the peer’s contact identity wire. Includes: **text** (`delivery_msg_v1` on the mailbox), call signaling (`ghal_bol_call_v1` + **transport KEM** `CALL_CIPHER_TRANSPORT_V2`), call **audio and video** media (`derive_call_media_keys_from_transport` + per-frame AES-GCM seal on `/ghal-bol/call/*` substreams). Do **not** ship peer-facing plaintext payloads or disable media/signaling E2E for “performance” without an explicit product decision.
8. **Caching — immutable only on disk** — Persist to disk only data that is **user-owned and does not change meaningfully without user action** (keystore, contacts, transcript, preferences). If a value **can change** (coord presence, mDNS LAN port, bridge endpoints) and relying on a stale copy **could break chat or WAN**, **do not cache it** — refetch live (`GET /v1/peers/…`, mDNS events, bridge pending). In-memory session mirrors and short storm throttles are OK when cleared on failure. New cache only with an explicit documented exception in TRANSPORT.md § “Caching policy”.
9. **Avoid assumed timers for async P2P work** — When policy (A) depends on work with unknown duration, a worker (B) owns it until the stack reports an outcome; B **notifies subscribers** and A reacts **instantly** — never “wait N seconds then retry.” Timers only for guardrails (in-flight observation, storm throttles, keepalive, register dedupe). See [TRANSPORT.md](docs/TRANSPORT.md) § “Event-driven async — avoid assumed timers”.

## Repository layout

| Path | Role |
|------|------|
| `ghal_bol_core/` | Rust library (`rlib`). Product logic. UI entry: `host` |
| `ghal_bol_app/` | Makepad 2 UI. Calls `ghal_bol_core::host` in-process |
| `ghal_bol_coord/` | Coordination + WAN call bridge |
| `ghal_bol_delivery/` | Delivery server (text mailbox) |
| `docs/` | Design + wire specs (source of truth with code) |
| `scripts/` | Build/pack for Makepad (`build_android_app.sh`, `package_linux_release.sh`) |

**Workspace root** = directory containing root `README.md` and `Cargo.toml`.

## Architecture (one picture)

```text
┌──────────────────────────────────────────────────────────────┐
│  ghal_bol_app — Makepad 2                                     │
│  Identity, hub, chat, invites, calls. Poll refreshes UI only │
│  MUST NOT: send acks, own outbox, merge transcripts          │
└────────────────────────────┬─────────────────────────────────┘
                             │ ghal_bol_core::host
┌────────────────────────────▼─────────────────────────────────┐
│  ghal_bol_core                                                │
│  delivery_runtime — text messaging                                │
│  native connect — voice/video calls (+ call signaling)            │
│  dm_event_handler, contacts, transcripts, invites             │
│  p2p_runtime                                                  │
└──────────────────────────────────────────────────────────────┘
```

## Where to put new code

| Concern | Owner | Primary files |
|---------|--------|----------------|
| Text send/recv, delivery acks | **Rust** | `ghal_bol_core/src/delivery_runtime.rs`, `delivery_client.rs`, `delivery_read_acks.rs` |
| Call connect, signaling, media | **Rust** | `ghal_bol_core/src/connect/` |
| Apply events → JSON stores | **Rust** | `ghal_bol_core/src/dm_event_handler.rs` |
| Contacts / unread / preview / **`is_known` / `is_blocked`** | **Rust** (`contacts_v1.rs`); trust UI **Makepad** — `docs/DESIGN.md` § Contact trust |
| Transcript lines, `delivery` | **Rust** | `ghal_bol_core/src/dm_transcript_v1.rs`, `dm_transcript_store.rs` |
| Invites format 2 | **Rust** | `connect_invite_v1.rs` |
| Hub, roster, which room is open | **Makepad** | `ghal_bol_app/src/main.rs` via `host::set_open_room` |
| Display ticks (no policy) | **Makepad** | transcript `delivery` rendered in the chat column |
| Start network / dm peers | **Rust** | `host::start_network` |
| Poll loop (UI refresh only) | **Makepad** | `App::drain_events` |
| OS network hints (Wi‑Fi link, default route) | **Rust** | `android_network.rs`, `linux_network.rs`, `network_transport.rs` |

## Message state (do not break this)

Read **`docs/DESIGN.md`** before touching acks, ticks, the open room, or transcripts — especially **Truthful ticks**, **Room open, leave, and read acks**, and **Transcripts**.

- **Recipient decides** delivery/read; sender never invents ticks or sends `ack_request`.
- **Truthful UI only** — show `delivery` / read ticks after native transcript patch on poll; never optimistic UI state.
- **No sync between devices** — each side has its own transcript; disagreement is normal until acks arrive.
- **Delivery always** (`ack_received`); **read only** when the hub has the room open for **new** inbound and the app is visible (Android).
- **After leave:** no **new** `ack_read` for **new** mail; **must keep** retrying `ack_read` for messages seen while the room was open until the sender confirms.
- **Hub room close:** `host::set_open_room(None)` — leave drain runs in native.
- **Hub room open:** `host::set_app_visible(true)` when resumed (Android) then `host::set_open_room(Some(peer))` — native opens the read gate then the foreground room.
- **Android inactive:** `host::set_app_visible(false)` — no new read receipts; do not clear the open room solely because the app is backgrounded.
- **Linux desktop:** do **not** treat brief focus loss as leaving the room or turning the read gate off while the chat pane is open. Open order: visible, then `set_open_room(peer)`.
- **Makepad poll refreshes UI only** — never sends acks. Ack transmission does not depend on poll.
- **Read acks are near-single-shot:** one immediate `ack_read` per text id in-room, then ~1 s retries **only until** peer `ack_received` confirms — see DESIGN.md § Room open. Duplicate `ack_read` floods are **bugs**.
- **Transcript load:** merge **peer id + public key** conversation keys so chat is not empty while the roster preview exists.

## Process

Native connect runs **in the same process** as `ghal_bol_app`. `host::unlock` installs the identity. `host::start_network` starts the node. `host::poll_event` lets the UI refresh.

**Lock** keeps the node running. **Log out** calls `host::lock`, which stops the node.

## Build & run

```bash
cargo run -p ghal_bol_app
cargo test -p ghal_bol_core
./scripts/build_android_app.sh
```

Linux debug data: `~/.local/share/com.ghalbol.debug/` (release: `~/.local/share/com.ghalbol/`).

## Anti-patterns (do not reintroduce)

- **Rust warning suppressions** — no `#[allow(dead_code)]` / `RUSTFLAGS=-A warnings` to hide build warnings; delete unused code or wire it up.
- Mutual-QR requirement or “both need each other’s public key from QR”.
- UI filtering that drops inbound events.
- Hub `stores_updated` → full roster reload storm (preview debounce on inbound text; **roster** bump on `peer_identified` only — not every text).
- Restarting full native connect on each contact upsert.
- Sender sending `ack_request`.
- In-room **only** `ack_read` with **no** `ack_received`.
- Read-ack **burst/upkeep storms** (retry every poll tick, emit every wire ack to UI, `stores_updated` on no-op patch).
- **Outbox burst double-send** — `resync_outbox_burst_for_peer` must skip rows sent within `OUTBOX_RESEND_INTERVAL_MS`; backlog/new rows still drain on stream-open.
- **Dual transcript writers** — opening `chat_transcript_v1.json` outside `dm_transcript_store`.
- Empty reload that clears the open chat when a merged load returns 0 rows during same-room refresh.
- Contact trust UI that changes **ack policy**, **foreground order**, or blocks **`ack_received`** for `is_known: false` peers — see DESIGN.md § Contact trust.
- A second UI stack or out-of-process RPC for the Makepad app.

## Debugging checklist (one message, two devices)

**Logs:** In-app App log shows `Native/flow` connectivity snapshots every ~30s and numbered `P2P` `step=` journey lines. Full native detail on stderr/logcat: `grep ghal_bol`. Optional: `GHAL_BOL_VERBOSE_LOG=1` before start.

**Dial — parallel LAN + WAN:** On Wi‑Fi, coord/bridge and mDNS/direct TCP **both run** for configured contacts (TRANSPORT.md § “Both links active”). **`profile=lan` vs `mobile-data` follows OS default route** (`os=` in `Native/flow`).

| Step | Sender | Receiver |
|------|--------|----------|
| 1 | `send_text_dm queued` | — |
| 2 | `peer_connected` / `chat_ready` on poll | same |
| 3 | — | inbound text + `ack_received sent` |
| 4 | poll: `ack_received` → outbound `delivered` | `stores_updated` on poll |

| Symptom | Check |
|---------|--------|
| `queued` forever, no `chat_ready` | peers registered? guest has host `public_key_hex`? `host::start_network`? |
| Receiver gets text, sender no tick | delivery ack logs? stale open-room suppressing delivery? |
| Many `ack_read` same `ref=` | Broken confirm loop or retry throttle — DESIGN.md § Room open |
| Blue tick missing | Sender not getting `ack_read`; room open + visible? |
| False blue tick | Read-ack seed without `received_at_ms` or past `chat_room_exit_at_ms` |
| Read tick missing after leave | Leave drain: `set_open_room(None)`; queue not cleared |
| Hub preview OK, chat empty | Transcript key split; merged load (peer id + public key) |
| Ticks without peer ack | Fake state — Makepad must not promote |
| Wire OK, empty roster | unlock then `host::start_network` with `app_namespace` |
| Host no contact after scan | `peer_identified` / inbound text → roster bump |
| Call still active after window close | `host::end_call`; node shares the process |
| Wrong `profile=` / missing `os=` | Rebuild; `os=wifi\|cell/…` must flip ~1s — TRANSPORT.md § Network truth |
| Coord lookup 404 | Peer not on coord yet |
| Call still active after window close | `host::end_call`; node shares the process |
| Android: read on screen, sender single tick | `set_app_visible(false)` while backgrounded; resume must reopen the read gate |
| Android: no messages after reboot until app opened | Activity is the process; open Ghal Bol once |
| Android: no messages with screen off | Battery optimization / OEM autostart — not the read gate alone |
| Linux: quiet after login | Autostart `~/.config/autostart/com.ghalbol.desktop` starts `ghal_bol_app` |
| Incoming-call tap does not show UI | Poll + D-Bus activate in `incoming_call_notify.rs` |

## Doc index

| File | Use |
|------|-----|
| `docs/DESIGN.md` | Layers, ticks, room open/close, transcripts, trust |
| `docs/MAKEPAD_UI.md` | Makepad 2 shell (`ghal_bol_app`) |
| `docs/IDENTITY.md` | Local identity model |
| `docs/MULTI_ALGO.md` | Multi-algorithm identity wire |
| `docs/GHAL_BOL_DM_MSG_V1.md` | Wire + ack kinds + upkeep |
| `docs/GHAL_BOL_URI_SCHEME.md` | QR / `ghalbol://` invites |
| `docs/GHAL_BOL_VOICE_V1.md` | Call signaling |
| `docs/GHAL_BOL_CALL_NATIVE_V2.md` | Native voice engine |
| `docs/GHAL_BOL_VIDEO_NATIVE_V1.md` | Native video engine |
| `docs/COORDINATION_SERVER.md` | Coord server |
| `docs/TRANSPORT.md` | Native transport policy |
| `docs/README.md` | Full doc index |
