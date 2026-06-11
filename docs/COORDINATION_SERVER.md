# Coordination server (`ghal_bol_server`)

**Tier 1 only** — presence and endpoint discovery. No message bodies or transcripts.

It also runs a co-located **libp2p Circuit Relay v2** node for NAT/CGNAT traversal (transport helper; relays only transient E2E frames until DCUtR upgrades clients to a direct link — not a message store). See [TRANSPORT.md](TRANSPORT.md) § "Ghal Bol relay".

```text
Peer A / B  →  register / heartbeat  →  ghal_bol_server (SQLite)  →  GET /v1/peers/{hex}  →  P2P dial
                 └─ reserve circuit on relay ─→  GET /v1/relay  →  register /p2p-circuit  →  DCUtR direct
```

## Client (`ghal_bol`)

After unlock: configure the **coord server list** (today a single URL; the API is an array for future redundancy). On P2P listen: **register + heartbeat on every reachable server** in the list. Before dial: **lookup across the list** — stop on first successful result for that attempt; on reconnect after a drop, repeat the full list.

Set URLs in `ghal_bol_ui/env/.env.development` (debug) or `env/.env.production` (release) via `GHAL_BOL_COORD_URLS` (JSON array or comma-separated). No hardcoded coord URLs in the app binary.

**WAN policy:** coord + co-located relay are **required** for internet peer discovery. When coord is unreachable, LAN (mDNS) still works; the node keeps retrying all configured servers. Do **not** fall back to Kademlia DHT or public libp2p bootstrap peers for WAN peer lookup — **libp2p remains** for transport (relay, DCUtR, mDNS, streams). See [STORY.md](STORY.md) and [TRANSPORT.md](TRANSPORT.md).

## Run server

**Local dev** — `run_server.sh` starts the binary + **bore** for WAN relay; expose coord HTTP with ngrok separately:

```bash
./ghal_bol_server/deploy/run_server.sh          # terminal 1
ngrok http 8765                                 # terminal 2
curl -s http://127.0.0.1:8765/v1/relay | jq     # relay must show enabled + addrs
```

**Production** — Google Cloud VM at `coord.ghalbol.com`: systemd + nginx, **no bore**, public TCP `:4002` for relay. See [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md).

## Production (`coord.ghalbol.com`)

Public HTTPS coordination is served by **nginx + TLS** in front of `ghal_bol_server` on `127.0.0.1:8765`. Relay is **not** behind nginx — clients dial `coord.ghalbol.com:4002` directly. Example config: [ghal_bol_server/deploy/nginx-coord.conf](../ghal_bol_server/deploy/nginx-coord.conf).

App builds bundle the URL via `ghal_bol_ui/env/.env.production`:

```bash
GHAL_BOL_COORD_URLS=["https://coord.ghalbol.com"]
```

Smoke against production:

```bash
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

## ngrok (dev / staging)

ngrok carries **HTTP coord only**. WAN relay uses **bore** (started by `run_server.sh`), not ngrok `tcp` on the free plan.

```bash
ngrok http 8765
```

Point `GHAL_BOL_COORD_URLS` at the ngrok `https://…` URL. See **Local dev stack** below and [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md).

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
| GET | `/v1/relay` |

Register signature: `ghal_bol:register:v1` + nonce + pubkey (SHA-256 → secp256k1 ECDSA DER).

`GET /v1/relay` → `{ enabled, peer_id, addrs }` — the co-located relay's stable PeerId and dialable base multiaddrs (clients append `/p2p/<peer_id>/p2p-circuit`). `enabled:false` or empty `addrs` when no public relay is configured (dev: bore not running).

## Troubleshooting

### Interpreting coord HTTP access logs

| Pattern | Likely cause | Action |
|---------|--------------|--------|
| `GET /v1/relay` 200, many `GET /v1/peers/…` 404, **no** register | Relay TCP unreachable or clients stuck waiting for circuit | `nc -zv` on `/v1/relay` addr; restart `run_server.sh`; restart apps |
| `GET /v1/health` 200, `/v1/relay` empty addrs | Server up but bore skipped or relay disabled | See deploy README bore-skip reasons |
| `peer registered` in **server** logs but lookup 404 | TTL expired (~90s) or wrong coord URL in app | Heartbeat/register failing; check app `coord_registered` |
| Works after second `run_server.sh` start | First start had no bore | Always confirm `Starting bore:` line appears |

