# Direct messages — `ghal_bol_msg_v1`

Ghal Bol uses **one-to-one framed streams** on native protocol **`/ghal-bol/msg/1.0.0`**. Messages are **signed JSON envelopes** (`format_version`: **2**) with **algorithm-specific identity signatures** and **transport-layer ciphertext** for text bodies. See [MULTI_ALGO.md](MULTI_ALGO.md). Transport details: [TRANSPORT.md](TRANSPORT.md).

**Design overview** (layers, chat-room rules, asymmetric contacts): see **[DESIGN.md](DESIGN.md)** first.

> **Scope:** Product **text** uses **`GHAL_BOL_DELIVERY_URL`** ([GHAL_BOL_DELIVERY.md](GHAL_BOL_DELIVERY.md)). This spec covers **call signaling** frames on `/ghal-bol/msg/1.0.0` (and related native mux kinds). Native connect is **not** the product chat messaging path. See [DESIGN.md](DESIGN.md) § Goals.

## Transport stack

| Layer | Value |
|-------|--------|
| App framing | 4-byte little-endian length + UTF-8 JSON envelope |
| Envelope tag | `ghal_bol_msg_v1` (`format_version`: **`2`**) |
| **Transport (native connect)** | Stream protocol `/ghal-bol/msg/1.0.0` on TCP + Noise + channel mux for **call signaling**; **LAN** mDNS; WAN **calls** use the coord bridge ([TRANSPORT.md](TRANSPORT.md)). Product **text** uses delivery, not this path. |

**Long-lived session:** one bidirectional mux session per remote **identity wire** when possible. A dedicated writer task sends frames; inbound read loop on the same session.

## Identity model

| Key | Use |
|-----|-----|
| **Identity key** (e.g. secp256k1 compressed, **66 hex**) | Contact **identity wire**, envelope **signatures**, Noise session binding |
| **Transport key** (X25519, per DM stream) | Message **encryption** for text bodies (`DM_CIPHER_TRANSPORT_V2`) |

Peers are keyed by normalized **identity wire** (`docs/MULTI_ALGO.md`). The keystore holds one logical identity; DM payload confidentiality uses **transport KEM keys** exchanged via `transport_kem_hello`, not the identity private key.

**Trust on the wire:** native connect **Noise** plus a detached identity signature prove the remote party. App-layer frames must additionally satisfy:

- Valid identity signature on the envelope.
- `sender_public_key_hex` must match the remote identity wire on that session (binding check in `connect/`).

Remote keys are known from the **connect invite** (`public_key_hex`). After connect the node opens `/ghal-bol/msg/1.0.0`, exchanges `transport_kem_hello`, then speaks signed `ghal_bol_msg_v1` text frames.

## Envelope (`ghal_bol_msg_v1`)

Common fields:

| Field | Notes |
|-------|-------|
| `id` | Opaque message id; used for acks and dedupe |
| `kind` | `text` \| `voice` \| `attachment_offer` \| `ack_received` \| `ack_read` \| `attachment_complete` \| `transport_kem_hello` (`ack_request` reserved — **never sent**) |
| `sender_public_key_hex` | Sender identity wire (bare secp256k1 hex or `algo:hex`) |
| `recipient_public_key_hex` | Recipient identity wire |
| `ciphertext_hex` | Sealed inner JSON for `text`, `voice`, and `attachment_offer`; empty for acks and `transport_kem_hello` |
| `transport_x25519_hex` | On **`transport_kem_hello` only:** this node's X25519 transport public key (64 hex chars) |
| `created_at_ms` | Unix milliseconds (frame construction time) |
| `received_at_ms` | On **`ack_received` only:** when the recipient **first** accepted the referenced text (`ref_id`). Recipient authority; **stable on duplicate text retries**; omitted on `ack_read`. |
| `signature_hex` | Signature over canonical JSON (all fields except `signature_hex`) |
| `ref_id` | On acks: original text message `id` |

### Attachments (`attachment_offer`)

**WAN (delivery):** same E2E mailbox rail as text/voice. Sealed inner carries the file:

```json
{
  "attachment_version": 2,
  "file_name": "report.pdf",
  "mime_type": "application/pdf",
  "size_plaintext": 123456,
  "sha256_plaintext": "<hex>",
  "file_b64": "<plaintext bytes>"
}
```

Delivery seal = identity offline seal (`delivery_msg_v1`). Cap: **3 MB** sealed inner (`ATTACH_MAX_SEALED_INNER_BYTES`). Recipient writes `ghal_bol/attach/downloads/…` and sets transcript `local_path` — no separate download hop. **Coord is not used.**

