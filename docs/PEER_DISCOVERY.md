# How Ghal Bol Peer Discovery and Direct Connection Works

This document explains how Ghal Bol peers discover each other and establish direct peer-to-peer communication using a centralized coordination server.

The goal of the architecture is:

- direct peer communication
- minimal infrastructure dependency
- decentralized communication ownership
- lightweight centralized coordination

The coordination server helps peers find each other, but it is not the primary communication transport.

**Invite domain:** `ghalbol.com`

**Invite shapes:** `https://ghalbol.com/connect/<public_key_hex>` (QR / share) and `ghalbol://connect/<public_key_hex>` (app deep link). Optional `?alias=…`.

**Web handoff:** If the app is not installed (or App Links are not verified), `https://ghalbol.com/connect/…` loads the static invite page on Firebase Hosting — it tries `ghalbol://`, offers copy link, and links to Play Store / [Linux download](WEB_SITE.md). See [WEB_SITE.md](WEB_SITE.md).

---

# Important Concept

The most important thing to understand is:

> The invite URL is NOT the actual peer connection.

The URL only identifies **which peer** (by `public_key_hex`) we want to reach.

Actual communication happens **directly between peers** after the app discovers the peer’s current network endpoints from the coordination server.

---

# Example Goal

Suppose a peer’s identity is their compressed secp256k1 public key (66 hex characters), for example:

```text
0324653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c
```

A web invite link:

```text
https://ghalbol.com/connect/0324653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c
```

Or a native app link:

```text
ghalbol://connect/0324653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c
```

At first glance, it may appear that the HTTPS URL **directly connects** to that device. That is **not** how networking works.

Instead:

- the URL **identifies** the peer (`public_key_hex`)
- the app asks the coordination server for **current reachable endpoints**
- the app then opens a **direct** transport (QUIC) to those endpoints

**Implementation:** libp2p dial + `/ghal-bol/msg/1.0.0` stream. Invite and coord flow unchanged — see [TRANSPORT.md](TRANSPORT.md).

---

# Correct Mental Model

## Wrong Mental Model

```text
https://ghalbol.com/connect/<public_key_hex>
        ↓ (magically routes HTTP to the peer’s phone)
peer device
```

This is incorrect. DNS and HTTPS terminate at **your** web/app-link layer; they do not open a socket to a random peer device.

---

## Correct Mental Model

```text
Invite URL identifies public_key_hex
        ↓
App asks coordination server: where is this key reachable now?
        ↓
Server returns endpoints (QUIC, IPv6/IPv4, online flag)
        ↓
App connects directly peer-to-peer and runs a sync session
```

This is the actual architecture.

---

# Complete Connection Flow

## Step 1 — User Opens an Invite

User opens either:

```text
https://ghalbol.com/connect/<public_key_hex>
```

or:

```text
ghalbol://connect/<public_key_hex>
```

The link means: **“Connect me to this identity.”**

On mobile, the HTTPS link is typically handled as an **App Link / deep link** into `ghal_bol_ui`; the browser is not the chat transport.

---

## Step 2 — Ghal Bol App Starts

The Flutter shell opens (or receives the deep link). The native worker (`ghal_bol`, including the Android background service) learns:

```text
Target identity = <public_key_hex>
```

The contact may be created locally if this is the first time seeing that key.

- **Scanner (guest):** saving the invite sets **`is_known: true`**, **`is_blocked: false`**.
- **Host (first inbound text only):** new row starts **`is_known: false`**, **`is_blocked: false`** → hub shows an **Unknown** control; in the room, **Add** / **Block** banner; first outbound send counts as **Add**.