### Dev session checklist

1. Terminal 1: `./ghal_bol_server/deploy/run_server.sh` — keep running  
2. Terminal 2: `ngrok http 8765`  
3. `curl -s http://127.0.0.1:8765/v1/relay | jq` — enabled + addrs  
4. `nc -zv <relay-ip> <relay-port>` — must connect  
5. App `GHAL_BOL_COORD_URLS` = ngrok **https** URL  
6. Rebuild native + restart apps after **every** server restart (bore port change)

Full detail: [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md) § “Regression prevention”, [TRANSPORT.md](TRANSPORT.md) § “WAN prerequisites”.

## Environment

| Variable | Default |
|----------|---------|
| `GHAL_BOL_SERVER_LISTEN` | `127.0.0.1:8765` (binary); `run_server.sh` uses `0.0.0.0:8765`. Dual-stack: the server also binds the counterpart-family wildcard (`[::]:<port>`, IPv6 `V6ONLY`) on the same port so coord HTTP is reachable over both IPv4 and IPv6; a missing stack logs a warning and continues single-stack |
| `GHAL_BOL_SERVER_DB` | `~/.local/share/com.ghalbol/ghalbol_server/coord.db` |
| `GHAL_BOL_SERVER_PRESENCE_TTL_SECS` | `90` |
| `GHAL_BOL_RELAY_ENABLE` | `1` (set `0` to disable the relay node) |
| `GHAL_BOL_RELAY_LISTEN` | `0.0.0.0:4002` (raw TCP — **open this port** directly; it is not proxied by the HTTP/TLS nginx front). Dual-stack: the relay also listens on the counterpart-family wildcard (`[::]:<port>`) so it accepts both IPv4 and IPv6 clients |
| `GHAL_BOL_RELAY_PUBLIC_HOST` | unset → advertises **both** `/dns6/<host>/tcp/<port>` and `/dns4/<host>/tcp/<port>` (IPv6 first; e.g. `coord.ghalbol.com`) so clients can reserve over either family. Native IPv6 needs an `AAAA` record; IPv4-only/NAT64 clients map the host themselves |
| `GHAL_BOL_RELAY_PUBLIC_ADDRS` | unset → comma-separated dialable multiaddrs (overrides `_PUBLIC_HOST`) |

## `coord_client`

```bash
cargo build -p ghal_bol_server --release
./target/release/coord_client http://127.0.0.1:8765 demo-two-peers
```

`-k` = skip TLS verify (ngrok / self-signed).

## Local dev stack

One `ghal_bol_server` on your desktop for Linux + Android builds.

```bash
./ghal_bol_server/deploy/run_server.sh          # listens 0.0.0.0:8765
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
```

Set `GHAL_BOL_COORD_URLS` in `ghal_bol_ui/env/.env.development` (e.g. `["http://127.0.0.1:8765"]` on desktop, `["http://10.0.2.2:8765"]` on emulator). From the phone: `curl http://<desktop-lan-ip>:8765/health` must succeed.

**Rebuild native after Rust changes** (quit app first):

```bash
./scripts/sync_ghal_bol_native_for_flutter.sh   # Linux
./scripts/pack_android_workspace_jni_libs.sh    # Android
cd ghal_bol_ui && flutter run
```

Two-device test: server running → desktop `flutter run` + QR → phone scan → send both ways. Same Wi‑Fi uses mDNS; different subnets need coord lookup (URLs from env).

## Related

- [STORY.md](STORY.md) — human-authored connectivity policy (agents: read only, never edit)
- [TRANSPORT.md](TRANSPORT.md) — WAN/LAN dial policy, invites, multiple coord servers
- [ghal_bol_server/README.md](../ghal_bol_server/README.md)
- [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md)