**LAN oversized only:** native-connect mux `/ghal-bol/attach/1.0.0` (`CHANNEL_ATTACH`). Offer inner has `blob_id`, `content_key_b64`, hashes, `expires_at_ms` (no `file_b64`). Recipient fetches ciphertext chunks over the **LAN** session, decrypts, then may send `attachment_complete`. See [ATTACHMENTS_PLAN.md](ATTACHMENTS_PLAN.md).

### Text encryption

Inner JSON `{"text":"…"}` is AES-256-GCM via `ghal_bol_core::symmetric_seal`. After hex decode, ciphertext **must** begin with prefix **`0x03`** (**transport KEM v2**):

| Prefix | Path | When |
|--------|------|------|
| `0x03` | **Transport KEM v2** — X25519 ECDH + HKDF(`ghal_bol_dm_transport_v2` + sorted identity wire pair) | After both peers exchanged `transport_kem_hello` on the DM stream |

**Transport hello:** on stream open each node sends one signed `transport_kem_hello` frame advertising `transport_x25519_hex`. Outbound text is queued until the peer's transport key is known. Payload confidentiality is decoupled from identity **private** keys (identity wires still bind HKDF `info`).

### Voice messages

`kind: voice` uses the same signed envelope, transport KEM v2 seal, outbox retry, transcript patch, and `ack_received` / `ack_read` authority as native-connect text. The sealed inner JSON carries one Opus voice-note payload and metadata such as `duration_ms`, `sample_rate_hz`, and `channels`; poll/transcript rows expose only playback metadata (`msg_kind: "voice"`, `duration_ms`, `audio_path` / `local_path`) after native persists the local audio file.

## Delivery, read state, and sync

Ghal Bol does **not** pull chat history from the other device. Reliability is **sender resend until ack** plus **recipient-driven read state**. The **local transcript** (`chat_transcript_v1.json`) is the source of truth for what still needs network work; the native node rescans it about every **1 s** while P2P is up.

**Canonical narrative:** [DESIGN.md](DESIGN.md) § “Message state — intent and how Ghal Bol implements it”.

### Intent (summary)

- **Recipient authority** — only the device that received the text sends delivery/read signals.
- **Truthful UI** — ticks reflect the transcript after poll (see [DESIGN.md](DESIGN.md) § Truthful ticks).
- **No shared state machine** — each peer’s transcript may disagree until acks arrive; no “sync ticks” RPC.
- **Delivered vs read** — two steps; read requires an **open chat room** in the hub (see DESIGN.md).
- **Leave** — stop **new** read for **new** mail; **keep retrying** `ack_read` for inbound with **`received_at_ms ≤ chat_room_exit_at_ms`** (frozen on leave) until the sender confirms; delivery always continues in the native node.
- **Wire** — `text` carries body only; **`ack_received`** / **`ack_read`** carry progress (`ref_id` = text `id`). Sender learns only from those acks on poll, not from status embedded in a resent text frame.

### Local fields (each device)

| Role | Field | Progress |
|------|-------|----------|
| **Sender** (outbound) | `delivery` | P2P: `pending` → `delivered` → `read` when peer acks arrive. Delivery mode: `pending` → `sent` → `delivered` → `read` — see [GHAL_BOL_DELIVERY.md](GHAL_BOL_DELIVERY.md) |
| **Sender** (outbound) | `received_at_ms` | Set from peer **`ack_received.received_at_ms`** (when they first got the text); first value wins |
| **Recipient** (inbound) | `received_at_ms` | Set once on **first local accept** of the text; never overwritten on duplicate/resend |
| **Recipient** (inbound) | `read_ack_sent` | true after we sent `ack_read` and peer confirmed with `ack_received` on our inbound `id` |

**Outbox:** clears on peer **`ack_received`** or **`ack_read`** (`ack_read` implies delivered).

### Receiver behaviour (native)

On every verified inbound **`text`** (including duplicates), in `connect/`:

1. **Always** `send_inbound_delivery_ack` → **`ack_received`** including **`received_at_ms`** (first local accept time; send now or enqueue in `pending_delivery_acks`; ~1 s upkeep retries). Log tag: `delivery_ack`.
2. If **`app_ack_read_enabled`** and **`live_foreground_peer == peer`**: also `send_inbound_read_ack_if_possible` → **`ack_read`**.

