# Direct messages — `ghal_bol_msg_v1`

Ghal Bol uses **one-to-one framed streams** on libp2p protocol **`/ghal-bol/msg/1.0.0`**. **Gossipsub is not used.** Messages are **signed JSON envelopes** with **secp256k1-sealed** text bodies. Transport details: [TRANSPORT.md](TRANSPORT.md).

**Design overview** (layers, chat-room rules, asymmetric contacts): see **[DESIGN.md](DESIGN.md)** first.

## Transport stack

| Layer | Value |
|-------|--------|
| App framing | 4-byte little-endian length + UTF-8 JSON envelope |
| Envelope tag | `ghal_bol_msg_v1` (`format_version`: **`2`**) |
| **Transport (libp2p)** | Stream protocol `/ghal-bol/msg/1.0.0`; underneath QUIC/TCP, Noise, Yamux; LAN mDNS; WAN **coord lookup + relay** ([TRANSPORT.md](TRANSPORT.md)) |

**Long-lived session:** one bidirectional channel per remote **PeerId** when possible. A dedicated writer task sends frames; inbound read loop on the same session.

## Identity model

| Key | Use |
|-----|-----|
| **secp256k1** (compressed, **66 hex**) | libp2p **PeerId**, envelope **signatures**, and message **encryption** (single key per device) |

**PeerId** is derived from the secp256k1 public key (`ghal_bol::peer_id_util`). The keystore holds one logical identity; there is no separate “encryption key” on the wire.

**Trust on the wire:** libp2p **Noise** proves the remote party owns the connection’s **PeerId**. App-layer frames must additionally satisfy:

- Valid **secp256k1** signature on the envelope.
- `sender_public_key_hex` must **derive to** the libp2p **PeerId** on that stream (binding check in `chat_server.rs`).

Remote keys are known from the **connect invite** (`public_key_hex`) and/or derived from the remote libp2p **PeerId** (secp256k1 identity). There is **no** separate `peer_hello` handshake — after connect the node opens `/ghal-bol/msg/1.0.0` and speaks `ghal_bol_msg_v1` frames (sign + seal) directly.

## Envelope (`ghal_bol_msg_v1`)

Common fields:

| Field | Notes |
|-------|-------|
| `id` | Opaque message id; used for acks and dedupe |
| `kind` | `text` \| `ack_received` \| `ack_read` (`ack_request` reserved — **never sent**) |
| `sender_public_key_hex` | Sender secp256k1 public key (**66 hex**) |
| `recipient_public_key_hex` | Recipient secp256k1 public key (**66 hex**) |
| `ciphertext_hex` | Sealed inner `{"text":"…"}` for `kind: text`; empty for acks |
| `created_at_ms` | Unix milliseconds (frame construction time) |
| `received_at_ms` | On **`ack_received` only:** when the recipient **first** accepted the referenced text (`ref_id`). Recipient authority; **stable on duplicate text retries**; omitted on `ack_read`. |
| `signature_hex` | Signature over canonical JSON (all fields except `signature_hex`) |
| `ref_id` | On acks: original text message `id` |

### Text encryption

Inner JSON `{"text":"…"}` is sealed with `ghal_bol::secp256k1_seal` (ephemeral secp256k1 ECDH + SHA-256 + AES-256-GCM). Wire blob: `u32_le(ephemeral_len) || ephemeral_pubkey || nonce (12) || ciphertext+tag`.

## Delivery, read state, and sync

Ghal Bol does **not** pull chat history from the other device. Reliability is **sender resend until ack** plus **recipient-driven read state**. The **local transcript** (`chat_transcript_v1.json`) is the source of truth for what still needs network work; the native node rescans it about every **1 s** while P2P is up.

**Canonical narrative:** [DESIGN.md](DESIGN.md) § “Message state — intent and how Ghal Bol implements it”.

### Intent (summary)

