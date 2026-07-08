# Ghal Bol calls — native voice over the P2P link

**Status:** **Shipping** on Linux desktop and Android when both peers negotiate `voice_engine: native_v2`. iOS not started.

**Goal:** Voice where the media engine lives in **Rust** and rides the **direct peer connection we already establish** — no separate STUN/TURN/SDP stack, no MoQ. One identity, one crypto story, one transport.

Read first: [AGENTS.md](../AGENTS.md) (golden rules), [DESIGN.md](DESIGN.md) (layers + E2E), [TRANSPORT.md](TRANSPORT.md) (libp2p stack), [GHAL_BOL_VOICE_V1.md](GHAL_BOL_VOICE_V1.md) (signaling). Video: [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md).

---

## Why native over the existing link

| Driver | Detail |
|--------|--------|
| **Golden rule #1** | `ghal_bol` (Rust) owns product logic — capture, codec, transport, jitter, and E2E live in Rust (`call_media/`). |
| **We already have the link** | libp2p provides encrypted connections (TCP/QUIC + Noise + relay + coord). Calls reuse that link via `/ghal-bol/call/1.0.0`. |
| **No new servers** | Direct when possible; coord/relay only as fallback — same as chat. |
| **One E2E story** | `derive_call_media_keys_from_transport` + per-frame AES-GCM seal (transport KEM after `TransportKemHello`). |

**Why not MoQ.** Media-over-QUIC broadcast protocols target one-to-many fan-out, not 1:1 interactive calls. Not used here.

---

## Architecture pieces

| Piece | Implementation |
|-------|----------------|
| **Signaling** | `call_sig_v1.rs`, `call_state.rs`, `call_ffi.rs` over the DM stream |
| **Media key** | `call_media_key.rs` (`derive_call_media_keys_from_transport`) |
| **Media engine** | Rust: capture → APM → Opus → seal → transport → jitter → decode → playback |
| **Media transport** | `/ghal-bol/call/1.0.0` on the existing libp2p peer connection |
| **Flutter** | `call_controller.dart`, `call_screen.dart`, `call_ringtone.dart` — UI + FFI control only |

---

## Architecture

```text
┌───────────────────────── ghal_bol_ui (Flutter) ─────────────────────────┐
│ Call UI, ring/back tones, mute/speaker/video toggles, device picker      │
│ Calls native via ghal_bol_ffi_call_* ; renders state from poll events    │
└───────────────────────────────┬─────────────────────────────────────────┘
                                 │ FFI / daemon RPC (control only)
┌───────────────────────────────▼─────────────────────────────────────────┐
│ ghal_bol (Rust)                                                          │
│  call_sig_v1 / call_state   — signaling on the DM stream (unchanged)     │
│  call_media (NEW)           — pipeline + jitter + session lifecycle      │
│    capture → APM(AEC/NS/AGC) → Opus enc → seal → SEND                    │
│    RECV → unseal → jitter buffer → Opus dec(PLC) → mix → playback        │
│  call media keys            — derive_call_media_keys_from_transport       │
│  chat_server.rs             — the peer connection + a media substream    │
│                               (/ghal-bol/call/1.0.0) or QUIC datagrams   │
└──────────────────────────────────────────────────────────────────────────┘
```

Audio device I/O is platform code (below), but **encode/decode/jitter/crypto/
transport are all Rust** and shared across platforms.

---

## Media pipeline (voice)

Per 20 ms frame (48 kHz mono, Opus):

```text
TX:  mic frame ─▶ APM (echo-cancel using the speaker/render reference,
                       noise-suppress, auto-gain) ─▶ Opus encode (FEC+DTX)
                  ─▶ seal (identity media key) ─▶ packet{seq,ts,payload}
                  ─▶ transport.send (unreliable-preferred)

RX:  transport.recv ─▶ unseal ─▶ jitter buffer (reorder, ~100–160 ms,
                       drop stale, Opus PLC on gaps) ─▶ Opus decode
                  ─▶ playback ring buffer ─▶ speaker
```

Defaults to validate, then tune: 48 kHz mono; 20 ms frames; Opus 16–40 kbps VBR
with **in-band FEC** + **DTX**; jitter buffer 8 slots (160 ms) adaptive; AEC3
analysis fed the render (playback) stream as the echo reference.

