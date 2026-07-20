# Ghal Bol – Delivery Server Design

Implementation crate: [`ghal_bol_delivery/`](../ghal_bol_delivery/).

## Purpose

The delivery server exists **only** to provide reliable, temporary delivery of end-to-end encrypted messages.

It is **not** intended to become a cloud storage service, conversation archive, or identity provider.

The philosophy is:

- Device owns the identity.
- Device owns the private keys.
- Messages remain end-to-end encrypted.
- Server assists only with availability and reliable delivery.

## Delivery Server

Responsibilities:

- Temporary encrypted message storage.
- Offline delivery.
- Delivery acknowledgements.
- TTL management. (TTL is actually decided by the user but there will be a minimum and maximum threshold values this server will allow)
- Quota management.
- Cleanup.

It does **not**:

- decrypt messages
- generate identities
- manage encryption keys
- permanently store conversations (in future it will provide cloud service based on tier)

---

# Messaging Philosophy

Messaging should prioritize:

- reliability
- simplicity
- predictable behaviour

instead of forcing native connect messaging for **WAN text**.

**Why WAN text left native connect:** relay-based DM required both peers online at once; offline or sleeping recipients lost messages; mobile CGNAT + handover made outbox/ack paths unreliable. Chat’s core job is **guaranteed delivery when the recipient returns** — a mailbox model fits; realtime P2P does not. **Privacy unchanged:** payloads are E2E encrypted with the same contact identity keys; the server stores opaque ciphertext and cannot read content.

Voice and video remain P2P-first because they are inherently realtime sessions.

---

# Message Flow

## Sending

1. Sender encrypts the message locally.
2. Sender uploads encrypted payload to the delivery server.
3. Server stores the encrypted payload.
4. Server returns success.
5. Sender is now free to go offline.

---

## Recipient

Recipient periodically connects to the delivery server.

If pending messages exist:

1. Download encrypted payload.
2. Decrypt locally.
3. Store locally.
4. Send delivery acknowledgement.

---

## Delivery Acknowledgement

After recipient acknowledges:

Server:

- marks message as delivered
- deletes the message but still keeps metadata to keep track of message state (recv ack and read ack)


So 
Message flow is 
Sender ─────► Delivery Server ─────► Recipient

and the state flow is
Recipient ─────► Delivery Server ─────► Sender

---

# Outbound ticks (sender UI)

When **`GHAL_BOL_DELIVERY_URL`** is set, **chat text** uses the delivery server — not native connect DM acks. The sender sees **four** outbound states. Flutter **displays** native transcript `delivery` only; it does not invent ticks.

| UI (sender) | Transcript `delivery` | Meaning | Authority / wire |
|-------------|----------------------|---------|------------------|
| **No tick** — clock / schedule icon | `pending` | Message saved locally; **not yet confirmed** on the delivery server | Local only until upload succeeds |
| **Single black tick** (`done`) | `sent` | Message **left the device** and the server accepted the upload | Server `message.upload.ok` → native `patch_outgoing_delivery(…, "sent")` |
| **Double black tick** (`done_all`, neutral) | `delivered` | Message **reached the recipient** (they decrypted and acked) | Recipient `inbox.ack` → server `message.ack_to_sender` → `delivery=delivered` |
| **Double blue tick** (`done_all`, blue) | `read` | Recipient **read** the message in the open chat room | Recipient `inbox.read` → server `message.read_to_sender` → `delivery=read` (wire: [GHAL_BOL_DELIVERY_WIRE_V1.md](GHAL_BOL_DELIVERY_WIRE_V1.md)) |

**Monotonic only:** `pending` → `sent` → `delivered` → `read`. Never downgrade.

**Not the same as legacy P2P ticks:** native connect DM used `pending` → single tick at `delivered` (`ack_received`) → blue double at `read` (`ack_read`). Delivery mode adds an explicit **server-received** step (`sent`) and uses **double black** for recipient delivery. See [DESIGN.md](DESIGN.md) § “Delivery mode — outbound ticks”.

**Truthful UI:** never show `sent` until the server returns `message.upload.ok`; never show `delivered` / `read` until the server relays recipient acks. Upload HTTP/WSS failure leaves `pending` (or `failed` on hard send error).

**Read receipts:** blue tick uses `inbox.read` → `message.read_to_sender` on the delivery worker (`delivery_read_acks.rs`), gated by the same hub UI session as P2P (`GhalBolUiSession` / `p2p_sync_ui_session`). Flutter maps transcript `delivery=read` only — no Dart ack logic.

---

# Temporary Storage Only

The server acts as a temporary mailbox.

It is NOT cloud storage.

Messages exist only until one of the following:

- delivered
- expired

---

# Free Users

Pending queue limit:

```
500 MB

```

TTL:

```
7 days

```

After 7 days:

- message expires
- server deletes it
- sender is informed that delivery expired

---

# Sender Behaviour

Sender always keeps the original message locally.

If message expires:

UI should indicate something like:

```
Expired
Tap to resend

```

Tapping resend:

- uploads encrypted message again
- starts a NEW delivery attempt
- resets TTL to another 7 days

---

# Extend TTL

While a message is still **queued** on the server, the sender may **extend TTL** (within server min/max bounds). Expired messages must be **resent** (new upload) — extend is rejected once `state=expired`.

# User-visible mailbox

