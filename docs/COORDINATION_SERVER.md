# Coordination server (`ghal_bol_coord`)

**Tier 1** — signed peer **presence**, **endpoint lookup**, and a co-located **WAN call byte bridge**. No message bodies, transcripts, or DM mailboxes.

| Tier | Role | Product path |
|------|------|----------------|
| **1 — coord** | Who is online, dialable public TCP (when any), pair outbound WSS for calls | This doc |
| **2 — delivery** | WAN **text** mailbox (E2E, offline) | [GHAL_BOL_DELIVERY.md](GHAL_BOL_DELIVERY.md) |
| **3 — premium relay** | Optional paid backup (not shipping on coord1) | [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) |

The Makepad app (`ghal_bol_app`) calls **`ghal_bol_core::host`** in-process. After unlock, **`host::start_network`** starts native connect; **`coord_runtime`** talks to coord over HTTPS. **WAN text** always uses the delivery server when configured; **WAN voice/video calls** use the coord bridge when LAN/mDNS is not enough. See [DESIGN.md](DESIGN.md), [AGENTS.md](../AGENTS.md), and [TRANSPORT.md](TRANSPORT.md).

```text
                    ┌─────────────────────────────────────┐
                    │ ghal_bol_coord (SQLite + in-memory) │
                    │  POST register / heartbeat          │
                    │  GET  /v1/peers/{identity}          │
                    │  WSS  /v1/bridge/connect (calls)    │
                    └──────────────▲──────────────────────┘
           register / lookup      │        bridge request + WSS
                                  │
              ┌───────────────────┴───────────────────┐
              │ ghal_bol_core (coord_runtime,       │
              │ connect bridge_ws, p2p_runtime)     │
              └───────────────────▲─────────────────┘
                                  │ host::
              ┌───────────────────┴───────────────────┐
              │ ghal_bol_app (Makepad 2)                │
              └─────────────────────────────────────────┘
```

---

## Client configuration

Set coord base URLs in `env/.env.development` (debug) or `env/.env.production` (release):

```bash
GHAL_BOL_COORD_URLS=["https://coord.ghalbol.com"]
# home example:
GHAL_BOL_COORD_URLS=["https://coord1.ghalbol.com:8443"]
```

JSON array or comma-separated list; no hardcoded URLs in the app binary. Optional: `GHAL_BOL_COORD_INSECURE_TLS=1` for local HTTPS with a self-signed cert.

**Multi-server policy:** register and heartbeat on **every** URL in the list. **Lookup** (`GET /v1/peers/…`) tries servers **in order** and stops on the first success for that dial attempt; after a disconnect, repeat from the first server.

Broader connectivity rules: [TRANSPORT.md](TRANSPORT.md) § Connectivity lifecycle.

### Register and heartbeat (`coord_runtime.rs`)

**Goal:** stay present on coord with a **correct** row when the device has publishable endpoints — steady, not spam. **`coord_registered`** in logs means a recent successful register or heartbeat on at least one configured server.

| Trigger | Action |
|---------|--------|
| Publishable **public TCP** endpoint set **changed** | Full `POST /v1/register` on **all** coord URLs (`schedule_register_presence_force`) |
| Last register **failed** or never succeeded | Retry register (min gap **2 s**) |
| **Network handover** (endpoints invalidated) | Re-register when a new publishable set is known |
| **Presence stale** — no successful heartbeat in **~70 s** (`PRESENCE_STALE_MS`; server TTL **90 s**) | Force re-register |
| **`host::start_network`** | Register when listen endpoints are known; do not trust a prior process |
| Endpoints **unchanged** and recently registered | **Throttle** full register (min gap **10 s** when registered); **`POST /v1/heartbeat` every 25 s** |

Implementation: `should_throttle_register`, `spawn_register_presence_inner`, `coord_register_tick` in `ghal_bol_core/src/coord_runtime.rs`.

**CGNAT / mobile-only peers** often have **no** public routable IPv4 to post. They still place **WAN calls** via `POST /v1/bridge/*` + WSS; **WAN text** uses delivery only. LAN chat uses mDNS/direct TCP.

---

## Presence model

| Source | Stored in SQLite | Removed |
|--------|------------------|---------|
| Client `POST /v1/register` | **Public routable IPv4** `tcp`/`quic` endpoints (peer inbound DM listen), capabilities, optional `ipv4`/`ipv6` hints | Heartbeat TTL expiry (~90 s without heartbeat) |
| **Rejected at POST** | RFC1918 LAN, loopback, CGNAT-only hosts posted as “public”, empty endpoint list | — |

Server validation mirrors the client: `tcp` requires a globally routable IPv4 host ([`routes.rs`](../ghal_bol_coord/src/routes.rs) `validate_endpoints`). Stored rows are filtered again in [`presence.rs`](../ghal_bol_coord/src/presence.rs).

---

## WAN call bridge

Opaque byte pipe: coord pairs two **outbound** WebSocket connections; peers run the same Noise XX + mux as LAN inside the tunnel. Full pairing flow, Noise roles, and limits: [GHAL_BOL_CONNECT_V1.md](GHAL_BOL_CONNECT_V1.md) § WAN call bridge.