**Crates (proven 1:1-P2P-over-QUIC references, June 2026): `voicemcu`, `proscenium`, `aura`, `occupyashanti/echo`.** All four run **Opus over a direct QUIC connection** (datagrams or length-prefixed streams) with a small jitter buffer + Opus PLC — i.e. exactly this engine's shape. The Ghal Bol voice engine was built from this pattern (`audiopus` + `cpal` + `ringbuf` + in-house jitter + per-frame AES-GCM).

| Concern | Crate | Notes |
|---------|-------|-------|
| Codec | `audiopus` / `opus` (libopus) | FEC, DTX, PLC built in. **Shipped.** |
| AEC / NS / AGC | **`sonora`** (pure-Rust AEC3 + NS + AGC2, [crates.io](https://crates.io/crates/sonora) v0.1.0, Feb 2026, MSRV 1.91) — optional C++ APM bindings fallback | `sonora` is benchmarked at **C++ parity** (≈13 µs per 48 kHz mono frame) and is **pure Rust → no C++/NDK dep**, so it cross-compiles cleanly for the Android `:p2p` libs. Prefer the OS voice-comm AEC on mobile where available; `sonora` is the desktop + fallback canceller. |
| Desktop capture/playback | `cpal` | Linux/macOS/Windows. **Shipped.** |
| Lock-free buffers | `ringbuf` | SPSC mic/speaker rings off the realtime thread |
| Jitter buffer | small in-house | seq/ts reorder + PLC trigger (modeled on `voicemcu`). **Shipped.** |

---

## Transport decision (the one big choice)

A call wants an **unreliable** datagram channel: drop a late audio packet, never
retransmit (retransmits = head-of-line latency = "robot voice"). Our libp2p stack
gives **reliable, ordered streams** (`/ghal-bol/msg/1.0.0` via `libp2p-stream`
over QUIC/TCP+yamux). So there are two options:

### Option A — media over a libp2p substream (recommended v1)
Open a second protocol `/ghal-bol/call/1.0.0` on the **same** libp2p connection
(mirrors how `/ghal-bol/msg/1.0.0` is opened in `chat_server.rs`).

- ➕ Reuses **everything**: NAT traversal, relay `/p2p-circuit` fallback, Noise, peer auth,
  coord discovery, the urgent-reconnect/keepalive work already in `chat_server.rs`.
- ➕ Fastest path to a working call; least new transport code.
- ➖ Reliable+ordered → under packet loss, latency can build (HOL blocking).
  Mitigate: tiny frames, send-queue bounded with **drop-oldest** so we never
  block; a fresh frame supersedes a stuck one. Good on healthy/LAN links; the
  weak case is lossy cellular.

### Option B — raw QUIC unreliable datagrams (v2 optimization)
A dedicated **`quinn`** QUIC connection between the peers (addresses learned from
coord/libp2p), audio as **unreliable datagrams** (RFC 9221) — **the exact shape
`voicemcu` / `proscenium` use** and the path this engine was conceptually started
from.

- ➕ Ideal media transport: no HOL blocking, lowest jitter — a late audio packet is
  dropped, never retransmitted (retransmit = head-of-line latency = "robot voice").
- ➕ Field-proven recipe (from `voicemcu`): CBR Opus, a **hard ceiling on encoded
  frame size** + capped `quinn` MTU discovery so each datagram stays under tight
  VPN/CGNAT MTUs (e.g. Tailscale); same jitter-buffer + Opus PLC on both ends.
- ➖ `rust-libp2p`'s QUIC does **not** expose datagrams to the app, so this is a
  *parallel* transport: we own connection setup for hard NATs. More code, more failure modes.
  Reuse coord/relay-learned addresses for the `quinn` dial; signaling stays on the
  reliable DM stream.

**Plan:** ship **A** first (reuse the link, prove the media engine), measure loss
behaviour, then add **B** as an opt-in fast path if cellular jitter demands it.
Control/signaling stays on the reliable DM stream either way.

---

## End-to-end encryption

- Media key = `derive_call_media_keys_from_transport(call_id, transport_kem)` — same
  `TransportKemHello` session keys as DM text and call signaling (golden rule #7).
- **Option A:** the libp2p stream is already Noise-encrypted peer-to-peer; we add
  a thin per-frame seal with the transport media key so a relay/path never sees
  plaintext (defense in depth, matches chat's seal-then-transport model).
- **Option B:** datagrams are sealed with the transport media key (QUIC TLS also
  wraps them); key never on the wire.
- Connect-time: media keys derive in parallel with device open; failure to derive
  must **not** silently drop to plaintext — fail the call's E2E or surface it
  (no peer-facing plaintext, per golden rule #7).

---

## Control / FFI surface (additions)

Reuse the `ghal_bol_ffi_call_*` pattern (`call_ffi.rs`) and the poll event path
(`apply_p2p_event_json` → Flutter reload). Dart sends **control only**; media
never crosses FFI.

| FFI (proposed) | Purpose |
|----------------|---------|
| `ghal_bol_ffi_call_media_start` | Begin capture/playback + media session for `call_id` (after `accept`). |
| `ghal_bol_ffi_call_media_stop` | Tear down pipeline + transport. |
| `ghal_bol_ffi_call_set_mic_muted` / `_set_speaker` / `_set_output_route` | Device/route control. |
| `ghal_bol_ffi_call_media_stats` | Poll: rtt, loss, jitter, bitrate, e2e-active, peer-key short → in-call chip + App log. |

Events on poll: `call_media_connected`, `call_media_stats`, `call_media_failed`
(UI shows "connected" only on real media flow, mirroring v1's truthful-status rule).

---

## Mobile audio (the real platform work)

Rust handles codec/jitter/crypto/transport on all platforms; **capture/playback +
hardware AEC** need per-OS integration:

| Platform | Capture/playback | Echo cancellation |
|----------|------------------|-------------------|
| Linux/macOS/Windows | `cpal` | software (`sonora`/`aec3`) — no system AEC |
| **Android** | Oboe / AAudio (`VOICE_COMMUNICATION` source) | **hardware** AEC/NS via the OS voice-comm path when available; software fallback |
| **iOS** | Voice-Processing AudioUnit (`kAudioUnitSubType_VoiceProcessingIO`) | **hardware** AEC built into the VPIO unit |

On mobile, the OS voice-comm audio path already gives AEC/NS — so prefer it and
treat `sonora` as the desktop/fallback canceller. This is the largest chunk of
new platform code and the main quality risk (echo on speakerphone, threading).

---

## Phased rollout

| Phase | Scope | Exit criteria |
|-------|-------|---------------|
| **P0** | Rust voice PoC, **desktop only**, Option A transport, fixed bitrate, no UI wiring (CLI/test harness) | Two desktops hold a clear 2-way call over a real libp2p link; measured RTT/jitter/loss |
| **P1** | Wire P0 into `call_controller` via FFI on **Linux**; ring UX kept; in-call stats chip | Linux↔Linux production voice call |
| **P2** | **Android** capture/playback + hardware AEC; speaker/route toggles | Android↔Android and Android↔Linux voice solid on Wi-Fi + cellular |
| **P3** | **iOS** VPIO path | iOS interop |
| **P4** | Option B (QUIC datagrams) if cellular jitter needs it | Lower jitter on lossy links |
| **P5** | **Video** — [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md) | Native video over `/ghal-bol/call-video/1.0.0` |

---

## Risks & open questions

- **Loss behaviour of Option A** on cellular — must measure before committing;
  drop-oldest send queue is the mitigation, Option B is the escape hatch.
- **AEC quality** cross-platform, especially desktop speakerphone and Bluetooth.
- **Mobile realtime audio threading** (xruns, latency) — Oboe/AudioUnit tuning.
- **Battery/CPU** of a Rust APM on mobile vs OS-accelerated voice paths.
- **Build size / NDK** — adding libopus + APM to the Android `:p2p` libs.
- **Video** — [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md).

## Non-goals (v2)

- Group calls / SFU.
- Replacing **video** in the first releases.
- Browser/web calls (FFI is native; web would need a separate path).
- MoQ.

---

## Implementation status

| Phase | State |
|-------|-------|
| **P0 engine core** | **Done.** `ghal_bol/src/call_media/` — `MediaFrame`, `MediaCrypto` (AES-256-GCM, per-direction nonce), `JitterBuffer` (reorder + PLC), `AudioCodec` trait + `NullCodec`. 7 unit tests. |
| **P1 Opus** | **Done.** `OpusEncoderCodec`/`OpusDecoderCodec` (audiopus, Voip + in-band FEC, PLC). `MediaEngine::new_opus`. Opus round-trip test. Builds vendored libopus (needs `cmake`). |
| **P2 transport** | **Done.** `/ghal-bol/call/1.0.0` substream in `chat_server.rs`: a second `control.accept(...)` loop + per-call TX `open_stream`. **Two streams per call** (each side opens its TX, accepts its RX) to avoid glare; first frame is a `{"call_id"}` header, then length-prefixed sealed packets. Registry `SessionState::call_media` maps `call_id → {peer_id, controls, wire_in_tx}`; inbound RX is peer-verified. `OutboundCmd::CallMediaStart/Stop/SetMicMuted` (priority 0). Stopped on node shutdown. |
| **P3 FFI / daemon** | **Done.** `p2p_runtime::p2p_call_media` (action `start`/`stop`/`set_mic_muted`), C FFI `ghal_bol_ffi_p2p_call_media`, daemon RPC `p2p_call_media`. Stats currently surfaced via `native_log` `call_media` lines (`sent=/recv=` every 3 s); a poll event is a later refinement. |
| **P4 desktop audio** | **Done.** `cpal` capture/playback on a dedicated audio thread (cpal `Stream` is `!Send`); down-mix to mono + linear resample to/from 48 kHz; `SilenceAudioBackend` fallback for headless/Android. **AEC not yet** — use headphones to avoid echo until AEC lands. **Next:** add **`sonora`** (pure-Rust AEC3 + NS + AGC2, confirmed available June 2026) in the capture path, fed the playout stream as the echo reference; cross-compiles for Android `:p2p` (no C++ dep). OS voice-comm AEC preferred on mobile when available. |
| **P5 Flutter** | **Done (voice).** Invite/accept negotiate `voice_engine: native_v2`; `CallController` uses native media (`GhalBolP2p.callMediaStart/Stop/SetMicMuted`). E2EE chip is truthful (native voice is always identity-E2E). |
| **P6 Android** | **Done (build + plumbing).** `cpal` reuses its Oboe (AAudio/OpenSL) backend on `target_os = "android"`, gated by `set_android_audio_ready()` — the `:p2p` JNI `initAndroidAudio` hands cpal the JavaVM + Context via `ndk_context`. libopus is cross-built static per ABI by `scripts/build_android_opus.sh` (audiopus_sys can't cross-compile it) and linked via `LIBOPUS_*` from `pack_android_workspace_jni_libs.sh`; the `.so` statically embeds opus and needs only `libc++_shared.so`/`libOpenSLES.so` (already shipped by the app). The `:p2p` service gains a `microphone` FGS type (re-promoted at call start once `RECORD_AUDIO` is granted). `CallController` advertises `native_v2` on Android behind `kAndroidNativeVoice`. **Known gaps (device-test):** cpal uses Oboe's default (media) input/route, so there is **no hardware AEC and no earpiece/speaker route control** — clean on a headset, echo on speaker. Proper fix = drive Oboe directly with `VOICE_COMMUNICATION` preset + `MODE_IN_COMMUNICATION` (bypassing cpal) or a software APM. |

### UI session and privacy (do not regress)

See [DESIGN.md](DESIGN.md) § “Call UI lifecycle and privacy”. Summary:

- **`p2p_force_end_active_call`** — stops `CallMediaStop` / `CallVideoStop`, sends **`hangup`**, clears `call_active` + `call_state`, dismisses OS incoming-call notification.
- **Daemon / `:p2p` tracks UI RPC sockets** — when the last Flutter socket closes (`ui_session_ended`), force-end runs automatically (covers **Ctrl+C** on `flutter run`).
- **`ui_session_prepare_reconnect`** — 5s suppress during login unlock socket drop so calls are not torn down mid-unlock.
- **`p2p_take_incoming_call_wake`** — Linux daemon notification tap → Flutter presents call UI.
- **Flutter** — GTK X / call-screen pop / `AppLifecycleState.detached` also call force-end (belt-and-suspenders).
- **Video call end (2026-06-15)** — `CallController._endLocal` stops native voice/video, releases **`CallVideoTexturePool`** textures on hangup (not on widget dispose), dismisses call UI without blocking on hangup RPC. Prevents orphan media and Linux Flutter crashes during/after video teardown. Detail: [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md) § “Flutter video textures and call end”, [DESIGN.md](DESIGN.md) § “Call UI lifecycle and privacy”.

**Never ship:** UI gone but native media still up and peer still in a call. **Never ship:** releasing GPU call textures from `NativeCallVideoView.dispose` during an active call.

### Desktop device-test steps (Linux↔Linux)

1. Quit any running Flutter app (the sync script stops a stale daemon).
2. `./scripts/sync_ghal_bol_native_for_flutter.sh` — rebuilds the lib + `ghal_bol_daemon` (now linked against cpal/ALSA + libopus) and copies them into the bundle.
3. `cd ghal_bol_ui && flutter run` on each desktop (use `--release` to hit the real coord server).
4. Place a voice call between the two contacts. Use **headphones** on at least one side (no AEC yet).
5. In the in-app App log, filter `call_media`. Expect on both sides: `start call_id=… local_is_a=…`, `inbound media stream from …`, `rx stream attached …`, then `call_id=… sent=N recv=M` ticking up every ~3 s. Audio should be two-way and clear.
6. Toggle mute → peer's `recv` keeps climbing but the audio goes silent; hang up → `tx/rx stream closed` and the session stops.

### Android device-test steps (Android↔Android / Android↔Linux)

1. Quit Flutter. Build native: `./scripts/pack_android_workspace_jni_libs.sh` (set `ANDROID_NDK_HOME`; `PACK_ANDROID_ARM64_ONLY=1` for a phone-only fast path). This first cross-builds static `libopus.a` per ABI (`scripts/build_android_opus.sh`), then the `:p2p` lib.
2. `cd ghal_bol_ui && flutter run` (use `--release` for the real coord server). Grant the **microphone** permission when prompted.
3. Place a voice call. Use a **headset** on the Android side (no AEC yet — speaker will echo).
4. In the App log, look for: `android audio ready (cpal/Oboe enabled)` (from `initAndroidAudio`), then on the call the same `call_media` `start … / inbound media stream / sent=N recv=M` lines as desktop. Audio should be two-way.
5. If you hear remote audio but the peer hears nothing, check the log for `mic disabled` / a `startForeground(mic=…)` warning — the `:p2p` service did not get the microphone FGS type (grant `RECORD_AUDIO`, then the call re-promotes it; relaunch once if it was denied earlier).

**Engine ↔ platform contract (for P2/P4):** the engine is driven by three calls —
`on_capture(pcm)->wire` (from the audio capture callback), `on_wire(bytes)` (from
the media stream reader), `on_playout(&mut pcm)` (from the audio playback callback,
every 20 ms). Audio I/O and transport are the only platform-specific pieces.

## References (June 2026)

- **1:1 P2P-call-over-QUIC stacks (proven):** `voicemcu` (Opus over **unreliable QUIC datagrams**, `quinn` + `ringbuf` + jitter/PLC), `proscenium` (P2P 1:1 voice over a dedicated QUIC protocol, `cpal`+Opus 48 kHz mono 20 ms, length-prefixed streams), `aura`, `occupyashanti/echo`.
- **Rust audio processing (AEC/NS/AGC):** **`sonora`** (pure-Rust AEC3 + NS + AGC2, v0.1.0 Feb 2026, ≈C++ parity) — fallback C++ AEC bindings (v2.1.0 May 2026).
- **Codec:** Opus (FEC/DTX/PLC) via `audiopus`. **Transport shape:** QUIC unreliable datagrams (RFC 9221).
- **Reference architecture for the broader pipeline** (camera, HW codecs, adaptive bitrate, per-track streams): the iroh team's **`iroh-live`** (`moq-media` / `rusty-codecs` / `rusty-capture`) — see [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md).
- **Why not MoQ/CDN for 1:1:** industry writeups on MoQ vs interactive 1:1 calls; Cloudflare MoQ. (We borrow MoQ's *per-track independent stream* idea but run it over our **own direct** connection — no relay/CDN fan-out.)