| Situation | Action |
|-----------|--------|
| **Any** inbound text (native connect, UI optional) | **`ack_received`** (mandatory) |
| Chat room **open** in UI for this peer | **`ack_received`** + **`ack_read`** |
| Hub / background / room **closed** | **`ack_received` only** (no new `ack_read`) |
| User **enters** chat | Makepad: **`host::set_app_visible(true)`** then **`host::set_open_room(Some(peer))`**. Native opens the read gate and runs read-ack catch-up. |
| User **leaves** / app **paused** | Makepad: **`host::set_open_room(None)`** on leave; on Android background **`host::set_app_visible(false)`**. Native freezes **`chat_room_exit_at_ms`** and drains pending read acks; **new** mail → **`ack_received` only** until the room opens again. |

**Do not** use “in-room → `ack_read` only” without step 1. Lock hides the hub and leaves the node running, so delivery acks must not depend on the chat screen.

**Leave backlog:** inbound with **`received_at_ms` set** and **`received_at_ms ≤ chat_room_exit_at_ms`** still gets **`ack_read`** after leave. Closing the room stops read acks for later inbound only. See [DESIGN.md](DESIGN.md) § Room open, leave, and read acks.

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

**Streams:** one long-lived `/ghal-bol/msg/1.0.0` per remote identity wire when possible; open only if missing (no connect spam). mDNS (LAN) and coord bridge / public TCP (WAN calls) dial configured contacts only.

### Mechanisms (summary)

| Mechanism | Behaviour |
|-----------|----------|
| **Transcript** | Survives restart; native re-seeds outbox and read-ack queue from disk. |
| **In-memory outbox** | Fast resend path; purged when transcript says delivered/read. |
| **Inbound dedupe** | Duplicate `id` → delivery ack only, no second UI row. |
| **Open room** | `host::set_open_room` — Makepad sets the open conversation; native applies leave/enter and read-ack catch-up. |
| **UI ticks (outgoing)** | `pending` → `delivered` (`ack_received`) → `read` (`ack_read`). |
| **Hub poll → stores** | `dm_event_handler` on each poll applies `dm_message` to contacts + transcript (Makepad does not duplicate). |

There is **no** “give me messages since timestamp X” RPC. Offline delivery depends on the sender’s outbox after reconnect.

### Implementer checklist

Before changing ack policy, verify:

1. **Recipient only** sends `ack_received` / `ack_read`; **sender never** sends `ack_request`.
2. **`ack_received`** on **every** inbound text, always retried from native queue (not gated on Makepad poll or room state).
3. **`ack_read`** only when room is open in UI (`live_foreground_peer` + `app_ack_read_enabled`); may be sent **in addition to** `ack_received`.
4. **Never** clear the read-ack queue on leave.
5. **Never** set `read_ack_sent` on enter alone — only after peer `ack_received` confirms our `ack_read`.
6. **Sender outbox:** resend **text** only until `ack_received` or `ack_read`.
7. **Never** skip `ack_received` because native still has a stale foreground peer after the UI exited.
8. **Read retry throttle:** after a successful wire `ack_read`, set `last_send_ms`; do not resend the same id until `OUTBOX_RESEND_INTERVAL_MS` unless never sent.
9. **Room-enter / leave backlog:** **`dispatch_read_ack_pass`** — seed only when `received_at_ms` set, `read_ack_sent: false`, and `received_at_ms ≤ cutoff_ms` (`chat_room_exit_at_ms` or live session); one drain pass; no multi-hundred-round bursts.
10. **Read-ack eligibility:** never queue `ack_read` without **`received_at_ms`** (not received locally). **`ack_received`** on the wire includes **`received_at_ms`** (recipient authority; stable on duplicate text).
11. **Poll emit gate:** `GossipChatEvent::DmMessage` for acks only when outbox/transcript state actually advances (see DESIGN.md § Room open — near-single-shot read acks).
12. **`apply_inbound_ack`:** return `stores_updated = false` when `patch_outgoing_delivery` / `patch_inbound_read_ack_sent` returns unchanged.
13. **Confirm loop:** inbound `ack_read` → always wire `ack_received` back; inbound `ack_received` with pending read ack → `mark_read_ack_confirmed`. **`mark_read_ack_confirmed` only when `has_pending_read_ack`** — never `has_seen_inbound_id` alone.
14. **Leave drain:** `pending_read_acks` not cleared on leave; freeze **`chat_room_exit_at_ms`** before drain; **`host::set_open_room(None)`** first, then visibility changes if needed.
15. **Transcript keys:** `load_merged` / patch paths expand peer id + `public_key_hex` so threads and ack patches match (DESIGN.md § Transcripts). Poll replay dedupe and `apply_inbound_ack` use **`inbound_transcript_lookup_keys`**.
16. **Truthful ticks:** UI shows `delivery` / read only after `dm_event_handler` patches transcript; `stores_updated` only on real change.

