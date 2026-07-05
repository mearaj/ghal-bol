# Ghal Bol — system design

This document is the **single design reference** for how Ghal Bol is meant to work: layers, messaging state, chat-room semantics, and P2P lifecycle. Wire-level detail lives in [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md); invites in [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md).

**AI / new contributors:** read [../AGENTS.md](../AGENTS.md) first, then this file. Transport (libp2p): [TRANSPORT.md](TRANSPORT.md). Connectivity policy: [TRANSPORT.md](TRANSPORT.md) § **Connectivity lifecycle**.

## Goals

- **Direct peer-to-peer** text between people who already know each other (QR / link handoff).
- **No server-side chat history** — each device keeps its own transcript.
- **Recipient-authority** delivery and read state — the sender does not invent ticks.
- **Resilience** — sender retries **text** until acked; recipient retries **acks** until the stream accepts them.
- **Thin UI** — Flutter handles navigation and rendering; **`ghal_bol`** owns crypto, **libp2p** transport, outbox, **all ack send/retry**, and persistence. Flutter **polls** only to refresh UI from native stores.

## Layer split

```text
┌──────────────────────────────────────────────────────────────┐
│  ghal_bol_ui — Flutter (main process)                         │
│  Screens, hub/chat, QR, composer                              │
│  FFI: identity unlock, contacts list, transcript read         │
│  RPC client: p2p_start, send, poll, foreground (see below)    │
│  Poll P2pEventBridge → refresh UI only (no ack logic)         │
└───────────────┬──────────────────────────────┬───────────────┘
                │ dart:ffi (same data dir)      │ Unix socket JSON-RPC
                ▼                              ▼
┌───────────────────────────┐    ┌──────────────────────────────┐
│  ghal_bol in UI process    │    │  ghal_bol in :p2p / daemon    │
│  Keystore, contacts_v1,    │    │  libp2p node, outbox,          │
│  transcript read/write   │    │  ack send/retry, dm_event_    │
│  via FFI                 │    │  handler on p2p_poll         │
└───────────────────────────┘    └──────────────────────────────┘
         shared on-disk stores (contacts_v1.json, chat_transcript_v1.json)
```

**Linux / Android:** libp2p runs **out-of-process** (`ghal_bol_daemon` or `GhalBolP2pService` in `:p2p`). The UI process still loads `libghal_bol.so` for identity and store I/O over FFI. Both processes must use the **same** data directory and `app_namespace`. One **namespace root** per build holds keystore, prefs, and `ghal_bol/` (contacts, transcript): Linux `~/.local/share/com.ghalbol.debug/` (debug) or `~/.local/share/com.ghalbol/` (release); Android `{app_flutter}/com.ghalbol.debug/` (debug) or `{app_flutter}/` (release). See [IDENTITY.md](IDENTITY.md).

| Concern | Owner |
|---------|--------|
| libp2p listen/dial, streams, outbox, ack send/retry | `chat_server.rs`, `p2p_runtime.rs` (in **:p2p** / daemon) |
| Coord endpoint → dial helpers | `coord_runtime.rs`, `dm_transport/` |
| Envelope crypto | `msg_v1.rs`, `secp256k1_seal.rs` |
| Apply `dm_message` → contacts + transcript | `dm_event_handler.rs` (on **`p2p_poll`** in the P2P process) |
| Contacts / previews / unread | `contacts_v1.rs` (disk; UI reads via FFI) |
| Transcript lines, `delivery`, `read_ack_sent`, `received_at_ms` | `dm_transcript_store.rs`, `dm_transcript_v1.rs` |
| Invite build/parse/verify | `connect_invite_v1.rs`, `invite_ffi.rs` |
| Hub, roster, foreground, layout | `chat_hub_screen.dart` |
| Delivery tick **display** rules (comments) | `dm_delivery_sync.dart` |
| P2P RPC + poll bridge | `p2p_runtime.rs`, `daemon/server.rs`, `ghal_bol_p2p.dart`, `p2p_event_bridge.dart` |
| Android background permissions (screen off) | `android_background_readiness.dart`, `BackgroundReadiness.kt`, `embedder_storage.dart` |

**Rule:** New protocol or state-machine behaviour belongs in **Rust** first, exposed via FFI or daemon RPC; Dart should not re-implement ack policy, outbox, or `dm_message` store merges.

**`ghal_bol_ui` should stay minimal** — only what is required for smooth UI (screens, hub foreground signals, poll bridge, FFI wrappers). See [ARCHITECTURE.md](ARCHITECTURE.md).

## Identity and trust

- One **secp256k1** keypair per device → libp2p **PeerId**, signatures, and message sealing.
- Connect invite (format **2**) carries **`public_key_hex` only** — no PeerId, no IP/multiaddrs on the wire.
- On each DM stream: Noise proves PeerId; every frame’s `sender_public_key_hex` must **match** that PeerId.

## End-to-end encryption (product rule)

**All communications that are between two known contacts using their secp256k1 keys must be E2E** — only those two identities (or holders of the matching private keys) can read the payload.

| Channel | E2E mechanism | Keys |
|---------|----------------|------|
| **Text DM** | `ghal_bol_msg_v1` sealed inner JSON | Ephemeral ECDH + AES-GCM to recipient pubkey; signed envelope |
| **Call signaling** (invite, video_on/off, …) | `ghal_bol_call_v1` | Same seal + signature on DM stream |
| **Call media** (audio + in-call video) | Per-frame AES-GCM seal on libp2p substreams | `call_media_key.rs`: ECDH(local secret, peer `public_key_hex`) + HKDF(`call_id`, both pubkeys); see [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md), [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md) |

libp2p **Noise** encrypts the transport; it is not a substitute for app-layer E2E above. New features (voice messages, attachments, group chat) must follow the same rule unless explicitly scoped otherwise in design docs.

Implementation detail: [GHAL_BOL_VOICE_V1.md](GHAL_BOL_VOICE_V1.md) (calls), [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) (chat). [AGENTS.md](../AGENTS.md) golden rule 7.

## Asymmetric “who knows whom”

