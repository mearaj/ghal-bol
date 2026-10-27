# Coordination server (`ghalbol_coord`)

**Tier 1** — signed peer **presence**, **endpoint lookup**, and a co-located **WAN call byte bridge**. No message bodies, transcripts, or DM mailboxes.

| Tier | Role | Product path |
|------|------|----------------|
| **1 — coord** | Who is online, dialable public TCP (when any), pair outbound WSS for calls | This doc |
| **2 — delivery** | **Text** mailbox (E2E, offline) | [GHALBOL_DELIVERY.md](GHALBOL_DELIVERY.md) |
| **3 — premium relay** | Optional paid backup (not shipping on coord1) | [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) |

The Makepad app (`ghalbol_app`) calls **`ghalbol_core::host`** in-process. After unlock, **`host::start_network`** starts native connect (calls) and delivery (text); **`coord_runtime`** talks to coord over HTTPS. **Text** always uses the delivery server when configured; **voice/video calls** use native connect (LAN or coord bridge). See [DESIGN.md](DESIGN.md), [AGENTS.md](../AGENTS.md), and [TRANSPORT.md](TRANSPORT.md).

```text
                    ┌─────────────────────────────────────┐
                    │ ghalbol_coord (SQLite + in-memory) │
                    │  POST register / heartbeat          │
                    │  GET  /v1/peers/{identity}          │
                    │  WSS  /v1/bridge/connect (calls)    │
                    └──────────────▲──────────────────────┘
           register / lookup      │        bridge request + WSS
                                  │
              ┌───────────────────┴───────────────────┐
              │ ghalbol_core (coord_runtime,       │
              │ connect bridge_ws, p2p_runtime)     │
              └───────────────────▲─────────────────┘
                                  │ host::
              ┌───────────────────┴───────────────────┐
              │ ghalbol_app (Makepad 2)                │
              └─────────────────────────────────────────┘
```

---

## Client configuration

Set coord base URLs in `apps/ghalbol/env/development.env` / `production.env` (baked into the app — see [env/README.md](../env/README.md); a local `env/.env.<profile>` overrides them for desktop runs):

```bash
GHALBOL_COORD_URLS=["https://coord.ghalbol.com"]
# home example:
GHALBOL_COORD_URLS=["https://coord1.ghalbol.com:8443"]
```

JSON array or comma-separated list; no hardcoded URLs in the app binary. Optional: `GHALBOL_COORD_INSECURE_TLS=1` for local HTTPS with a self-signed cert.

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

Implementation: `should_throttle_register`, `spawn_register_presence_inner`, `coord_register_tick` in `ghalbol_core/src/coord_runtime.rs`.

**CGNAT / mobile-only peers** often have **no** public routable IPv4 to post. They still place **WAN calls** via `POST /v1/bridge/*` + WSS; **text** uses delivery only.

---

## Presence model

| Source | Stored in SQLite | Removed |
|--------|------------------|---------|
| Client `POST /v1/register` | **Public routable IPv4** `tcp`/`quic` endpoints (peer inbound DM listen), capabilities, optional `ipv4`/`ipv6` hints | Heartbeat TTL expiry (~90 s without heartbeat) |
| **Rejected at POST** | RFC1918 LAN, loopback, CGNAT-only hosts posted as “public”, empty endpoint list | — |

Server validation mirrors the client: `tcp` requires a globally routable IPv4 host ([`routes.rs`](../ghalbol_coord/src/routes.rs) `validate_endpoints`). Stored rows are filtered again in [`presence.rs`](../ghalbol_coord/src/presence.rs).

---

## WAN call bridge

Opaque byte pipe: coord pairs two **outbound** WebSocket connections; peers run the same Noise XX + mux as LAN inside the tunnel. Full pairing flow, Noise roles, and limits: [GHALBOL_CONNECT_V1.md](GHALBOL_CONNECT_V1.md) § WAN call bridge.

