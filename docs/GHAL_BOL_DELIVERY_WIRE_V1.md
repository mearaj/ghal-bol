# Ghal Bol delivery wire — v1

Canonical WebSocket + HTTP contract for [`ghal_bol_delivery`](../ghal_bol_delivery/).

Product intent: [`GHAL_BOL_DELIVERY.md`](GHAL_BOL_DELIVERY.md).

## Transport

| Endpoint | Purpose |
|----------|---------|
| `GET /health` | Liveness + aggregate mailbox metrics (no auth) |
| `GET /v1/challenge?identity_wire=…` | Issue one-time session nonce (optional; may also inline in WS) |
| `WS /v1/ws` | Persistent bidirectional session — **clients connect; server never dials** |

All WebSocket payloads are **one JSON object per text frame** (UTF-8).

Common envelope fields:

| Field | Type | Notes |
|-------|------|-------|
| `type` | string | Frame discriminator (required) |
| `request_id` | string | Optional client correlation id; echoed on responses |

## Session authentication

Mirror coord challenge signing ([`coord_register_auth.rs`](../ghal_bol_core/src/coord_register_auth.rs)).

### Challenge bytes

```
ghal_bol:delivery:session:v1\n<nonce_hex>\n<identity_wire_lower>
```

Sign with the device identity key (algorithm-specific, same rules as `POST /v1/register`).

### Handshake (client → server)

1. `session.open` — `{ "type": "session.open", "identity_wire": "…" }`
2. Server → `session.challenge` — `{ "type": "session.challenge", "nonce_hex": "…" }`
3. `session.auth` — `{ "type": "session.auth", "signature_hex": "…" }`
4. Server → `session.ready` — policy + quota snapshot (see below)

After `session.ready`, the connection is bound to `identity_wire`. All mailbox operations on this socket are scoped to that identity.

### Per-operation signatures (upload / extend)

Sensitive mutating ops include an inline proof even on an authenticated socket:

| Domain | Used by |
|--------|---------|
| `ghal_bol:delivery:upload:v1\n<nonce_hex>\n<message_id>\n<recipient_wire_lower>` | `message.upload` |
| `ghal_bol:delivery:extend:v1\n<nonce_hex>\n<message_id>` | `mailbox.ttl.extend` |

Server issues a fresh `op_nonce_hex` in `session.ready` and rotates it after each successful signed op (or TTL 60s).

## Policy and quota (server → client)

### `policy.limits`

```json
{
  "type": "policy.limits",
  "min_ttl_secs": 3600,
  "max_ttl_secs": 2592000,
  "default_ttl_secs": 604800
}
```

### `quota.status`

```json
{
  "type": "quota.status",
  "allocated_bytes": 524288000,
  "used_bytes": 12345,
  "pending_count": 2
}
```

Sent on `session.ready` and after upload/delete/expiry. Client may request with `quota.status` (no extra fields).

## Delivery message envelope (E2E)

Share tag: **`ghal_bol_delivery_msg_v1`**.

Stored and transmitted as opaque JSON; server **must not** decrypt.

```json
{
  "ghalbol.share": "ghal_bol_delivery_msg_v1",
  "format_version": 1,
  "message_id": "uuid-or-monotonic-id",
  "sender_wire": "<identity>",
  "recipient_wire": "<identity>",
  "created_at_ms": 1710000000000,
  "ciphertext_hex": "<identity-sealed inner JSON>",
  "signature_hex": "<identity sig over canonical body>"
}
```

**Inner plaintext** (before seal): `{"text":"…"}` — same semantic as DM v1 text body.

**Cipher (v1 shipping):** `OFFLINE_CIPHER_SECP256K1_V1` from [`offline_seal_v1.rs`](../ghal_bol_core/src/offline_seal_v1.rs) for secp256k1 recipients. Non-secp256k1: error until extended per [`MULTI_ALGO.md`](MULTI_ALGO.md).

**Outer signature** covers canonical JSON of `{ message_id, sender_wire, recipient_wire, created_at_ms, ciphertext_hex }` (sorted keys, no whitespace).

Server validates:

- `sender_wire` matches authenticated session identity
- `recipient_wire` parses as known algorithm
- `signature_hex` verifies
- `message_id` unique per sender (resend-replace rules below)

Server stores `envelope` blob + metadata only.

## Client → server frames

### `message.upload`

```json
{
  "type": "message.upload",
  "envelope": { … },
  "ttl_secs": 604800,
  "op_nonce_hex": "…",
  "signature_hex": "…"
}
```

| Field | Notes |
|-------|-------|
| `ttl_secs` | Optional; clamped to `[min_ttl_secs, max_ttl_secs]`; default `default_ttl_secs` |
| Resend-replace | Same `message_id` while row `state=queued` replaces blob and refreshes `expires_at` |
| After expiry | New upload creates new queued row |

Responses: `message.upload.ok` or `error` (`quota_exceeded`, `invalid_envelope`, `forbidden`).

### `inbox.ack`

Recipient acknowledges delivery (deletes ciphertext, retains metadata).

```json
{
  "type": "inbox.ack",
  "message_id": "…",
  "sender_wire": "…"
}
```

Scoped: `recipient_wire == session identity` and row addressed to recipient.

Response: `inbox.ack.ok`.

### `inbox.read`

Recipient read receipt — see § `message.read_to_sender` above.

### `mailbox.outbox.list`

Sender views **own** pending + recent metadata (no ciphertext).

```json
{ "type": "mailbox.outbox.list", "include_expired": true }
```

### `mailbox.ttl.extend`

