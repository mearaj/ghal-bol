# Coordination server (`ghal_bol_server`)

**Tier 1 only** — presence and endpoint discovery. No message bodies or transcripts.

```text
Peer A / B  →  register / heartbeat  →  ghal_bol_server (SQLite)  →  GET /v1/peers/{hex}  →  P2P dial
```

## Client (`ghal_bol`)

After unlock: `coord_set_base_url`. On P2P listen: register + heartbeat. Before dial: lookup → `bootstrap_peers`.

Set URL via `GHAL_BOL_COORD_URL` (see `ghal_bol_ui/env/`). Defaults: desktop `http://127.0.0.1:8765`, emulator `http://10.0.2.2:8765`.

## Run server

```bash
./ghal_bol_server/deploy/run_server.sh
curl -s http://127.0.0.1:8765/health
```

## Production (`coord.ghalbol.com`)

Public HTTPS coordination is served by **nginx + TLS** in front of `ghal_bol_server` on `127.0.0.1:8765`. Example config: [ghal_bol_server/deploy/nginx-coord.conf](../ghal_bol_server/deploy/nginx-coord.conf).

App builds bundle the URL via `ghal_bol_ui/env/.env.production`:

```bash
GHAL_BOL_COORD_URL=https://coord.ghalbol.com
```

Smoke against production:

```bash
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

## ngrok (dev / staging)

```bash
ngrok http 8765
```

Point `GHAL_BOL_COORD_URL` at the ngrok `https://…` URL. See [LOCAL_DEV_STACK.md](LOCAL_DEV_STACK.md).

```bash
COORD_URL=https://YOUR.ngrok-free.dev ./ghal_bol_server/deploy/smoke_coord.sh
```

## HTTP API (v1)

| Method | Path |
|--------|------|
| GET | `/health` |
| POST | `/v1/register/challenge` |
| POST | `/v1/register` |
| POST | `/v1/heartbeat` |
| GET | `/v1/peers/{public_key_hex}` |
| GET | `/v1/peers` |

Register signature: `ghal_bol:register:v1` + nonce + pubkey (SHA-256 → secp256k1 ECDSA DER).

## Environment

| Variable | Default |
|----------|---------|
| `GHAL_BOL_SERVER_LISTEN` | `127.0.0.1:8765` (binary); `run_server.sh` uses `0.0.0.0:8765` |
| `GHAL_BOL_SERVER_DB` | `~/.local/share/com.ghalbol/ghalbol_server/coord.db` |
| `GHAL_BOL_SERVER_PRESENCE_TTL_SECS` | `90` |

## `coord_client`

```bash
cargo build -p ghal_bol_server --release
./target/release/coord_client http://127.0.0.1:8765 demo-two-peers
```

`-k` = skip TLS verify (ngrok / self-signed).

## Related

- [ghal_bol_server/README.md](../ghal_bol_server/README.md)
- [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md)
