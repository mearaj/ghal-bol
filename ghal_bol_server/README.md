# ghal_bol_server

Production **Tier 1 coordination** server for Ghal Bol: signed peer registration, SQLite presence, endpoint lookup.

It also runs a co-located **libp2p Circuit Relay v2** node for NAT/CGNAT traversal (clients reserve a circuit; coord lists `/p2p-circuit` dial addrs for WAN peers). The relay carries only transient end-to-end-encrypted transport frames — it does **not** store message bodies or transcripts, and is **not** the Tier 2 peer blob relay or the Tier 3 paid backup relay (see [docs/PREMIUM_SERVICES.md](../docs/PREMIUM_SERVICES.md)).

## Deployments

| Host | Install | Relay TCP |
|------|---------|-----------|
| **Home** `coord1.ghalbol.com` | `./deploy/install_coord1_home.sh` | **`:55002`** (router forward) |
| **GCP** `coord.ghalbol.com` | `./deploy/deploy_server.sh` | **`:4002`** |
| **Loopback smoke** | `cargo run -p ghal_bol_server` | `:4002` default |

Full walkthrough: **[deploy/README.md](deploy/README.md)**.

## Run (loopback smoke)

```bash
cargo run -p ghal_bol_server
```

With public relay advertised (home/GCP-style):

```bash
GHAL_BOL_RELAY_PUBLIC_HOST=coord1.ghalbol.com cargo run -p ghal_bol_server
```

Defaults:

| Setting | Default |
|---------|---------|
| Listen | `127.0.0.1:8765` |
| SQLite | `~/.local/share/com.ghalbol.coord/ghalbol_server/coord.db` |

Same data root namespace as the coord server only (`com.ghalbol.coord` — not the Flutter app `com.ghalbol`).

## Server smoke (no Flutter)

```bash
./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=https://coord1.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

Manual CLI: `cargo build -p ghal_bol_server --release` then `./target/release/coord_client http://127.0.0.1:8765 demo-two-peers`. See [docs/COORDINATION_SERVER.md](../docs/COORDINATION_SERVER.md).

## Test

```bash
cargo test -p ghal_bol_server --test e2e_production
cargo test -p ghal_bol_server --test http_api
cargo test -p ghal_bol_server
```

## Presence model (WAN directory)

| Source | What may appear in SQLite | When removed |
|--------|---------------------------|--------------|
| Client `POST /v1/register` | **Public routable IPv4 TCP** only (peer’s own inbound DM listen) | Heartbeat TTL expiry (`GHAL_BOL_SERVER_PRESENCE_TTL_SECS`) |
| Relay `reservation ACCEPTED` | `libp2p` `/p2p-circuit/…` (server-authoritative) | Relay reservation ends — **circuit row only**; public TCP from `POST` is kept |
| **Never** | LAN RFC1918, CGNAT-only, relay bootstrap `GET /v1/relay` host:port, client-posted `/p2p-circuit` | Rejected at `POST` (400) or filtered at store |

CGNAT/mobile peers rely on the relay row. Desktop peers with UPnP/public IP may have **both** public TCP and a circuit row. See [docs/TRANSPORT.md](../docs/TRANSPORT.md) § “Hybrid coord presence”.

## HTTP API (v1)

| Method | Path | Body |
|--------|------|------|
| GET | `/health` | — (`database: true` when SQLite answers) |
| POST | `/v1/register/challenge` | `{ "public_key_hex": "<identity wire>" }` |
| POST | `/v1/register` | `public_key_hex` (identity wire), `nonce_hex`, `signature_hex`, `endpoints[]`, optional `ipv4` / `ipv6` / `transport_capabilities` |
| POST | `/v1/heartbeat` | `{ "public_key_hex": "<identity wire>" }` |
| GET | `/v1/peers/{[algo:]public_key_hex}` | Identity wire in path (URL-encode `:` as `%3A`) |
| GET | `/v1/peers` | — online peers (heartbeat within TTL) |
| GET | `/v1/relay` | — `{ enabled, peer_id, addrs }` for the co-located relay |

## Environment

| Variable | Default |
|----------|---------|
| `GHAL_BOL_SERVER_LISTEN` | `127.0.0.1:8765` |
| `GHAL_BOL_SERVER_DB` | `~/.local/share/com.ghalbol.coord/ghalbol_server/coord.db` |
| `GHAL_BOL_SERVER_PRESENCE_TTL_SECS` | `90` |
| `GHAL_BOL_RELAY_ENABLE` | `1` |
| `GHAL_BOL_RELAY_LISTEN` | `0.0.0.0:4002` (GCP). Home coord1: **`0.0.0.0:55002`** via `install_coord1_home.sh` |
| `GHAL_BOL_RELAY_PUBLIC_HOST` | unset — set on home/GCP (e.g. `coord1.ghalbol.com`, `coord.ghalbol.com`) |
| `GHAL_BOL_RELAY_PUBLIC_ADDRS` | unset — optional comma-separated multiaddrs override |
| `GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES` | `0` |
| `GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER` | `16` |

## Deploy

See [deploy/README.md](deploy/README.md).