The app shows each user **their own** pending metadata only: `message_id`, `size_bytes`, `expires_at`, `state`. The server never exposes plaintext or other users' quota/mailbox rows. See [`GHAL_BOL_DELIVERY_WIRE_V1.md`](GHAL_BOL_DELIVERY_WIRE_V1.md).

# Resend Behaviour

Resend MUST NOT create duplicate pending copies.

Instead:

If message is still pending:

- replace existing queued copy
- refresh TTL

If already expired:

- create new pending entry
- TTL starts from current time

Only one queued copy should ever exist.

---

# TTL

TTL is always based on the latest delivery attempt.

Example:

Day 1

Send

Expires Day 8

Day 3

Resend

Expires Day 10

Day 6

Resend

Expires Day 13

This prevents permanent server storage while allowing sender-controlled retries.

---

# Quota

Quota represents:

Maximum pending queue.

NOT allocated storage.

Example:

```
500 MB

```

means:

User may occupy at most 500 MB of pending messages.

It does NOT reserve 500 MB.

Storage is only consumed by actual pending messages.

---

# Cleanup Rules

Delete immediately after:

- Recipient has received the message.

Delete after TTL:

- message expired

---

# Why TTL Exists

Without TTL:

- abandoned accounts
- deleted apps
- lost phones

would permanently consume storage.

TTL guarantees automatic space recovery.

---

# Why Quota Exists

Quota prevents:

- storage abuse
- infinite pending queues
- inactive users consuming unlimited storage

Queue is bounded.

---

# Server Philosophy

The delivery server is:

- temporary mailbox
- encrypted queue
- availability service

It is NOT:

- cloud backup
- archive
- conversation history

---

# Privacy Model

Server stores:

- encrypted payload
- delivery metadata
- expiry metadata

Server never stores plaintext.

Server never possesses private keys.

Identity remains device-owned.

---

# LAN Behaviour

LAN messaging remains available.

However:

Messages should still be uploaded to the delivery server.

Reason:

If peer suddenly disconnects from LAN:

delivery is still guaranteed.

LAN provides:

- lower latency

Server provides:

- reliability

---

# Voice / Video Calls

Remain P2P.

Coordination/Relay server continues handling:

- peer discovery
- NAT traversal
- relay allocation

Delivery server is not involved.

---

# Why Messaging Is Server-Assisted

Reasons:

- sender can immediately go offline
- recipient may come online later
- reliable delivery
- simpler messaging state machine
- simpler client retry logic

Trying to keep messaging purely P2P introduces many edge cases:

- sender offline
- recipient offline
- retry scheduling
- abandoned devices
- persistent availability

The delivery server solves these cleanly.

---

# Deployment and canonical hostname

**Users never configure the delivery host.** The app ships a single production URL:

`GHAL_BOL_DELIVERY_URL=wss://delivery.ghalbol.com:55003`

When the operator moves the backend (home PC → cloud), **repoint the GoDaddy A record** for `delivery.ghalbol.com` — not the app. Clients reconnect to the same URL after a brief DNS propagation blip.

| Concern | Owner |
|---------|--------|
| Home install + DDNS + nginx WSS | [`ghal_bol_delivery/deploy/`](../ghal_bol_delivery/deploy/) — see [DELIVERY_HOME.md](../ghal_bol_delivery/deploy/DELIVERY_HOME.md) |
| Client URL | `ghal_bol_ui/env/.env.production` — canonical `delivery.ghalbol.com` |
| Calls / relay | Unchanged — [`ghal_bol_coord`](../ghal_bol_coord/) only |

Home stack mirrors coord1: in-process GoDaddy DDNS (`GHAL_BOL_DDNS_CREDENTIALS`), user systemd unit on loopback **8770**, nginx TLS on **55003** (home high port, like coord1 relay **55002**), router forward **55003** only.

---

# Mailbox migration (operator)

Pending ciphertext lives in SQLite (`mailbox.db`). All rows are opaque envelopes keyed by `(sender_wire, message_id)` — safe to export as a file.

| Step | Action |
|------|--------|
| 1 | Stop the old service to freeze uploads, then run `mailbox-stats` |
| 2 | `ghal_bol_delivery export-mailbox --out archive.tar.zst` |
| 3 | Copy archive to the prepared new host and stop its service |
| 4 | `ghal_bol_delivery import-mailbox --in archive.tar.zst --replace` |
| 5 | Verify stats, then start the new service |
| 6 | Flip DNS A record for `delivery.ghalbol.com` |
| 7 | `./ghal_bol_delivery/deploy/verify_delivery.sh` |

Full runbook: [DELIVERY_HOME.md § Mailbox migration](../ghal_bol_delivery/deploy/DELIVERY_HOME.md#mailbox-migration-home--new-host).

Ephemeral WebSocket sessions are not migrated; clients reconnect automatically.

---

# Why Not Multiple Delivery Servers

Not adopted.

Reason:

Multiple delivery servers require:

- synchronization
- routing
- federation
- conflict handling
- duplicate suppression

This greatly increases protocol complexity.

Single delivery server keeps architecture simple.

---

# Overall Goals

The delivery server should provide:

- reliable delivery
- bounded storage
- bounded resource usage
- simple protocol
- minimal trust
- temporary encrypted storage only

The strongest privacy guarantees remain:

- device-owned identities
- end-to-end encryption
- local private keys
- no phone number
- no email
- server cannot decrypt user messages
- server stores messages only as long as necessary for delivery

