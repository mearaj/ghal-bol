# ghal_bol_coord

Production **Tier 1 coordination** server for Ghal Bol: signed peer registration, SQLite presence, endpoint lookup, and a co-located **WAN call byte bridge** (pairs two outbound WebSockets and forwards opaque ciphertext — see [docs/GHAL_BOL_CONNECT_V1.md](../docs/GHAL_BOL_CONNECT_V1.md)).

The bridge does **not** store message bodies or transcripts, and is **not** the Tier 2 peer blob relay or the Tier 3 paid backup relay (see [docs/PREMIUM_SERVICES.md](../docs/PREMIUM_SERVICES.md)).

## Deployments

| Host | Install | Notes |
|------|---------|-------|
| **Home** `coord1.ghalbol.com` | `./deploy/install_coord1_home.sh` | HTTPS + bridge WSS on **8443** — [deploy/COORD1_HOME.md](deploy/COORD1_HOME.md) |
| **GCP** `coord.ghalbol.com` | `./deploy/deploy_server.sh` | See [deploy/GCP.md](deploy/GCP.md) |
| **Loopback smoke** | `cargo run -p ghal_bol_coord` | `127.0.0.1:8765` |

Full walkthrough: **[deploy/README.md](deploy/README.md)**.

## Run (loopback smoke)

```bash
cargo run -p ghal_bol_coord
```

Defaults:

| Setting | Default |
|---------|---------|
| Listen | `127.0.0.1:8765` |
| SQLite | `~/.local/share/com.ghal_bol.coord/ghal_bol_coord/coord.db` |

Same data root namespace as the coord server only (`com.ghal_bol.coord` — not the Makepad app `com.ghalbol`).

## Server smoke (no Makepad)

```bash
./ghal_bol_coord/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_coord/deploy/smoke_coord.sh
COORD_URL=https://coord1.ghalbol.com:8443 ./ghal_bol_coord/deploy/smoke_coord.sh
```

Manual CLI: `cargo build -p ghal_bol_coord --release` then `./target/release/coord_client http://127.0.0.1:8765 demo-two-peers`. See [docs/COORDINATION_SERVER.md](../docs/COORDINATION_SERVER.md).

## Test

```bash
cargo test -p ghal_bol_coord --test e2e_production
cargo test -p ghal_bol_coord --test http_api
cargo test -p ghal_bol_coord
```

## Presence model (WAN directory)

| Source | What may appear in SQLite | When removed |
|--------|---------------------------|--------------|
| Client `POST /v1/register` | **Public routable IPv4 TCP** (peer’s own inbound DM listen) and capabilities | Heartbeat TTL expiry |
| **Never** | LAN RFC1918, CGNAT-only endpoints posted as “public” | Rejected at `POST` or filtered at store |

CGNAT/mobile peers reach each other for **calls** via the WAN bridge (`/v1/bridge/*`). See [docs/TRANSPORT.md](../docs/TRANSPORT.md) § “Hybrid coord presence”.

## HTTP API (v1)

| Method | Path | Body |
|--------|------|------|
| GET | `/health` | — (`database: true` when SQLite answers; `bridge: true`) |
| POST | `/v1/register/challenge` | `{ "public_key_hex": "<identity wire>" }` |
| POST | `/v1/register` | `public_key_hex`, `nonce_hex`, `signature_hex`, `endpoints[]`, optional `ipv4` / `ipv6` / `transport_capabilities` |
| POST | `/v1/heartbeat` | `{ "public_key_hex": "<identity wire>" }` |
| GET | `/v1/peers/{[algo:]public_key_hex}` | Identity wire in path (URL-encode `:` as `%3A`) |
| GET | `/v1/peers` | — online peers (heartbeat within TTL) |
| POST | `/v1/bridge/challenge` | Bridge auth challenge |
| POST | `/v1/bridge/request` | Request a call bridge to a peer |
| GET | `/v1/bridge/pending` | Pending bridge offers for this identity |
| GET | `/v1/bridge/connect` | WebSocket upgrade — opaque byte pipe |

## Environment

See [docs/COORDINATION_SERVER.md](../docs/COORDINATION_SERVER.md) and `deploy/README.md` for `GHAL_BOL_COORD_*` / bridge limits.
