# Ghal Bol Connect v1 — native transport wire spec

Status: **shipping target** — replaces the libp2p transport stack (mDNS + TCP + Noise + relay)
with a purpose-built native stack. Zero `libp2p*` crates in any workspace `Cargo.toml` or
`Cargo.lock` once the migration lands.

Scope of the connect layer:

- **LAN text** — additive fast mirror next to the delivery server (never instead of it).
- **LAN + WAN voice/video calls** — realtime media streams, E2E sealed on device.

WAN **text** is out of scope here: it always goes through [`ghal_bol_delivery`](GHAL_BOL_DELIVERY.md)
(E2E encrypted mailbox, offline guarantee). See "Parallel LAN + WAN invariant" below.

---

## Approved dependency stack (July 2026, pinned)

Only **popular, actively maintained** crates. Re-evaluate quarterly. As of **12 July 2026**:

| Concern | Crate | Version pin | Why |
|---|---|---|---|
| Async runtime | `tokio` | `1` | Workspace standard (core, coord, delivery) |
| LAN discovery | `mdns-sd` | `0.20` | Leading pure-Rust mDNS/DNS-SD; Android/Linux/desktop; no async-runtime lock-in |
| Transport encryption | `snow` | `0.10` | De facto Noise Protocol implementation (Noise spec rev 34) |
| Framing / buffers | `bytes` | `1` | Standard for length-prefixed mux |
| Bridge WSS client | `tokio-tungstenite` + `rustls` | workspace | Same stack as `delivery_client.rs` |
| Coord bridge server | `axum` | `0.8` | Already serving coord + delivery HTTP |
| Optional media datagrams (later) | `quinn` + `rustls` | `0.11` / `0.23` | Pure-Rust QUIC; only if TCP bridge latency proves insufficient |

**Explicitly rejected:** `libp2p*` (all), obscure Noise forks, custom mDNS, `webrtc-rs` as default,
alpha (`0.0.x`) protocol crates.

**In-house only where no popular crate fits:** the channel mux (8-byte header, ~100 LOC) and the
coord bridge pairing logic (product-specific).

---

## Identity and session keying

There is **no PeerId and no Multiaddr**. A remote peer is keyed by its normalized contact
**`identity_wire`** string (`docs/MULTI_ALGO.md`): bare hex secp256k1 or `algorithm:hex`.

- Dial targets are `ip:port` (LAN, from mDNS) or a bridge token URL (WAN, from coord).
- Session tables, outbox rows, foreground/room state, transcript keys — all identity-wire keyed.
- `DecryptedIdentity::to_libp2p_keypair()` is deleted; the connect layer proves identity with a
  detached signature from the same device identity key used for `msg_v1` envelopes.

### Identity commitment (mDNS privacy)

LAN advertisements do not broadcast the full identity wire. They carry a 16-byte hex
**identity commitment**:

```text
idc = hex( SHA-256( "ghal_bol_connect_v1/idc" || identity_wire_normalized ) )[0..32]
```

Contacts already know each other's `identity_wire` (QR invite), so each side computes the
commitments of its saved contacts and matches advertisements locally. Non-contacts on the same
LAN learn only that *a* Ghal Bol node is present, not which identity.

---

## LAN discovery (mDNS)

Service type: **`_ghalbol._tcp.local.`** via `mdns-sd`.

- Instance name: the `idc` commitment (hex, 32 chars).
- Port: the ephemeral TCP listen port of the connect listener (fresh every process start —
  never cached on disk; see TRANSPORT.md "Caching policy").
- TXT records:

| Key | Value | Notes |
|---|---|---|
| `v` | `1` | Connect protocol version |
| `idc` | identity commitment | Same as instance name (explicit for resolvers) |

Events drive dial policy (never timers):

- `ServiceResolved` for a known contact commitment → open one LAN TCP connection (in-flight
  guard per identity; no parallel dial storm).
- `ServiceRemoved` → close the LAN socket for that peer. **Delivery WS and WAN bridge are not
  touched** (see invariant below).

Discovery cadence comes from the `mdns-sd` browse refresh — do not rebind the TCP listener or
restart the daemon to force re-discovery.

---

## Transport handshake (Noise + identity proof)

Pattern: **`Noise_XX_25519_ChaChaPoly_BLAKE2s`** (`snow`), prologue bytes
`"ghal_bol_connect_v1"`. Static X25519 keys are generated **per process start** — they are
transport keys, not identity. Identity binding happens with an explicit proof inside the
encrypted handshake payloads:

1. TCP connect. Initiator sends Noise XX message 1 (`-> e`).
2. Responder sends message 2 (`<- e, ee, s, es`) with encrypted payload = **identity proof**.
3. Initiator sends message 3 (`-> s, se`) with encrypted payload = **identity proof**.
4. Both sides verify the peer proof; on failure the connection is closed immediately.

**Identity proof** (JSON, inside the Noise payload):

```json
{
  "identity_wire": "<normalized contact identity wire>",
  "sig_hex": "<identity signature over sign bytes>"
}
```

Sign bytes: `"ghal_bol_connect_v1/proof" || noise_handshake_hash || x25519_static_public`.
The signature uses the device identity key (secp256k1/ed25519/ecdsa-p256 — same
`identity_sign` scheme as `msg_v1`). This binds the Noise session statics to the contact
identity: a MITM cannot splice sessions, and the E2E rule (golden rule 7) holds — the
transport session key is rooted in both device identity keys.

After the handshake all traffic is Noise transport-mode ciphertext. Each mux frame (below) is
one Noise message; max Noise message size 65535 bytes bounds the mux payload chunking.

