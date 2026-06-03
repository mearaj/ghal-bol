# Transport — libp2p data plane

**Status:** **libp2p is the production P2P transport.** A prior plan to replace libp2p with a custom native QUIC/TCP stack was **evaluated and discarded** (May 2026). This document is the canonical reference for how peers connect today.

**For AI / new sessions:** Read [AGENTS.md](../AGENTS.md) and [DESIGN.md](DESIGN.md) first. Transport changes must **not** move ack policy, outbox, or transcript merge into Flutter.

---

## Summary

Ghal Bol separates **chat protocol** from **transport**:

| Layer | Implementation |
|-------|----------------|
| **Chat protocol** | `ghal_bol_msg_v1` — signed JSON envelopes on framed streams ([GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md)) |
| **Transport** | **libp2p 0.56** swarm in `ghal_bol/src/p2p/chat_server.rs` |
| **Discovery (Tier 1)** | `ghal_bol_server` register/lookup + libp2p mDNS (LAN) + Kademlia (WAN fallback) |
| **Policy** | Outbox, ack send/retry, foreground gates, transcripts — same module tree regardless of transport |

libp2p is **not** the chat protocol. It provides encrypted connections, stream multiplexing, and dial/listen. All product semantics live in Rust (`chat_server.rs`, `dm_event_handler.rs`, stores).

---

## libp2p stack (current)

Enabled in `ghal_bol/Cargo.toml` and wired in `chat_server.rs`:

| libp2p piece | Role in Ghal Bol |
|--------------|------------------|
| **QUIC / TCP + Noise + Yamux** | Encrypted connections and multiplexing |
| **`libp2p-stream` `/ghal-bol/msg/1.0.0`** | Framed DM channel for `ghal_bol_msg_v1` |
| **mDNS** | LAN discovery of configured peers |
| **Kademlia** | WAN peer routing when coord lookup is insufficient |
| **Relay + DCUtR** | NAT traversal (partial; Tier-2 product relay still planned) |
| **AutoNAT, UPnP, Identify** | Reachability and peer metadata |
| **Gossipsub** | **Not used** — do not reintroduce for 1:1 DM |

Identity: one **secp256k1** keypair per device → libp2p **PeerId** via `libp2p-identity` (`keystore_v1.rs`, `peer_id_util.rs`).

---

## Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│  ghal_bol_ui — screens, hub foreground, poll (unchanged role)   │
└────────────────────────────┬────────────────────────────────────┘
                             │ FFI / daemon JSON-RPC
┌────────────────────────────▼────────────────────────────────────┐
│  ghal_bol — policy + stores (unchanged role)                       │
│  msg_v1, outbox, pending_read_acks, dm_event_handler, stores     │
└────────────────────────────┬────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────────┐
│  p2p/chat_server.rs — libp2p swarm + sessions                    │
│  Stream protocol /ghal-bol/msg/1.0.0; writer task per peer       │
│  Outbox, ack_received/ack_read, foreground/leave drain         │
└────────────────────────────┬────────────────────────────────────┘
                             │ register / heartbeat / lookup
