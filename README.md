# Ghal Bol

<p align="center">
  <img src="ghal_bol_ui/assets/for-feature-graphic-1.png" alt="Ghal Bol Icon for playstore's feature graphic" width="1536">
</p>

Gh`al Bol is an end-to-end encrypted messenger: device-owned identity, local transcripts, and minimal server trust.

**Text chat (WAN)** uses the [`ghal_bol_delivery`](ghal_bol_delivery/) server — a temporary encrypted mailbox. The server never decrypts messages; only sender and recipient keys can. **LAN text** and **voice/video calls** use **native connect** (LAN/Voice/Video pure P2P where possible, otherwise P2P with the help of a coord relay server).

The system is designed around:

> Reliable encrypted text when peers are apart; realtime P2P when they share a LAN or place a call.

Unlike traditional messengers, Ghal Bol does **not** store chat history in the cloud. Each device keeps its own transcript. The delivery server holds ciphertext **only until** the recipient acknowledges delivery (then deletes payload, keeps metadata for acks/TTL).

**Why WAN text moved off pure P2P:** relay chat required both peers online at the same time and could not guarantee delivery when one peer was offline, asleep, or on a flaky mobile network — unacceptable for the core job of **text messages**. Voice and video remain P2P-first because they are inherently realtime sessions.

**Architecture & transport:** [docs/DESIGN.md](docs/DESIGN.md), [docs/TRANSPORT.md](docs/TRANSPORT.md), [docs/GHAL_BOL_DELIVERY.md](docs/GHAL_BOL_DELIVERY.md).

**Invites & coordination:** [docs/GHAL_BOL_URI_SCHEME.md](docs/GHAL_BOL_URI_SCHEME.md), [docs/COORDINATION_SERVER.md](docs/COORDINATION_SERVER.md)

---

# Core Philosophy

Ghal Bol separates identity from infrastructure.

Users own:
- their cryptographic identity
- their local transcript
- their delivery state
- their communication history

Servers only assist with:
- peer coordination
- endpoint discovery
- presence tracking
- relay coordination

The server is not the owner of chats, identities, or messages.

---

# Identity Model

Each installation uses a local cryptographic identity (public/private secp256k1 keypair). The app can **create** a new key on first setup or **import** an existing 64-hex private key or encrypted keystore backup. Optional **premium infrastructure** (paid Tier 3 relay) is separate from identity and payments — see [docs/IDENTITY.md](docs/IDENTITY.md) and [docs/PREMIUM_SERVICES.md](docs/PREMIUM_SERVICES.md).

Identities are:
- local-only
- user-owned
- self-generated
- independent from centralized accounts

The system does not require:
- phone numbers
- email addresses
- usernames controlled by servers

Peer connections can be established through QR codes, invite links, or public key exchange.

**Invite URLs (domain we use today):**

- Web: `https://ghalbol.com/connect/<public_key_hex>`
- App: `ghalbol://connect/<public_key_hex>`

