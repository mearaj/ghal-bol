# Ghal Bol calls — signaling (`ghal_bol_call_v1`)

**Status:** Signaling implemented in Rust (`call_sig_v1.rs`, `call_state.rs`). **Voice and video media** run in Rust over libp2p substreams — see [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) (voice) and [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md) (video).

Read [DESIGN.md](DESIGN.md) and [AGENTS.md](../AGENTS.md): **Rust owns signaling and media**; Flutter is call UI, permissions, and FFI control only.

---

## Product model

| Rule | Detail |
|------|--------|
| **One call type** | No separate “voice call” vs “video call” buttons. Tap **Call** → audio call. |
| **Default media** | Audio only (`media: "audio"` on invite). |
| **Video** | In-call **Video** toggle → `video_on` / `video_off` native signals; starts/stops `/ghal-bol/call-video/1.0.0`. |
| **Signaling transport** | Same libp2p DM stream as chat (`ghal_bol_call_v1` frames). |
| **Media transport** | `/ghal-bol/call/1.0.0` (voice) and `/ghal-bol/call-video/1.0.0` (video) on the existing peer connection. |
| **Media encryption** | Identity-bound AES-GCM per frame (`call_media_key.rs` + `MediaCrypto`); libp2p Noise on the wire. |

---

## Signaling kinds (`call_sig_v1`)

| Wire name | When |
|-----------|------|
| `invite` | Outbound ring; payload `{ "media": "audio" }` (or `"audio_video"`). May include `voice_engine` / `video_engine` capability tags. |
| `accept` | Callee accepts; may echo engine tags. |
| `reject` | Decline. |
| `hangup` | End call. |
| `video_on` | Start native video substream + camera; payload may negotiate `{w,h,fps,codec}` caps. |
| `video_off` | Stop camera + tear down video substream; voice continues. |
| `key_request` | RX → TX: force an immediate video keyframe (after loss / late join). |

All envelopes use `ghalbol.share = ghal_bol_call_v1`, `ref_id` = `call_id`, encrypted inner JSON, secp256k1 signature (see `ghal_bol/src/call_sig_v1.rs`).

---

## Call phases (Rust `call_state`)

| Phase | Meaning |
|-------|---------|
| `idle` | No call with this contact. |
| `outgoing_ringing` | We sent `invite`. |
| `incoming_ringing` | Peer sent `invite`. |
| `connected` | Accepted; native media may still be starting. |

`video_on` / `video_off` / `key_request` are only valid when a call exists for that `call_id`.

---

## Flutter API

- **Outbound:** `GhalBolCall.sendSignal(...)` → native `p2p_call_signal` (FFI or daemon).
- **Inbound:** poll `kind: call_signal` → `CallController` drives UI + native media FFI.
- **Media:** `GhalBolP2p.callMediaStart/Stop/SetMicMuted`, `callVideoStart/Stop`, textures via `NativeCallVideoView`.
- **Do not** send call signals from Dart except through `GhalBolCall` (state checks stay in Rust for outbound).

---

## Permissions

- **Microphone** — required for all calls.
- **Camera** — only when user enables video.

---

## Media (native — not in this doc)

Voice pipeline, transport, FFI, and device-test steps: [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md).

Video pipeline, textures, and call end: [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md).

**Debug:** More → **App log** → filter **Calls**. Lines use `[Call]`, `[Call/Media]`, `[Call/Video]`. Look for `call_media start`, `sent=N recv=M`, `call_video`, `media_e2ee` / identity key prefix.

**In-call UI:** Green lock chip when identity E2E is active: **End-to-end encrypted · contact key** `03a1f2b3…`. Media must not connect without a derived identity key (golden rule 7).

---

## Media encryption (E2EE)

| Layer | What it protects |
|-------|------------------|
| **DM signaling** (`ghal_bol_call_v1`) | Invite, accept, video_on/off — sealed to peer secp256k1 + signed. |
| **libp2p transport** | Noise on connections and substreams. |
| **Per-frame seal** | Opus / video chunks sealed with `derive_call_media_keys_from_identity` before substream write. |

**Key derivation (both peers, same result):**

```text
ikm = SHA256( ECDH(my_identity_secret, peer_public_key_66hex) )
pair = sort_lowercase(local_pubkey_66hex, peer_pubkey_66hex) concatenated
media_key     = HKDF-SHA256(salt = call_id, ikm, info = "ghal_bol_call_media_v1" || pair)
ratchet_salt  = HKDF-SHA256(salt = call_id, ikm, info = "ghal_bol_call_media_ratchet_v1" || pair)
```

Video uses a distinct HKDF `info` (e.g. `ghal_bol_call_video_v1`) — see [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md).

Rust: `ghal_bol/src/call_media_key.rs`, `call_media/crypto.rs`, FFI `ghal_bol_ffi_p2p_call_media` / `p2p_call_video`.

---

## Non-goals

- Group calls
- Server-side SFU
- Call history in transcript (optional later)