Summary:

1. Caller: `POST /v1/bridge/challenge` → `POST /v1/bridge/request` (signed).
2. Callee learns the offer via delivery push and/or `GET /v1/bridge/pending?identity_wire=…`.
3. Both dial `GET /v1/bridge/connect?bridge_id=…&token=…` (WebSocket upgrade).
4. Coord forwards binary frames until hangup, idle timeout, session max, or byte cap.

Bridge **`connect_url`** in JSON responses uses `GHALBOL_COORD_PUBLIC_URL` / `GHALBOL_COORD_BASE_URL` on the server (must match what clients use for HTTPS/WSS).

### Bridge limits (server env)

| Parameter | Default | Env |
|-----------|---------|-----|
| Session max duration | 4 h | `GHALBOL_BRIDGE_MAX_SECS` |
| Unpaired pending TTL | 90 s | `GHALBOL_BRIDGE_PENDING_SECS` |
| Max relayed bytes | unlimited (`0`) | `GHALBOL_BRIDGE_MAX_BYTES` |
| Max concurrent bridges per identity | 4 | `GHALBOL_BRIDGE_MAX_PER_PEER` |
| Idle timeout | 120 s | `GHALBOL_BRIDGE_IDLE_SECS` |

A new `POST /v1/bridge/request` for the same caller→callee (or same `call_id`) **replaces** stale unpaired entries so WSS retries are not blocked.

---

## Run and smoke

### Loopback (dev)

```bash
cargo run -p ghalbol_coord
curl -s http://127.0.0.1:8765/health | jq
```

Defaults: listen **`0.0.0.0:8765`** (`GHALBOL_COORD_LISTEN`; prod/home units use **`127.0.0.1:8765`** behind nginx); SQLite under `~/.local/share/com.ghalbol.coord/ghalbol_server/coord.db` (namespace **`com.ghalbol.coord`**, not the app `com.ghalbol`).

### Automated smoke

```bash
./ghalbol_coord/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghalbol_coord/deploy/smoke_coord.sh
COORD_URL=https://coord1.ghalbol.com:8443 ./ghalbol_coord/deploy/smoke_coord.sh
```

Runs crate tests, builds `coord_client`, then `health` + `demo-two-peers` against `COORD_URL` when set.

Manual CLI:

```bash
cargo build -p ghalbol_coord --release
./target/release/coord_client http://127.0.0.1:8765 demo-two-peers
```

Point the app at your server: set `GHALBOL_COORD_URLS` to a reachable `http://…:8765` (or HTTPS) and restart after unlock/`start_network`.

---

## Production (`coord.ghalbol.com`)

- **HTTPS:** nginx `:443` → `127.0.0.1:8765` ([`deploy/nginx-coord.conf`](../ghalbol_coord/deploy/nginx-coord.conf)).
- **Install:** `./ghalbol_coord/deploy/deploy_server.sh` — see [deploy/GCP.md](../ghalbol_coord/deploy/GCP.md).

```bash
GHALBOL_COORD_URLS=["https://coord.ghalbol.com"]
COORD_URL=https://coord.ghalbol.com ./ghalbol_coord/deploy/smoke_coord.sh
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
./ghalbol_coord/deploy/install_coord1_home.sh
./ghalbol_coord/deploy/verify_coord1.sh
```

```bash
GHALBOL_COORD_URLS=["https://coord1.ghalbol.com:8443"]
```

Bridge WSS: `wss://coord1.ghalbol.com:8443/v1/bridge/connect`. DDNS + HTTPS once: [deploy/COORD1_HOME.md](../ghalbol_coord/deploy/COORD1_HOME.md).

---

## HTTP API (v1)

Routes: [`ghalbol_coord/src/routes.rs`](../ghalbol_coord/src/routes.rs).

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

