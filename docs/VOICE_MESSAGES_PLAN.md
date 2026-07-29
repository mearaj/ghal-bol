# Voice messages — implementation plan

**Status:** Implemented in Rust and Flutter; continue using this doc for limits and follow-up testing.

**Depends on:** [DESIGN.md](DESIGN.md), [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md), [AGENTS.md](../AGENTS.md) (golden rules 1, 7).

**Related:** [ATTACHMENTS_PLAN.md](ATTACHMENTS_PLAN.md) — same E2E mailbox rail for normal-sized files; LAN mux only for oversized local transfers.

---

## Product goal

WhatsApp-style **voice notes** in 1:1 chat:

- User **records** a short clip in the composer.
- Clip is sent as **one DM message** (same path as text).
- Recipient sees a **voice bubble** (duration, play/pause).
- **Delivery and read ticks** use the same recipient-authority ack model as text.

**Not in scope:** live voice calls ([GHAL_BOL_VOICE_V1.md](GHAL_BOL_VOICE_V1.md), [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md)) — those use call signaling + `/ghal-bol/call/1.0.0` streaming media keys, not DM envelopes.

---

## Design principles

| Rule | Detail |
|------|--------|
| **Same rail as text** | One frame on `/ghal-bol/msg/1.0.0`; outbox; `ack_received` / `ack_read`; transcript patch on poll. |
| **Full payload in one send** | Entire encoded audio inside the sealed inner JSON — no chunking, no sender-served download link (v1). |
| **Same E2E as text** | Inner JSON → `seal_to_secp256k1_public` → `ciphertext_hex` → signed envelope. **Not** call media keys. |
| **Rust owns behaviour** | Record policy, Opus encode/decode, send/retry, decrypt, transcript — expose FFI/RPC to Flutter for UI only. |
| **Truthful ticks** | Flutter never promotes delivery/read; native transcript + poll only ([DESIGN.md](DESIGN.md)). |

---

## Limits (v1)

| Limit | Value | Rationale |
|-------|-------|-----------|
| **Max duration** | **120 seconds (2 min)** | Product minimum; WhatsApp in-chat notes have no short cap, but 2 min is comfortable for users and wire size. |
| **Max frame size** | **≤ 3 MB** sealed envelope body budget (hard stop before send) | DM `read_frame` rejects **> 4 MB** (`frames.rs`); leave headroom for JSON + hex overhead. |
| **Codec** | **Opus**, mono, voice-optimized bitrate | Reuse Opus expertise from `call_media/`; ~16–24 kbps target → ~240–360 KB audio for 2 min, well under cap. |
| **Channels** | 1:1 DM only | Same as current chat. |

Enforce **both** max duration (UI + native) and max encoded bytes (native reject before seal).

---

## Wire format

### Envelope (unchanged shell)

Same `ghal_bol_msg_v1` envelope as text. Add `MsgKind::Voice` in `msg_v1.rs` (wire: `"voice"`).

| Field | Voice message |
|-------|----------------|
| `kind` | `voice` |
| `id` | Opaque message id (acks use this as `ref_id`) |
| `ciphertext_hex` | Sealed inner JSON (below) |
| `signature_hex` | secp256k1 over canonical envelope |

### Inner JSON (plaintext before seal)

```json
{
  "codec": "opus",
  "duration_ms": 45000,
  "sample_rate_hz": 48000,
  "channels": 1,
  "audio_b64": "<base64-encoded Opus payload>"
}
```

- **`audio_b64`** — full recording, one blob (v1).
- **`duration_ms`** — for UI waveform/duration label; also stored on transcript row.
- Version inner schema with a `"voice_msg_version": 1` field if future codecs are added.

### Encryption (same as DM text — transport KEM)

1. `serde_json` inner bytes  
2. Seal with **transport KEM v2** (`DM_CIPHER_TRANSPORT_V2`) after `TransportKemHello` — same path as text DM (`transport_kem_v1.rs`, `msg_v1.rs`)  
3. Hex-encode → `ciphertext_hex`  
4. Sign envelope with sender device key  

Decrypt: verify signature → open transport seal → parse inner JSON → decode Opus → play.

**Do not** use `derive_call_media_keys_from_transport` for voice notes — that is for live call media substreams only.

---

## Delivery, read, and transcript

Mirror text ([GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md), [DESIGN.md](DESIGN.md)):

| Concern | Voice (same as text) |
|---------|----------------------|
| Inbound delivery | Always `ack_received` with `received_at_ms` |
| Inbound read | `ack_read` only when read gate open (`may_send_in_room_read_ack`) |
| Outbound ticks | `pending` → `delivered` → `read` from peer acks on poll |
| Outbox | Retry whole message until `ack_received` or `ack_read`; dedupe by `message_id` |
| Leave backlog | Same `chat_room_exit_at_ms` / `dispatch_read_ack_pass` rules |

### Transcript row extensions

Extend `StoredChatLine` / poll JSON (Rust-owned):