Summary:

1. Caller: `POST /v1/bridge/challenge` → `POST /v1/bridge/request` (signed).
2. Callee learns the offer via delivery push and/or `GET /v1/bridge/pending?identity_wire=…`.
3. Both dial `GET /v1/bridge/connect?bridge_id=…&token=…` (WebSocket upgrade).
4. Coord forwards binary frames until hangup, idle timeout, session max, or byte cap.

Bridge **`connect_url`** in JSON responses uses `GHAL_BOL_COORD_PUBLIC_URL` / `GHAL_BOL_COORD_BASE_URL` on the server (must match what clients use for HTTPS/WSS).

### Bridge limits (server env)

| Parameter | Default | Env |
|-----------|---------|-----|
| Session max duration | 4 h | `GHAL_BOL_BRIDGE_MAX_SECS` |
| Unpaired pending TTL | 90 s | `GHAL_BOL_BRIDGE_PENDING_SECS` |
| Max relayed bytes | unlimited (`0`) | `GHAL_BOL_BRIDGE_MAX_BYTES` |
| Max concurrent bridges per identity | 4 | `GHAL_BOL_BRIDGE_MAX_PER_PEER` |
| Idle timeout | 120 s | `GHAL_BOL_BRIDGE_IDLE_SECS` |

A new `POST /v1/bridge/request` for the same caller→callee (or same `call_id`) **replaces** stale unpaired entries so WSS retries are not blocked.

---

## Run and smoke

### Loopback (dev)

```bash
cargo run -p ghal_bol_coord
curl -s http://127.0.0.1:8765/health | jq
```

Defaults: listen **`0.0.0.0:8765`** (`GHAL_BOL_COORD_LISTEN`; prod/home units use **`127.0.0.1:8765`** behind nginx); SQLite under `~/.local/share/com.ghalbol.coord/ghalbol_server/coord.db` (namespace **`com.ghalbol.coord`**, not the app `com.ghalbol`).

### Automated smoke

```bash
./ghal_bol_coord/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_coord/deploy/smoke_coord.sh
COORD_URL=https://coord1.ghalbol.com:8443 ./ghal_bol_coord/deploy/smoke_coord.sh
```

Runs crate tests, builds `coord_client`, then `health` + `demo-two-peers` against `COORD_URL` when set.

Manual CLI:

```bash
cargo build -p ghal_bol_coord --release
./target/release/coord_client http://127.0.0.1:8765 demo-two-peers
```

Point the app at your server: set `GHAL_BOL_COORD_URLS` to a reachable `http://…:8765` (or HTTPS) and restart after unlock/`start_network`.

---

## Production (`coord.ghalbol.com`)

- **HTTPS:** nginx `:443` → `127.0.0.1:8765` ([`deploy/nginx-coord.conf`](../ghal_bol_coord/deploy/nginx-coord.conf)).
- **Install:** `./ghal_bol_coord/deploy/deploy_server.sh` — see [deploy/GCP.md](../ghal_bol_coord/deploy/GCP.md).

```bash
GHAL_BOL_COORD_URLS=["https://coord.ghalbol.com"]
COORD_URL=https://coord.ghalbol.com ./ghal_bol_coord/deploy/smoke_coord.sh
```

**nginx:** must proxy **`/v1/bridge/connect`** with WebSocket upgrade headers (`Upgrade`, `Connection`) on the same vhost as coord HTTP, or WSS dials fail with HTTP 400.

---

## Home (`coord1.ghalbol.com`)

| Path | Port |
|------|------|
| Coord HTTPS + bridge WSS | nginx **8443** → loopback **8765** |
| Delivery WSS (same host, separate binary) | **55003** → nginx → delivery **8770** |

**Router:** forward **8443** and **55003** (TCP) to the home host — not a separate “relay” port on coord.

```bash
./ghal_bol_coord/deploy/install_coord1_home.sh
./ghal_bol_coord/deploy/verify_coord1.sh
```

```bash
GHAL_BOL_COORD_URLS=["https://coord1.ghalbol.com:8443"]
```

Bridge WSS: `wss://coord1.ghalbol.com:8443/v1/bridge/connect`. DDNS + HTTPS once: [deploy/COORD1_HOME.md](../ghal_bol_coord/deploy/COORD1_HOME.md).

---

## HTTP API (v1)

Routes: [`ghal_bol_coord/src/routes.rs`](../ghal_bol_coord/src/routes.rs).

