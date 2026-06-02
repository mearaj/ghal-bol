# Ghal Bol — tiered communication architecture

Ghal Bol uses a multi-tier communication model to balance speed, decentralization, reliability, infrastructure cost, user ownership, and scalability.

Instead of routing all traffic through centralized servers or relying only on direct peer connectivity, the system **prefers the lowest-cost path that works** and falls back when needed.

---

## Priority order

Always try, in order:

1. **Tier 1** — direct peer-to-peer sync (free, preferred)
2. **Tier 2** — encrypted blobs on other peers as temporary relays (free, optional)
3. **Tier 3** — encrypted blobs on Ghal Bol cloud backup relay (paid, optional)

The same encrypted message object, message ID, and sync semantics apply on every tier; only the **transport path** changes.

---

## Tier 1 — direct peer-to-peer (free)

**Status: implemented today.** Transport is **libp2p** (`/ghal-bol/msg/1.0.0` streams over QUIC/TCP, Noise, Yamux). Coordination server handles presence and endpoint discovery. See [TRANSPORT.md](TRANSPORT.md).

```text
Peer A ↔ Peer B
```

The coordination server (`ghal_bol_server`) assists only with:

- presence tracking
- endpoint discovery
- signed registration

It does **not** carry message bodies on the data path after lookup.

### Typical flow

1. Peer A registers endpoints + heartbeat with `ghal_bol_server`.
2. Peer B looks up A via `GET /v1/peers/{public_key_hex}`.
3. Peers open a **direct** transport (QUIC preferred) and run a **short sync session** (messages, acks, transcript cursor).
4. Transcripts and delivery state stay on peers.

See [PEER_DISCOVERY.md](PEER_DISCOVERY.md) for invites (`ghalbol.com`) vs coordination (`coord.ghalbol.com` in deploy).

### Advantages

- Lowest latency and server bandwidth
- Maximum peer ownership
- IPv6- and LAN-friendly

---

## Tier 2 — peer relay network (free)

**Status: planned — not implemented in `ghal_bol` or `ghal_bol_server` yet.**

```text
Peer A ↔ Relay peers ↔ Peer B
```

When direct sync is temporarily impossible (recipient offline or unreachable), the sender may distribute **encrypted opaque blobs** across other online peers.

Relay peers:

- cannot decrypt content
- do not own transcripts
- hold blobs only until sync succeeds or policy expires
- operate under bounded storage and duration

### Intended flow

1. Sender asks coordination server for relay candidates (future API).
2. Server suggests multiple online relay peers.
3. Sender pushes encrypted blobs to relays.
4. When recipient is reachable, direct sync (Tier 1) pulls missing messages; relay copies are discarded.

Root [README.md](../README.md) describes this model at a high level; implementation is future work.

---

## Tier 3 — centralized backup relay (paid)

**Status: planned product tier — not the same as today’s `ghal_bol_server`.** Billing and entitlement design: [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md).

```text
Peer A ↔ Ghal Bol backup relay ↔ Peer B
```

Optional **premium** layer for stronger offline delivery: the service temporarily stores **encrypted** blobs and assists delayed sync.

Important distinctions:

| | Coordination server (today) | Tier 3 backup relay (future) |
|--|-----------------------------|------------------------------|
| Role | Presence + endpoint lookup | Temporary encrypted blob store |
| Message bodies | Not stored | Encrypted blobs only, bounded TTL |
| Transcripts | Peer-owned | Still peer-owned |
| Billing | Infrastructure for all users | Optional paid reliability |

Even in Tier 3 (when built):

- messages stay end-to-end encrypted
- the cloud is not the transcript owner
- storage is temporary, not a permanent archive

---

## Synchronization philosophy (all tiers)

Ghal Bol is **synchronization-oriented**, not “one fragile socket forever”:

- reconnects are normal
- sessions are short-lived and resumable
- mobile NAT / CGNAT / app suspend are expected
- peers exchange missing messages, acks, and sync cursors

This matches the sync model in [README.md](../README.md).

---

## Why tiers exist

| Approach | Problem |
|----------|---------|
| Pure centralized (WhatsApp-style) | Platform owns inbox, heavy infra, metadata concentration |
| Pure decentralized swarm | Hard on mobile, unpredictable ops |
| **Tiered Ghal Bol** | Coordinate lightly; sync directly when possible; optional relays only when needed |

---

## Economic model (product direction)

**Free:** Tier 1 direct sync + Tier 2 peer relays (when implemented).

**Paid:** Tier 3 backup relay for users who want stronger offline delivery guarantees without giving up E2E encryption or local transcript ownership. Payment rails and membership are separate from messaging keys — see [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md).

---

## Implementation map (repo)

| Tier | Component | Today |
|------|-----------|--------|
| 1 coordination | `ghal_bol_server` — register, heartbeat, peer lookup | **Shipped** |
| 1 transport | `ghal_bol` libp2p sync engine (`chat_server.rs`) | **Shipped** |
| 1 UI | `ghal_bol_ui` | Shell; wires to native worker |
| 2 relay | Coordination + `ghal_bol` relay protocol | Not started |
| 3 backup | Separate relay service or server feature | Not started |

---

## Related docs

- [README.md](../README.md) — identity, sync, relay overview
- [WHAT_GHAL_BOL_SOLVES.md](WHAT_GHAL_BOL_SOLVES.md) — product problems and vision
- [PEER_DISCOVERY.md](PEER_DISCOVERY.md) — invites and coordination lookup
- [../ghal_bol_server/README.md](../ghal_bol_server/README.md) — HTTP API for Tier 1 coordination
- [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) — optional paid infrastructure and crypto payment model