The link identifies the peer only; the app resolves live endpoints via `ghal_bol_coord`, then connects directly. See [docs/TRANSPORT.md](docs/TRANSPORT.md) § Discovery. **HTTPS invites:** with verified App Links (`/.well-known/assetlinks.json` on the site), Android opens the app directly; otherwise the browser shows the [web invite handoff](docs/WEB_SITE.md). **Linux desktop:** [download page](https://ghalbol.com/download/linux).

---

# Presence and Registration

Each peer maintains lightweight presence with the coordination server.

When a peer connects:
1. The server sends a nonce challenge.
2. The peer signs the nonce using its private key.
3. The server verifies the signature.
4. The peer’s endpoint registration is accepted.

This ensures that a peer can only register endpoints for identities it actually owns.

The server stores:
- peer public key
- current reachable endpoints
- transport capabilities
- heartbeat timestamp
- optional IPv6 and IPv4 addresses

Presence is ephemeral and expires automatically if heartbeats stop.

---

# Networking Model

Every peer in Ghal Bol acts as both:
- client
- server

Peers communicate directly whenever possible.

Connection policy for a configured contact ([TRANSPORT.md](docs/TRANSPORT.md) § “Both links active”):

1. Existing active session (resume)
2. **Parallel on Wi‑Fi:** coord lookup + coord bridge + public TCP (WAN) **and** mDNS → direct TCP (LAN) when the contact is on the local network — **both links stay active** when connected
3. **Mobile-data / CGNAT:** WAN (coord bridge) only when no active LAN
4. **LAN loss:** WAN is already connected — immediate fallback without tearing down coord
5. **Coord bridge** on the **Ghal Bol coord server** for NAT/CGNAT — WAN calls are bridged over WebSocket.

Peer **discovery over WAN requires coord** when both peers have internet. When coord is unreachable, **LAN (mDNS) still works**; the background node keeps retrying all configured coord servers. The app does **not** fall back to Kademlia DHT or public bootstrap peers for WAN discovery. Multiple coord servers are supported as a list (today a single production entry).

The system is designed to be:
- IPv6-first
- direct-communication-first
- reconnect-friendly
- mobile-aware

QUIC over UDP is the preferred transport because of:
- lower latency
- multiplexed streams
- better reconnect behavior
- improved mobile roaming support

---

# Synchronization Model

Ghal Bol is synchronization-oriented rather than stream-oriented.

Instead of treating communication as a permanent socket session, peers perform short-lived synchronization sessions where they:
- exchange pending messages
- exchange acknowledgements
- synchronize transcript state
- resume interrupted transfers

Synchronization sessions are:
- reconnectable
- resumable
- idempotent
- replay-safe

Every message has:
- globally unique ID
- sender identity
- recipient identity
- timestamp
- encrypted payload

The protocol assumes reconnects and temporary disconnections are normal.

---

# Message Delivery

**WAN text** uses the [`ghal_bol_delivery`](ghal_bol_delivery/) server — a temporary E2E encrypted mailbox. The server stores ciphertext only; it cannot decrypt. When the recipient acknowledges delivery, the payload is deleted (metadata retained for acks/TTL). This guarantees offline delivery when the recipient returns — the core requirement native connect WAN chat could not meet.

**LAN text** uses native connect direct streams (`mDNS` / TCP) when both peers share a LAN — same E2E envelope format, no server hop.

**Voice and video** use native connect on LAN and WAN (coord bridge) — realtime sessions where both peers must be online.

Typical WAN text flow:

1. Sender encrypts message locally and uploads ciphertext to delivery server.
2. Server notifies recipient (WebSocket push when online).
3. Recipient decrypts locally, sends delivery/read acks through the server.
4. Each device keeps its own transcript; ticks are recipient-authority only.

See [docs/GHAL_BOL_DELIVERY.md](docs/GHAL_BOL_DELIVERY.md) and [docs/DESIGN.md](docs/DESIGN.md).

---

# Future: Temporary Distributed Relay (Tier 2)

A decentralized peer-blob relay (Tier 2; Tier 3 paid backup — see [docs/PREMIUM_SERVICES.md](docs/PREMIUM_SERVICES.md)) remains a **future** option. **WAN text today** uses `ghal_bol_delivery`, not Tier 2.

---

# Mobile Networking Philosophy

Mobile networking is inherently unstable because of:
- NAT
- CGNAT
- WiFi/mobile switching
- app suspension
- battery optimizations and OEM autostart / “pause if unused” policies (Android: hub **`AndroidBackgroundReadiness`** after unlock — see `docs/DESIGN.md` § “Fixed 2026-07-05”)
- temporary reachability loss

Ghal Bol assumes:
- reconnects are normal
- sessions are temporary
- synchronization must be resumable
- presence is ephemeral

The protocol is therefore designed around deterministic synchronization instead of fragile long-lived socket assumptions.

---

# Protocol Design Principles

## Local Ownership

Peers own:
- identity
- transcript
- delivery state
- synchronization state

## Minimal Trust

Servers assist coordination but do not own:
- message history
- identities
- transcript truth
- communication state

## Idempotency

All synchronization operations must be replay-safe.

Duplicate synchronization attempts must never corrupt state or create inconsistent transcripts.

## Deterministic Behavior

The project prioritizes:
- predictable synchronization
- observable behavior
- practical reliability

over theoretical decentralization purity.

---

# Future Possibilities

Potential future extensions include:
- LAN-first synchronization
- direct WiFi communication
- voice/video calls
- encrypted attachment streaming
- decentralized relay reputation systems
- opportunistic peer caching
- relay incentives
- local network discovery

---

# Non-Goals

Ghal Bol is intentionally not:
- a cloud messaging platform
- a social network
- a centralized chat service
- a permanent message archive
- a phone-number-based identity system

---

# Project Direction

Ghal Bol explores a middle ground between fully centralized messengers and fully decentralized swarm-based systems.

By combining:
- centralized coordination
with
- direct peer synchronization

the project aims to provide:
- fast realtime communication
- resilient direct messaging
- low infrastructure dependence
- strong user ownership
- modern peer-to-peer networking

without the complexity and unpredictability of large-scale decentralized swarm architectures.

---

# Repository

Open this directory as the workspace root (the folder that contains this `README.md`).

| Path | Role |
|------|------|
| `ghal_bol_core/` | Rust core: identity, native connect sync engine, local stores |
| `ghal_bol_coord/` | Coordination server — see [ghal_bol_coord/README.md](ghal_bol_coord/README.md) |
| `ghal_bol_delivery/` | Delivery server (WAN text mailbox) — [docs/GHAL_BOL_DELIVERY.md](docs/GHAL_BOL_DELIVERY.md), home deploy [ghal_bol_delivery/deploy/](ghal_bol_delivery/deploy/) |
| `ghal_bol_ui/` | Flutter UI shell — [ghal_bol_ui/README.md](ghal_bol_ui/README.md) |
| `docs/` | [Design](docs/DESIGN.md), [transport](docs/TRANSPORT.md), [identity](docs/IDENTITY.md), [coord server](docs/COORDINATION_SERVER.md), [web site](docs/WEB_SITE.md), [doc index](docs/README.md) |
| `firebase.json` | Firebase Hosting for **ghalbol.com** (static web build) |
| `scripts/deploy_web_firebase.sh` | `flutter build web` + `firebase deploy --only hosting` |

---

# Production release (checklist)

**Full step-by-step guide:** [docs/PRODUCTION_RELEASE.md](docs/PRODUCTION_RELEASE.md) (P0 ship test → P1 infra → P2 Play Store)

**Coord server (live):** `https://coord.ghalbol.com` — smoke: `COORD_URL=https://coord.ghalbol.com ./ghal_bol_coord/deploy/smoke_coord.sh`

| Phase | Key steps |
|-------|-----------|
| **P0** | Release keystore → `flutter build apk/appbundle --release` → two-phone ship test |
| **P1** | CI green, VM systemd + reboot, commit & push |
| **P2** | [Privacy policy](docs/PRIVACY_POLICY.md) online → [Play listing](docs/PLAY_STORE_LISTING.md) → AAB upload |

Build/run details: [ghal_bol_ui/env/README.md](ghal_bol_ui/env/README.md). Server deploy: [ghal_bol_coord/deploy/README.md](ghal_bol_coord/deploy/README.md).