- **Recipient authority** — only the device that received the text sends delivery/read signals.
- **Truthful UI** — ticks reflect transcript after poll only; never optimistic or cross-device sync (see [DESIGN.md](DESIGN.md) § “Truthful status in the UI”).
- **No shared state machine** — each peer’s transcript may disagree until acks arrive; no “sync ticks” RPC.
- **Delivered vs read** — two steps; read requires an **open chat room** in the hub (see DESIGN.md).
- **Leave** — stop **new** read for **new** mail; **keep retrying** `ack_read` for inbound with **`received_at_ms ≤ chat_room_exit_at_ms`** (frozen on leave) until the sender confirms; delivery always continues in `:p2p`.
- **Wire** — `text` carries body only; **`ack_received`** / **`ack_read`** carry progress (`ref_id` = text `id`). Sender learns only from those acks on poll, not from status embedded in a resent text frame.

### Local fields (each device)

| Role | Field | Progress |
|------|-------|----------|
| **Sender** (outbound) | `delivery` | `pending` → `delivered` → `read` when peer acks arrive |
| **Sender** (outbound) | `received_at_ms` | Set from peer **`ack_received.received_at_ms`** (when they first got the text); first value wins |
| **Recipient** (inbound) | `received_at_ms` | Set once on **first local accept** of the text; never overwritten on duplicate/resend |
| **Recipient** (inbound) | `read_ack_sent` | true after we sent `ack_read` and peer confirmed with `ack_received` on our inbound `id` |

**Outbox:** clears on peer **`ack_received`** or **`ack_read`** (`ack_read` implies delivered).

### Receiver behaviour (native)

On every verified inbound **`text`** (including duplicates), in `chat_server.rs`:

1. **Always** `send_inbound_delivery_ack` → **`ack_received`** including **`received_at_ms`** (first local accept time; send now or enqueue in `pending_delivery_acks`; ~1 s upkeep retries). Log tag: `delivery_ack`.
2. If **`app_ack_read_enabled`** and **`live_foreground_peer == peer`**: also `send_inbound_read_ack_if_possible` → **`ack_read`**.

| Situation | Action |
|-----------|--------|
| **Any** inbound text (`:p2p` / daemon, UI optional) | **`ack_received`** (mandatory) |
| Chat room **open** in UI for this peer | **`ack_received`** + **`ack_read`** |
| Hub / background / room **closed** | **`ack_received` only** (no new `ack_read`) |
| User **enters** chat | Hub: **`set_app_ack_read_enabled(true)`** then **`set_foreground_peer`**. Native: **`begin_chat_room_session`** + **`dispatch_read_ack_pass`** (`RunReadAckCatchup` if gate opens after foreground). |
| User **leaves** / app **paused** | Hub: **`set_foreground_peer(null)`** and await **first**, then **`set_app_ack_read_enabled(false)`**. Native: freeze **`chat_room_exit_at_ms`**, **`dispatch_read_ack_pass`** with frozen cutoff; **new** mail → **`ack_received` only** until room opens again |

**Do not** use “in-room → `ack_read` only” without step 1 when the networking process can outlive the Flutter UI process.

**Leave backlog (normative):** inbound with **`received_at_ms` set** and **`received_at_ms ≤ chat_room_exit_at_ms`** (contact field, frozen on leave/switch) must still get **`ack_read`** on the wire after leave. Closing the room only closes the gate for **future** inbound; it does **not** cancel queued read acks. See [DESIGN.md](DESIGN.md) § “Leave / backlog” and § “Inbound `received_at_ms` and read-ack eligibility”.

On inbound **`ack_read`** (peer read our outbound text `id` = `ref_id`):

1. **Always** reply **`ack_received`** with `ref_id` = that text `id` (so the peer stops read retries).
2. Update local outbound delivery → `read` (monotonic).
3. Enqueue a **poll/UI event only on the first** transition (outbound still in outbox / not yet `read` in memory). **Do not** emit one poll event per wire retry.

On inbound **`ack_received`**:

- `ref_id` matches our outbound id → mark outbox delivered; patch outbound **`received_at_ms`** from ack when present; poll event only if outbox still tracked that id or transcript changed.
- `ref_id` matches inbound id we sent `ack_read` for → **`mark_read_ack_confirmed`** → `read_ack_sent` in transcript (only after this confirm); poll event only if we had a pending read ack for that id.

### Read-ack wire volume (normative)