┌────────────────────────────▼────────────────────────────────────┐
│  ghal_bol_server — Tier-1 coordination (no message bodies)       │
└─────────────────────────────────────────────────────────────────┘
```

**Process split (Linux / Android):** libp2p runs **out-of-process** in `ghal_bol_daemon` or Android `:p2p` (`GhalBolP2pService`). The UI process loads `libghal_bol.so` for identity and store I/O; both share the same data directory.

---

## Discovery (Tier 1)

Typical WAN flow ([PEER_DISCOVERY.md](PEER_DISCOVERY.md)):

1. Guest scans host QR → stores `public_key_hex`.
2. Both peers register endpoints with `ghal_bol_server`.
3. Lookup `GET /v1/peers/{public_key_hex}` → dial returned endpoints via libp2p.
4. Open `/ghal-bol/msg/1.0.0` stream; speak `ghal_bol_msg_v1`.

**LAN:** mDNS for configured contacts when on the same network. **WAN fallback:** Kademlia when coord paths fail.

Coord publishes `tcp`, `quic`, and `libp2p` multiaddrs; `coord_runtime.rs` and `dm_transport/addr.rs` help filter and rank dial targets before libp2p dials.

### LAN vs WAN dial policy (2026)

Native code ranks dial addresses **per contact**, not only per device interface:

1. **WAN-first** when the peer is not on the local LAN and the device likely has internet (coord heartbeat/register OK, public DHT bootstrap connected, or public IPv4).
2. **LAN-first** when the peer was seen via mDNS (or same RFC1918 /24), or when internet is likely down — mDNS and direct TCP still work on the LAN.
3. **Coord** — register/lookup every ~3s while P2P runs; HTTP failures do not stop libp2p; lookups fall back to DHT/mDNS. Disconnected DM contacts get a throttled coord lookup every 3s (8s for LAN-local peers).
4. **Handover** — Wi‑Fi with internet still runs WAN recovery (relay listen + coord) instead of treating “on Wi‑Fi” as “WAN not needed”. Mobile-data path still resets stale bootstrap TCP when switching routes.

See [DESIGN.md](DESIGN.md) § “Dial strategy — LAN vs WAN”.

### Roaming and reconnect (2026)

Connectivity is designed for **either peer** changing network:

- **This device** — Android `ConnectivityManager` in `:p2p`; all platforms poll local interfaces every 1s; app resume calls `p2p_notify_network_change`.
- **Other peer** — detected when the DM libp2p connection closes; native immediately runs a throttled **reconnect pass** (coord lookup + Kademlia + mDNS), not a multi-minute wait.
- **After relay/coord register** — all contacts get a fresh lookup so new WAN addresses are used.

Throttles remain (800ms–3s between coord lookups per peer) to avoid flooding the network while still beating the old 5–10 minute stall window.

---

## Helper modules (not a separate transport)

| Path | Role |
|------|------|
| `ghal_bol/src/p2p/chat_server.rs` | libp2p swarm, streams, outbox, ack policy |
| `ghal_bol/src/p2p/dht_bootstrap.rs` | Kademlia bootstrap multiaddr ranking |
| `ghal_bol/src/p2p_runtime.rs` | Background node thread, poll queue |
| `ghal_bol/src/dm_transport/` | **Dial-address helpers only** — parse coord endpoints; libp2p still uses `Multiaddr` on the wire |
| `ghal_bol/src/coord_runtime.rs` | Register/listen snapshot, lookup → dial addrs |

There is **no** standalone native TCP/QUIC listener stack. Do not assume `dm_transport/` replaces libp2p.

---

## Footprint (approximate, May 2026)

| Metric | Value |
|--------|-------|
| Dependency crates (`cargo tree -p ghal_bol`) | ~967 |
| libp2p-named crates | ~70 |
| Release `libghal_bol.so` / `ghal_bol_daemon` | ~20 MB unstripped, ~15 MB stripped |
| `chat_server.rs` | ~4k lines — transport + session + ack policy |

Accepting libp2p keeps NAT traversal, mDNS, and relay tooling at the cost of binary size and build time. The discarded native-stack plan targeted smaller binaries but was not pursued.

---

## Stable FFI / daemon surface

Do not rename without a version bump:

- `p2p_start` / `p2p_stop` (avoid stop on contact upsert — [AGENTS.md](../AGENTS.md))
- `register_dm_peer` / `sync_contacts`
- `send_text_dm`
- `p2p_poll` / `apply_p2p_event_json`
- `p2p_set_foreground_peer` / `p2p_set_app_ack_read_enabled`
- Coord: register, heartbeat, lookup ([COORDINATION_SERVER.md](COORDINATION_SERVER.md))

---

## Invariants (do not break when touching transport)

| Invariant | Owner |
|-----------|--------|
| `ghal_bol_msg_v1` envelope, ack kinds, `ref_id` rules | `msg_v1.rs`, [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) |
| Recipient sends `ack_received` always; `ack_read` only in-room; leave backlog | `chat_server.rs` — not Dart |
| Sender outbox until peer acks | Same |
| Flutter **never** sends acks; poll refreshes UI only | [DESIGN.md](DESIGN.md) |
| Guest scans host QR; host may have zero contacts | `connect_invite_v1`, `dm_event_handler` |
| `p2p_poll` → `apply_p2p_event_json` → disk → UI reload | `p2p_runtime.rs`, `dm_event_handler.rs` |
| No `p2p_stop` on every contact change | `register_dm_peer` / `sync_contacts` |

---

## AI handoff — common mistakes

1. **Reimplementing ack policy in Dart** — forbidden; see DESIGN.md.
2. **Assuming libp2p was removed** — it was not; read this file.
3. **Reintroducing gossipsub** for 1:1 DM — wrong model.
4. **Requiring mutual QR** — guest-only host key from QR is intentional.
5. **Clearing `pending_read_acks` on leave** — breaks DESIGN leave backlog.
6. **Restarting libp2p on every contact upsert** — use hot `register_dm_peer` instead.

---

## Related documents

| Doc | Relationship |
|-----|----------------|
| [DESIGN.md](DESIGN.md) | Canonical product behaviour |
| [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) | Wire format and ack kinds |
| [COMMUNICATION_TIERS.md](COMMUNICATION_TIERS.md) | Tier 1 direct P2P + coord |
| [PEER_DISCOVERY.md](PEER_DISCOVERY.md) | Invite + coord lookup flow |
| [COORDINATION_SERVER.md](COORDINATION_SERVER.md) | Run/test coord |
| [LOCAL_DEV_STACK.md](LOCAL_DEV_STACK.md) | LAN dev stack |

---

## Changelog

| Date | Change |
|------|--------|
| 2026-05-31 | Canonical transport doc; libp2p confirmed as production stack. |