Same string in JSON (`public_key_hex`) and in `/v1/peers/{…}` paths. **Bare hex** (no `:`) means implicit **`secp256k1`**; other algorithms use `algorithm:hex` (e.g. `ed25519:…` → path `ed25519%3A…`). Server: `normalize_identity_wire()` in [`identity.rs`](../ghalbol_coord/src/identity.rs).

**Register signature:** canonical message `ghalbol:register:v1\n<nonce_hex>\n<identity_wire>` (wire lowercased), signed with the device identity key — secp256k1 ECDSA DER, ed25519, or ecdsa-p256 DER per algorithm ([`auth.rs`](../ghalbol_coord/src/auth.rs)).

**Bridge request signature:** domain `ghalbol:bridge:request:v1` (see challenge response `message_domain`).

**`endpoints[]`:** dial addresses `{ scheme, host, port }` — not identity strings. Default capabilities if omitted: `tcp`, `sync-v1`.

---

## Server environment

| Variable | Purpose |
|----------|---------|
| `GHALBOL_COORD_LISTEN` / `GHALBOL_SERVER_LISTEN` | Bind address (default `0.0.0.0:8765`) |
| `GHALBOL_COORD_DB` / `GHALBOL_SERVER_DB` | SQLite path or directory |
| `GHALBOL_COORD_CHALLENGE_TTL_SECS` | Register/bridge challenge TTL (default 120 s) |
| `GHALBOL_COORD_PRESENCE_TTL_SECS` | Offline threshold (default 90 s) |
| `GHALBOL_COORD_PURGE_INTERVAL_SECS` | Background purge (default 30 s) |
| `GHALBOL_COORD_PUBLIC_URL` / `GHALBOL_COORD_BASE_URL` | Public HTTPS base for `connect_url` in bridge JSON |
| `GHALBOL_DDNS_CREDENTIALS` | Home coord1: in-process GoDaddy DDNS |
| `GHALBOL_BRIDGE_*` | Bridge limits (table above) |

Deploy walkthrough: [ghalbol_coord/deploy/README.md](../ghalbol_coord/deploy/README.md), [ghalbol_coord/README.md](../ghalbol_coord/README.md).

---

## Troubleshooting

### HTTP access patterns

| Pattern | Likely cause | Action |
|---------|--------------|--------|
| `GET /v1/bridge/connect` **400**, client logs missing `upgrade` | nginx not forwarding WebSocket | Home: `./ghalbol_coord/deploy/enable_coord1_https.sh`; GCP: add `Upgrade` / `Connection` on `/v1/bridge/connect` |
| `GET /v1/peers/…` **404** | Never registered, TTL expired, or wrong coord URL in app | Check register/heartbeat logs; verify `GHALBOL_COORD_URLS`; `curl /health` |
| Register **400** on `endpoints` | LAN/CGNAT host posted as public TCP | Expected on mobile-only; use bridge for calls, delivery for text |
| Heartbeat **404** / “peer not on coord” | Row expired or DB wiped | Client forces re-register; check server uptime and TTL |
| Bridge **400** “bad token” / “unknown bridge” | Expired pending, wrong token side, or second leg too slow | Retry `bridge/request`; check `GHALBOL_BRIDGE_PENDING_SECS` |
| Bridge **400** “bridge limit per identity” | Too many concurrent/pending bridges | Wait for idle close or hang up other calls |

### Session checklist

1. Coord process running (`ghalbol-coord1` user service or GCP systemd).
2. `curl -s https://…/health | jq` — `database: true`, `bridge: true`.
3. Smoke: `COORD_URL=… ./ghalbol_coord/deploy/smoke_coord.sh`.
4. App rebuilt with correct `GHALBOL_COORD_URLS`; unlock → `start_network`.
5. For home calls across the internet: router **8443** open; WSS reaches `/v1/bridge/connect`.

### Tests

```bash
cargo test -p ghalbol_coord --test http_api
cargo test -p ghalbol_coord --test e2e_production
cargo test -p ghalbol_coord
```