| Phase | Expected wire count per text `id` |
|-------|-----------------------------------|
| In-room first receive | 1× `ack_received` + 1× `ack_read` (immediate, same stream handler pass) |
| Until sender confirms our `ack_read` | ≤ ~1 `ack_read` retry per second (`OUTBOX_RESEND_INTERVAL_MS`) |
| After sender `ack_received` confirm | **0** further `ack_read` for that id |
| Sender sees read tick | **1** inbound `ack_read` poll apply per id (duplicate wire frames must not re-trigger `stores_updated`) |

**Violations:** dozens of `ack_read` per second for the same `ref_id`, or `poll drain saturated` with hundreds of `dm_message` ack events for a handful of messages — implementation bug (burst/upkeep/emit/poll), not user error.

### Sender behaviour (native)

| Step | Behaviour |
|------|-----------|
| Send | Append transcript, track outbox, write frame when stream ready. |
| **~1 s upkeep** | `transcript_sync_outbound_tick`: merge transcript into outbox, drop rows already `delivered`/`read`, resend pending over open `/ghal-bol/msg/1.0.0`. |
| Until `ack_received` or `ack_read` | Same `id` **text** may be resent (~1s upkeep). No ack frames from sender. |
| After `ack_received` or `ack_read` | Remove from outbox; `delivery` → `delivered` or `read`. |

**Streams:** one long-lived `/ghal-bol/msg/1.0.0` per remote **PeerId** when possible; open only if missing (no connect spam). coord/relay (WAN) + mDNS (LAN) dial configured contacts only.

### Mechanisms (summary)

| Mechanism | Behaviour |
|-----------|----------|
| **Transcript** | Survives restart; native re-seeds outbox and read-ack queue from disk. |
| **In-memory outbox** | Fast resend path; purged when transcript says delivered/read. |
| **Inbound dedupe** | Duplicate `id` → delivery ack only, no second UI row. |
| **Foreground FFI** | `ghal_bol_ffi_p2p_set_foreground_peer` — hub sets open conversation; native applies immediately via `sync_foreground_peer_now`. |
| **UI ticks (outgoing)** | `pending` → `delivered` (`ack_received`) → `read` (`ack_read`). |
| **Hub poll → stores** | `dm_event_handler` on each poll applies `dm_message` to contacts + transcript (Flutter does not duplicate). |

There is **no** “give me messages since timestamp X” RPC. Offline delivery depends on the sender’s outbox after reconnect.

### Implementer checklist

Before changing ack policy, verify:

1. **Recipient only** sends `ack_received` / `ack_read`; **sender never** sends `ack_request`.
2. **`ack_received`** on **every** inbound text, always retried from native queue (not gated on Flutter poll or room state).
3. **`ack_read`** only when room is open in UI (`live_foreground_peer` + `app_ack_read_enabled`); may be sent **in addition to** `ack_received`.
4. **Never** clear the read-ack queue on leave.
5. **Never** set `read_ack_sent` on enter alone — only after peer `ack_received` confirms our `ack_read`.
6. **Sender outbox:** resend **text** only until `ack_received` or `ack_read`.
7. **Never** skip `ack_received` because `:p2p` still has a stale foreground peer after the UI exited.
8. **Read retry throttle:** after a successful wire `ack_read`, set `last_send_ms`; do not resend the same id until `OUTBOX_RESEND_INTERVAL_MS` unless never sent.
9. **Room-enter / leave backlog:** **`dispatch_read_ack_pass`** — seed only when `received_at_ms` set, `read_ack_sent: false`, and `received_at_ms ≤ cutoff_ms` (`chat_room_exit_at_ms` or live session); one drain pass; no multi-hundred-round bursts.
10. **Read-ack eligibility:** never queue `ack_read` without **`received_at_ms`** (not received locally). **`ack_received`** on the wire includes **`received_at_ms`** (recipient authority; stable on duplicate text).
11. **Poll emit gate:** `GossipChatEvent::DmMessage` for acks only when outbox/transcript state actually advances (see DESIGN.md § “Read receipts — wire volume”).
12. **`apply_inbound_ack`:** return `stores_updated = false` when `patch_outgoing_delivery` / `patch_inbound_read_ack_sent` returns unchanged.
13. **Confirm loop:** inbound `ack_read` → always wire `ack_received` back; inbound `ack_received` with pending read ack → `mark_read_ack_confirmed`. **`mark_read_ack_confirmed` only when `has_pending_read_ack`** — never `has_seen_inbound_id` alone (false `read_ack_sent` on disk; § DESIGN.md “Fixed 2026-06-19”).
14. **Leave drain:** `pending_read_acks` not cleared on leave; freeze **`chat_room_exit_at_ms`** before drain; `set_app_ack_read_enabled(false)` does not clear foreground — hub `SetForegroundPeer(null)` first.
15. **Transcript keys:** `load_merged` / patch paths expand peer id + `public_key_hex` so old threads and ack patches match (see DESIGN.md § “Transcript threads”). Poll replay dedupe and `apply_inbound_ack` use **`inbound_transcript_lookup_keys`** — single-bucket ack apply caused `has_out=false` / stuck delivery tick (§ “Fixed 2026-06-19”).
16. **Truthful ticks:** UI shows `delivery` / read only after `dm_event_handler` patches transcript; `stores_updated` only on real change.

