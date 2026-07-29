# Attachments (1:1) — product + implementation plan

**Status:** Mailbox E2E shipped — file bytes ride the same sealed rail as voice/text.
**WAN:** `ghal_bol_delivery` (identity-sealed ciphertext only). **LAN:** native-connect DM (or mux for oversized). **Coord is never used for attachments.**

---

## Goal

User A shares a **file** with user B in 1:1 chat:

1. A encrypts the file **to B’s identity** (same offline/delivery seal as text and voice).
2. For normal sizes, A puts the file inside the sealed DM/delivery inner (`file_b64`).
3. B decrypts offline, writes a local copy, and acks like any other DM.
4. Oversized files between **LAN** peers only may use the native-connect attach mux.

**Not in scope:** Ghal Bol-operated blob CDN ([PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) Tier 2/3), group shares, coord/relay file transfer.

---

## Architecture (aligned with DESIGN.md)

| Rule | Choice |
|------|--------|
| **E2E for WAN content** | Full file in sealed delivery envelope — same as voice notes. |
| **P2P only where product allows** | LAN text/attachments + voice/video **calls**. Not WAN DM. |
| **No coord for files** | Coord/bridge is calls (+ optional LAN reachability), never attachment storage or fetch. |
| **Recipient authority** | Delivery/read ticks from B; optional `attachment_complete` for LAN mux. |
| **Rust owns policy** | Pack, seal, upload, persist, size caps — Flutter is picker + bubble. |

```text
WAN (delivery URL set):
  A: pack AttachmentInner → seal → delivery upload
  B: open envelope → write downloads/ → transcript local_path set (no download tap)

LAN (peer on native connect):
  Same sealed DM inner when under mailbox cap
  Oversized only: stage ciphertext + attach mux fetch on CHANNEL_ATTACH
```

---

## Control / data plane

### Mailbox inner (`attachment_version: 2`)

```json
{
  "attachment_version": 2,
  "file_name": "report.pdf",
  "mime_type": "application/pdf",
  "size_plaintext": 123456,
  "sha256_plaintext": "<hex>",
  "file_b64": "<plaintext file bytes>"
}
```

- Sealed to recipient identity for **delivery**; transport KEM for **LAN DM** (same split as text/voice).
- Cap: `ATTACH_MAX_SEALED_INNER_BYTES` = **3 MB** sealed inner (matches voice). Typical usable plaintext ≈ 2 MB after base64.
- Wire kind remains `attachment_offer` for transcript/`msg_kind` compatibility.

### LAN mux (oversized only)

Protocol `/ghal-bol/attach/1.0.0` on native connect `CHANNEL_ATTACH`:

- Offer JSON carries `blob_id`, `content_key_b64`, hashes, TTL — **no** `file_b64`.
- Recipient taps Download → `p2p_attachment_fetch` → chunks over LAN session.
- Max plaintext **100 MB** (`MAX_FILE_SIZE_BYTES`).
- Requires a live **LAN** connect session — never coord relay for bytes.

### `p2p_attachment_fetch`

Start command only (does not block the daemon socket for the whole transfer). Returns:

| Result | Meaning |
|--------|---------|
| `{ ok, local_path }` | Done inside start grace |
| `{ ok, downloading: true }` | LAN mux transfer in progress |
| `{ error }` | No offer / expired / no LAN session |

---

## Flutter

- Inbound mailbox attachments arrive with `local_path` already set → show **Downloaded**.
- Download button only when `local_path` empty (LAN mux / legacy).
- File picker + send → `p2p_send_attachment` (Rust chooses mailbox vs LAN mux).

---

## Ownership

| Concern | Owner |
|---------|--------|
| Pack / size reject / seal / upload | **Rust** `attach_v1`, `delivery_*`, `msg_v1`, `p2p_runtime` |
| Transcript + `local_path` | **Rust** `dm_event_handler` / `dm_transcript_store` |
| LAN mux serve/fetch | **Rust** `attach_v1` + `connect/frames` |
| Picker / bubble / download UI | **Flutter** |

---

## Explicit non-goals

- Uploading attachment bytes to **coord**.
- Sender-served WAN fetch over connect/relay (removed — that was the old plan).
- Putting multi‑hundred‑MB files in one delivery WS frame without a future blob tier.

---

## References

- Product split: [DESIGN.md](DESIGN.md) § Why pure P2P WAN text was dropped
- Wire: [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) § Attachments
- Delivery: [GHAL_BOL_DELIVERY.md](GHAL_BOL_DELIVERY.md)
- Voice (same mailbox pattern): [VOICE_MESSAGES_PLAN.md](VOICE_MESSAGES_PLAN.md)