**Simultaneous dial:** if both peers dial each other (mDNS resolves on both sides), both
connections may complete; the side with the lexicographically smaller `identity_wire` closes
its **outbound** duplicate once an inbound session for the same identity is up. One session per
peer is the steady state; a brief overlap is harmless (frame handling is idempotent by
`message_id`).

---

## Channel mux

Replaces yamux/libp2p-stream. Fixed 8-byte header per frame, inside the Noise transport
ciphertext:

```text
+----------------+----------------+------------------------+
| channel u32 BE | length u32 BE  | payload (length bytes) |
+----------------+----------------+------------------------+
```

| Channel | Contents | Max payload |
|---|---|---|
| `0` | Messaging + signaling JSON frames (existing envelopes) | 1 MiB |
| `1` | Call audio (sealed media frames, `GHAL_BOL_CALL_NATIVE_V2.md`) | 256 KiB |
| `2` | Call video (sealed media frames, `GHAL_BOL_VIDEO_NATIVE_V1.md`) | 256 KiB |
| `3`–`15` | Reserved | — |
| `0xFFFFFFFF` | Keepalive ping/pong (empty payload = ping, 1 byte `0x01` = pong) | 1 B |

Payloads larger than one Noise message (65519 bytes of plaintext) are split across consecutive
Noise messages by the writer and reassembled by length before frame dispatch.

Keepalive: ping every **20 s** of write inactivity; peer answers pong; session closed after
**120 s** without any inbound bytes. These are product-controlled guardrail timers, not policy
timers (TRANSPORT.md "Event-driven async").

### Channel 0 — messaging and signaling

Channel 0 carries the **unchanged** envelopes:

- `ghal_bol_msg_v1` (text, `ack_received`, `ack_read`) — `docs/GHAL_BOL_DM_MSG_V1.md`,
  including transport-KEM v2 sealing after `TransportKemHello`.
- `ghal_bol_call_v1` signaling — `docs/GHAL_BOL_VOICE_V1.md`.

The connect layer moves bytes; message-level E2E, signatures, ack policy, outbox, and
transcript merge stay where they are today (Rust core, transport-agnostic modules).

### Channels 1–2 — call media

Same sealed frame bytes as today (`derive_call_media_keys_from_transport` + per-frame
AES-GCM). The 64 KiB yamux-era `CALL_MEDIA_MAX_FRAME` cap is lifted to 256 KiB per mux frame;
engines may keep smaller frames for latency.

---

## WAN call bridge (replaces libp2p relay)

The coord server pairs two **outbound** client connections and pipes opaque bytes. No
reservations, no `/p2p-circuit`, no Multiaddr, no relay v2.

### Pairing flow

1. Caller: `POST /v1/bridge/request` `{ "peer_identity_wire": "...", "call_id": "..." }`,
   signed with the device identity (same auth model as delivery/coord presence).
   Response: `{ "bridge_id": "...", "token": "...", "connect_url": "wss://coord…/v1/bridge/connect" }`.
2. Coord notifies the callee (delivery WS push or coord presence poll) with the same
   `bridge_id` + a callee token.
3. Both peers open **outbound** `GET /v1/bridge/connect?bridge_id=…&token=…` upgraded to a
   WebSocket (binary frames), or plain TCP on the dedicated bridge port with a one-line token
   preamble.
4. Coord pairs the two sockets and forwards bytes bidirectionally until hangup, TTL, or byte
   budget.
5. Inside the bridged byte stream the peers run the exact **same Noise XX handshake + channel
   mux** as LAN — the bridge sees only ciphertext.

### Limits (product-controlled)

| Parameter | Default | Env |
|---|---|---|
| Session max duration | 4 h | `GHAL_BOL_BRIDGE_MAX_SECS` |
| Max relayed bytes | unlimited (`0`) | `GHAL_BOL_BRIDGE_MAX_BYTES` |
| Max concurrent bridges per identity | 4 | `GHAL_BOL_BRIDGE_MAX_PER_PEER` |
| Idle timeout | 120 s (keepalive above) | `GHAL_BOL_BRIDGE_IDLE_SECS` |

---

## Parallel LAN + WAN invariant (critical)

**LAN never disables WAN.**

| Path | Role |
|---|---|
| Delivery upload (WAN text) | **Primary, mandatory** whenever `GHAL_BOL_DELIVERY_URL` is set. Every outbound text reaches the server so an offline peer still gets it. |
| LAN connect (text) | Additive fast mirror of the same `message_id` for an instant `delivered` tick. Recipient merge is idempotent by `message_id`. |
| WAN bridge (calls) | Stays available while a call is active even when a LAN path exists. |
| LAN connect (calls) | Lower-latency media path when both peers are on the same LAN. |

Concretely:

1. `send_text` **always** uploads to delivery first when the URL is set — LAN presence never
   routes a text away from the server.
2. The delivery WebSocket worker keeps running on Wi‑Fi/LAN; mDNS discovery does not stop it.
3. mDNS `ServiceRemoved` closes only the LAN socket; delivery + bridge are untouched.
4. A peer going offline mid-chat never strands the last messages on the sender's device.

---

## Non-goals

- Kademlia / DHT / gossipsub / mesh discovery — never.
- Pure P2P WAN without a server — CGNAT reality unchanged; the bridge is the call-reachability
  equivalent of the delivery server.
- WebRTC/ICE — only reconsidered if quinn datagrams prove insufficient for media.
- Disk caching of ports, bridge tokens, or discovery results — live lookup only.