| Method | Path | Body / query | Purpose |
|--------|------|--------------|---------|
| GET | `/health` | — | `ok`, `service`, `database`, `bridge` |
| POST | `/v1/register/challenge` | `{ "public_key_hex" }` | Nonce for register signature |
| POST | `/v1/register` | `public_key_hex`, `nonce_hex`, `signature_hex`, `endpoints[]`, optional `ipv4` / `ipv6` / `transport_capabilities` | Upsert presence |
| POST | `/v1/heartbeat` | `{ "public_key_hex" }` | Refresh `last_heartbeat` |
| GET | `/v1/peers/{identity_wire}` | Path = identity wire (URL-encode `:` as `%3A`) | One online peer or **404** |
| GET | `/v1/peers` | — | All peers within TTL |
| POST | `/v1/bridge/challenge` | `{ "caller_identity_wire" }` | Nonce for bridge request |
| POST | `/v1/bridge/request` | `caller_identity_wire`, `peer_identity_wire`, `call_id`, `nonce_hex`, `signature_hex` | Create bridge; returns `bridge_id`, `token`, `connect_url` |
| GET | `/v1/bridge/pending` | `?identity_wire=` (callee) | Incoming bridge offers |
| GET | `/v1/bridge/connect` | `?bridge_id=&token=` | **WebSocket upgrade** — opaque byte pipe |

### Identity wire

Same string in JSON (`public_key_hex`) and in `/v1/peers/{…}` paths. **Bare hex** (no `:`) means implicit **`secp256k1`**; other algorithms use `algorithm:hex` (e.g. `ed25519:…` → path `ed25519%3A…`). Server: `normalize_identity_wire()` in [`identity.rs`](../ghal_bol_coord/src/identity.rs).

**Register signature:** canonical message `ghal_bol:register:v1\n<nonce_hex>\n<identity_wire>` (wire lowercased), signed with the device identity key — secp256k1 ECDSA DER, ed25519, or ecdsa-p256 DER per algorithm ([`auth.rs`](../ghal_bol_coord/src/auth.rs)).

**Bridge request signature:** domain `ghal_bol:bridge:request:v1` (see challenge response `message_domain`).

**`endpoints[]`:** dial addresses `{ scheme, host, port }` — not identity strings. Default capabilities if omitted: `tcp`, `sync-v1`.

---

## Server environment

| Variable | Purpose |
|----------|---------|
| `GHAL_BOL_COORD_LISTEN` / `GHAL_BOL_SERVER_LISTEN` | Bind address (default `0.0.0.0:8765`) |
| `GHAL_BOL_COORD_DB` / `GHAL_BOL_SERVER_DB` | SQLite path or directory |
| `GHAL_BOL_COORD_CHALLENGE_TTL_SECS` | Register/bridge challenge TTL (default 120 s) |
| `GHAL_BOL_COORD_PRESENCE_TTL_SECS` | Offline threshold (default 90 s) |
| `GHAL_BOL_COORD_PURGE_INTERVAL_SECS` | Background purge (default 30 s) |
| `GHAL_BOL_COORD_PUBLIC_URL` / `GHAL_BOL_COORD_BASE_URL` | Public HTTPS base for `connect_url` in bridge JSON |
| `GHAL_BOL_DDNS_CREDENTIALS` | Home coord1: in-process GoDaddy DDNS |
| `GHAL_BOL_BRIDGE_*` | Bridge limits (table above) |

Deploy walkthrough: [ghal_bol_coord/deploy/README.md](../ghal_bol_coord/deploy/README.md), [ghal_bol_coord/README.md](../ghal_bol_coord/README.md).

---

## Troubleshooting

### HTTP access patterns

| Pattern | Likely cause | Action |
|---------|--------------|--------|
| `GET /v1/bridge/connect` **400**, client logs missing `upgrade` | nginx not forwarding WebSocket | Home: `./ghal_bol_coord/deploy/enable_coord1_https.sh`; GCP: add `Upgrade` / `Connection` on `/v1/bridge/connect` |
| `GET /v1/peers/…` **404** | Never registered, TTL expired, or wrong coord URL in app | Check register/heartbeat logs; verify `GHAL_BOL_COORD_URLS`; `curl /health` |
| Register **400** on `endpoints` | LAN/CGNAT host posted as public TCP | Expected on mobile-only; use bridge for calls, delivery for text |
| Heartbeat **404** / “peer not on coord” | Row expired or DB wiped | Client forces re-register; check server uptime and TTL |
| Bridge **400** “bad token” / “unknown bridge” | Expired pending, wrong token side, or second leg too slow | Retry `bridge/request`; check `GHAL_BOL_BRIDGE_PENDING_SECS` |
| Bridge **400** “bridge limit per identity” | Too many concurrent/pending bridges | Wait for idle close or hang up other calls |

### Session checklist

1. Coord process running (`ghal-bol-coord1` user service or GCP systemd).
2. `curl -s https://…/health | jq` — `database: true`, `bridge: true`.
3. Smoke: `COORD_URL=… ./ghal_bol_coord/deploy/smoke_coord.sh`.
4. App rebuilt with correct `GHAL_BOL_COORD_URLS`; unlock → `start_network`.
5. For home calls across the internet: router **8443** open; WSS reaches `/v1/bridge/connect`.

### Tests

```bash
cargo test -p ghal_bol_coord --test http_api
cargo test -p ghal_bol_coord --test e2e_production
cargo test -p ghal_bol_coord
```