| Role | After QR |
|------|----------|
| **Guest** (scanner) | Knows host’s `public_key_hex` → derives PeerId → registers `dm_peer` → dials |
| **Host** (QR shown) | May have **zero** contacts until first inbound connection or frame; first inbound text may create an **unknown** roster row until the user **Add**s or replies — see [Contact trust (`is_known` / `is_blocked`)](#contact-trust-is_known--is_blocked) |

Both sides must run P2P. For **configured** contacts only (no public directory): **WAN** (coord lookup + relay/public paths) and **LAN** (mDNS `_ghalbol._tcp` / direct TCP when that peer is on the local LAN) run **in parallel** on Wi‑Fi — both stay active when connected ([TRANSPORT.md](TRANSPORT.md) § “Both links active”). Do not use coord RFC1918 addrs alone as LAN; mDNS discovery on this device is required for the LAN path.

## Truthful status in the UI (critical)

Users must **never** see delivery or read ticks that the local device has not earned from the network. The UI is a **view** of on-disk transcript + poll-applied acks — not a guess, not “optimistic sync”, not a mirror of the other phone.

| Principle | Meaning |
|-----------|---------|
| **Recipient authority** | Only the peer who received the text sends `ack_received` / `ack_read`. Our UI must not show “delivered” or “read” on outbound lines until **`dm_event_handler`** patches `delivery` after an inbound ack on **`p2p_poll`**. |
| **No sender self-promotion** | The sender never sends `ack_request` and must not bump ticks locally when send succeeds — only when peer acks arrive. |
| **Transcript is source of truth** | Ticks come from `chat_transcript_v1.json` (`delivery` on outbound, `read_ack_sent` on inbound). Flutter **reloads** that state; it does not invent it. |
| **Poll applies, does not send** | `p2p_poll` runs `apply_p2p_event_json` in `:p2p` / daemon. Poll **never** transmits acks. A fast poll loop does not mean “more read receipts”. |
| **No-op = no UI storm** | If transcript is already `delivered` / `read` / `read_ack_sent`, native returns **`stores_updated: false`** and poll **drops** duplicate ack events — so the UI is not spam-reloaded with fake “new” state. |
| **Disagreement is normal** | Outbound `pending` while the peer already has the message is valid. Do not “fix” the UI by syncing the other device’s model. |

**Wrong (fake or misleading state):**

- Showing read/delivered because the user opened the chat, before peer acks are applied to transcript.
- Setting `read_ack_sent` on enter-room without peer **`ack_received`** confirming our **`ack_read`**.
- Emitting a poll/UI event for every wire ack retry when transcript did not change.
- Merging duplicate transcript rows in Dart that hide missing native acks.

**Right:** show exactly what is in the native transcript after poll/FFI merge, with monotonic-only updates (pending → delivered → read). All LAN and WAN paths write to the **same** transcript store ([Unified message state (E)](#unified-message-state-e--single-source-of-truth-2026-06-17)); `read` never downgrades to `delivered` when duplicate acks arrive on different paths.

## Message state — intent and how Ghal Bol implements it

Read this before changing acks, ticks, foreground, or transcript fields. The goal is the same as a well-behaved P2P chat: **the recipient owns delivery/read**, **each phone keeps its own view**, **progress is learned from the network over time**, not from forcing both UIs to match.

### Core intent (product rules)

1. **Only the recipient advances “delivered” and “read” for a given text.** The sender must not bump its own checkmarks locally. The sender may only **retry the same text** until the peer’s signals arrive.

2. **Sender and recipient are allowed to disagree.** Your outbound row can still say `pending` while the peer already has the text. That is normal. There is **no** feature to “sync read state” or download the other device’s UI model.

3. **Two levels, not one.** “I got it” (delivered) and “I read it in the conversation” (read) are separate decisions. Delivered must happen even when the app is in the background. Read is tied to **having the chat room open**, not to having a name highlighted in a list.

4. **Monotonic progress.** State only moves forward: pending → delivered → read on the sender; inbound moves toward read_ack_sent on the recipient. Duplicates merge upward, never downgrade.

5. **Long-lived P2P + ~1 s upkeep.** A background networking process keeps streams and retries until the sender’s copy of outbound mail is acknowledged. UI poll is for **showing** what native already stored — not for **sending** receipts.

### Mental model: one text, two local views

```text
Peer A (sender)                         Peer B (recipient)
─────────────────                       ───────────────────
Stores outbound line                    Stores inbound line
  delivery: pending → …                   read_ack_sent: false → true
  (ticks in UI)                           (no ticks on B for A's send)

B decides when A's text is "delivered"  → B sends ack_received(ref_id)
B decides when A's text is "read"       → B sends ack_read(ref_id) [room open]

A's upkeep resends same text id         B's upkeep retries acks if stream was down
until ack_received or ack_read          until written on wire
```

### Recipient lifecycle

| When | What should happen | Ghal Bol |
|------|-------------------|----------|
| **First time** peer’s text is accepted | Mark “delivered” toward sender; persist locally | Wire **`ack_received`**; append transcript; update contact preview (`dm_event_handler` on poll) |
| **Duplicate** same `id` (sender resend) | Still ensure delivered signal; no second bubble | **`ack_received`** if needed; dedupe transcript |
| **Chat room open** for that peer | Treat visible thread as read toward sender | Hub sets **`live_foreground_peer`** + **`app_ack_read_enabled`** → native **`ack_read`** after **`ack_received`**; seed backlog + one pass on enter |
| **Chat room closed** (list, other tab, navigated back, app paused) | New mail stays “delivered” only | **`ack_received` only** for **new** inbound |
| **Left room but mail arrived while open** | Still finish read signaling for that backlog | **Keep** read-ack retry queue; do **not** clear on leave |
| **Mail arrives after leave** | Do not auto-read | **`ack_received` only** until room opens again |
| **Not received locally yet** | Do not read-ack toward sender | No inbound transcript row with **`received_at_ms`** → **never** queue **`ack_read`** (avoids false read when B sent but A never got the text) |
| **Opened chat UI** | Does **not** by itself mean read is done on the wire | **`read_ack_sent`** only after sender’s **`ack_received`** confirms our **`ack_read`** |

Native code: `send_inbound_delivery_ack`, `send_inbound_read_ack_if_possible`, `pending_delivery_acks` / `pending_read_acks` in `chat_server.rs`. Runs in **`:p2p` / daemon** — not in the Flutter isolate.

**Zombie mux recovery (2026-06-24, extended 2026-06-25, 2026-06-28, 2026-07-02):** If the stream writer flag is set but frames fail (`send_ack_frame` / outbox resync error while `writer_open_for_peer`), native calls **`request_dm_stream_reopen`** — not Flutter poll. Symptom: repeating `resync N pending` with `conn=true,stream=true`. **2026-06-25:** after LAN→mobile handover, Wi‑Fi side may keep zombie **direct** mux while phone is on relay — inbound works, outbound never gets `ack_received`; fix is asymmetric mux detection + coord lookup for foreground peer ([TRANSPORT.md](TRANSPORT.md) § **Post-mortem 2026-06-25**). **2026-06-28:** reopen cannot replace a writer that still *looks* alive, so when the peer's frames arrive on a live **duplicate** stream the duplicate handler **adopts** the writer slot (`adopt_duplicate_mux_as_writer`, gated by `duplicate_mux_should_take_over`); a **writer-generation epoch** (`finalize_dm_writer_if_current`) stops a stale handler's later close from deleting the live writer (TRANSPORT.md § **Post-mortem 2026-06-28**). **2026-07-02:** on Wi‑Fi, relay `InboundCircuitEstablished` during handover sets **`relay_inbound_handover_active`** (`should_mark_relay_inbound_handover`) so duplicate stream-open adopt fires immediately — not after 5s reconcile / 40s burst (TRANSPORT.md § **Asymmetric mux recovery** steps 5–7). **Known follow-on:** once mux recovers, pending outbox and acks may **burst** in one drain (`burst resync N pending row(s)`) — multi-second stalls between bursts during handover are transport-level, not a Dart poll bug; do not route through `NetworkHelper`.

### Sender lifecycle

| When | What should happen | Ghal Bol |
|------|-------------------|----------|
| User sends | Persist locally, queue on wire when stream ready | `p2p_send_text_dm` → transcript `delivery: pending` + outbox |
| Peer slow / offline | Same message id retried ~1 s | Outbox + `transcript_sync_outbound_tick` |
| Peer signals delivered | Stop resending; single-check tick | Inbound **`ack_received`** on poll → `delivery: delivered` |
| Peer signals read | Stop resending; read tick | Inbound **`ack_read`** on poll → `delivery: read` (implies delivered) |
| Peer echoes nothing locally | Sender must **not** self-promote ticks | Sender **never** sends `ack_request`; only applies peer acks |

### What “chat room open” means

**Open** = the user is in the **conversation UI** for that contact, and the hub has told native which peer is foreground.

| Meaning | Ghal Bol |
|---------|----------|
| **Is** open | Hub engaged: narrow **`_narrowShowRoom`** or split **`_splitChatEngaged`** + **`set_foreground_peer`** + **`set_app_ack_read_enabled(true)`** |
| **Is not** open | Contact row selected in hub only; chat pane mounted off-room (`hubPollsEvents`); app paused — **`ChatScreen` must not** set foreground in hub mode |

Read marking applies only while the **conversation screen is active** in the hub, not when the contact merely appears in a list.

### How Ghal Bol differs on the wire (same intent)

Some stacks store `Received` / `Read` **on the message struct** and echo the **full message** on the stream when the sender retries; the sender merges a higher `State` from that echo.

**Ghal Bol does not do that.** Text frames carry body + `id` only. Progress uses **separate** signed envelopes:

| Intent | Wire in Ghal Bol | Local transcript |
|--------|------------------|------------------|
| Delivered | **`ack_received`** (`ref_id` = text `id`) | Outbound **`delivery`**: `delivered` |
| Read | **`ack_read`** (`ref_id` = text `id`) | Outbound **`delivery`**: `read`; inbound **`read_ack_sent`** after confirm |
| Sender learns | Inbound ack events on **`p2p_poll`** → `dm_event_handler` | Not from a resent text blob with embedded status |

Read still reaches the sender when the recipient has marked read locally and the stream is up: native **retries `ack_read`** on upkeep and on **enter/leave read-ack passes** (`dispatch_read_ack_pass`), instead of embedding `Read` inside a resent text payload.

### Leave / backlog (do not get this wrong)

This is a **sensitive** product rule. Users expect mail they **received while actively in the room** (per **`received_at_ms` ≤ frozen `chat_room_exit_at_ms`**) → the other side should eventually get **read**, even after they go back to the list or switch tabs.

| | Behaviour |
|---|-----------|
| **Wrong** | Leaving the chat **clears** the read-ack queue or **stops** all `ack_read` work. |
| **Wrong** | Turning off `app_ack_read_enabled` **clears** `live_foreground_peer` before native can run leave drain (loses which peer to flush). |
| **Wrong** | Hub disables read gate **before** `SetForegroundPeer(null)`, so leave seed/drain never runs. |
| **Wrong** | Read-ack seed from “inbound row exists” without **`received_at_ms`** — false read when sender transmitted but this device never accepted. |
| **Right** | Leaving stops **new** `ack_read` for **new** inbound only. |
| **Right** | Eligible inbound (`received_at_ms` set, `read_ack_sent: false`, `received_at_ms ≤ chat_room_exit_at_ms`) stays in `pending_read_acks`; native **keeps retrying** `ack_read` (~1 s) until the sender’s **`ack_received`** confirms each id. |
| **Right** | **`ack_received`** for **all** inbound (including after leave) continues in `:p2p` — delivery is never gated on the room. |

**Native (`chat_server/`):**

- **`pending_read_acks` is not cleared on leave** — only per-id dequeue on confirm or successful policy.
- On leave/switch: **`freeze_chat_room_for_peer`** (writes **`chat_room_exit_at_ms`** on the contact) then **`dispatch_read_ack_pass`** with the frozen cutoff (not gated on `app_ack_read_enabled`).
- While in-room: live session clock + foreground contact **`chat_room_exit_at_ms`** update ~1 s (`tick_chat_room_session_if_active` on `dm_upkeep_tick`).
- **`last_room_peer`**: remembers who was in the room if `SetForegroundPeer` enter was still queued — leave drain still runs.
- **Leave read-ack drain**: only from **`SetForegroundPeer(null)`** or peer switch on the outbound queue — **not** from `set_app_ack_read_enabled(false)`.
- **`set_app_ack_read_enabled(false)` does not call `sync_foreground_peer_now(None)`** — foreground is cleared by hub via **`SetForegroundPeer(null)`** so leave logic sees the previous peer.

**Hub close order (`chat_hub_screen.dart`):**

1. `setForegroundConversation(null)` and await (native leave drain + clear foreground).
2. Then `setAppAckReadEnabled(false)` (stop **new** in-room read; backlog drain already scheduled).

**Hub open order:**

1. `setAppAckReadEnabled(true)` first (read gate on before enter-room cmd may run on outbound queue).
2. `setForegroundConversation(peer)` and await.
3. Native: **`begin_chat_room_session`** + **`dispatch_read_ack_pass`** on enter. When **`SetForegroundPeer`** is skipped (same pk already foreground — e.g. Android **`inactive`→`resumed`**), **`p2p_sync_ui_session`** still queues **`RunReadAckCatchup`** so backlog **`ack_read`** drains (§ “Fixed 2026-06-29”).

### Inbound `received_at_ms` and read-ack eligibility

**Problem:** Seeding read acks from “inbound row exists + `read_ack_sent: false`” can falsely read-ack mail the sender transmitted but this device **never accepted** (outbox on their side, no local receive yet).

**Rule:** **`received_at_ms`** is the recipient’s **first local accept** time for that text `id`.

| Where | Who sets it | Stability |
|-------|-------------|-----------|
| Inbound transcript row | Recipient on first wire accept / poll append | **Set once**; duplicate text resends must **not** overwrite |
| Wire **`ack_received`** | Recipient in the signed ack envelope | Same value as inbound row; sender learns when peer got the text |
| Outbound transcript row | Sender from peer’s **`ack_received.received_at_ms`** | First value wins |

**Chat room session cutoff (`contacts_v1.json`):**

| Field | Role |
|-------|------|
| Live session clock (in-memory) | Updates ~1 s while room open + read gate + UI visible |
| **`chat_room_exit_at_ms`** on each contact | Mirrors the live clock for the **foreground** peer while in-room; **frozen** on leave, peer switch, or UI inactive |

**Read-ack pass (enter + leave):** queue **`ack_read`** only when **`received_at_ms` is set**, **`read_ack_sent: false`**, and **`received_at_ms <= chat_room_exit_at_ms`** for that contact (after leave/switch, the frozen contact value is the cutoff).

**Room enter:** begin session clock + read-ack pass with live cutoff.

**Room leave / switch:** freeze prior contact’s **`chat_room_exit_at_ms`**, then read-ack pass with that frozen cutoff.

**Legacy rows** (inbound without `received_at_ms` on disk but with `message_id`): treated as received for eligibility only; new accepts always persist **`received_at_ms`**.

### Who must not touch policy

| Layer | May | Must not |
|-------|-----|----------|
| **`chat_server` (`:p2p`)** | Send/retry acks; outbox text resend | — |
| **`dm_event_handler` (on poll)** | Write contacts + transcript from events | Send acks |
| **Flutter** | Foreground peer, composer, poll UI | Send acks; merge dm policy; assume synced ticks across devices |

Duplicate inbound `id`: delivery ack if needed; **no** second transcript row.

## Chat room — native gates (summary)

| Situation | Recipient sends |
|-----------|-----------------|
| Any inbound text | **`ack_received`** (always, including `:p2p` background) |
| Room **open** | **`ack_received`** + **`ack_read`** |
| Room **closed** / new mail after leave | **`ack_received` only** |
| **Enter** room | Seed transcript + **one pass** of queued **`ack_read`** for backlog (then ~1 s retries only until confirm) |
| **Leave** / pause | Clear foreground; **retry** queued read acks; no new **`ack_read`** for new mail |

Hub clears foreground on pause so `:p2p` never keeps a stale “in room” flag and skips delivery. **`ack_received` always precedes `ack_read`.**

## Read receipts — wire volume, confirm loop, poll (do not flood)

**Healthy behaviour:** for each text `id`, the recipient sends **at most one immediate `ack_read` while the room is open**, then **at most ~1 wire retry per second** until the sender confirms. The sender should see **one** inbound `ack_read` poll apply per message (blue tick), not hundreds.

**Duplicate `ack_read` for the same `ref_id` is not normal** and almost always means the implementation broke the confirm loop or retried too fast — not that “dedupe at poll” is the product fix.

### Confirm loop (both peers must implement)

```text
Recipient (room open)                         Sender (our outbound text id = X)
─────────────────────                         ─────────────────────────────────
Inbound text id=X
  → ack_received(ref=X)  ──────────────────→  delivery: delivered
  → ack_read(ref=X)        ──────────────────→  delivery: read  (first time only)
                                              → MUST send ack_received(ref=X)
                    ←──────────────────────────   (confirms they got our ack_read)
mark_read_ack_confirmed(X)
  → stop read retries for X
  → read_ack_sent on inbound row (transcript)
```

- **`ref_id` on `ack_read` / `ack_received` is always the original text message `id`**, not the ack frame’s own id.
- On inbound **`ack_read`**: native **always** replies **`ack_received(ref_id=text id)`** so the peer can stop retrying — even if our transcript was already `read`.
- On inbound **`ack_received`**: if `ref_id` matches an inbound id we sent **`ack_read` for**, call **`mark_read_ack_confirmed`** and dequeue read retries.

### Native send rules (`chat_server.rs`)

| Rule | Implementation |
|------|----------------|
| First send in-room | `send_inbound_read_ack_if_possible` after `ack_received`; enqueue + try wire immediately |
| Enter / leave pass | **`dispatch_read_ack_pass`**: seed when `received_at_ms` set + `read_ack_sent: false` + `received_at_ms ≤ cutoff_ms`; drain once |
| Retry cadence | `PendingReadAck.last_send_ms`: no second wire send for same id until **`OUTBOX_RESEND_INTERVAL_MS` (~1 s)** |
| Poll events | Emit inbound **`ack_read`** to the poll queue **always** (first apply patches `read`; wire retries no-op in `apply_inbound_ack`). Emit outbound **`ack_received`** when outbox had the row or read-ack confirm advanced. |
| Transcript on poll | `dm_event_handler::apply_inbound_ack` returns **`stores_updated` only if** `patch_outgoing_delivery` / `patch_inbound_read_ack_sent` returns **changed** |
| Poll drain | `p2p_poll_event` skips returning duplicate ack events that did not change stores |

### Flutter rules (display only)

| Rule | Where |
|------|--------|
| Hub sets foreground + `app_ack_read_enabled` | `chat_hub_screen.dart` when room open — see **Linux desktop layout sync** below |
| Chat must not set foreground when `hubPollsEvents` | `chat_screen.dart` |
| Acks arrive via `ingestP2pEvent` | Do not also full-merge transcript on every `previewChangeCount` for the open room |
| Delivery ticks | Debounced `mergeTranscriptFromNative(deliveryOnly: true)` on ack polls — not one FFI reload per retry |
| Never send acks from Dart | Poll refreshes UI only |

### Linux desktop — layout sync and read gate

Native **`ack_read`** is gated on **`may_send_in_room_read_ack`** in **`ghal_bol`** (`chat_server.rs`): `app_ui_visible` + `app_ack_read_enabled` + foreground peer on the **`:p2p`/daemon** thread. Flutter pushes that snapshot via **`p2p_sync_ui_session`** only (`GhalBolUiSession`).

**Verified 2026-06-15:** Linux desktop ↔ Android LAN chat **>10 minutes** continuous (`conn=true`, `stream=true`), delivery/read ticks without window resize. Root cause of the prior “resize fixes ticks” bug was documented in App log (see below).

**Shipping behaviour (`chat_hub_screen.dart`, `p2p_event_bridge.dart`, `ghal_bol_ui_session.dart`) — keep this:**

| Mechanism | Rule |
|-----------|------|
| `_isHubChatRoomOpen` | **Split shell:** room is open when Chats tab + **`_selectedConversationKey`** (66-hex pk) — **not** `_splitChatEngaged`. **Narrow shell:** `_narrowShowRoom`. |
| `_syncNativeForegroundIfLayoutChanged` | Called from `build()`; runs **post-frame** only. Sync when `_layoutSyncedRoomOpen != _isHubChatRoomOpen` — **not** on every frame when an RPC failed. Layout **close** debounced ~120 ms so resize flicker does not leave-drain. |
| Room open sync | **`setVisible(true)` then `setRoom(pk)`** — required so `p2p_sync_ui_session` returns `read_receipts: true`. **`setRoom` alone is not enough** if `_uiVisibleDesired` is still false. |
| `_layoutSyncedRoomOpen` / `_lastSyncedForegroundPk` | Updated after `_syncNativeForegroundPeerAsync` completes `awaitApplied()`. |
| Linux read-gate nudge | Debounced **`GhalBolUiSession.nudge()`** (+ `setVisible(true)`) on inbound text / stream ready / preview bump / 8 s keepalive while room open — **belt-and-suspenders only**; primary fix is Linux `inactive` rule below. |
| `node_ready` in hub | Poll refresh only (`setState`) — **no** extra session reapply from hub (bridge already runs `_reapplyDeferredSessionRpc` on `node_ready`). |
| GTK minimize | `paused`/`hidden` do **not** clear room (minimize ≠ leave). |
| **Linux `inactive`** | **Do not** `setVisible(false)`. GTK window drag / brief focus loss must **not** set `:p2p` `read=false` while chat pane is visible. Log: `lifecycle inactive on Linux — read gate unchanged`. **Android `inactive` still gates read off** — no new **`ack_read`** while shade/task switcher; **`resumed`** must re-open the gate **and** native must run read catch-up (§ “Fixed 2026-06-29”). |

**Delivery/read ticks (sender view):** blue tick only after peer **`ack_read`** patches transcript on poll (`mergeTranscriptFromNative(deliveryOnly: true)`). Never promote ticks in Dart.

#### Fixed 2026-06-15 — Linux desktop read ticks (do not regress)

**Symptom:** inbound **`ack_received`** but no **`ack_read sent`** while chat pane visibly open; tiny window resize “fixed” it temporarily.

**Log proof (Linux App log):**
```text
ui_session_applied visible=true room=<pk> read=true
lifecycle inactive → ui not visible (room unchanged)   ← old bug
ui_session_applied visible=false room=<pk> read=false  ← gate off, room still open
```
No matching `resumed` → read gate stayed off until layout/`resumed` re-sync.

**Fix (Dart hub only — no native change):** skip `GhalBolUiSession.setVisible(false)` on **`AppLifecycleState.inactive`** when `Platform.isLinux`; always **`setVisible(true)`** before **`setRoom(pk)`** on room open and on read-gate nudge.

**Verified:** Android ↔ Linux desktop LAN, **>10 min** soak — messages, **`ack_read sent`**, blue ticks, **`conn=true,stream=true`** without resize.

**Regression — never reintroduce:**
- `setVisible(false)` on Linux **`inactive`** (restores the resize workaround bug).
- Fixing ticks with the forbidden **`lastApplySucceeded`** patch (broke P2P for hours — § below).
- Fake ticks in Flutter without native transcript patch.

#### FORBIDDEN — reverted 2026-06-15 “`lastApplySucceeded`” hub session patch

**Never reintroduce** (human or AI). Attempted in `chat_hub_screen.dart`, `p2p_event_bridge.dart`, `ghal_bol_ui_session.dart`; **reverted** after production break. This is **not** the current implementation.

| Forbidden change | Why |
|------------------|-----|
| `GhalBolUiSession.lastApplySucceeded` / `uiSessionLastApplyOk` / `_uiSessionLastApplyOk` | Tracks RPC ok — then used to drive retry loops |
| `_invalidateNativeForegroundSync()` on resume, `node_ready`, call end, `_attachHubChat` | Forces `_layoutSyncedRoomOpen = null` → burst of close/open sync |
| `_syncNativeForegroundIfLayoutChanged` retry when `!lastApplySucceeded` (from `build()` every frame) | **Session RPC storm** on daemon state socket while P2P recovering |
| Hub `node_ready`: invalidate + `reapplyUiSession()` + `_syncNativeForegroundPeer()` **on top of** bridge reapply | Duplicate `p2p_sync_ui_session`; fights libp2p upkeep |
| `_attachHubChat` → extra `_syncNativeForegroundPeer()` | Double `room open` sync on every `ChatScreen` mount |
| Only mark `_layoutSyncedRoomOpen` after native `ok: true`; remove optimistic pre-set before sync | Intended to fix read gate; paired with per-frame retry it **stopped working chat** |

**Observed harm (adb logcat, Android + Linux, 2026-06-15):**

- **`room closed → sync ui session (no room, leave drain)`** storms at hub bootstrap (×3–4 before user opens a chat) → native **`SetForegroundPeer(None)`** + leave drain while `:p2p` still starting.
- Bursts of **`room open → sync ui session`** → foreground churn during relay negotiation.
- **`stream_ready_count=0`**, **`dm connection closed`**, sends stuck at **`outbound waiting: not connected`** — P2P messaging **broken** (not a tick cosmetic issue).
- UI **`conv=solo` / `pk=(none)` / `transcript reload … rows=0`** while sibling files under `{namespace}/ghal_bol/` still on disk (`contacts_v1.json`, `chat_transcript_v1.json`) — looks like **total data loss**; users create a **new identity** or hit **`identity_split` / `resetFirstTimeIdentity`** paths → **keystore effectively gone** without a single `delete` API call.

**Canonical rule:** P2P session sync must stay **low volume**. Linux read ticks were fixed **2026-06-15** with the **`inactive`/Linux** rule above — **not** the reverted patch. If read gate drifts again, use debounced **`GhalBolUiSession.nudge()`** + verify `setVisible(true)` on room open; **never** per-frame `build()` loops, **`lastApplySucceeded`**, or duplicate hub+bridge reapply.

#### Fixed 2026-06-19 — delivery/read ticks (confirm loop + transcript keys)

**Symptoms (Android ↔ Linux desktop, same LAN):**

| Symptom | What users saw |
|---------|----------------|
| Sender stuck on **single tick** | Android sent text; desktop got it (`ack_received` on wire) but poll never patched `delivery=delivered` — `inbound ack no matching row: kind=ack_received has_out=false` |
| **False read** on desktop | Hub showed thread as read / zero unread while the chat room was **closed**; or thousands of inbound rows had `read_ack_sent: true` on disk though the peer never received `ack_read` (no blue tick on their side) |
| **Unread drift** | Opening a room cleared unread; leaving or switching rooms left stale unread on the old contact |
| **Ack storm** (bad intermediate fix) | `seeded 1320 pending read ack(s)`, hundreds of `ack_read` per second, `poll drain saturated batch=32 totalEvents=196` — P2P connect degraded |

**Root causes (native, not Flutter):**

1. **Confirm loop treated “saw inbound text” as “sent `ack_read`”.** In `chat_server.rs`, inbound `ack_received` called `mark_read_ack_confirmed` when `has_pending_read_ack(ref_id) **or** has_seen_inbound_id(ref_id)`. Every accepted inbound text id enters the seen set. A peer’s unrelated `ack_received` (e.g. confirming *their* delivery ack) could flip **`read_ack_sent=true`** on disk **without** this device ever queueing `ack_read` — violating § “Read receipts — wire volume, confirm loop” (`read_ack_sent` only after **our** `ack_read` and peer confirm).

2. **Poll and ack patches used a single conversation bucket.** `apply_inbound_ack` and poll-replay dedupe called `load_merged` with one key (`public_key_hex` from the event) while older lines lived under the **libp2p peer id** bucket (or vice versa). Outbound acks did not patch ticks; poll replay re-appended unread for the same `message_id`.

3. **Foreground leave did not clear hub unread** for the room being left — only entering a room called `clear_unread`.

**Fix (shipped — Rust only):**

| Area | Change |
|------|--------|
| Confirm loop | `mark_read_ack_confirmed` only when `has_pending_read_ack(ref_id)` — **removed** `has_seen_inbound_id` from that branch (`chat_server.rs`) |
| Merged keys | `inbound_transcript_lookup_keys` in `dm_event_handler.rs` — pk + wire peer id + contact buckets; used by poll-replay dedupe, `apply_inbound_ack`, and `seed_read_acks_for_peer_from_transcript` |
| Hub unread | `set_foreground_peer`: `clear_unread` on **leave/switch** (old pk) and on **enter** (new pk) |
| Room-enter seed | `received_at_ms` set + `read_ack_sent: false` on disk; seed keys via `inbound_transcript_lookup_keys` — **not** “all inbound rows” and **not** unreceived mail |

**Attempted fixes that were reverted (do not reintroduce):**

- Seeding **every** unconfirmed inbound row on room enter (ignoring `read_ack_sent`) → wire/poll ack storm.
- Extra hub/bridge `p2p_sync_ui_session` / `nudge` / per-open session reapply — fought P2P startup (see § “FORBIDDEN — reverted 2026-06-15”).
- Gating contact `unread_count` on the native read gate instead of `foreground_public_key_hex` — hid real unread when the room was closed.

**Legacy transcript note:** Rows written while the loose confirm loop was active may still show `read_ack_sent: true` without the peer ever getting a blue tick. **New** messages after this fix follow the normative confirm loop. Repairing old rows is optional data hygiene, not required for new chat.

**Code:** `ghal_bol/src/p2p/chat_server/chat_room_session.rs`, `outbox_acks.rs` (`dispatch_read_ack_pass`, `seed_read_acks_for_peer_from_transcript`), `frames.rs`, `ghal_bol/src/dm_event_handler.rs` (`inbound_transcript_lookup_keys`, `apply_inbound_ack`). Tests: `apply_inbound_text_poll_replay_peer_id_bucket_no_double_unread`, `msg_v1::ack_received_includes_received_at_ms_in_signature`.

#### Fixed 2026-06-29 — single tick after peer read, transcript order, background outbox

**Symptoms (Android ↔ Linux desktop):**

| Symptom | What users saw |
|---------|----------------|
| **Single tick while peer “already read”** | Reader had the thread on screen (sometimes during Android **`inactive`**); sender stayed at **`delivered`** — no **`ack_read`** on wire |
| **Different last message** hub vs chat | Hub preview showed an older line; open chat sorted correctly by time |
| **Outbox stuck after reconnect** | Pending outbound not draining until hub room opened; **`chat_ready`** with **`outbox_pending>0`** but no send |

**Root causes (native):**

1. **Android `inactive`→`resumed` skipped read catch-up.** Shade/task switcher sets **`read=false`** while foreground pk stays set. On **`resumed`**, **`p2p_sync_ui_session`** turned the read gate on but **`p2p_set_foreground_peer`** returned **`unchanged`** → no **`SetForegroundPeer`** enter pass and no **`RunReadAckCatchup`**. Mail viewed while the gate was off never got **`ack_read`** sent to the sender.

2. **Transcript append order ≠ display order.** **`append_if_new`** appended at array tail while **`load_merged`** sorts by **`created_at_ms`**. Batches that crossed on the wire (local outbound after remote inbound with older timestamp) left disk order wrong and confused hub preview vs thread.

3. **Hub preview not transcript-authoritative.** **`record_inbound_preview`** on poll could run before a later local outbound line was persisted, leaving **`last_message_preview`** stale.

4. **Stream-open outbox ordering.** Periodic **`resync_pending_outbox`** running before the stream-open **burst** could mark rows **`on_wire`** within **`OUTBOX_RESEND_INTERVAL_MS`**, so the burst skipped them even when the peer never got the frame. **`chat_ready`** also ran before **`ensure_dm_peer_from_libp2p`** / transcript sync in some peer-key races.

**Fix (shipped — Rust only):**

| Area | Change |
|------|--------|
| Read catch-up on gate open | **`p2p_sync_ui_session`**: when **`ui_visible && room`**, always **`queue_read_ack_catchup`** after enabling read gate — even if foreground pk unchanged (`p2p_runtime.rs`) |
| Transcript order | **`insert_line_in_thread_order`** in **`append_if_new`** — disk order matches **`created_at_ms`** sort (`dm_transcript_store.rs`) |
| Hub preview | **`refresh_thread_preview_from_transcript`** on **`chat_ready`**; **`record_thread_message_preview`** on outbound send (`contacts_v1.rs`, `outbox_wire.rs`, `outbound.rs`) |
| Background outbox | **`chat_ready`**: **`ensure_dm_peer_from_libp2p`** + transcript sync before emit; **burst before** periodic resync; burst skips only **`on_wire`** rows inside resend interval; transcript replay when in-memory outbox still pending (`outbox_wire.rs`, `dm_dial.rs`, `session.rs`) |

**Regression — never reintroduce:**

- Relying on **`SetForegroundPeer`** alone for read catch-up when Android **`inactive`** can skip it.
- Treating “user saw the bubble” as **`ack_read`** without read gate + wire send.
- Appending transcript lines without chronological insert (breaks preview refresh and merged load assumptions).

**Code:** `p2p_runtime.rs` (`queue_read_catchup_for_room`), `dm_transcript_store.rs`, `contacts_v1.rs`, `outbox_wire.rs`, `dm_dial.rs`, `session.rs`. Tests: `append_if_new_inserts_by_created_at_ms_not_append_order`, `refresh_thread_preview_uses_latest_created_at_in_transcript`.

#### Boot auto-start and unlock notification

**Problem:** After device reboot, the `:p2p` service is dead — no messages are received until the user manually opens the app and enters their password.

**Solution:** A `BOOT_COMPLETED` receiver starts the service automatically when a keystore file exists (user has previously created or imported an identity). The daemon runs locked (cannot decrypt the secp256k1 key without the password), so it posts a high-priority notification prompting the user to open the app and unlock.

**Flow:**

1. **Device boots** → `BootReceiver.onReceive()` checks `hasKeystore()` (looks for `keystore_v1.json` under `NativeStorage.dataRoot`). If found, starts `GhalBolP2pService` via `startForegroundService`.
2. **Service starts** → `onStartCommand` runs normal setup (wake lock, multicast lock, daemon thread, connectivity callbacks), then calls `postUnlockNotificationIfNeeded()`.
3. **Unlock notification** → posted on `ghalbol_unlock` channel (`IMPORTANCE_HIGH` for heads-up) with `PendingIntent` to `MainActivity`. Title: "Ghal Bol", body: "Enter your password to start receiving messages". `setAutoCancel(true)`.
4. **User taps notification** (or opens app manually) → `MainActivity` starts → Flutter shows `IdentityScreen` → user enters password → FFI unlock + daemon unlock RPC → P2P starts.
5. **Notification dismissed** → `cancelUnlockNotification()` called from `chat_hub_screen.dart` on hub bootstrap, and from `GhalBolP2pService` on logout stop.

**Same logic for `START_STICKY`:** `postUnlockNotificationIfNeeded` runs in `onStartCommand`, so if Android restarts the service after a process kill (while the Flutter UI process is dead and `SessionCredentials` lost), the same notification flow applies.

**No keystore = no boot start:** If the user has never created an identity (fresh install), `BootReceiver` does nothing — no service start, no notification.

**Code:** `BootReceiver.kt`, `AndroidManifest.xml` (`RECEIVE_BOOT_COMPLETED` + `<receiver>`), `GhalBolP2pService.kt` (`postUnlockNotificationIfNeeded`, `cancelUnlockNotification`, `UNLOCK_CHANNEL_ID`), `MainActivity.kt` (`cancelUnlockNotification` method channel), `embedder_storage.dart` (`cancelUnlockNotification()`), `chat_hub_screen.dart` (hub bootstrap cancel).

#### Fixed 2026-07-05 — Android background readiness (screen off)

**Problem:** With the device locked and the screen off, inbound messaging could stop even though `:p2p` stayed alive as a **`remoteMessaging` foreground service** with **`WAKE_LOCK`** and multicast lock. This is **not** the read-receipt gate (`app_ui_visible` / `inactive`) — **`ack_received`** could still work for a short window after lock, then wire traffic and heartbeats went silent until the user turned the screen on again.

Root causes stack in layers:

| Layer | Effect |
|-------|--------|
| **Stock Android** | Doze / App Standby while not on the battery-optimization ignore list; Android 11+ **“Pause app activity if unused”** (`isAutoRevokeWhitelisted` false) |
| **OEM power managers** | Autostart / high-background-power restrictions on Xiaomi, Oppo/Realme, Vivo, Huawei/Honor, OnePlus, Asus, etc. — often **no manifest permission**; user must allow in manufacturer settings |
| **Aggressive OEM freezers** | Some devices (e.g. Vivo **`fast_freezer`**) freeze the **entire app UID** seconds after `SCREEN_OFF` (`am_app_frozen` in the **events** log buffer). FGS alone does not exempt the UID from that layer — user OEM settings + stock exemptions are still required |

A prior attempt (2026-07-03, reverted) added **`WifiLock`** and hibernation-only checks without a full sequential onboarding flow; it did **not** fix screen-off delivery in testing. **Do not reintroduce** that as the primary fix.

**Solution:** After unlock, **`ChatHubScreen._bootstrapHub`** runs **`AndroidBackgroundReadiness.runIfNeeded`** **before** `P2pEventBridge.ensureStarted`. Steps run **one at a time**, each skipped when already satisfied:

1. **Notifications** (Android 13+) — `POST_NOTIFICATIONS`; required for visible FGS notification
2. **Battery optimization** — manifest **`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`** + system **`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`** when `PowerManager.isIgnoringBatteryOptimizations` is false
3. **Unused-app pause** — when `PackageManager.isAutoRevokeWhitelisted` is false, open app settings to disable “Pause app activity if unused”
4. **OEM background** — when `BackgroundReadiness` detects a resolvable manufacturer autostart/background settings activity and autostart is not already verified (Vivo: content-provider query when available; others: one-time settings shortcut after user opens OEM screen)

**Prompt rules (regression guards):**

- **Do not prompt** if the step is already satisfied (re-check before each dialog)
- **Do not overlap** prompts — await dialog dismiss, `Permission.request()` return, or app **resume** after settings before the next step
- **Do not** request notification permission from **`ghal_bol_listener_foreground_io.dart`** — that raced hub onboarding

**Manifest (canonical):** `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_REMOTE_MESSAGING`, `WAKE_LOCK`, `POST_NOTIFICATIONS`, `RECEIVE_BOOT_COMPLETED`, `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`, plus call FGS types as needed.

**Code:** `android_background_readiness.dart`, `BackgroundReadiness.kt`, `MainActivity.kt` (embedder method channel), `embedder_storage.dart`, `AndroidManifest.xml`, `chat_hub_screen.dart` (`_bootstrapHub` order), `ghal_bol_listener_foreground_io.dart` (start FGS only — no permission prompt).

**Verified:** User-confirmed screen-off messaging after completing the prompted steps (2026-07-05).

#### Linux desktop — daemon auto-start and unlock notification

**Problem:** After a system reboot or fresh login, `ghal_bol_daemon` is not running. The user must manually open the app to start receiving messages.

**Solution:** XDG autostart entry + daemon-side unlock notification (same UX as the Android flow).

**Flow:**

1. **XDG autostart install:** On the first successful daemon unlock, Flutter writes `~/.config/autostart/com.ghalbol.daemon.desktop` with `Exec=<absolute_daemon_path>`. Re-runs on each unlock to update the path if the bundle moved. Removed on logout via `removeLinuxAutostart()`.

2. **Login → daemon starts** via XDG autostart. No keystore password available — session stays locked.

3. **Unlock wake (10 s grace):** The daemon spawns a thread that sleeps 10 seconds (override: `GHAL_BOL_UNLOCK_GRACE_SECS`). If `session_unlocked()` is still false **and** the UI is not already present (`ui_presence` runtime marker or an active UI RPC socket), it raises the desktop app (`gtk-launch` + D-Bus `Activate` + `unlock_wake` file polled by Flutter) and posts a `notify-rust` notification as fallback: "Ghal Bol — Enter your password to start receiving messages".

4. **User sees unlock screen** → enters password → daemon gets the `unlock` RPC → P2P starts (stale `unlock_wake` cleared on successful unlock).

5. **Grace period prevents false wakes:** Flutter touches `$XDG_RUNTIME_DIR/ghalbol/ui_present` as soon as the shell starts. During grace the daemon also records if the UI was ever present — if the user opened the app and closed it without unlocking, the end-of-grace auto-wake is **skipped** (they already engaged; not nagged again). If the app is still open at grace end, auto-wake is also skipped. **One auto-wake per daemon start** when the user never opened the app. Dismissing the fallback notification does **not** re-open the app; only tapping **Open** does. **Does not** call `GhalBolUiSession`, change read gates, or auto-unlock.

**Namespace detection (portable):** On unlock, Flutter writes `Environment=GHAL_BOL_APP_NAMESPACE=…` into the XDG autostart desktop entry. The daemon resolves the GTK application id via `GHAL_BOL_APP_NAMESPACE` when set, else release (`com.ghalbol`) or debug (`com.ghalbol.debug`) keystore on disk.

**Systemd alternative:** `scripts/ghal-bol-daemon.user.service` is still available for users who prefer `systemctl --user enable`. The daemon unlock wake works the same way regardless of how it was started.

**Code:** `linux_desktop_launch.rs`, `app_paths::detect_keystore_app_namespace`, `daemon/paths.rs` (`unlock_wake`), `ghal_bol_daemon.rs` (`spawn_unlock_reminder`), `p2p_event_bridge.dart` (`startLinuxWakePollIfNeeded`), `ghal_bol_daemon_client_io.dart` (`installLinuxAutostart`), `bootstrap_native.dart` (early wake poll).

### Regression symptoms (treat as bugs)

| Log / behaviour | Likely cause |
|-----------------|--------------|
| Same `ack_read` `ref=` many times per second | Read retry without confirm; burst rounds ≫ 1; upkeep ignoring `last_send_ms` |
| `poll drain saturated` + `totalEvents` ≫ message count | Every wire retry emitted to UI; `stores_updated` on no-op transcript patch; or **session-sync storm** (`chat room enter` / `sync_ui_session` every frame) flooding `NativeLog` poll events so delivery/read ack applies never reach Flutter |
| Burst `chat room enter` / `sync_ui_session` while room unchanged | Repeated `SetForegroundPeer` for same peer — starves poll + upkeep; sender sees no delivery/read ticks though recipient got text | Use `p2p_sync_ui_session` dedupe + `p2p_nudge_read_catchup` for Linux nudge; hub skips re-sync when `_lastSyncedForegroundPk` matches |
| `patch outbound delivery=read` in a tight loop on unlock | Draining stale poll queue; fix emit + apply gates |
| `Large outgoing transaction` / app dies copying logs | Log + poll storm; fix native volume first |
| Blue tick never appears | Confirm `ack_received` not sent or not applied; foreground/room gates wrong — not “send more ack_read” |
| Linux desktop: chat open, `ack_received` only, no `ack_read sent` | **Regression:** Linux **`inactive`** called `setVisible(false)` while room open (`read=false` in `ui_session_applied`). Fix: § “Fixed 2026-06-15 — Linux desktop read ticks”. **Do not** use forbidden `lastApplySucceeded` patch. |
| Android/iOS: chat dead, `stream_ready_count=0`, many `room closed` + `leave drain` at hub open | **Regression:** forbidden session-sync patch or hub foreground storm — check log for burst `sync_ui_session` / `set_foreground_peer (none)` before first `chat_ready`. |
| App “empty” after session churn, `conv=solo rows=0`, files still on disk under `ghal_bol/` | UI/session desync — not directory wipe. Do not create new identity; fix foreground sync. See forbidden patch table above. |
| Single tick while peer “read” it | Recipient never sent `ack_read` (room/gate closed on **their** device — common: Android **`inactive`** without resume catch-up). Check **reader** logs: `ack_read sent`, `read gate opened — catch-up ack_read`. Sender: `patch outbound delivery=read`. § “Fixed 2026-06-29”. |
| `inbound ack no matching row` / `has_out=false` on `ack_received` | Ack patch used single transcript bucket — fix merged keys (`inbound_transcript_lookup_keys`). § “Fixed 2026-06-19”. |
| Desktop `read_ack_sent=true` on mass inbound, peer no blue tick | Loose confirm loop (`has_seen_inbound_id` in `mark_read_ack_confirmed`) — § “Fixed 2026-06-19”. |
| `seeded N pending read ack(s)` with N ≫ room backlog (hundreds+) | Read-ack seed ignoring eligibility (`received_at_ms`, `read_ack_sent`, cutoff) — fix `dispatch_read_ack_pass`, not “seed all inbound”. |
| Android: messages stop ~minutes after lock; `:p2p` PID unchanged; events log `am_app_frozen` / `fast_freezer` | OEM UID freezer +/or battery optimization / unused-app pause / autostart — not Flutter poll or read gate. Hub must run **`AndroidBackgroundReadiness`** before P2P; user completes stock + OEM prompts. § “Fixed 2026-07-05”. |
| Overlapping Android permission dialogs at unlock | **`Permission.notification.request`** in listener foreground **and** hub readiness — regression. Listener starts FGS only; readiness owns all prompts sequentially. § “Fixed 2026-07-05”. |

**Do not fix floods by:** larger poll batches, Dart-side ack filtering alone, or “dedupe” without fixing confirm + retry cadence in Rust.

**Anti-patterns (do not reintroduce):**

- In-room path that sends **`ack_read` only** with **no** `ack_received` when `:p2p` can outlive the UI.
- Relying on Flutter poll to **send** acks (poll only applies events to stores).
- Clearing read-ack retry queues on leave.
- **`set_app_ack_read_enabled(false)` clearing `live_foreground_peer`** before `SetForegroundPeer(null)` (breaks leave backlog drain).
- Hub **disabling read gate before** `setForegroundConversation(null)` on room close.
- Showing ticks from Flutter logic without a transcript/poll patch (fake delivered/read).
- Loading chat with a **single** conversation key when history spans peer id + public key buckets.
- Setting `read_ack_sent` on enter without sender `ack_received` confirm.
- **`mark_read_ack_confirmed` on `has_seen_inbound_id`** — receiving inbound text must not count as having sent `ack_read`; confirm only when `has_pending_read_ack` (§ “Fixed 2026-06-19”).
- **Room-enter seed of all inbound rows** or seed without **`received_at_ms` + cutoff** — causes false read and ack/poll storms.
- Sender emitting `ack_request`.
- Mutual-QR / “both sides must scan” requirement.
- **High-volume `ack_read` retry** (burst rounds ≫ 1, upkeep every tick without `last_send_ms`, or 128×512 style bursts).
- **Emitting a poll/UI event for every wire ack retry** when transcript delivery / `read_ack_sent` is already at target.
- **`stores_updated` on no-op** transcript patches (forces hub/chat FFI reload storms).
- **Hub `previewChangeCount` → full `mergeTranscriptFromNative`** while the open chat already handles the same events in `ingestP2pEvent`.
- Treating duplicate read acks as expected — fix the confirm loop and retry cadence instead.
- **Linux desktop `inactive` → `setVisible(false)`** while chat room open — sets `:p2p` `read=false` without leave drain; ticks stall until resize/`resumed`. **Keep** `lifecycle inactive on Linux — read gate unchanged`. Android shade/task switcher still uses `inactive` gate-off.
- **WifiLock-only / hibernation-dialog-only screen-off fix (reverted 2026-07-03)** — did not restore screen-off delivery in testing; use sequential **`AndroidBackgroundReadiness`** instead. § “Fixed 2026-07-05”.
- **Overlapping Android background permission prompts** — listener foreground must not call `Permission.notification.request()`; hub **`AndroidBackgroundReadiness.runIfNeeded`** runs before P2P and owns all steps one at a time.
- **Assuming FGS + `WAKE_LOCK` exempt `:p2p` from all OEM freezers** — user must still complete battery optimization + OEM autostart settings when prompted.
- **Forbidden 2026-06-15 hub session patch** — `lastApplySucceeded`, `_invalidateNativeForegroundSync`, per-frame `!lastApplySucceeded` retry in `_syncNativeForegroundIfLayoutChanged`, hub `node_ready` + `_attachHubChat` session reapply. **Broke P2P messaging** and indirectly caused identity/data loss UX. § “FORBIDDEN — reverted 2026-06-15”. **Do not** substitute for the Linux `inactive` fix.

## UI session contract (integrator app ↔ native P2P)

**Scope:** `ghal_bol` (and `:p2p` / daemon) — **not** `ghal_bol_server`. The coord server never knows whether the UI is foreground; it only stores dialable endpoints. Any app integrating `ghal_bol` must drive this contract.

**Problem this solves:** Read receipts, foreground peer, and “app visible” were three separate RPC flags (`set_app_ack_read_enabled`, `set_foreground_peer`, `set_app_ui_visible`). They could drift (P2P recover re-enabled read without visibility; Android `inactive` left read on; defaults were `ack_read=true`). That caused **regressions** — wrong blue ticks, leave drain broken, or ack storms when one flag was fixed in isolation.

### Single native gate for **new** read receipts

Rust `may_send_in_room_read_ack(peer)` requires **all** of:

| Flag | Meaning |
|------|---------|
| `app_ui_visible` | App interactive (resumed; not inactive/paused/hidden) |
| `app_ack_read_enabled` | Read gate on (derived when room open + visible) |
| `foreground_peer == peer` | Hub has this conversation open |

**Leave backlog** (`pending_read_acks` retries after the user left the room) is **not** gated on `app_ui_visible` — eligible mail (`received_at_ms ≤` frozen **`chat_room_exit_at_ms`**) still gets `ack_read` until the sender confirms.

**Delivery** (`ack_received`) is **never** gated on UI — `:p2p` may outlive the UI process.

### Atomic integrator API

Flutter calls **`p2p_sync_ui_session`** (state RPC) with:

- `ui_visible` — lifecycle (resumed vs inactive/paused)
- `room_public_key_hex` — open conversation, or omit/`null` when no room

Native applies in one place (`p2p_sync_ui_session` in `p2p_runtime.rs`):

| Transition | Order |
|------------|--------|
| **Close room** | `SetForegroundPeer(null)` → leave drain → `app_ack_read_enabled=false` |
| **Open room** (visible) | `app_ack_read_enabled=true` → `SetForegroundPeer(peer)` if pk changed → **`RunReadAckCatchup`** always queued (`queue_read_catchup_for_room`) |
| **Inactive** (room still in UI stack) | `app_ack_read_enabled=false`; foreground pk unchanged — **no** new in-room `ack_read` |
| **Resumed** (same room) | `app_ack_read_enabled=true`; catch-up queued even when `SetForegroundPeer` is `unchanged` |

**Safe default:** `app_ack_read_enabled` starts **`false`** until the integrator syncs an open, visible room.

### Flutter ownership (`GhalBolUiSession` only)

**Integrator rule:** UI code must call **`GhalBolUiSession`** (`ghal_bol_ui_session.dart`) — not `GhalBolP2p.setForegroundPeer`, `setAppAckReadEnabled`, or `setAppUiVisible` (deprecated).

| API | When | Native (`p2p_sync_ui_session`) |
|-----|------|----------------------------------|
| `GhalBolUiSession.setVisible(true/false)` | `resumed` / `inactive` / `paused` | `app_ui_visible` + read gate (see lifecycle tables) |
| `GhalBolUiSession.setRoom(pk/null)` | Hub room open/close | foreground peer + leave drain on `null` |
| `GhalBolUiSession.awaitApplied()` | After close/open ordering | wait for state RPC |

`P2pEventBridge` coalesces desired state and issues one `syncUiSession` per change. **Poll events** (`peer_connected`, `chat_ready`, `node_ready`) are display hints only — never used for ack or send policy.

### Lifecycle (Android)

| State | Hub / bridge behaviour |
|-------|-------------------------|
| `resumed` | `setUiVisible(true)` + re-sync room if open |
| `inactive` | `setUiVisible(false)` — **no new read**; room desired state unchanged (shade / task switcher) |
| `paused` / `hidden` / `detached` | `setUiVisible(false)` + `setForegroundConversation(null)` — full close + leave drain |

### Lifecycle (Linux desktop)

| State | Hub / bridge behaviour |
|-------|-------------------------|
| `resumed` | `setUiVisible(true)` + `_syncNativeForegroundPeer()` if room open |
| **`inactive`** | **No session RPC** — read gate unchanged (window drag / focus flicker). **Never** `setVisible(false)` here. |
| `paused` / `hidden` | **No room close** (minimize ≠ leave) — read acks unchanged |
| GTK **close (X)** | `linuxWindowClosedByUser` → room close + leave drain via `notifyNativeUiExited` |

Android `paused`/`hidden`/`detached` clears room. Linux **`inactive` is not Android `inactive`** — do not unify without platform check.

### Anti-patterns (caused wrong read ticks)

- Split RPCs without `p2p_sync_ui_session` — flags drift after recover / node_ready.
- `app_ack_read_enabled` default **true** — read ticks before hub opens a room.
- Gating read on foreground only, ignoring `app_ui_visible` — Android **`inactive`** must gate read off; Linux **`inactive`** must **not** (see lifecycle tables).
- `set_app_ack_read_enabled(false)` clearing foreground before `SetForegroundPeer(null)` — breaks leave drain.
- P2P recover blindly re-enabling read when `_foregroundDesired` is set but app is backgrounded.

## Flutter: who sets foreground

`ChatHubScreen` owns room + lifecycle signals when it polls P2P (`hubPollsEvents: true` on embedded `ChatScreen`).

| Layout | Room “open” when |
|--------|------------------|
| **Narrow** (phone) | Chats tab + `_narrowShowRoom` + selected contact (`_selectedConversationKey`) |
| **Split** (desktop) | Chats tab + **`_selectedConversationKey`** (66-hex pk) — right pane shows [`ChatScreen`](../ghal_bol_ui/lib/chat_screen.dart) for that key. **Do not** gate on `_splitChatEngaged`; back/history can clear engaged while the thread stays visible (that caused recv-only + spurious leave drain). |

`ChatScreen` **must not** call `p2pSetForegroundPeer` when `hubPollsEvents` is true (IndexedStack keeps chat mounted off-room).

On **pause / background** (mobile), hub clears room via `setForegroundConversation(null)`. On **Android `inactive`**, only `setUiVisible(false)` — room stays desired until pause or explicit leave. **Linux `inactive`:** no visibility change.

Native applies foreground **synchronously** via `sync_foreground_peer_now` when FFI/RPC sets peer — inbound handler uses **`may_send_in_room_read_ack`**. **`last_room_peer`** is updated whenever foreground is set to a peer (for leave drain).

UI session RPCs use the **dedicated state socket** so they are not queued behind `send_text_dm` or bulk sync.

### Room open vs closed — decision table

| User situation | `app_ui_visible` | Read gate + foreground | Inbound **new** text | Backlog from while room was open |
|----------------|------------------|------------------------|----------------------|----------------------------------|
| In conversation UI, app resumed | `true` | gate on + foreground peer | `ack_received` + `ack_read` | Retries until confirm |
| Room open but `inactive` (shade) | `false` | gate off | `ack_received` only | Retries continue |
| Contact selected, list only / split not engaged | `true`/`false` | no room | `ack_received` only | — |
| Left room / paused | `false` | no room | `ack_received` only | **`ack_read` retries continue** |
| App background, `:p2p` alive | `false` | no room | `ack_received` only | Same backlog rule |

Selecting a row in the roster is **not** “room open”. Only the hub rules in the table above count.

## Transcript threads and conversation keys

`chat_transcript_v1.json` stores one array per **conversation key**. Contacts use **`libp2p_peer_id` when known**, else **`public_key_hex`**.

Historically, some threads were stored under the **public key** before PeerId was learned; newer lines may use **peer id**. That must not look like an empty chat or block ack patches.

| Operation | Key rule |
|-----------|----------|
| **Write** (append/save) | Canonical key = `SavedContact.conversationKey` (peer id preferred). |
| **Read** (chat UI, ack patch, seed on enter/leave) | **`load_merged`** expands to **peer id + public key** for the same contact (`expand_conversation_keys` in Rust; `allConversationKeys` in Flutter). Poll replay and inbound ack apply use **`inbound_transcript_lookup_keys`** (`dm_event_handler.rs`) — same rule, wire `from` peer id included. |
| **Patch delivery / read_ack_sent** | Try all expanded keys so old rows under `public_key_hex` still get ticks and confirms. **`apply_inbound_ack`** must use merged keys — single-bucket patch caused stuck single-tick (§ “Fixed 2026-06-19”). |

**Symptom if broken:** hub preview shows `last_message_preview` (contacts) but the chat pane is empty, or outbound ticks never update for old messages — usually a **key mismatch**, not missing P2P.

**Rule:** never show transcript lines in the UI without loading through merged keys for the active contact.

### Hub chat — stable thread id (`hubThreadKey`) — regression guard

On daemon platforms the hub mounts one [`ChatScreen`](../ghal_bol_ui/lib/chat_screen.dart) per selected contact (`ValueKey("hub-chat-<pk>")`). **Which transcript bucket to load is not the same as the roster row object.**

| Role | Source | Stable across roster reload? |
|------|--------|------------------------------|
| **Thread id** (load/save key, send target, `didUpdateWidget` room switch) | `hubThreadKey` from hub `_selectedConversationKey` (66-hex `public_key_hex`) | **Yes** |
| **Roster metadata** (alias, trust banner, preview, `is_known`) | `activeContact` (`SavedContact` from `ContactStore`) | **No** — row can be **null for a frame** after send, poll, or `ContactStore` reload |

**Regression (2026-06):** tying thread identity to `activeContact?.conversationKey` caused cross-room history loss. After send or poll, roster reload often rebuilt the hub with `activeContact == null` while the user was still in the same room. `didUpdateWidget` treated that as `conversationKey` changing `A → ""`, cleared lines, reloaded with `conv=solo`, and opening another chat showed empty or wrong history. **Disk was fine** — Flutter dropped or painted the wrong thread.

**Do not reintroduce:**

- Room-switch detection from `activeContact?.conversationKey` alone when `hubPollsEvents` is true.
- Transcript load/send keyed only on `activeContact` when the hub already knows `_selectedConversationKey`.
- Extra reload/clear hacks instead of a stable hub thread key.

**Required contract:**

1. [`chat_hub_screen.dart`](../ghal_bol_ui/lib/chat_hub_screen.dart) passes `hubThreadKey: _selectedConversationKey` into `ChatScreen`.
2. [`chat_screen.dart`](../ghal_bol_ui/lib/chat_screen.dart) uses `hubThreadKey` for `_conversationKey()`, `_conversationKeysForLoad()`, `_recipientPublicKeyHex()`, and `_threadKeyForWidget()` in `didUpdateWidget` (not `activeContact` alone).
3. `initState` loads transcript when `hubThreadKey` is set even if `activeContact` is briefly null.

**Symptom if broken:** `transcript reload skipped conv=solo` or `conv=solo rows=0` while a room is open; sending in chat A empties chat B on next open; log shows `Contacts list` then missing `transcript reload conv=<pk> rows=N` for the selected peer.

## P2P lifecycle

### Stream-first symmetric connect (wire layer)

**Ideas reference only:** older [protonet](https://github.com/mearaj/protonet) (`protonet-as-reference`, 4+ years) had one `chatStreams` entry per contact and 1 Hz upkeep — useful as a **historical sketch**, not a target to mirror. Ghal Bol’s connect layer is **stream-first on the wire**: one live DM mux per contact, upkeep noop while the writer is up, coord + relay + mDNS for discovery (not Kademlia), and no `disconnect_peer` while a route may still work.

The connect layer that made the original serverless libp2p build reliable: **both peers connected within a few seconds** with no coord server — only DHT bootstrap and the rules below. Ghal Bol **must keep this wire shape** in `:p2p` / daemon. WAN adds coord + relay for **discovery**; LAN adds mDNS — **both run in parallel** ([TRANSPORT.md](TRANSPORT.md) § “Parallel LAN + WAN transport”).

| Rule | Meaning |
|------|---------|
| **Both listen** | Each node accepts inbound libp2p connections and inbound DM streams (`/ghal-bol/msg/1.0.0`). |
| **One stream per contact** | At most one live DM stream per contact, keyed by `public_key_hex` / derived PeerId (`dm_peer_stream_up`). While the stream writer is live, `dm_upkeep` **does nothing** for that contact — no coord lookup, no `disconnect_peer`, no identify dials. **Dual links:** when both LAN and WAN libp2p connections exist, **both stay up**; upkeep noop for the mux does **not** mean WAN is idle or should be closed. Missing relay hop on Wi‑Fi → **`needs_additive_relay_dial`** (background additive relay) — **not** declaring the mux unstable every tick ([TRANSPORT.md](TRANSPORT.md) § Post-mortem 2026-06-24). If both sides open outbound before either accept wins the writer slot, extra inbound streams are **read-only** (process DM frames; do not drain) until the mux closes. |
| **Symmetric roles** | No permanent listener/caller. Either peer may accept inbound or open outbound; same stream handler on both paths. |
| **Send = connect** | Outbound text uses: no stream → ensure libp2p connection → open one stream → write. UI never dials or owns connect policy. |
| **Parallel transport** | LAN (mDNS) and WAN (coord + relay) stacks and per-peer dials run **concurrently** — both links **active** when connected; see TRANSPORT.md § “Parallel LAN + WAN transport”. |
| **Single upkeep owner** | `dm_upkeep` (~1s) walks contacts: **if stream up → skip**; else missing stream → connect attempts on any available path; pending outbox drains when `chat_ready`. **"Single owner" = one owner of the stream/connection lifecycle, not one function that issues every dial.** Each transport's **dial is event-driven by that transport's own discovery**: **LAN dials are owned by the mDNS `Discovered` handler** (never timer re-dials from the candidate cache — TRANSPORT.md § “Ephemeral LAN TCP ports”), and **WAN dials are owned by coord lookup** (in the upkeep tick / `notify_coord_lookup`-woken). The **identify** handler must **not** add a third competing `swarm.dial` when coord is configured — it only ingests addrs and signals the owners. The shared per-peer in-flight guards (`try_claim_lan_dial_slot`, `should_routed_dial`, `circuit_dial_in_flight`) are **required guardrails** (happy-eyeballs, CGNAT probe, parallel LAN+WAN), not workarounds to remove. |

**PeerId** is derived from secp256k1 `public_key_hex` (already). **Discovery** is coord HTTP + relay (WAN) and mDNS (LAN) — parallel layers below the single wire mux.

**Target latency:** when a route exists, `peer_connected` → `chat_ready` within **seconds**, as in the original build — not minutes of blocked paths.

**Violations (regressions):** tearing down relay when direct LAN connects; separate LAN/WAN message stores; downgrading `read` → `delivered` on duplicate acks; Flutter dial or transcript policy; uncoordinated dial spam (many dials/s per peer); **`dm_peer_chat_link_stable=false` solely because relay hop missing**; clearing **`circuit_dial_in_flight`** on urgent reconnect; blocking coord HTTP on tokio swarm loop; **skipping coord lookup for foreground/outbox peer during LAN handover** ([TRANSPORT.md](TRANSPORT.md) § **Post-mortem 2026-06-25**). See [TRANSPORT.md](TRANSPORT.md) § “Parallel LAN + WAN transport”, § **Post-mortem 2026-06-24**, § **Post-mortem 2026-06-25**.

### Unified message state (E) — single source of truth (2026-06-17)

**Problem:** If LAN and WAN were separate application paths with separate stores, duplicate frames during handover could race (e.g. WAN `ack_received` then LAN `ack_read` for the same message, or two transcript rows for one send).

**Two layers — do not conflate:**

| Layer | LAN | WAN | Interference rule |
|-------|-----|-----|-------------------|
| **Transport (libp2p)** | mDNS + direct TCP | coord + relay circuit | **Parallel** — both may connect and stay active; throttles only; never tear down relay when direct LAN appears ([TRANSPORT.md](TRANSPORT.md) § Parallel LAN + WAN) |
| **Application (E)** | Same handler for every frame | Same handler for every frame | **One store** — path-agnostic merge; never separate “LAN transcript” vs “WAN transcript” |

```text
Peer A                          Peer B
  │                               │
  ├─ LAN link ──► /ghal-bol/msg ──┤
  │                               │
  └─ WAN link ──► /ghal-bol/msg ──┘
                      │
                      ▼
              dm_event_handler + dm_transcript_store (E)
              chat_transcript_v1.json — one row per message_id
```

**Model:** All message and ack state for a contact lives in **one Rust store (E)** — keyed by `conversation_key` / `message_id`, never by “which link delivered this frame”.

| Concern | Owner | Store / API |
|---------|--------|-------------|
| Outbound queue | Rust `:p2p` | in-memory outbox + `dm_transcript_v1` pending rows |
| Transcript lines | Rust **E** | `dm_transcript_store.rs` → `chat_transcript_v1.json` |
| Inbound text | Rust **E** | `append_if_new` — dedupe by `message_id` |
| Outbound delivery ticks | Rust **E** | `patch_outgoing_delivery` — **monotonic** rank: `sent` < `delivered` < `read` |
| Inbound read-ack sent flag | Rust **E** | `patch_inbound_read_ack_sent_for_thread` |
| Ack send/retry | Rust | `chat_server.rs` — not Flutter |
| Wire mux | Rust | **One** live `/ghal-bol/msg/1.0.0` stream per contact when possible; frames from either link feed the same handler |

**Merge rules (invariant):**

- Duplicate inbound text (same `message_id` from LAN or WAN wire): **one row** (`append_if_new`).
- Duplicate outbound acks for same `ref_id`: apply **higher** delivery rank only — **`read` always wins over `delivered`**; never downgrade. If E already has `read` from LAN and a late WAN `ack_received` arrives, E keeps `read` (`patch_outgoing_delivery` rank check).
- **`ack_read` implies delivered** — product semantics: read tick on sender side only after recipient read; inbound `ack_received` after `ack_read` for the same id is a no-op rank-wise.
- Flutter **reloads** native state on poll — no parallel Dart transcript writer, no path-specific caches.

**Wrong (forbidden):** LAN path writes store C, WAN path writes store D; Flutter merges C+D; or clearing WAN link when LAN connects and losing in-flight acks on the relay mux without E having persisted state.

**Supersedes:** any implication that LAN and WAN could own separate “C” and “D” stores per path.

1. **Unlock** — UI: FFI `createOrUnlockIdentity`; daemon: `unlock` with the same namespace and password (must match public key). Both call `set_p2p_handler_context(app_namespace)`.
2. **`p2p_start`** — `dm_peers: [{ "public_key_hex": "…" }]`, `bootstrap_peers: []`, `app_namespace`. If the node is **already running**, native still refreshes handler context and re-registers all `dm_peers` from config (daemon may survive UI restarts).
3. **Contact added** (scan) → `sync_contacts` **hot-registers** keys on the **running** node — **no full `p2p_stop` / restart** for roster changes.
4. **Route** → coord lookup + relay (WAN) and mDNS (LAN) run **in parallel** — see [TRANSPORT.md](TRANSPORT.md) § “Parallel LAN + WAN transport”.
5. **Connect** → libp2p `ConnectionEstablished` toward derived PeerId (guest from stored `public_key_hex`; host may learn peer on first `peer_identified` or inbound text).
6. **Stream** → open `/ghal-bol/msg/1.0.0` if none live (inbound accept or outbound open — same handler).
7. **`chat_ready`** → outbound stream writer up; safe to send frames; outbox drains without opening a hub room.
8. **Poll** → `p2p_poll` → JSON events; `dm_event_handler` updates on-disk stores; Flutter reloads roster/transcript via FFI.

**Host after scan (asymmetric):** scanner’s roster updates immediately from QR. Host may show **zero** contacts until `peer_identified` or first inbound `dm_message` (text) creates/updates the row — poll must bump **roster** on those `stores_updated` events, not only preview.

### Event chain for delivery (debugging)

```text
register_dm_peer → lookup+dial → peer_connected → chat_ready → outbound_sent
                                                      ↘ dm_message (inbound text)
```

If sends stay `queued` / `not connected yet`, the break is in the **native chain** (dm_peer registered, dial, stream, handler context) — not missing Flutter ack logic.

### Background listener (`:p2p` / daemon)

**Android:** `GhalBolP2pService` in process **`:p2p`** (foreground + multicast lock). JSON-RPC on `filesDir/.../ghalbol/p2p.sock`. Same `configure_android_data_directory` path as Flutter (`getApplicationDocumentsDirectory`).

**Android screen off:** FGS + wake lock are necessary but not always sufficient. Hub unlock runs **`AndroidBackgroundReadiness`** (battery optimization, unused-app pause, OEM autostart) **before** P2P start — see § “Fixed 2026-07-05 — Android background readiness”.

**Linux desktop:** **`ghal_bol_daemon`** under `libexec/`. Socket: `$XDG_RUNTIME_DIR/ghalbol/p2p.sock` (or `GHAL_BOL_DAEMON_SOCKET`).

**Both:**

- libp2p, outbox, and **all ack send/retry** run here — **not** in the Flutter isolate.
- Flutter **poll** refreshes UI from disk after `dm_event_handler` runs on each `p2p_poll`.
- **UI lock** (`bootstrap_native.dart` `_uiLocked`): hides hub UI only — **does not** `p2p_stop`, stop `:p2p` / daemon, or stop poll. **Logout / delete identity** stops P2P.
- Hub pause / leave room: clear **conversation** foreground (`setForegroundConversation`) for read-ack policy — not the Android foreground service.
- Logout / delete identity: `p2p_stop` + lock.

**Do not** run `scripts/sync_ghal_bol_native_for_flutter.sh` while the Linux app holds an open daemon socket (stops `ghal_bol_daemon` → `Broken pipe`). Android native rebuild uses `pack_android_workspace_jni_libs.sh` only.

### First identity create → P2P bootstrap (code path only)

After **Create identity** succeeds, `onUnlockedSession` runs `GhalBolBackground.ensureRunning` (same callback as unlock of an existing keystore). That starts the poll loop and async `syncContacts` → `p2p_start` in `:p2p` / daemon. It does **not** wait for opening a chat.

Coord HTTP register in Rust waits until listen addrs are publishable — see `coord_runtime.rs`. **Hybrid model:** clients POST public/LAN `tcp` only; relay server owns `/p2p-circuit` presence on reservation; CGNAT clients poll coord for self-circuit when register payload is empty (`promote_relay_presence_if_visible`). WAN relay recovery runs when coord URL is configured (`chat_server.rs` coord tick). Full detail: [TRANSPORT.md](TRANSPORT.md) § “Hybrid coord presence”, § “LAN ↔ WAN handover”.

### Dial strategy — parallel LAN + WAN (native — `chat_server.rs`, not Flutter)

| LAN | WAN | Together |
|-----|-----|----------|
| mDNS `Discovered` → direct TCP for that contact | coord lookup → relay circuit + public TCP when registered | **Both stacks always on** on Wi‑Fi; per-peer both links may be connected |

Rules:

1. **Always run WAN/coord** while configured and network is up — even when mDNS shows LAN peers.
2. **LAN path is opt-in by discovery** — mDNS sighting for that contact, not global LAN-first for all roster peers.
3. **Mobile-data / CGNAT** (no active Wi‑Fi LAN): coord relay dials only (throttled). Probe-style `listen_on(…/p2p-circuit)` when bootstrap TCP down — [TRANSPORT.md](TRANSPORT.md) § “CGNAT / mobile-data relay reservation”.
4. **Wi‑Fi with LAN:** mDNS and coord/relay dial **in parallel** for the same peer when stream is down — throttled, not mutually deferred. **Do not close relay links** when direct LAN connects; **both links stay active** (LAN + WAN each doing their job). See TRANSPORT.md § “Both links active”.
5. **Outbound dial to peer’s relay circuit** after coord lookup: proceed when lookup succeeds — **do not** wait for own `reservation accepted`. Throttle: `should_routed_dial` in `dial_dm_peer_addr`.
6. **Wi‑Fi toggle on LAN** — event-driven ([TRANSPORT.md](TRANSPORT.md) § “LAN stability — cold start and Wi‑Fi toggle”): `kick_lan_dm_rediscovery_after_handover` + WAN recovery in parallel. LAN dials remain **mDNS `Discovered` only** — never upkeep re-dial from cache.

**Event-driven rule (general):** avoid assumed timers whenever policy waits on async work with unknown duration — worker owns the work, subscribers react on reported facts. See TRANSPORT.md § “Event-driven async — avoid assumed timers”.

**Anti-patterns:** separate LAN/WAN transcript or outbox stores; downgrading `read` → `delivered`; **closing relay when direct LAN connects**; **blocking LAN upkeep during WAN recovery**; Dart dial policy; **`dm_upkeep` re-dialing stale LAN TCP ports**; port-ranking heuristics; uncoordinated coord-relay dial spam on CGNAT; blocking peer relay dials until own circuit listens; coord lookup or mDNS **disk** caches; timer-based grace windows — see TRANSPORT.md § “Parallel LAN + WAN transport”, § “Ephemeral LAN TCP ports”.

**Network watch:** Android connectivity + profile poll → WAN relay recovery when coord URL is set. UI lock does not stop `:p2p` / daemon / poll.

### Steady connection (both peers online)

When two contacts are both online the link must stay **steady** and recover instantly from blips — never an idle drop or a backoff-delayed reconnect that the user perceives as lag. Native (`chat_server.rs`) enforces this; see [TRANSPORT.md](TRANSPORT.md) § “Steady connection”:

- **Keepalive ping** keeps an idle DM/relay connection alive (ping interval **15s** < `idle_connection_timeout` 45s/300s), so the next message reuses the live link instead of paying a reconnect.
- **Urgent reconnect** after `dm connection closed`: the peer’s key is urgent for ~30s — coord lookup **skips** the `peer_not_on_server` backoff and the 1s upkeep tick retries immediately (`mark_dm_reconnect_urgent` / `is_pk_reconnect_urgent`), cleared on **`chat_ready`** (live mux — not bare libp2p `ConnectionEstablished`; `conn=true` alone is not reconnect per TRANSPORT.md § Post-mortem 2026-06-24).
- **Reserve on all configured coord relays in parallel, throttled per relay** (`try_relay_reservations` / `try_relay_reservation`, per-relay `RELAY_RESERVE_THROTTLE_MS`). Do **not** use public IPFS bootstrap peers for relay reservation or WAN peer discovery ([TRANSPORT.md](TRANSPORT.md) § Connectivity lifecycle).
- **CGNAT/mobile:** throttle bootstrap relay dials (`issue_bootstrap_dials`); probe-style `listen_on` at startup and when bootstrap TCP is not up yet — § “CGNAT / mobile-data relay reservation” in TRANSPORT.md. **Both peers** must `reservation accepted` + coord register for **bidirectional** coord visibility; one-sided success still means no chat. **Outbound** dial to a peer’s relay circuit after coord lookup does **not** wait for own reservation — see TRANSPORT.md § “Outbound peer relay dials vs own reservation”.

## Call UI lifecycle and privacy (do not regress)

**Product invariant:** there must **never** be an active voice/video call (native media up, peer still in-call) when the user has **no UI session** to see or control it. This is a privacy and safety requirement — fixes have regressed when other call features landed; treat any orphan call as **P0**.

**Signaling wire discipline (native — do not fake in Flutter):**

- Outbound **ringing** only after `invite` is on the DM stream (`call_signal_sent`), not when FFI returns.
- Drop queued/stale `invite` frames when the call ended or age > 45s — no ghost rings minutes later.
- Caller shows **Answered** as soon as `accept` arrives; media connect is a separate phase.
- Hangup/reject purges pending signals for that `call_id` on both native queue and poll buffer.
- **Android:** `:p2p` posts the full-screen incoming-call notification when an `invite` arrives on the wire (UI process may be killed). Tapping it must restore ringing UI via `p2p_call_status` + poll.

### Process split (same on Linux and Android)

| Layer | Linux desktop | Android |
|-------|---------------|---------|
| **UI** | Flutter main process | Flutter main process |
| **P2P + calls** | `ghal_bol_daemon` (`libexec/`) | `:p2p` (`GhalBolP2pService`) |
| **Survives UI exit?** | **Yes** — daemon keeps libp2p for DM/acks | **Yes** — `:p2p` keeps libp2p for DM/acks |
| **Must end call when UI gone?** | **Yes** | **Yes** |

**Dev confusion:** `flutter run` **Ctrl+C** kills the **Flutter UI only**; the daemon / `:p2p` **stay running** (by design — DM and acks continue). That is **not** permission to keep a call alive: when the last UI RPC socket closes, native **`p2p_force_end_active_call`** must stop media and send **`hangup`**.

### Teardown paths (all required)

| User action | Expected behaviour |
|-------------|-------------------|
| **Linux GTK close (X)** during call | Flutter `onWindowClosedByUser` → `_endLocal` + `_stopNativeCallIfStillActive`; hide window (app may stay in tray) |
| **Leave call screen** (back / pop) while connected | `onCallScreenDismissedWhileLive` → hang up + stop media + **`CallVideoTexturePool.releaseCall`** |
| **Hangup / remote hangup / call end** | `_endLocal`: UI phase → `ended` immediately; **`callMediaStop` / `callVideoStop`** + **`CallVideoTexturePool.releaseCall`** (async ok); **`CallDesktopNativeCamera.stop`**; then `hangup` signal. **Do not** block UI teardown on RPC. |
| **Ctrl+C / UI process kill** | UI sockets EOF → daemon `:p2p` **`ui_session_ended`** → `p2p_force_end_active_call` |
| **`AppLifecycleState.detached`** (best-effort) | Flutter `notifyUiProcessExiting` → same force-end |
| **Login unlock socket reconnect** | `ui_session_prepare_reconnect` suppresses hangup for ~5s (transient EOF only) |
| **Logout / identity delete** | `p2p_stop` clears call state |

**Linux native video teardown (2026-06-15 — do not regress):** GPU textures are released on **call end** via `CallVideoTexturePool.releaseCall(call_id)` after `callVideoStop`, **not** on `NativeCallVideoView.dispose` / PiP widget rebuild (`releaseWidget` is intentionally a no-op — releasing on dispose caused Flutter **SIGSEGV** on Linux during in-call texture updates). Ending a video call must stop capture, stop native video session, release textures, then dismiss call UI — never leave textures registered while tearing down the call screen.

Native implementation: `daemon/ui_session.rs` (socket counting), `p2p_runtime::p2p_force_end_active_call` (media stop + hangup + `call_active` / `call_state` clear + dismiss OS notification), `call_controller.dart` `_endLocal` / `_stopNativeCallIfStillActive`.

### Incoming call while UI hidden (Linux)

- OS notification is shown by **daemon** (`incoming_call_notify.rs`), not only GTK in Flutter.
- Tap must **present** the window: D-Bus `Application.Activate` + **`incoming_call_wake`** file under `$XDG_RUNTIME_DIR/ghalbol/`.
- Flutter poll bridge **consumes** the wake file (`p2p_take_incoming_call_wake`) and calls `CallIncomingAlert.presentWindow()` + `CallController.onAppForeground()`.

### Regression checklist (manual — two devices)

1. Connected **video** call → Linux **X** → peer must see hangup within seconds; no camera/mic on either side; **no Flutter SIGSEGV** (textures released on end, not on widget dispose).
2. Connected **video** call → **Hang up** button → same; call UI closes cleanly on both sides.
3. Same with **`flutter run` Ctrl+C** during call (UI only) — daemon must hang up; Android peer must not stay in-call.
4. Pop call screen during connected call → hangup on both sides.
5. Incoming call with app hidden → notification tap → call UI visible and answerable.
6. Login unlock (UI lock, not logout) during call → call **continues** (suppress window applies).
7. Linux desktop chat open → drag window / brief focus loss → send inbound text → **`ack_read sent`** without resize (`inactive` must **not** log `read=false` with room still set). Soak **>10 min** LAN Android↔Linux: **`conn=true,stream=true`**, ticks stable. **Do not** fix regressions with forbidden `lastApplySucceeded` patch (§ “FORBIDDEN — reverted 2026-06-15”).
8. **Android screen off:** unlock → complete **`AndroidBackgroundReadiness`** prompts if shown → lock device → peer sends text → **`ack_received`** / delivery tick within normal WAN/LAN latency (not only while screen on). If dead: check battery optimization, unused-app pause, OEM autostart; events buffer may show `am_app_frozen`. § “Fixed 2026-07-05”.

Wire detail: [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) § “UI session and privacy”.

## Contacts and roster preview

On inbound `dm_message` (text), native `dm_event_handler`:

- Always updates **last message preview** on the contact (either direction).
- Increments **unread** once per inbound text when the message is **not** from the foreground peer **while the read gate is open** (visible + `ack_read` enabled + room open — same predicate as in-room read receipts). Background/inactive with room still on the UI stack must still bump unread. Clears **unread** when the hub opens that peer’s room (`clear_unread` on foreground). Poll replay of the same `message_id` does not bump again. **Poll still emits `stores_updated` on inbound text** so Flutter reloads the roster after wire persist (apply may be a no-op replay).
- Contact trust fields (`is_known`, `is_blocked`) follow [Contact trust](#contact-trust-is_known--is_blocked) — preview and unread behavior for unknown peers is unchanged unless the peer is **blocked**.

## Contact trust (`is_known` / `is_blocked`)

### Intent

After an asymmetric connect (one side scanned, the other did not), the **host** may see a new roster row from **first inbound text** before choosing to trust that peer. Contact trust is how the **local** UI expresses that choice:

- **`is_known`** — user accepts this peer on **this** device.
- **`is_blocked`** — user blocks this peer on **this** device.

Trust is **per device** (`contacts_v1.json` only). It is **not** on the wire, not synced between phones, and not part of DM frames.

**Hard requirement:** This feature is **additive**. It must **never** change existing messaging behavior: P2P registration, outbox, `ack_received` / `ack_read`, hub foreground open/close order, transcript keys, poll, delivery/read ticks, or invite scan flow for peers the user already added.

**No legacy / backward compatibility:** Do **not** keep parallel block lists, infer trust from “we chatted before”, or migrate old preference keys. Block state lives **only** on the contact row as `is_blocked`. Every persisted contact includes both booleans explicitly.

### Stored fields (`contacts_v1.json`)

Each contact row has two required booleans:

| Field | Type | Meaning |
|-------|------|---------|
| **`is_known`** | `bool` | `true` when the user accepted this peer on this device; `false` when the peer is **unknown** in the UI. |
| **`is_blocked`** | `bool` | `true` when the user blocked this peer on this device. |
| **`chat_room_exit_at_ms`** | `i64` (optional) | **Rust-owned.** Last active in-room moment with this peer; mirrors live session clock while foreground; **frozen** on leave/switch/inactive. Read-ack cutoff — see § “Inbound `received_at_ms` and read-ack eligibility”. |

**Initial values when the row is created:**

| How the row appears | `is_known` | `is_blocked` |
|---------------------|------------|--------------|
| User **scans** peer A’s invite (peer B scanned A) or otherwise saves a contact they chose | `true` | `false` |
| Row appears because **first inbound text** arrived and this key was not already in the roster (typical **peer A** after B scanned and messaged) | `false` | `false` |

**User actions (persist immediately):**

| Action | `is_known` after | `is_blocked` after | UI after |
|--------|------------------|--------------------|----------|
| Tap **Add** on room banner | `true` | `false` | Banner gone; hub **Unknown** control gone |
| Tap **Block** on room banner | (unchanged) | `true` | Banner gone; block UX (no normal chat) |
| Send **any** outbound text in that room (including the first character) | `true` | `false` | Same as **Add** — banner gone; **Unknown** control gone |

Do not infer `is_known` from preview text, unread, or transcript length — **only** these fields and the rules above.

### Canonical flow (peer B scans peer A, then messages)

Naming matches [Asymmetric “who knows whom”](#asymmetric-who-knows-whom): **B** = guest (scanner), **A** = host (QR shown).

1. **B** scans **A**’s QR → **B**’s roster gets **A** with `is_known: true`, `is_blocked: false` (B chose to add A).
2. **B** sends a message to **A**.
3. **A** receives it → native creates/updates a roster row for **B** with `is_known: false`, `is_blocked: false` (A did not scan B).
4. On **A**’s **hub** contact list, the row for **B** shows a **highlighted button-like widget** on the **right** side of the item, labeled **`Unknown`**. Show it only while `is_known == false` **and** `is_blocked == false`.
5. **A** taps the row and enters the chat **room** → a **banner** at the **top** of the room with two option buttons: **Add** and **Block**.
6. **Add** → `is_known: true`; persist; remove banner and hub **Unknown** control.
7. **Block** → `is_blocked: true`; persist; remove banner; apply block UX.
8. If **A** sends even a **single** outbound message before tapping Add → same as step 6 (`is_known: true`); banner and **Unknown** control must not remain.

Peers who were already `is_known: true` (everyone added via scan on this device) must see **no** new banner or **Unknown** control.

### UI specification

| Surface | When visible | What to show |
|---------|----------------|--------------|
| Hub roster item (right side) | `!is_known && !is_blocked` | Highlighted button-like control, text **`Unknown`** |
| Chat room (top) | Room open for that peer and `!is_known && !is_blocked` | Banner with **`Add`** and **`Block`** buttons |
| Hub + room | `is_known == true` | No **Unknown** control; no trust banner |
| Hub + room | `is_blocked == true` | No trust banner; blocked interaction rules apply |

Preview, unread, and opening the room for an unknown peer behave as today except for this added chrome and block policy.

### Layer ownership (implementation later)

| Concern | Owner |
|---------|--------|
| Persist `is_known` / `is_blocked`; set defaults on create/upsert | **Rust** — `contacts_v1.rs`, `dm_event_handler` on first inbound text |
| Hub **Unknown** control | **Flutter** — `chat_hub_screen.dart` (read contacts via FFI) |
| Room **Add** / **Block** banner; dismiss rules | **Flutter** — `chat_screen.dart` |
| First outbound send ⇒ `is_known: true` | **Flutter** triggers native update before/at send; **Rust** stores |
| Block enforcement (ignore new inbound from blocked peer, etc.) | **Rust** preferred; Flutter must not treat blocked peers as normal chat |

Use the existing contacts reload path (`contacts_list` / roster bump after poll). **Do not** add a second contact or block store in Dart.

### Invariants (do not break)

- **`is_known: false`** does **not** stop receiving messages, writing transcript, or sending **`ack_received`** — it only drives trust UI until Add, Block, or first outbound send.
- **`is_blocked: true`** gates product interaction (e.g. hide/normal chat); must not corrupt transcript or ack state for data already accepted.
- Trust UI must not alter hub **foreground** order (`setForegroundConversation` / `setAppAckReadEnabled`), leave backlog, or read-ack confirm loop documented above.
- Removing separate “blocked peer id” preferences is intentional — **`is_blocked` on the contact is the only block flag**.

## Connect invite (summary)

- Single wire format: **`format_version`: 2**, `public_key_hex` only.
- **One codec** in Rust and Dart (`invite_uri_codec.dart` / `connect_invite_v1.rs`) — share and scan must use the same bytes.
- Verify in native; Dart fallback parse only when needed for dev builds.

Details: [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md).

## Persistence

| File | Contents |
|------|----------|
| `contacts_v1.json` | Roster, alias, preview, unread, **`is_known`**, **`is_blocked`**, **`chat_room_exit_at_ms`** (read-ack cutoff; Rust-owned) |
| `chat_transcript_v1.json` | Per-conversation lines; outbound `delivery` + `received_at_ms`; inbound `read_ack_sent` + `received_at_ms` |
| Keystore | Encrypted identity under app namespace |

Transcript survives restart; native re-seeds outbox and read-ack queues from disk.

### On-disk store ownership (single writer — do not overlap)

**`ghal_bol` owns product state.** Flutter is a **read-mostly view** plus explicit user edits (alias, trust, composer). Two processes must not perform competing read-modify-write on the same JSON file.

| Store | Android / Linux (`:p2p` / daemon) | In-process FFI (dev / tests) |
|-------|-----------------------------------|------------------------------|
| **`chat_transcript_v1.json`** | **`:p2p` only** writes (poll `dm_event_handler`, `send_text_dm` append, ack patches, read-ack confirm). UI loads via **`p2p_transcript_load_merged`** (state RPC) — never `transcript_save` / `append` from Dart. | UI may append/save via FFI when no daemon; still use `dm_transcript_store` in Rust. |
| **`contacts_v1.json`** | **`:p2p`** writes inbound preview/unread/merge on poll. UI writes **user intent** only (alias, trust, manual add/remove) via FFI. | Same split. |

**Rust rule:** every transcript read/write goes through **`dm_transcript_store`** (in-process mutex + `chat_transcript_v1.json.lock` flock). Do **not** open `chat_transcript_v1.json` from `dm_transcript_v1` or `chat_server` without that lock — concurrent append + `read_ack_sent` patch used to **clobber** rows (messages vanished on disk).

**Flutter rules (daemon platforms):**

| Do | Do not |
|----|--------|
| `syncTranscriptView` / `transcriptLoadThreadView` to **display** native state (full snapshot, revision-guarded) | `ChatTranscriptStore.save` / `appendIfNew` / delivery patches (no-ops on daemon — keep call sites hub-gated) |
| `ingestP2pEvent` for the open room — schedule sync when poll carries `stores_updated` + matching `conversation_key` | Incremental Dart merge (`deliveryOnly`, `_mergeStoredRowsIntoLines`) or full reload on every `previewChangeCount` |
| Keep visible lines when reload returns **0 rows** during same-room refresh | `force: true` empty reload that clears persisted bubbles |

### Transcript UI view contract (revision sync)

**Native owns the painted view.** `:p2p` bumps a monotonic **`transcript_revision`** per canonical thread view key (66-hex public key when known) on every disk mutation (`append_if_new`, delivery/read-ack patches, `save_thread`). Poll events that change stores include **`conversation_key`** and **`transcript_revision`** when handler context is set.

**Flutter hub chat** loads via **`p2p_transcript_load_merged`** → `{ revision, lines }` and **replaces** persisted bubbles from that snapshot when `revision > _paintedTranscriptRevision` (or `force`). No incremental inbound merge, no `deliveryOnly` paint path — delivery/read ticks come from the same native rows acks already updated on disk.

**Triggers:** poll `stores_updated` for the open thread; room open/resume; `hubThreadKey` change; after outbound send (force sync). Single-flight + ~100 ms debounce on burst ack polls.

**Symptom if broken:** chat lines disappear while the hub preview still shows text; or outbound/inbound rows vanish after read-ack confirm while another append was in flight. Fix native locking and UI wipe guards — not “merge harder in Dart”.

## What we explicitly do **not** do

- Pull chat history from the peer’s device (“give me messages since T”).
- Mirror the other side’s delivery/read flags without ack frames.
- Put multiaddrs or PeerId on new connect invites.
- Restart the whole libp2p node on every contact list change.
- Use gossipsub for 1:1 DM (streams only).
- Store block state outside `contacts_v1.json` (`is_blocked` on the contact row is the only block flag; no legacy blocked-peer-id list).
- Infer `is_known` from chat history or preview alone.

## Code map (quick)

| Topic | Path |
|-------|------|
| AI entry | `AGENTS.md` |
| Layer contract | `docs/ARCHITECTURE.md` |
| Design (this file) | `docs/DESIGN.md` |
| DM wire + acks | `docs/GHAL_BOL_DM_MSG_V1.md` |
| URI / QR | `docs/GHAL_BOL_URI_SCHEME.md` |
| Stream node + ack send | `ghal_bol/src/p2p/chat_server.rs` |
| P2P FFI + poll | `ghal_bol/src/p2p_runtime.rs`, `p2p_ffi.rs` |
| Daemon RPC | `ghal_bol/src/daemon/server.rs` |
| Event → stores | `ghal_bol/src/dm_event_handler.rs` |
| Contacts / transcript | `ghal_bol/src/contacts_v1.rs`, `dm_transcript_store.rs` |
| Hub / foreground | `ghal_bol_ui/lib/chat_hub_screen.dart` |
| Chat UI (display) | `ghal_bol_ui/lib/chat_screen.dart` |
| P2P start / dm_peers | `ghal_bol_ui/lib/p2p_network_coordinator.dart` |
| Poll bridge | `ghal_bol_ui/lib/p2p_event_bridge.dart` |
| Tick labels (comments) | `ghal_bol_ui/lib/dm_delivery_sync.dart` |
| Android `:p2p` service | `ghal_bol_ui/android/.../GhalBolP2pService.kt` |