```json
{
  "type": "mailbox.ttl.extend",
  "message_id": "…",
  "extend_secs": 86400,
  "op_nonce_hex": "…",
  "signature_hex": "…"
}
```

Rules:

- Row must be `queued` and owned by session sender
- `new_expires_at = min(now + extend_secs, uploaded_at + max_ttl_secs)`
- Reject if already `expired` (client must `message.upload` resend)

### `quota.status`

```json
{ "type": "quota.status" }
```

### `ping`

```json
{ "type": "ping" }
```

## Server → client frames

### `message.inbound`

Push to **recipient** when a queued message exists (on connect and after upload).

```json
{
  "type": "message.inbound",
  "envelope": { … },
  "expires_at_ms": 1710604800000
}
```

### `message.ack_to_sender`

Push to **sender** when recipient acks.

```json
{
  "type": "message.ack_to_sender",
  "message_id": "…",
  "recipient_wire": "…",
  "delivered_at_ms": 1710000001000
}
```

Client patches outbound transcript `delivery=delivered`.

### `message.read_to_sender` (read receipt)

Push to **sender** when recipient sends `inbox.read` (chat room open on recipient — same product intent as P2P `ack_read`).

```json
{
  "type": "message.read_to_sender",
  "message_id": "…",
  "recipient_wire": "…",
  "read_at_ms": 1710000002000
}
```

Client patches outbound transcript `delivery=read` (implies `delivered`; monotonic rank in `dm_transcript_store`).

Response: `inbox.read.ok`.

## Outbound transcript states (delivery mode)

Canonical sender-side `delivery` values when `GHAL_BOL_DELIVERY_URL` is set:

| `delivery` | UI tick | Set when |
|------------|---------|----------|
| `pending` | Clock / no tick | Local append; upload not confirmed |
| `sent` | Single black ✓ | `message.upload.ok` |
| `delivered` | Double black ✓✓ | `message.ack_to_sender` |
| `read` | Double blue ✓✓ | `message.read_to_sender` |

P2P libp2p DM (no delivery URL) keeps `pending` → `delivered` → `read` via `ack_received` / `ack_read` on `/ghal-bol/msg/1.0.0` — see [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md).

### `inbox.read` (client → server)

Recipient read receipt (scoped like `inbox.ack`):

```json
{
  "type": "inbox.read",
  "message_id": "…",
  "sender_wire": "…"
}
```

Server relays `message.read_to_sender` to the sender's connected session(s). Metadata row retains read state after ciphertext delete (same as delivery ack).

### `message.expired`

Push to **sender** when TTL sweeper deletes a row.

```json
{
  "type": "message.expired",
  "message_id": "…",
  "recipient_wire": "…",
  "expired_at_ms": 1710604800000
}
```

### `mailbox.outbox.snapshot`

Response to `mailbox.outbox.list`:

```json
{
  "type": "mailbox.outbox.snapshot",
  "rows": [
    {
      "message_id": "…",
      "recipient_wire": "…",
      "size_bytes": 512,
      "uploaded_at_ms": 1710000000000,
      "expires_at_ms": 1710604800000,
      "state": "queued"
    }
  ]
}
```

`state`: `queued` | `delivered` | `expired` (metadata rows may remain after blob delete).

### `quota.warning`

Proactive notice when `used_bytes / allocated_bytes >= 0.9`.

### `error`

```json
{
  "type": "error",
  "code": "quota_exceeded",
  "message": "human readable",
  "request_id": "optional"
}
```

| Code | Meaning |
|------|---------|
| `unauthorized` | Auth failed |
| `forbidden` | Cross-peer access |
| `quota_exceeded` | Upload would exceed allocation |
| `invalid_envelope` | Parse/sig/cipher failure |
| `not_found` | Unknown message_id |
| `ttl_invalid` | Extend/upload TTL out of bounds |
| `expired` | Extend on expired row |

## Access control summary

| Operation | Allowed when |
|-----------|----------------|
| `mailbox.outbox.list` | `sender_wire == session identity` |
| `mailbox.ttl.extend` | Sender owns row, `state=queued` |
| `message.upload` | `envelope.sender_wire == session identity` |
| `quota.status` | Own quota row only |
| `inbox.ack` | `envelope.recipient_wire == session identity` |

**Never** return another peer's quota, mailbox rows, or ciphertext to the wrong session.

## HTTP `/health`

```json
{
  "ok": true,
  "service": "ghal_bol_delivery",
  "instance_id": "home-pc",
  "schema_version": 1,
  "connected_peers": 3,
  "pending_messages": 12,
  "pending_bytes": 45678,
  "oldest_pending_age_secs": 42
}
```

`instance_id` and `schema_version` are for **operator migration verification** — not shown in the Flutter UI.

## Logging (server)

Structured fields only: `identity_wire`, `message_id`, `bytes`, `state`, `reject_reason`. **Never** log `ciphertext_hex`, inner plaintext, or private keys.

## Environment

| Variable | Default |
|----------|---------|
| `GHAL_BOL_DELIVERY_LISTEN` | `0.0.0.0:8770` |
| `GHAL_BOL_DELIVERY_DATA_DIR` | platform data dir |
| `GHAL_BOL_DELIVERY_MIN_TTL_SECS` | `3600` |
| `GHAL_BOL_DELIVERY_MAX_TTL_SECS` | `2592000` (30d) |
| `GHAL_BOL_DELIVERY_DEFAULT_TTL_SECS` | `604800` (7d) |
| `GHAL_BOL_DELIVERY_QUOTA_BYTES_PER_PEER` | `524288000` (500 MB) |

Client: `GHAL_BOL_DELIVERY_URL` (`ws://` or `wss://` base, path `/v1/ws`).