### Android background listener

After unlock, native libp2p runs in a **Rust worker thread** (not the Flutter UI isolate). The **1 s upkeep** loop (sender text resend, **recipient** delivery/read ack retries) keeps running while the process lives.

`GhalBolP2pService` is a **foreground service** in process **`:p2p`** so libp2p can run when the Flutter activity is backgrounded or swiped away. It holds a **multicast lock** for mDNS. Flutter only **polls** events for UI over `p2p.sock`; **sending `ack_received` / `ack_read` does not depend on the poll timer** — only on the native upkeep loop in `chat_server.rs`.

**Android:** P2P runs in process **`:p2p`** (`GhalBolP2pService` foreground + `filesDir/.../p2p.sock`). The Flutter UI process only polls over the socket — libp2p CPU/RAM is not on the UI thread/process. OEMs can still kill `:p2p`; do **not** call `p2p_stop` / stop the service except logout/delete identity. UI lock keeps P2P running.

**Linux desktop:** P2P runs in **`ghal_bol_daemon`** (Unix socket). Closing the Flutter UI does **not** stop libp2p. Rebuild: `scripts/sync_ghal_bol_native_for_flutter.sh` (copies `libghal_bol.so` + daemon into `ghal_bol_ui/linux/`).

**Android:** Rebuild: `scripts/pack_android_workspace_jni_libs.sh` only → `build/android-native-ndk/` (Gradle `jniLibs`; shared by UI and `:p2p`). Do not use `sync` on the phone workflow.

Code: `ghal_bol/src/p2p/chat_server.rs`, `ghal_bol/src/dm_transcript_v1.rs`, `ghal_bol_ui/lib/dm_delivery_sync.dart`.

## Connect invite vs P2P config

| Step | What happens |
|------|----------------|
| Scan QR (format **2**) | App stores remote **`public_key_hex`**; **PeerId** is derived locally. |
| `p2p_start` | `dm_peers` lists `{ "public_key_hex": "<66 hex>" }` per contact. |
| Connect | coord/relay (WAN) or mDNS (LAN) dial; native opens `/ghal-bol/msg/1.0.0` on connect (no key-exchange prelude). |
| `chat_ready` | Outbound stream open; safe to send encrypted/signed frames. If this peer is **foreground**, native seeds read acks and runs **one pass** of queued `ack_read` for backlog. |
| Chat | Encrypt to recipient `public_key_hex`; sign with local secp256k1 key. |

See `docs/GHAL_BOL_URI_SCHEME.md` for invite formats. **No multiaddrs** are required on new invites (`multiaddrs: []`).

## Starting P2P (FFI / daemon RPC)

On **Linux and Android**, Flutter calls **`p2p_start`** on the **`:p2p` / `ghal_bol_daemon`** process via JSON-RPC (`ghal_bol_p2p.dart`). The UI process uses FFI for identity and store reads. Method names mirror FFI.

`p2p_start` JSON (FFI or daemon):

```json
{
  "bootstrap_peers": [],
  "dm_peers": [
    { "public_key_hex": "02…" }
  ],
  "transcript_path": "/path/to/chat_transcript_v1.json",
  "app_namespace": "com.ghalbol"
}
```

`bootstrap_peers` is optional; **public-key / PeerId** operation does not require invite multiaddrs. Empty array is normal.

