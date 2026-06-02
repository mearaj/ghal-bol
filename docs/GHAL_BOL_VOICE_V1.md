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
- Uses **OS default** mic/speaker (no `selectAudioOutput` / device pinning that forced Bluetooth hands-free on Linux).
- SDP offer/answer includes **gathered ICE candidates** (LAN-friendly when trickle over DM is slow).
- Voice calls still need a hidden **`RTCVideoView`** on the remote renderer for GTK audio output.
- Remote **audio + video** tracks merge into one `MediaStream` on `onTrack` (unified-plan fires twice); the remote renderer is re-bound when a video track arrives so desktop/mobile show peer video after renegotiation.
- UI shows **connected** only after **ICE connected** or remote audio track — not on SDP alone.

**Debug:** More → **App log** → filter **Calls**. Lines use `[Call]`, `[Call/WebRTC]`, `[Call/Media]` (`AppLog.logCallFlow`, independent of journey toggles). Look for `wire_rx signal=sdp_answer`, `ice_connected`, `remote_track`.

---

## Non-goals (v1)

- Group calls
- Server-side SFU
- Call history in transcript (optional later)