### Android background listener

After unlock, native connect runs on a Rust worker thread in the same process as the Makepad UI. The 1 s upkeep loop (sender text resend, recipient delivery and read ack retries) keeps running while the process lives. Sending `ack_received` / `ack_read` does not depend on the UI poll timer.

**Android and Linux** both run that node inside `ghal_bol_app`. The UI calls `ghal_bol_core::host` and polls only to refresh the screen. Lock hides the hub and leaves the node running. Log out calls `host::lock`, which stops it. Android OEMs can still freeze the process; do not stop the node except on log out or identity delete.

Rebuild the desktop app with `cargo build -p ghal_bol_app --release`. Android packaging is `./scripts/build_android_app.sh`.

Code: `ghal_bol_core/src/connect/`, `ghal_bol_core/src/dm_transcript_v1.rs`, `ghal_bol_core/src/dm_transcript_store.rs`.

## Connect invite vs connect config

| Step | What happens |
|------|----------------|
| Scan QR (format **2**) | App stores remote **`public_key_hex`** (identity wire). |
| `host::start_network` | Registers saved contacts by `public_key_hex`. |
| Connect | mDNS (LAN) and/or coord bridge / public TCP (WAN calls); native opens `/ghal-bol/msg/1.0.0` on connect (no key-exchange prelude beyond `transport_kem_hello`). |
| `chat_ready` | Outbound stream open; safe to send encrypted/signed frames. If this peer is **foreground**, native seeds read acks and runs **one pass** of queued `ack_read` for backlog. |
| Chat | Encrypt to recipient `public_key_hex`; sign with local identity key. |

See `docs/GHAL_BOL_URI_SCHEME.md` for invite formats. **No multiaddrs** are required on new invites.

## Starting native connect

The Makepad app and native connect are one process. `host::start_network` starts the connect worker and registers saved contacts.

`host::start_network` takes (among other fields):

```json
{
  "dm_peers": [
    { "public_key_hex": "02…" }
  ],
  "transcript_path": "/path/to/chat_transcript_v1.json",
  "app_namespace": "com.ghalbol"
}
```

**`already_running`:** If native connect is already up, `start_network` must still set the handler context and re-register every `public_key_hex` in `dm_peers`. Otherwise inbound events log `handler context not set` and stores do not update.

## UI send / poll / register

The app calls `ghal_bol_core::host`. The same behaviour lives in `p2p_runtime`.

| Call | Role |
|------|------|
| `host::start_network` | Start native connect and register saved contacts |
| `host::set_open_room` | Open chat, or clear it |
| `host::send_text` | Queue send; non-blocking |
| `host::poll_event` | JSON events for the UI; the poll does not send acks |

The open room is `host::set_open_room`. The chat column does not send acks.

Poll responses may include **`stores_updated": true`** after `dm_event_handler` writes contacts/transcript. Makepad bumps **roster** on `peer_identified` and inbound **text**; preview-only bumps for ack-only events.

### Poll event kinds

| `kind` | Meaning |
|--------|---------|
| `listening` | Local TCP listen address |
| `peer_connected` | native connection up |
| `peer_identified` | Remote `public_key_hex` |
| `chat_ready` | Outbound stream open; safe to send |
| `dm_message` | Inbound `text` or ack (`msg_kind`, `id`, `text`, `ref_id`, `sender_public_key_hex`, …) |
| `outbound_sent` | Outbound frame written to stream |
| `send_failed` | Outbound `message_id` could not be sent (outbox may still retry) |
| `dial_failed` | Dial error |

## Makepad persistence

| Store | Path / key |
|-------|------------|
| Contacts | `contacts_v1.json` — keyed by conversation identity (`public_key_hex`) |
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
| Stream + outbox + read acks | `ghal_bol_core/src/connect/` |
| Envelope crypto | `ghal_bol_core/src/msg_v1.rs`, `ghal_bol_core/src/transport_kem_v1.rs`, `ghal_bol_core/src/connect/transport_kem.rs` |
| Transcript helpers | `ghal_bol_core/src/dm_transcript_v1.rs`, `dm_transcript_store.rs` |
| Invite verify | `ghal_bol_core/src/connect_invite_v1.rs` |
| Network runtime | `ghal_bol_core/src/p2p_runtime.rs` |
| UI API | `ghal_bol_core/src/host.rs` |
| Poll → stores | `ghal_bol_core/src/dm_event_handler.rs` |
| UI invite | `connect_invite_v1.rs` |
