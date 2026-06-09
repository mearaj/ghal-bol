# Ghal Bol calls — `ghal_bol_call_v1`

**Status:** Signaling implemented in Rust; media (WebRTC) in Flutter. One call UX: **voice by default**, optional **video during the call**.

Read [DESIGN.md](DESIGN.md) and [AGENTS.md](../AGENTS.md): **Rust owns signaling** on the DM stream; Flutter owns **capture/render** (WebRTC) and UI only.

---

## Product model

| Rule | Detail |
|------|--------|
| **One call type** | No separate “voice call” vs “video call” buttons. Tap **Call** → audio call. |
| **Default media** | Audio only (`media: "audio"` on invite). |
| **Video** | In-call **Video** toggle → `video_on` + WebRTC renegotiation (`sdp_offer` / `sdp_answer`). |
| **Turn off video** | **Video off** → `video_off`; camera off, audio continues. |
| **Signaling transport** | Same libp2p DM stream as chat (`ghal_bol_call_v1` frames). |
| **Media transport** | WebRTC (UDP); STUN for NAT; TURN optional later. |
| **Media encryption** | DTLS-SRTP (WebRTC default) **plus** identity-bound **FrameCryptor** AES-GCM on encoded RTP (key from Rust `derive_call_media_key`). |

---

## Signaling kinds (`call_sig_v1`)

| Wire name | When |
|-----------|------|
| `invite` | Outbound ring; payload `{ "media": "audio" }` (or `"audio_video"` if ever needed). |
| `accept` | Callee accepts. |
| `reject` | Decline. |
| `hangup` | End call. |
| `sdp_offer` | WebRTC SDP offer (`payload.sdp`, `payload.type`). |
| `sdp_answer` | WebRTC SDP answer. |
| `ice` | Trickle ICE (`payload.candidate`, …). |
| `video_on` | Request in-call video (renegotiate). |
| `video_off` | Disable video track; audio continues. |

All envelopes use `ghalbol.share = ghal_bol_call_v1`, `ref_id` = `call_id`, encrypted inner JSON, secp256k1 signature (see `ghal_bol/src/call_sig_v1.rs`).

---

## Call phases (Rust `call_state`)

| Phase | Meaning |
|-------|---------|
| `idle` | No call with this contact. |
| `outgoing_ringing` | We sent `invite`. |
| `incoming_ringing` | Peer sent `invite`. |
| `connected` | Accepted; SDP/ICE may still be in progress. |

`video_on` / `video_off` / SDP / ICE are only valid when a call exists for that `call_id`.

---

## Flutter API

- **Outbound:** `GhalBolCall.sendSignal(...)` → native `p2p_call_signal` (FFI or daemon).
- **Inbound:** poll `kind: call_signal` → `CallController` drives UI + WebRTC.
- **Do not** send call signals from Dart except through `GhalBolCall` (state checks stay in Rust for outbound).

---

## Permissions

- **Microphone** — required for all calls.
- **Camera** — only when user enables video.

---

## Desktop media (Linux / Windows / macOS)

- Flutter **WebRTC** only; Rust does not capture/play audio.
- **Desktop:** ringtone unchanged. Callee opens mic with flutter_webrtc `optional` + `sourceId` (non-HFP), never top-level `deviceId`. Local capture before `setRemoteDescription`; defer `RTCVideoRenderer` remote bind until mic is open (`call_webrtc.dart`, `call_desktop_media.dart`).
- **Mobile:** no `setSpeakerphoneOn` at call start; `forceHandleAudioRouting: false`. User can toggle speaker in-call.
- SDP offer/answer includes **gathered ICE candidates** (LAN-friendly when trickle over DM is slow).
- Voice calls still need a hidden **`RTCVideoView`** on the remote renderer for GTK audio output.
- Remote **audio + video** tracks merge into one `MediaStream` on `onTrack` (unified-plan fires twice); the remote renderer is re-bound when a video track arrives so desktop/mobile show peer video after renegotiation.
- UI shows **connected** only after **ICE connected** or remote audio track — not on SDP alone.
- **Ring / ringback:** bundled WAV loops via `call_ringtone.dart` (incoming on invite, outgoing after `invite` sent); stops when media connects or call ends. Android incoming also vibrates.
- **Desktop capture:** echo cancellation / noise suppression enabled when a non–hands-free mic is pinned; left off for system default (avoids forcing Bluetooth HFP).

**Debug:** More → **App log** → filter **Calls**. Lines use `[Call]`, `[Call/WebRTC]`, `[Call/Media]` (`AppLog.logCallFlow`, independent of journey toggles). Look for `wire_rx signal=sdp_answer`, `ice_connected`, `remote_track`, `media_e2ee_ready`, `media_e2ee_identity` (shows peer pubkey prefix), `media_e2ee_tx` / `media_e2ee_rx` with `decrypt_ok`.

**In-call UI:** When media keys are active, the call screen shows a green lock chip: **End-to-end encrypted · contact key** `03a1f2b3…`. If E2EE setup fails, the call still connects on DTLS-SRTP (no chip).

**Connect latency:** Key derivation runs in parallel with `getUserMedia`; E2EE failure does not block the call. Remote track encrypt/decrypt attach is non-blocking.

---

## Media encryption (E2EE)

| Layer | What it protects |
|-------|------------------|
| **DM signaling** (`ghal_bol_call_v1`) | Invite, SDP, ICE — sealed to peer secp256k1 + signed. |
| **libp2p transport** | Noise on the stream; peeker on LAN/WAN sees encrypted bytes, not JSON. |
| **DTLS-SRTP** | Standard WebRTC media encryption between the two UDP endpoints. |
| **FrameCryptor** (`call_media_e2ee.dart`) | Second AES-GCM layer on **audio and video** RTP before SRTP; same identity-derived key; key is **not** in SDP. |

**Key derivation (both peers, same result)** — uses the same **66-hex secp256k1** identity as chat (private key in keystore, public key on the contact):

```text
ikm = SHA256( ECDH(my_identity_secret, peer_public_key_66hex) )   // same mixing as DM seal
pair = sort_lowercase(local_pubkey_66hex, peer_pubkey_66hex) concatenated
media_key     = HKDF-SHA256(salt = call_id, ikm, info = "ghal_bol_call_media_v1" || pair)
ratchet_salt  = HKDF-SHA256(salt = call_id, ikm, info = "ghal_bol_call_media_ratchet_v1" || pair)
```

Rust: `ghal_bol/src/call_media_key.rs` (`derive_call_media_keys_from_identity`), FFI `ghal_bol_ffi_call_media_key_hex` (requires unlocked identity). Flutter enables FrameCryptor on each RTP sender/receiver after local/remote tracks attach.

**Threat model notes:** A passive network observer still sees UDP timing/volume; a malicious **TURN/SFU** (not used in v1) could not decode frames without the media key. Compromised **local device** or OS mic path is out of scope. Rebuild native (`sync_ghal_bol_native_for_flutter.sh` / Android pack) after pulling this API.

---

## Non-goals (v1)

- Group calls
- Server-side SFU
- Call history in transcript (optional later)
