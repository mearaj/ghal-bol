# ghal_bol_server

Production **Tier 1 coordination** server for Ghal Bol: signed peer registration, SQLite presence, endpoint lookup.

It also runs a co-located **libp2p Circuit Relay v2** node for NAT/CGNAT traversal (clients reserve a circuit; coord lists `/p2p-circuit` dial addrs for WAN peers). The relay carries only transient end-to-end-encrypted transport frames — it does **not** store message bodies or transcripts, and is **not** the Tier 2 peer blob relay or the Tier 3 paid backup relay (see [docs/PREMIUM_SERVICES.md](../docs/PREMIUM_SERVICES.md)).

## Local dev vs production

| | **Local dev** (your laptop) | **Production** (Google Cloud VM) |
|---|---------------------------|----------------------------------|
| **Coord HTTP** | `run_server.sh` → `0.0.0.0:8765` | `https://coord.ghalbol.com` (nginx → loopback) |
| **WAN relay** | **bore** (auto in `run_server.sh`) | Public DNS + TCP `4002` — **no bore** |
| **How to start** | `./ghal_bol_server/deploy/run_server.sh` | `deploy_server.sh` from laptop → VM systemd |
| **App env** | `GHAL_BOL_COORD_URLS=https://coord.ghalbol.com` (default) | same |

Full walkthrough: **[deploy/README.md](deploy/README.md)**.

## Run (local dev)

```bash
cargo install bore-cli    # once
./ghal_bol_server/deploy/run_server.sh
```

Edit the config block at the top of `deploy/run_server.sh` to change listen address, bore, etc.

Set `GHAL_BOL_COORD_URLS` in `ghal_bol_ui/env/.env.development` (default: `https://coord.ghalbol.com`). Verify relay when running a local server:

```bash
curl -s http://127.0.0.1:8765/v1/relay | jq   # enabled:true, /ip4/…/tcp/… addrs
```

Bare binary (no bore — WAN chat/calls will not work across NAT):

```bash
cargo run -p ghal_bol_server
```

Defaults:

| Setting | Default |
|---------|---------|
| Listen | `127.0.0.1:8765` (`run_server.sh` uses `0.0.0.0:8765`) |
| SQLite | `~/.local/share/com.ghalbol.coord/ghalbol_server/coord.db` |

Same data root namespace as the coord server only (`com.ghalbol.coord` — not the Flutter app `com.ghalbol`).

`GHAL_BOL_SERVER_DB` may be a **file** (`…/coord.db`) or **directory** (`…/ghalbol_server`).

## Server smoke (no Flutter)

```bash
./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
```

Manual CLI: `cargo build -p ghal_bol_server --release` then `./target/release/coord_client http://127.0.0.1:8765 demo-two-peers`. See [docs/COORDINATION_SERVER.md](../docs/COORDINATION_SERVER.md).

## Test

**Production E2E (real process + TCP + disk SQLite):**

```bash
cargo test -p ghal_bol_server --test e2e_production
```

**Fast handler checks (in-process, no TCP):**

```bash
cargo test -p ghal_bol_server --test http_api
```

**All:**

```bash
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
| POST | `/v1/register/challenge` | `{ "public_key_hex": "<66 hex>" }` |
| POST | `/v1/register` | `public_key_hex`, `nonce_hex`, `signature_hex`, `endpoints[]`, optional `ipv4` / `ipv6` / `transport_capabilities` |
| POST | `/v1/heartbeat` | `{ "public_key_hex": "<66 hex>" }` |
| GET | `/v1/peers/{public_key_hex}` | — |
| GET | `/v1/peers` | — online peers (heartbeat within TTL) |
| GET | `/v1/relay` | — `{ enabled, peer_id, addrs }` for the co-located relay |

Registration signature:

```text
ghal_bol:register:v1
<nonce_hex>
<public_key_hex_lowercase>
```

SHA-256 digest → secp256k1 ECDSA (DER).

## Environment

| Variable | Default |
|----------|---------|
| `GHAL_BOL_SERVER_LISTEN` | `127.0.0.1:8765` |
| `GHAL_BOL_SERVER_DB` | `~/.local/share/com.ghalbol.coord/ghalbol_server/coord.db` |
| `GHAL_BOL_SERVER_CHALLENGE_TTL_SECS` | `120` |
| `GHAL_BOL_SERVER_PRESENCE_TTL_SECS` | `90` |
| `GHAL_BOL_SERVER_PURGE_INTERVAL_SECS` | `30` |
| `GHAL_BOL_RELAY_ENABLE` | `1` (relay node on; `0` disables) |
| `GHAL_BOL_RELAY_LISTEN` | `0.0.0.0:4002` (raw TCP — open this port; not behind the HTTP/TLS proxy) |
| `GHAL_BOL_RELAY_PUBLIC_HOST` | unset locally; **production:** `coord.ghalbol.com` |
| `GHAL_BOL_RELAY_PUBLIC_ADDRS` | unset on production; **local dev:** set automatically by bore in `run_server.sh` |
| `GHAL_BOL_RELAY_BORE` | local only: default on via `run_server.sh`; set `0` to skip bore |
| `GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES` | `0` (unlimited per circuit) |
| `GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER` | `16` |

## Deploy

Deploy: `./ghal_bol_server/deploy/deploy_server.sh`. See [deploy/README.md](deploy/README.md).

## Checklist

- [x] `cargo test -p ghal_bol_server --test e2e_production` green
- [x] Production coord live — `https://coord.ghalbol.com/health`
- [x] `COORD_URL=https://coord.ghalbol.com ./deploy/smoke_coord.sh` passes
- [ ] VM systemd enabled and survives reboot
- [ ] App ship test: two phones on mobile data via prod coord URL
