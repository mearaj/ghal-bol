# Ghal Bol calls — native voice over the P2P link

**Status:** **Shipping** on Linux desktop and Android when both peers negotiate `voice_engine: native_v2`. iOS not started.

**Goal:** Voice where the media engine lives in **Rust** and rides the **direct peer connection we already establish** — no separate STUN/TURN/SDP stack, no MoQ. One identity, one crypto story, one transport.

Read first: [AGENTS.md](../AGENTS.md) (golden rules), [DESIGN.md](DESIGN.md) (layers + E2E), [TRANSPORT.md](TRANSPORT.md) (native connect stack), [GHAL_BOL_VOICE_V1.md](GHAL_BOL_VOICE_V1.md) (signaling). Video: [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md).

---

## Why native over the existing link

| Driver | Detail |
|--------|--------|
| **Golden rule #1** | `ghal_bol_core` (Rust) owns product logic — capture, codec, transport, jitter, and E2E live in Rust (`call_media/`). |
| **We already have the link** | native connect provides encrypted connections (Noise + mux; LAN direct or WAN coord bridge). Calls reuse that link via `/ghal-bol/call/1.0.0`. |
| **No new servers** | Direct when possible; coord bridge only as WAN fallback — same as other realtime paths. |
| **One E2E story** | `derive_call_media_keys_from_transport` + per-frame AES-GCM seal (transport KEM after `TransportKemHello`). |

**Why not MoQ.** Media-over-QUIC broadcast protocols target one-to-many fan-out, not 1:1 interactive calls. Not used here.

---

## Architecture pieces

| Piece | Implementation |
|-------|----------------|
| **Signaling** | `call_sig_v1.rs`, `call_state.rs`, `host call controls` over the DM stream |
| **Media key** | `call_media_key.rs` (`derive_call_media_keys_from_transport`) |
| **Media engine** | Rust: capture → APM → Opus → seal → transport → jitter → decode → playback |
| **Media transport** | `/ghal-bol/call/1.0.0` on the existing native connect peer connection |
| **Makepad** | Call screen, incoming-call notification, `ghal_bol_core::host` |

---

## Architecture

