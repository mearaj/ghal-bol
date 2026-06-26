# Deploy `ghal_bol_server`

Two setups — do not mix them:

| | **Local dev** (laptop) | **Production** (`coord.ghalbol.com` on Google Cloud) |
|---|------------------------|------------------------------------------------------|
| **Coord HTTP** | `run_server.sh` → `0.0.0.0:8765` (smoke / server hacking) | nginx `:443` → `127.0.0.1:8765` |
| **WAN relay** | **bore** tunnels local `:4002` (auto) | Public DNS + TCP `:4002` on the VM — **no bore** |
| **Start** | `./ghal_bol_server/deploy/run_server.sh` | `./ghal_bol_server/deploy/deploy_server.sh` (from laptop) |
| **App config** | `GHAL_BOL_COORD_URLS=https://coord.ghalbol.com` (default) | same |

> **Relay is raw libp2p TCP** — not carried by nginx `:443`. Clients dial it for WAN chat, voice, and video. HTTP API (register / lookup / `GET /v1/relay`) is separate.

---

## Local dev (laptop)

```bash
./ghal_bol_server/deploy/run_server.sh
```

Edit the config block at the top of `run_server.sh` for listen address, bore, etc.

**App** — default `GHAL_BOL_COORD_URLS` in `ghal_bol_ui/env/.env.development` is `https://coord.ghalbol.com`. Only point the app at your local server when you are explicitly testing `ghal_bol_server` changes.

### Verify

```bash
curl -s http://127.0.0.1:8765/health
curl -s http://127.0.0.1:8765/v1/relay | jq
# expect: "enabled": true, "addrs": ["/ip4/…/tcp/<bore-port>", …]
```

If `addrs` is empty: install `bore-cli`, check `run_server.sh` output, or you set `GHAL_BOL_RELAY_BORE=0` / `GHAL_BOL_RELAY_PUBLIC_*` which skips bore.

### Verify relay TCP (required for WAN)

```bash
PORT=$(curl -s http://127.0.0.1:8765/v1/relay | jq -r '.addrs[0]' | sed -n 's|.*/tcp/\([0-9]*\).*|\1|p')
IP=$(curl -s http://127.0.0.1:8765/v1/relay | jq -r '.addrs[0]' | sed -n 's|.*/ip4/\([0-9.]*\)/.*|\1|p')
nc -zv "${IP}" "${PORT}"    # must succeed while server+bore are running
```

### Local opt-outs

| Env | Effect |
|-----|--------|
| `GHAL_BOL_RELAY_BORE=0` | Do not start bore (WAN across NAT will not work unless you set relay addrs another way) |
| `GHAL_BOL_RELAY_PUBLIC_ADDRS=…` | Use your own public multiaddrs; bore skipped |
| `GHAL_BOL_RELAY_ENABLE=0` | Disable relay node entirely |

---

## Regression prevention (WAN dev)

| | Coord HTTP | Relay TCP |
|---|------------|-----------|
| **Carries** | register, heartbeat, lookup, `/v1/relay` JSON | libp2p Noise + relay v2 |
| **Dev** | `run_server.sh` on `:8765` | bore (auto in `run_server.sh`) |

- **New bore port every run** — clients refetch `GET /v1/relay` after each server start.
- **Ctrl+C stops server and bore** — relay TCP dies immediately.

Relay PeerId: `~/.local/share/com.ghalbol.coord/ghalbol_server/relay_ed25519.key` — do not delete.

See [TRANSPORT.md](../../docs/TRANSPORT.md) § “WAN prerequisites” and [COORDINATION_SERVER.md](../../docs/COORDINATION_SERVER.md).

---

## Production (Google Cloud — `coord.ghalbol.com`)

Edit the config block at the top of `deploy_server.sh` (GCP target + bandwidth limits), then:

```bash
./ghal_bol_server/deploy/deploy_server.sh
```

See [GCP.md](GCP.md). Do not run `run_server.sh` or bore on the VM.

---

## Smoke (no Flutter)

```bash
./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

See [../../docs/COORDINATION_SERVER.md](../../docs/COORDINATION_SERVER.md).
