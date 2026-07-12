# Ghal Bol video calls — native video over the P2P link

**Status:** **Shipping** on Linux desktop and Android when both peers negotiate `video_engine: native_v1`. Display/capture polish (HW codec, adaptive bitrate) is in progress — see § Implementation status.

**Read first:** [AGENTS.md](../AGENTS.md) (golden rules — esp. #1 Rust owns product
logic, #7 identity E2EE), [DESIGN.md](DESIGN.md), [TRANSPORT.md](TRANSPORT.md) (libp2p
stack), and [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) (the shipping
native **voice** engine this extends). This doc reuses that engine's pipeline,
crypto, transport, and FFI patterns — video is an **additional media track**, not a
new stack.

---

## Goal & principles

Smooth, WhatsApp-grade **video** where the media engine lives in **Rust** and rides
the **direct peer connection we already establish**. Same identity, same crypto, same
transport as native voice.

| Principle | Consequence for video |
|-----------|-----------------------|
| **Rust owns media** (golden rule #1) | Capture→encode→seal→transport→jitter→decode happen in Rust; Flutter does camera-permission UI, the local/remote render surface, and toggle buttons via FFI. |
| **Reuse the link** | Video frames ride the **same libp2p connection** as chat + voice (a media substream), so we inherit NAT traversal, relay `/p2p-circuit` fallback, Noise, coord discovery, urgent-reconnect, keepalive, and the **LAN-shift** work in `chat_server.rs`. |
| **One E2E story** (golden rule #7) | Video frames are sealed with the transport media key (`derive_call_media_keys_from_transport`). Per-frame AES-GCM; key never on the wire. |
| **No new servers** | Direct when possible; relay (coord/libp2p) only as fallback — identical to chat and voice. |
| **Truthful UI** | "Video connected" only when decoded frames actually flow (mirrors voice + DESIGN.md truthful-status rule). |

**Why not MoQ:** see [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) § "Why not MoQ". For 1:1 interactive video we want the existing peer link + an unreliable-friendly media path, not a pub/sub broadcast protocol.

---

## What we reuse vs add

| Piece | Native voice today | Native video (this doc) |
|-------|--------------------|-------------------------|
| **Signaling** (`invite`/`accept`/`hangup`, `call_id`, phases) | `call_sig_v1.rs`, `call_state.rs` over the DM stream | **Reuse.** Add `video_engine: native_v1` negotiation; `video_on`/`video_off` + `key_request` (keyframe) signals. |
| **Identity media key** | `call_media_key.rs` `derive_call_media_keys_from_transport` | **Reuse**, with a distinct HKDF `info` for the video stream (key separation from audio — see § E2E). |
| **Per-frame crypto** | `call_media/crypto.rs` `MediaCrypto` (AES-256-GCM, `dir‖counter‖ct`) | **Reuse the construction**; separate `MediaCrypto` instance/counter per media kind so audio and video never share a nonce space. |
| **Transport** | `/ghal-bol/call/1.0.0` substream (`chat_server.rs`) | **New `/ghal-bol/call-video/1.0.0` substream** on the **same** connection (parallel to audio so a large video frame never head-of-line-blocks a 20 ms audio packet). Same two-streams-per-call (TX open / RX accept) + length-prefix framing + drop-oldest backpressure. |
| **Session lifecycle** | `call_media/session.rs`, `MediaControls`, `OutboundCmd::CallMedia*` | **Extend** `MediaControls` with camera on/off + a `VideoCodec` path; add `OutboundCmd::CallVideoStart/Stop/SetCameraEnabled`. |
| **Codec** | `call_media/codec.rs` `AudioCodec` (Opus) | **New `VideoCodec` trait** + encoder/decoder (see § Codec). |
| **Jitter / sync** | `call_media/jitter.rs` (20 ms audio frames) | **New frame-aware video jitter** (keyframe/delta aware) + A/V sync using `MediaFrame.ts` (already reserved, `mod.rs`). |
| **Capture / render** | `call_media/audio_device.rs` (cpal/Oboe) | **New camera capture** + **render-to-Flutter-texture** platform layer (see § Platform). |
| **FFI / poll** | `p2p_call_media`, `GhalBolP2p.callMedia*`, `call_media` log stats | **Add** `p2p_call_video` + `GhalBolP2p.callVideo*`; reuse poll/stats pattern. |
| **Call UI** | `call_screen.dart`, `call_controller.dart` | **Reuse**; local PiP + remote view bind to a **native video texture** (`NativeCallVideoView`). |

---

## Architecture

```text
┌───────────────────────── ghal_bol_ui (Flutter) ─────────────────────────┐
│ Call UI, camera-permission prompt, local PiP + remote video surface      │
│ bound to a native Texture (Texture id from FFI); video on/off toggle      │
│ Calls native via ghal_bol_core_ffi_call_video_* ; renders state from poll      │
└───────────────────────────────┬─────────────────────────────────────────┘
                                 │ FFI / daemon RPC (control + texture id only)
┌───────────────────────────────▼─────────────────────────────────────────┐
│ ghal_bol (Rust)                                                          │
│  call_sig_v1 / call_state   — signaling on the DM stream (unchanged)     │
│  call_media (voice, shipping) — audio pipeline + /ghal-bol/call/1.0.0     │
│  call_video (NEW)            — video pipeline + session                   │
│    camera capture → scale → encode(keyframe/delta) → seal → SEND          │
│    RECV → unseal → reorder/jitter(keyframe-aware) → decode → render       │
│    A/V sync via MediaFrame.ts vs the audio clock                          │
│  call media keys            — derive_call_media_keys_from_transport        │
│  chat_server.rs             — the peer connection + /ghal-bol/call-video  │
│                               substream (parallel to audio + DM)          │
└──────────────────────────────────────────────────────────────────────────┘
```

Camera capture + the render surface are platform code; **encode/decode/jitter/sync/
crypto/transport/packetization are Rust** and shared across platforms.

---

## Media pipeline (video)

```text
TX:  camera frame (NV12/I420) ─▶ scale/rotate to negotiated WxH@fps
        ─▶ encode (intra=keyframe / inter=delta, target bitrate from CC)
        ─▶ packetize (fragment > MAX_MEDIA_FRAME_BYTES into ordered chunks)
        ─▶ seal each chunk (identity video key) ─▶ packet{frame_seq, chunk_idx,
           chunk_cnt, flags(keyframe), ts, payload} ─▶ transport.send (drop-oldest)

RX:  transport.recv ─▶ unseal ─▶ reassemble frame (by frame_seq) ─▶ video jitter
        (reorder, bounded delay, **drop incomplete delta frames**, hold for keyframe
         after loss, request keyframe if stalled) ─▶ decode (skip undecodable until
         next keyframe) ─▶ render to texture; A/V sync against audio playout clock
```

**Defaults to validate, then adapt** (start conservative, scale with CC):

| Knob | Start | Notes |
|------|-------|-------|
| Resolution | 480p (640×480) | negotiate min(caller, callee) caps; raise to 720p when CC allows |
| Frame rate | 20–24 fps | adaptive; drop to 12–15 on congestion |
| Bitrate | 300–800 kbps | congestion-controlled (§ Congestion) |
| Keyframe interval | ~2 s + on-demand | `key_request` signal forces an immediate IDR after loss/join |
| Jitter target | 100–200 ms | larger than audio; video tolerates more buffering than glitching |

The **audio path is unchanged and independent** — voice stays on `/ghal-bol/call/1.0.0`
with its own 8-slot jitter buffer; video runs on its own substream so a big I-frame
never delays an audio packet.

---

## Codec

The codec layer is the part where 2026 Rust has caught up hard — there are now **proven, hardware-accelerated, FFmpeg-free** codec crates, so we do **not** hand-roll a codec.

| Concern | Plan (proven crates, June 2026) |
|---------|------|
| **Codec choice** | Negotiate **best mutually-supported codec** per call: prefer **AV1** (royalty-free, best quality/bitrate) where HW or fast SW is available; **H.264** as the universal fallback (HW encode on essentially every phone). The `VideoCodec` trait abstracts this so codecs slot in. |
| **HW-accelerated codec crates** | **`cros-codecs`** (Google/ChromeOS; VAAPI + V4L2 **H.264/HEVC/VP8/VP9/AV1 decode**, **H.264/VP9/AV1 encode**; ~1.1M downloads, production in crosvm — the mature Linux pick). **`yscv-video`** (2026, **pure-Rust, no FFmpeg**: H.264/HEVC/AV1 decode + HW via **VideoToolbox/VAAPI/NVDEC/MediaFoundation** + **`nokhwa` camera** in one crate — strong cross-platform pick). **`rav1e`** (AV1 encode) + **`rav1d`** (memory-safe `dav1d` AV1 decode) and **`openh264`** for SW/portable paths. |
| **HW acceleration by platform** | **Android:** `MediaCodec` (Surface in/out) — essential for battery/thermals; reachable via `yscv-video` MediaFoundation-equivalent path is Windows-only, so on Android drive `MediaCodec` directly (JNI, like the audio Oboe path) or via `cros-codecs` V4L2 where the SoC exposes it. **iOS:** VideoToolbox (via `yscv-video` or direct). **Linux/desktop:** `cros-codecs` VAAPI (Intel/AMD) / `yscv-video` NVDEC (NVIDIA), SW `rav1e`+`rav1d`/`openh264` fallback. |
| **Reference** | The iroh team's **`rusty-codecs`** (part of `iroh-live`) already wraps **H.264 (openh264) + AV1 (rav1e/rav1d) + Opus** with **VAAPI/V4L2/VideoToolbox HW accel and `wgpu` rendering** behind one API — a strong reference (and possible direct dependency) for the exact `VideoCodec`/render abstraction we need. |
| **Trait shape** | Mirror `AudioCodec` (`codec.rs`): `VideoEncoder { encode(&mut self, frame: &VideoFrame, force_keyframe: bool) -> Vec<EncodedChunk> }`, `VideoDecoder { decode(&mut self, chunk: Option<&[u8]>) -> Option<VideoFrame> }` with a `NullVideoCodec` for tests (passthrough), exactly like `NullCodec`. Each concrete impl wraps `cros-codecs` / `yscv-video` / `rav1e`+`rav1d`. |
| **Resilience** | Long-term-reference / temporal layering if the chosen codec supports it; otherwise periodic keyframes + keyframe-on-request. **No inter-frame decode across an unrecovered loss** — wait for the next keyframe instead of showing corruption. |

---

**Transport anchor:** video rides the **same proven model the voice engine started
from** — Opus/media over a **direct peer QUIC connection**, with **each media track on
its own channel** so a dropped video packet never blocks audio (the `voicemcu` /
`proscenium` / `iroh-live` insight). We deliberately do **not** adopt a MoQ relay/CDN;
we keep MoQ's *per-track-independent-stream* idea but run it over **our own** direct
libp2p/QUIC connection. Two phases, same as voice's Option A → Option B:

- **Phase-1 transport — libp2p substream `/ghal-bol/call-video/1.0.0`** — a third
  protocol on the same libp2p connection (`chat_server.rs`), opened/accepted exactly
  like `/ghal-bol/call/1.0.0` (CALL_STREAM_PROTOCOL): each side **opens TX** and
  **accepts RX**, first message is a `{"call_id"}` header, then length-prefixed sealed
  packets. Register in a `SessionState::call_video` map keyed by `call_id` (parallel to
  `call_media`), peer-verified inbound. Keeps a large I-frame off the audio + DM streams
  (no HOL blocking of a 20 ms voice packet or an ack); audio + DM behaviour untouched.
- **Phase-2 transport — raw QUIC unreliable datagrams (`quinn`, RFC 9221)** — the
  **preferred end state for video**, even more than for voice: video is loss-tolerant
  and frames are large, so retransmits (reliable streams) are exactly what causes
  freeze/HOL stalls. A parallel `quinn` connection (addresses learned from
  coord/libp2p) carries **audio datagrams + video datagrams as separate flows** — the
  `voicemcu` recipe (capped datagram size vs MTU, CBR/bitrate ceiling) extended with
  fragmentation for video. Control/signaling stays on the reliable DM stream. See
  [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) § "Transport decision".
- **Packetization:** the substream frame cap is `MAX_MEDIA_FRAME_BYTES` (64 KiB,
  `chat_server.rs`) and a UDP datagram is far smaller, so fragment each encoded frame
  into ordered chunks `{frame_seq, chunk_idx, chunk_cnt, keyframe, ts}`; reassemble on
  RX; drop the whole frame if any chunk is missing (then request a keyframe if it was a
  reference frame).
- **Backpressure = drop-oldest** (same as voice `session.rs`): the encoder must never
  block on a slow link; a fresh frame supersedes a stuck one. Prefer dropping delta
  frames over keyframes.

---

## Congestion control & adaptive quality

A 1:1 link still needs basic congestion control or video will bufferbloat the audio.

- **Signals:** per-substream RTT + loss + send-queue depth (drop-oldest count) +
  jitter, sampled like the voice `call_media` stats (every ~1–3 s).
- **Controller:** start with a simple **AIMD / GCC-lite** loop — increase target
  bitrate while loss is low and the queue is shallow; multiplicative back-off on a
  loss/queue spike. Map bitrate → encoder target + resolution/fps ladder steps.
- **Keyframe on recovery:** after a back-off or detected unrecovered loss, the RX side
  emits a `key_request` signal; the TX encoder forces an IDR so the picture recovers
  fast instead of staying frozen.
- **Audio priority:** under contention, **audio wins** — video bitrate/fps drop first;
  audio (its own stream + jitter buffer) is never starved.

---

## A/V sync

- Each `MediaFrame` already carries a `ts` field reserved "for A/V sync + diagnostics"
  (`call_media/mod.rs`). Stamp audio and video frames from a **shared monotonic call
  clock** at capture.
- The video jitter buffer releases a frame to the decoder/renderer aligned to the
  **audio playout clock** (audio is the master clock — humans tolerate video lag more
  than audio glitches). Bounded skew correction: drop/duplicate video frames rather
  than stretch audio.

---

## End-to-end encryption

- Video key derives from **`derive_call_media_keys_from_transport(call_id, transport_kem)`**
  as voice/chat (golden rule #7), but with a **distinct HKDF `info`** (e.g. `call_id:video`)
  so the video stream key ≠ audio stream key — separate keys + separate `MediaCrypto`
  counters guarantee no AES-GCM nonce reuse across the two media kinds.
- Each chunk is sealed with `MediaCrypto` (`dir‖counter‖ciphertext`) before it hits the
  substream; the substream is also Noise-encrypted peer-to-peer (defense in depth — a
  relay on the path never sees plaintext). Key never on the wire.
- Keys derive in parallel with camera open; **failure to derive must fail the video
  E2E or surface it — never silently fall back to plaintext video** (golden rule #7).
  Audio and the call itself continue regardless.

---

## Signaling changes

Reuse `call_sig_v1.rs` / `call_state.rs`. Native video signaling:

| Kind | Native v1 meaning |
|------|-------------------|
| `invite` / `accept` | Carry inner JSON `voice_engine`/`video_engine` capability tags (alongside `media`). Both sides supporting `video_engine: native_v1` ⇒ native video is available in-call. |
| `video_on` | Start the native video substream + camera; payload negotiates `{w,h,fps,codec}` caps (min of both sides). |
| `video_off` | Stop camera + tear down the video substream; voice continues. |
| `key_request` (**new**) | RX → TX: force an immediate keyframe (after loss / late join). |

`call_state.rs` tracks `video_enabled` per call and which **video engine** is active.
Negotiation mirrors `voice_engine: native_v2` in `call_controller.dart` (advertise →
parse remote → decide `_willUseNativeVideo`).

---

## Control / FFI surface (additions)

Reuse the `ghal_bol_core_ffi_p2p_call_media` / `p2p_call_media` pattern. Dart sends
**control only**; media + pixels never cross FFI as buffers — only a **texture id**
(or platform surface handle) does.

| FFI (proposed) | Purpose |
|----------------|---------|
| `ghal_bol_core_ffi_p2p_call_video` (action `start`/`stop`/`set_camera_enabled`/`switch_camera`) | Begin/end the video session + camera for `call_id` (after `accept` + `video_on`). |
| `ghal_bol_core_ffi_call_video_remote_texture` | Returns the platform **texture id** the engine renders the decoded peer frames into; Flutter shows it in a `Texture(textureId:)`. |
| `ghal_bol_core_ffi_call_video_local_texture` | Local camera preview texture id (or Flutter renders preview natively from the camera plugin — TBD per platform). |
| (poll) `call_video_connected` / `call_video_stats` / `call_video_failed` | Truthful UI: "video connected" only on real decoded-frame flow; stats = rtt/loss/fps/bitrate/resolution/e2e-active. |

---

## Platform capture / render (the real platform work)

Rust owns codec/jitter/sync/crypto/transport/packetization on all platforms;
**camera capture, hardware codec, and the render surface** need per-OS integration:

| Platform | Camera capture | HW codec | Render to Flutter |
|----------|----------------|----------|-------------------|
| **Android** | Camera2 / CameraX → `Surface`/`ImageReader` (NV12) | `MediaCodec` (Surface in/out) | Decode into a `SurfaceTexture` → Flutter external `Texture` |
| **iOS** | `AVCaptureSession` (`CVPixelBuffer`) | VideoToolbox (`yscv-video`) | `CVPixelBuffer` → Flutter `Texture` (Metal/CVMetalTexture) |
| **Linux/desktop** | `nokhwa` (V4L2) — cpal's video analog | `cros-codecs` VAAPI / `yscv-video` NVDEC; SW `rav1e`+`rav1d`/`openh264` | Decode → BGRA/`wgpu` → Flutter `Texture` (pixel-buffer registrar) |

**Crates that shrink this work:** **`nokhwa`** gives cross-platform camera capture on
desktop (and is the camera path inside `yscv-video`); the iroh team's **`rusty-capture`**
(part of `iroh-live`) already does cross-platform camera + screen capture (PipeWire,
V4L2, X11, ScreenCaptureKit/AVFoundation) and **`rusty-codecs`** pairs HW decode with
**`wgpu`** rendering — both are reference (or direct-dependency) candidates. Even so this
is the **largest chunk of new platform code** and the main quality/perf risk (camera
orientation, HW codec quirks, zero-copy texture paths, thermals). Mirror the voice
engine's split: a small `VideoBackend` trait (`start`/`stop`/`on_frame(cb)` for capture,
`present(frame)` for render) with platform impls and a `NullVideoBackend` for headless
tests.

---

## Phased rollout

| Phase | Scope | Exit criteria |
|-------|-------|---------------|
| **V0** | `VideoCodec` trait + `NullVideoCodec` + packetizer/reassembler + video jitter (keyframe-aware) + `MediaCrypto` video key — **engine core, no camera/transport**, unit-tested like the voice P0 | Round-trip + reorder/loss/keyframe-recovery unit tests pass (mirrors `call_media` tests) |
| **V1** | `/ghal-bol/call-video/1.0.0` substream in `chat_server.rs` + SW codec (H.264) + **desktop** camera (`nokhwa`) + render texture; CLI/test harness | Two Linux desktops hold a clear 2-way video call over a real libp2p link; measured rtt/fps/bitrate; audio stays native voice |
| **V2** | Wire V1 into `call_controller`/`call_screen` via FFI on **Linux** (native texture); congestion control + `key_request` | Linux↔Linux native video |
| **V3** | **Android** Camera2 + `MediaCodec` HW encode/decode + SurfaceTexture render; switch-camera; route toggles | Android↔Android and Android↔Linux video solid on Wi-Fi + cellular within battery/thermal budget |
| **V4** | **iOS** AVFoundation + VideoToolbox | iOS interop |
| **V5** | Option B (QUIC datagrams) for video if substream HOL hurts on lossy cellular | Lower jitter / faster recovery on lossy links |

---

## Risks & open questions

- **HW codec variance** (Android `MediaCodec` across vendors; color formats, latency).
- **Zero-copy render** to a Flutter `Texture` on each platform (avoid per-frame CPU copies/thermals).
- **Congestion control quality** for 1:1 (GCC-lite vs something more principled).
- **Battery/CPU/thermal** of camera + encode + decode + render on mobile.
- **Substream HOL** under cellular loss (mitigation: drop-oldest, keyframe-on-request; escape hatch: Option B datagrams).
- **Build size / NDK**: adding a video codec (+ HW codec glue) to the Android `:p2p` libs.

## Non-goals (v1)

- Group video / SFU.
- Screen share, recording, virtual backgrounds.
- Browser/web video (FFI is native).
- Replacing video on all platforms at once (phased; iOS not started).
- MoQ.

---

## Implementation status

| Phase | State |
|-------|-------|
| **V0 engine core** | **Done.** `ghal_bol_core/src/call_video/` — `RawVideoFrame`/`EncodedVideoFrame`, `VideoEncoder`/`VideoDecoder` traits + `NullVideoCodec`, frame **fragmentation/reassembly** (`packet.rs`, `Reassembler`), **keyframe-aware jitter** (`jitter.rs`, `VideoJitter` — waits for a keyframe, jumps to the next keyframe on a gap, raises a throttled `key_request`), and per-chunk **identity AES-256-GCM seal** reusing `call_media::MediaCrypto` with a distinct video key. `VideoEngine` drives `on_capture → wires`, `on_wire → reassemble+jitter`, `on_render → decoded frame`. **7 unit tests** (multi-chunk round-trip, out-of-order chunks, in-order render, single-frame loss recovery, keyframe-gating + request, tamper/wrong-key rejection). Unwired — does not touch the shipping voice/chat/networking paths. |
| **V1 codec** | **Done (desktop).** `call_video/codec_h264.rs` — `H264Encoder`/`H264Decoder` over **OpenH264** (`openh264` 0.9, bundled source built via `cc`; realtime config: 2 Mbps / 30 fps, accurate keyframe flag via Annex-B NAL scan). I420 in/out. `VideoEngine::new_h264()`. **4 tests** incl. a **full two-engine pipeline** (encode → fragment → seal → wire → reassemble → keyframe-aware jitter → decode). Desktop-gated; Android codec (`MediaCodec` / cross-built openh264) lands in its phase so the Android build stays untouched now. |
| **V1 transport + capture/render** | **Done (Linux desktop + Android).** `/ghal-bol/call-video/1.0.0` substream, `p2p_call_video` + `p2p_call_video_frame` FFI/daemon RPC. **Desktop:** `nokhwa` camera → I420. **Android:** Camera2 in `:p2p` (`AndroidVideoCapture.kt`) → JNI → Rust; OpenH264 cross-built via NDK (same bitstream as desktop). Flutter `NativeCallVideoView` pulls frames over daemon RPC on Android (UI process) or FFI on Linux. `CallController` negotiates `video_engine: native_v1` on **both** desktop and Android. |
| **V2 Android HW codec** | **Optional next.** `MediaCodec` H.264 encode/decode behind the same trait for lower CPU/battery (OpenH264 SW already ships). |
| **V2 Linux wiring** | **Done.** Native video when both peers advertise `native_v1` (Linux↔Linux, Linux↔Android, Android↔Android). |
| **V3 Android / V4 iOS / V5 datagrams** | Not started. |

### Flutter video textures and call end (Linux + Android)

| Rule | Detail |
|------|--------|
| **Register** | `NativeCallVideoView` → `GhalBolP2p.callVideoTexture` → `CallVideoTextureBridge.register` (shm RGBA). Pooled per `(call_id, track)` in `CallVideoTexturePool`. |
| **Release on hangup only** | `CallVideoTexturePool.releaseCall(call_id)` from `CallController._endLocal` / `_stopNativeCallIfStillActive` **after** `callVideoStop`. **`releaseWidget` is a no-op** — do not release on widget dispose/rebuild (PiP swap, route pop); that caused Linux Flutter **SIGSEGV** mid-call. |
| **End order** | Stop UI phase → `callMediaStop` / `callVideoStop` → `CallDesktopNativeCamera.stop` → `releaseCall` → optional async `hangup` — UI must not block on RPC. |
| **Privacy** | Same as voice: UI gone → `p2p_force_end_active_call` (daemon / `:p2p`). See [DESIGN.md](DESIGN.md) § “Call UI lifecycle and privacy”. |

**Current shipping reality:** native **voice** (`native_v2`) + **native video** (`native_v1`) when both peers negotiate the tags. Voice detail: [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md).

## References (June 2026)

- **Native voice engine + transport/E2E/phasing rationale:** [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) (this video plan reuses its identity key, per-frame seal, substream/datagram transport, and FFI patterns).
- **HW-accelerated Rust video codecs (no FFmpeg):** **`cros-codecs`** (Google/ChromeOS, VAAPI/V4L2 H.264/HEVC/VP8/VP9/AV1 dec + H.264/VP9/AV1 enc, ~1.1M downloads), **`yscv-video`** (pure-Rust H.264/HEVC/AV1 + VideoToolbox/VAAPI/NVDEC/MediaFoundation HW + `nokhwa` camera), **`rav1e`** (AV1 enc) + **`rav1d`** (safe `dav1d` AV1 dec), **`openh264`** (H.264). Android `MediaCodec` / iOS VideoToolbox for mobile HW.
- **Camera / capture:** `nokhwa` (desktop), the iroh team's **`rusty-capture`** (PipeWire/V4L2/X11/ScreenCaptureKit/AVFoundation), Camera2/CameraX (Android), AVFoundation (iOS).
- **Reference architecture (full P2P A/V pipeline in Rust over QUIC):** **`iroh-live`** (n0-computer) and its `moq-media` (capture/encode/decode/playout + **adaptive bitrate**), `rusty-codecs` (codecs + HW + `wgpu` render), `rusty-capture` — validates the per-track-stream + adaptive-bitrate design we adopt over our own direct connection.
- **Proven 1:1 P2P voice references the audio engine started from:** `voicemcu` (Opus over **unreliable QUIC datagrams**, `quinn`), `proscenium` (P2P voice over a dedicated QUIC protocol).
- **Transport shape:** libp2p substream (Phase 1) / `quinn` **QUIC unreliable datagrams** RFC 9221 (Phase 2, preferred end state). **Congestion:** GCC-lite / AIMD for 1:1; `moq-media` adaptive bitrate as reference.
