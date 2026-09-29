# Voice messages — implementation plan

**Status:** Implemented in Rust and Makepad; continue using this doc for limits and follow-up testing.

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
| **Rust owns behaviour** | Record policy, Opus encode/decode, send/retry, decrypt, transcript. Makepad calls `ghal_bol_core::host`. |
| **Truthful ticks** | Makepad never promotes delivery/read; native transcript + poll only ([DESIGN.md](DESIGN.md)). |

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
| Record UI and play | **Makepad** | Composer control calls `host::voice_note_toggle`, `host::send_voice_note`, and `host::play_voice_file` |
| Ticks display | **Makepad** | Read `delivery` from native transcript only |

**Makepad must not:** send acks, own outbox, invent ticks, or re-implement seal/open.

---

## Makepad UX (v1)

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

Do **not** put raw `audio_b64` in poll events — UI loads from transcript store / local audio file after native persists. On `voice` events, Makepad reloads the open transcript the same way as for text.

---

## Shipping

Voice notes are Opus, mono, 48 kHz, at most 120 seconds. The sealed inner stays within 3 MB. Audio files live under the app data `voice/` directory, keyed by message id. The composer calls `host::voice_note_toggle` and `host::send_voice_note`. Playback calls `host::play_voice_file`. Acks match text. A two-device soak (Android and Linux, LAN and WAN) still belongs on real hardware.

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
- Makepad-side ack or tick promotion.
- gzip on inner JSON for v1 (Opus already compresses audio; text-style seal is enough).
- Separate transcript store or LAN/WAN message stores for voice.

---

## Settled

1. **Sample rate:** 48 kHz, same as calls.
2. **Bubble:** duration and play. No waveform strip.
3. **Storage:** decrypted Opus on disk, metadata in the transcript.

---

## References

- Text wire: [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md)
- Calls (out of scope): [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md)
- Frame path: `ghal_bol_core/src/connect/frames.rs`
- Seal: `ghal_bol_core/src/transport_kem_v1.rs`, `ghal_bol_core/src/msg_v1.rs`