**`already_running`:** If libp2p is already up (daemon survived UI restart), native must still call `set_p2p_handler_context(app_namespace)` and re-register every `public_key_hex` in `dm_peers`. Otherwise inbound events log `handler context not set` and stores do not update.

## FFI send / poll / register

| Symbol | Role |
|--------|------|
| `ghal_bol_ffi_p2p_start(config_json)` | Start background libp2p node |
| `ghal_bol_ffi_p2p_register_dm_peer(peer_id, public_key_hex)` | Hot-register contact (`peer_id` may be null) |
| `ghal_bol_ffi_p2p_set_foreground_peer(peer_id_utf8)` | Open chat = peer id string; `null`/empty = none |
| `ghal_bol_ffi_p2p_send_text_dm(recipient_public_key_hex, text)` | Queue send; returns `{ "ok", "message_id", "queued"?: true }`; non-blocking |
| `ghal_bol_ffi_p2p_send_ack_dm(recipient_public_key_hex, ref_id, ack_kind)` | `ack_received` or `ack_read` only (recipient / tests; not for sender nudges) |
| `ghal_bol_ffi_p2p_poll_event()` | JSON events (see below); on daemon platforms Flutter uses **`p2p_poll` on the state RPC socket** |

**Foreground** is owned by **`ChatHubScreen`** when `hubPollsEvents` is true — see [DESIGN.md](DESIGN.md) § “Flutter: who sets foreground”. `ChatScreen` must not set foreground in that mode (IndexedStack keeps it mounted off-room). `P2pEventBridge.setForegroundConversation` → `p2p_set_foreground_peer` (state socket) → `sync_foreground_peer_now` in native.

Poll responses may include **`stores_updated": true`** after `dm_event_handler` writes contacts/transcript. Flutter bumps **roster** on `peer_identified` and inbound **text**; preview-only bumps for ack-only events.

### Poll event kinds

| `kind` | Meaning |
|--------|---------|
| `listening` | Local listen multiaddr |
| `peer_connected` | libp2p connection up |
| `peer_identified` | Remote `public_key_hex` |
| `chat_ready` | Outbound stream open; safe to send |
| `dm_message` | Inbound `text` or ack (`msg_kind`, `id`, `text`, `ref_id`, `sender_public_key_hex`, …) |
| `outbound_sent` | Outbound frame written to stream |
| `send_failed` | Outbound `message_id` could not be sent (outbox may still retry) |
| `dial_failed` | Dial error |

## Flutter persistence

| Store | Path / key |
|-------|------------|
| Contacts | `contacts_v1.json` — keyed by conversation identity (`public_key_hex` / derived PeerId) |
| Chat transcript | `chat_transcript_v1.json` — per conversation; outbound `delivery` + `received_at_ms`; inbound `read_ack_sent` + `received_at_ms` |
| Keystore | App namespace (`com.ghalbol` on Android) |

Transcripts survive app restarts; **network delivery** and **read-ack retries** are owned by the native node (outbox + `pending_read_acks`), with transcript used to re-seed backlog on enter-chat.

## Related docs

- **[DESIGN.md](DESIGN.md)** — architecture, message state, chat-room enter/leave, layer split.
- Connect invites: [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) (`ghal_bol_connect_v1`, format **2**).
- Doc index: [README.md](README.md).
- Product vision: root [README.md](../README.md).

## Source of truth (code)

| Area | Path |
|------|------|
| Stream + outbox + read acks | `ghal_bol/src/p2p/chat_server.rs` |
| Envelope crypto | `ghal_bol/src/msg_v1.rs`, `ghal_bol/src/secp256k1_seal.rs` |
| Transcript helpers | `ghal_bol/src/dm_transcript_v1.rs` |
| Invite verify | `ghal_bol/src/connect_invite_v1.rs` |
| P2P runtime + FFI shim | `ghal_bol/src/p2p_runtime.rs`, `p2p_ffi.rs` |
| Daemon RPC | `ghal_bol/src/daemon/server.rs` |
| Poll → stores | `ghal_bol/src/dm_event_handler.rs` |
| UI delivery rules | `ghal_bol_ui/lib/dm_delivery_sync.dart` |
| UI invite | `ghal_bol_ui/lib/ghalbol_connect_invite.dart` |