Full UI and persistence rules: [DESIGN.md — Contact trust](DESIGN.md#contact-trust-is_known--is_blocked).

---

## Step 3 — App Contacts Coordination Server

The worker asks `ghal_bol_server`:

```text
Where is this public_key_hex currently reachable?
```

Example API (implemented today):

```text
GET /v1/peers/<public_key_hex>
```

---

## Step 4 — Coordination Server Responds

The server returns the peer’s registered endpoints and freshness (if the peer is still within presence TTL).

Example shape:

```json
{
  "public_key_hex": "0324653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c",
  "endpoints": [
    { "scheme": "quic", "host": "2001:db8::1", "port": 4433 }
  ],
  "ipv6": "2001:db8::1",
  "ipv4": "45.x.x.x",
  "transport_capabilities": ["quic", "sync-v1"],
  "last_heartbeat_unix_ms": 1779362014612
}
```

The app now knows:

- whether the peer is still **present** (heartbeat within TTL)
- which **addresses** to try
- which **transport** to use (QUIC first)

---

## Step 5 — Direct Peer Connection and Sync

The app connects **directly** to the peer, for example:

```text
QUIC to [2001:db8::1]:4433
```

Then it runs a **short sync session** (pending messages, acknowledgements, transcript cursor) — not a permanent cloud inbox.

`ghalbol.com` and `ghal_bol_server` are **not** on the data path for message bodies after discovery.

---

# Role of the Coordination Server

The coordination server (`ghal_bol_server`) acts as:

- presence tracker
- endpoint registry
- signed registration gate
- peer lookup for configured contacts

The server does **not**:

- permanently relay all chat traffic (by default)
- permanently store message content
- own transcripts or delivery truth

Peers remain the owners of communication state on disk; the server assists **reachability**.

---

# Peer Registration Flow

Each online peer registers with the coordination server (from the background worker on Android, or the desktop sidecar).

Flow:

1. `POST /v1/register/challenge` with `public_key_hex`
2. Server returns a nonce
3. Peer signs canonical message `ghal_bol:register:v1` + nonce + key
4. `POST /v1/register` with signature and current endpoints
5. Peer sends `POST /v1/heartbeat` periodically while online

Example stored record (conceptual):

```json
{
  "public_key_hex": "0324653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c",
  "endpoints": [
    { "scheme": "quic", "host": "2001:db8::1", "port": 4433 },
    { "scheme": "quic", "host": "45.x.x.x", "port": 55000 }
  ],
  "last_heartbeat_unix_ms": 1779362014612,
  "transport_capabilities": ["quic", "sync-v1"]
}
```

This ensures:

- peers cannot register endpoints for keys they do not own
- online status stays fresh (TTL)
- lookup returns dialable targets, not stale DHT guesses

---

# Why This Architecture Is Useful

## Direct communication

Peers exchange data directly whenever a route exists — lower latency and less server bandwidth than cloud-routed chat.

## Lightweight infrastructure

The server coordinates; it does not need to be a giant message store.

## Better privacy posture

The coordination server should not hold transcript bodies. It still sees **metadata** (who registered, which IPs, heartbeat times). That is the trade for practical mobile reachability.

## IPv6-first dialing

Preferred connection order:

1. Existing active session (resume sync)
2. Direct IPv6
3. Direct IPv4
4. Tier 2 peer relay fallback (planned — [COMMUNICATION_TIERS.md](COMMUNICATION_TIERS.md))

---

# Invite Links: HTTPS vs Custom Scheme

## HTTPS — `https://ghalbol.com/connect/...`

Used for:

- QR codes printed or shared as normal URLs
- Android App Links into `ghal_bol_ui` when `/.well-known/assetlinks.json` is verified
- human-readable invites

When the app opens, networking is native. When it does not, the **static web** invite page ([WEB_SITE.md](WEB_SITE.md)) handles handoff only — not chat transport.

## Custom scheme — `ghalbol://connect/...`

Used for:

- direct “open in Ghal Bol” intents on devices that already have the app installed
- avoiding the impression that peers are HTTP servers

Both forms carry the same **`public_key_hex`**; only the handler differs.

---

# Difference Between Invite Layer and Peer Transport

| Layer | Role |
|-------|------|
| **Invite** (`ghalbol.com`, `ghalbol://`, QR) | Identify `public_key_hex`, open app |
| **Coordination** (`ghal_bol_server`) | Presence, signed register, lookup |
| **Transport** (QUIC, sync v1) | Messages, acks, direct sync between peers |

---

# Final Simplified Flow

```text
User opens:
  https://ghalbol.com/connect/<public_key_hex>
  or ghalbol://connect/<public_key_hex>

        ↓

Ghal Bol app + native worker start

        ↓

Worker asks ghal_bol_server:
  GET /v1/peers/<public_key_hex>

        ↓

Server replies with current QUIC endpoints (if online)

        ↓

Worker connects directly and syncs with peer

        ↓

Flutter UI reads local transcript (FFI); no cloud inbox
```

---

# Core Idea

> The invite link identifies the peer.  
> The coordination server helps discover the peer.  
> Communication happens directly between peers.

`ghalbol.com` is a **bootstrap and discovery** layer, not the chat transport.

This separation supports:

- lightweight infrastructure
- direct communication
- peer-owned transcripts
- realtime sync when both sides are reachable
- a simpler operational model than large decentralized swarms