```text
┌───────────────────────── ghal_bol_app (Makepad) ─────────────────────────┐
│ Call UI, ring/back tones, mute/speaker/video toggles, device picker      │
│ Calls native via host::call_* ; renders state from poll events    │
└───────────────────────────────┬─────────────────────────────────────────┘
                                 │ ghal_bol_core::host
┌───────────────────────────────▼─────────────────────────────────────────┐
│ ghal_bol (Rust)                                                          │
│  call_sig_v1 / call_state   — signaling on the DM stream (unchanged)     │
│  call_media (NEW)           — pipeline + jitter + session lifecycle      │
│    capture → APM(AEC/NS/AGC) → Opus enc → seal → SEND                    │
│    RECV → unseal → jitter buffer → Opus dec(PLC) → mix → playback        │
│  call media keys            — derive_call_media_keys_from_transport       │
│  connect/             — the peer connection + a media substream    │
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
| AEC / NS / AGC | **`sonora`** (pure-Rust AEC3 + NS + AGC2, [crates.io](https://crates.io/crates/sonora) v0.1.0, Feb 2026, MSRV 1.91) — optional C++ APM bindings fallback | `sonora` is benchmarked at **C++ parity** (≈13 µs per 48 kHz mono frame) and is **pure Rust → no C++/NDK dep**, so it cross-compiles cleanly for the Android NDK builds. Prefer the OS voice-comm AEC on mobile where available; `sonora` is the desktop + fallback canceller. |
| Desktop capture/playback | `cpal` | Linux/macOS/Windows. **Shipped.** |
| Lock-free buffers | `ringbuf` | SPSC mic/speaker rings off the realtime thread |
| Jitter buffer | small in-house | seq/ts reorder + PLC trigger (modeled on `voicemcu`). **Shipped.** |

---

## Transport decision (the one big choice)

A call wants an **unreliable** datagram channel: drop a late audio packet, never
retransmit (retransmits = head-of-line latency = "robot voice"). Our native connect stack
gives **reliable, ordered streams** (`/ghal-bol/msg/1.0.0` via `native-stream`
over QUIC/TCP+yamux). So there are two options:

### Option A — media over a native connect substream (recommended v1)
Open a second protocol `/ghal-bol/call/1.0.0` on the **same** native connection
(mirrors how `/ghal-bol/msg/1.0.0` is opened in `connect/`).

- ➕ Reuses **everything**: NAT traversal, coord bridge for WAN, Noise, peer auth,
  coord discovery, the urgent-reconnect/keepalive work already in `connect/`.
- ➕ Fastest path to a working call; least new transport code.
- ➖ Reliable+ordered → under packet loss, latency can build (HOL blocking).
  Mitigate: tiny frames, send-queue bounded with **drop-oldest** so we never
  block; a fresh frame supersedes a stuck one. Good on healthy/LAN links; the
  weak case is lossy cellular.

### Option B — raw QUIC unreliable datagrams (v2 optimization)
A dedicated **`quinn`** QUIC connection between the peers (addresses learned from
coord/native connect), audio as **unreliable datagrams** (RFC 9221) — **the exact shape
`voicemcu` / `proscenium` use** and the path this engine was conceptually started
from.

- ➕ Ideal media transport: no HOL blocking, lowest jitter — a late audio packet is
  dropped, never retransmitted (retransmit = head-of-line latency = "robot voice").
- ➕ Field-proven recipe (from `voicemcu`): CBR Opus, a **hard ceiling on encoded
  frame size** + capped `quinn` MTU discovery so each datagram stays under tight
  VPN/CGNAT MTUs (e.g. Tailscale); same jitter-buffer + Opus PLC on both ends.
- ➖ `rust-stack`'s QUIC does **not** expose datagrams to the app, so this is a
  *parallel* transport: we own connection setup for hard NATs. More code, more failure modes.
  Reuse coord-learned addresses for the `quinn` dial; signaling stays on the
  reliable DM stream.

**Plan:** ship **A** first (reuse the link, prove the media engine), measure loss
behaviour, then add **B** as an opt-in fast path if cellular jitter demands it.
Control/signaling stays on the reliable DM stream either way.

---

## End-to-end encryption

- Media key = `derive_call_media_keys_from_transport(call_id, transport_kem)` — same
  `TransportKemHello` session keys as DM text and call signaling (golden rule #7).
- **Option A:** the native stream is already Noise-encrypted peer-to-peer; we add
  a thin per-frame seal with the transport media key so a bridge/path never sees
  plaintext (defense in depth, matches chat's seal-then-transport model).
- **Option B:** datagrams are sealed with the transport media key (QUIC TLS also
  wraps them); key never on the wire.
- Connect-time: media keys derive in parallel with device open; failure to derive
  must **not** silently drop to plaintext — fail the call's E2E or surface it
  (no peer-facing plaintext, per golden rule #7).

---

## Call controls

The Makepad call screen calls `ghal_bol_core::host`. Media stays in Rust.

| Host | Purpose |
|------|---------|
| `start_voice_call` / `start_video_call` / `accept_incoming_call` | Open the call. |
| `set_mic_muted` / `set_speaker` | Mute and speaker. |
| `end_call` | Hang up and stop media. |
| `call_banner` / `call_picture_pngs` | Status line and video pictures. |
| `poll_event` | UI refresh. An `incoming_call_wake` event brings the call screen forward after a notification tap. |

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
| **P0** | Rust voice PoC, desktop, Option A transport | Two desktops clear 2-way call |
| **P1** | Wire into Makepad via **`host::`** call APIs on Linux | Linux↔Linux production voice |
| **P2** | Android capture/playback + AEC; speaker/route | Android↔Android and Android↔Linux |
| **P3** | iOS VPIO path | iOS interop |
| **P4** | Option B (QUIC datagrams) if cellular needs it | Lower jitter on lossy links |
| **P5** | Video — [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md) | Native video substream |

---

## Risks & open questions

- **Loss behaviour of Option A** on cellular — must measure before committing;
  drop-oldest send queue is the mitigation, Option B is the escape hatch.
- **AEC quality** cross-platform, especially desktop speakerphone and Bluetooth.
- **Mobile realtime audio threading** (xruns, latency) — Oboe/AudioUnit tuning.
- **Battery/CPU** of a Rust APM on mobile vs OS-accelerated voice paths.
- **Build size / NDK** — adding libopus + APM to the Android native build.
- **Video** — [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md).

## Non-goals (v2)

- Group calls / SFU.
- Replacing **video** in the first releases.
- Browser/web calls.
- MoQ.

---

## Implementation status

| Phase | State |
|-------|-------|
| **P0 engine core** | **Done.** `ghal_bol_core/src/call_media/` — `MediaFrame`, `MediaCrypto` (AES-256-GCM, per-direction nonce), `JitterBuffer` (reorder + PLC), `AudioCodec` trait + `NullCodec`. 7 unit tests. |
| **P1 Opus** | **Done.** `OpusEncoderCodec`/`OpusDecoderCodec` (audiopus, Voip + in-band FEC, PLC). `MediaEngine::new_opus`. Opus round-trip test. Builds vendored libopus (needs `cmake`). |
| **P2 transport** | **Done.** `/ghal-bol/call/1.0.0` substream in `connect/`: a second `control.accept(...)` loop + per-call TX `open_stream`. **Two streams per call** (each side opens its TX, accepts its RX) to avoid glare; first frame is a `{"call_id"}` header, then length-prefixed sealed packets. Registry `SessionState::call_media` maps `call_id → {peer_id, controls, wire_in_tx}`; inbound RX is peer-verified. `OutboundCmd::CallMediaStart/Stop/SetMicMuted` (priority 0). Stopped on node shutdown. |
| **P3 host controls** | **Done.** `host::start_voice_call`, `accept_incoming_call`, `set_mic_muted`, `set_speaker`, `end_call`. Stats are `native_log` `call_media` lines (`sent=/recv=` every 3 s). |
| **P4 desktop audio** | **Done.** `cpal` capture/playback on a dedicated audio thread (cpal `Stream` is `!Send`); down-mix to mono + linear resample to/from 48 kHz. Speaker audio is fed to Sonora AEC3 before the microphone frame is encoded. |
| **P5 Makepad** | **Done.** The call screen calls `host::start_voice_call`, `accept_incoming_call`, `set_mic_muted`, `set_speaker`, and `end_call`. |
| **P6 Android** | **Done.** `host::install_android_context` gives cpal the Java VM and activity. Sonora AEC3 runs on each microphone frame. The camera is NDK Camera2 in this process. Build with `./scripts/build_android_app.sh`. |

### UI session and privacy (do not regress)

See [DESIGN.md](DESIGN.md) § Calls and § Process. Summary:

- **`host::end_call`** stops voice and video, sends hangup, clears call state, and dismisses the incoming-call notification.
- **Quit and shutdown** call `host::end_call` before the process exits.
- **Incoming notification tap** sets a wake marker. `host::poll_event` returns `incoming_call_wake`, and the call screen opens.
- **Lock** hides the hub and leaves the node running. **Log out** stops the node.

**Never ship:** the window gone while native media is still up and the peer is still in the call.

### Desktop device-test steps (Linux↔Linux)

1. Quit any running Ghal Bol app.
2. `cargo build -p ghal_bol_app --release`.
3. `cargo run -p ghal_bol_app --release` on each desktop.
4. Place a voice call between the two contacts. Prefer headphones on at least one side.
5. In the in-app App log, filter `call_media`. Expect on both sides: `start call_id=…`, `inbound media stream`, then `sent=N recv=M` ticking up. Audio should be two-way.
6. Toggle mute → peer's `recv` keeps climbing but audio goes silent; hang up → streams close.

### Android device-test steps (Android↔Android / Android↔Linux)

1. Build and install with `./scripts/build_android_app.sh`. Grant the microphone and camera when asked.
2. Place a voice call. Sonora reduces speaker echo in the microphone path.
3. In the App log, look for `call_media` `start … / inbound media stream / sent=N recv=M`.
4. If the peer hears nothing, grant the microphone permission and place the call again.

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