| Field | Purpose |
|-------|---------|
| `msg_kind` | `"voice"` (or derive from envelope kind) |
| `duration_ms` | UI label |
| `audio_path` or `audio_ref` | Local file path after decrypt (per-device; not re-sent on wire) |

Hub preview: e.g. `"Voice message"` or `"🎤 0:45"` via `contacts_v1` preview helper (Rust).

---

## Layer ownership

| Concern | Owner | Notes |
|---------|--------|-------|
| Opus encode/decode | **Rust** (`ghal_bol`) | New small module or reuse `call_media/codec.rs` traits without call session keys |
| Build/seal/send envelope | **Rust** `msg_v1.rs`, `outbound.rs`, outbox | Parallel to `build_text_envelope` |
| Inbound verify/open/decode | **Rust** `frames.rs`, `dm_event_handler.rs` | Treat `voice` like `text` for ack gating |
| Transcript append/patch | **Rust** `dm_transcript_store.rs` | Chronological insert (existing) |
| Record UI, waveform, play | **Flutter** | Hold-to-record, timer, cancel; call FFI to start/stop/send |
| Ticks display | **Flutter** | Read `delivery` from native transcript only |

**Flutter must not:** send acks, own outbox, invent ticks, or re-implement seal/open.

---

## Flutter UX (v1)

- **Hold** mic in composer → record; release to send (or slide to cancel — product choice).
- **Timer** visible; hard stop at **2:00**.
- Outbound bubble: duration + sending spinner → ticks when native says so.
- Inbound bubble: play/pause; optional simple progress bar.
- **Read receipts:** same as text — room open + read gate; no special “played to end” requirement in v1 (optional v2).

---

## Native / poll events

Extend `dm_message` poll events:

```json
{
  "kind": "dm_message",
  "msg_kind": "voice",
  "id": "...",
  "duration_ms": 45000,
  "from": "...",
  "stores_updated": true
}
```

Do **not** put raw `audio_b64` in poll events — UI loads from transcript store / local audio file after native persists.

Hub `ingestP2pEvent`: treat `voice` like `text` for `syncTranscriptView(force: true)`.

---

## Implementation phases

### Phase 1 — Protocol + Rust send/receive

- [ ] `MsgKind::Voice` + `build_voice_envelope` / open path in `msg_v1.rs`
- [ ] `voice_msg_v1.rs` (inner schema, limits, Opus wrap)
- [ ] Outbound: FFI/RPC `send_voice_dm` → transcript + outbox (mirror `send_text_dm`)
- [ ] Inbound: `frames.rs` — ack path for `voice` same as `text`
- [ ] `apply_inbound_*` in `dm_event_handler.rs` — append transcript, preview bump
- [ ] Unit tests: round-trip seal/open, max size reject, duration cap

### Phase 2 — Flutter UI

- [ ] Composer record control + permission (mic)
- [ ] Voice bubble widget + audio playback from local path
- [ ] Hub preview string for voice

### Phase 3 — Soak + docs

- [ ] Android ↔ Linux LAN/WAN: send 2 min clip, ticks, reconnect resend
- [ ] Update [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) with `voice` kind (when shipping)
- [ ] Entry in [DESIGN.md](DESIGN.md) message kinds table (when shipping)

---

## Testing checklist

| Case | Expect |
|------|--------|
| 5 s voice, room open | `ack_received` + `ack_read`; blue tick on sender after poll |
| 120 s voice | Sends one frame; under size cap |
| 121 s / oversize encode | Native reject before send; user-visible error |
| Recipient offline | Outbox retry; delivers on `chat_ready` without opening room |
| Duplicate resend | One bubble; monotonic ticks |
| WAN handover mid-send | Outbox eventually drains (existing transport) |

---

## Anti-patterns (do not ship)

- Sending voice on `/ghal-bol/call/1.0.0` or call media keys.
- Chunked multi-frame voice in v1 (adds complexity; use attachments plan for large files).
- Plaintext audio in envelope or poll JSON.
- Flutter-side ack or tick promotion.
- gzip on inner JSON for v1 (Opus already compresses audio; text-style seal is enough).
- Separate transcript store or LAN/WAN message stores for voice.

---

## Open decisions (resolve before Phase 1 coding)

1. **Sample rate:** 48 kHz (match calls) vs 16 kHz (smaller) for voice notes.
2. **Waveform:** generate in Flutter from local file vs Rust preview peaks in transcript.
3. **Storage:** always write decrypted Opus to app data dir keyed by `message_id` vs inline in transcript JSON (prefer **file on disk**, metadata in transcript).

---

## References

- Text wire: [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md)
- Calls (out of scope): [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md)
- Frame limit: `ghal_bol_core/src/p2p/chat_server/frames.rs` (`4 * 1024 * 1024`)
- Seal: `ghal_bol_core/src/transport_kem_v1.rs`, `ghal_bol_core/src/msg_v1.rs`
