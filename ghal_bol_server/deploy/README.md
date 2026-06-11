# Deploy `ghal_bol_server`

Two setups — do not mix them up:

| | **Local dev** | **Production** (`coord.ghalbol.com` on Google Cloud) |
|---|---------------|--------------------------------------------------------|
| **Purpose** | Test on laptop + phone (different networks) | Live app / Play builds |
| **Coord HTTP** | ngrok `http` → `127.0.0.1:8765` | nginx `:443` → `127.0.0.1:8765` |
| **WAN relay** | **bore** tunnels local `:4002` (auto) | Public DNS + TCP `:4002` on the VM — **no bore** |
| **Start** | `./ghal_bol_server/deploy/run_server.sh` | `sudo systemctl start ghal-bol-server` |
| **App config** | `GHAL_BOL_COORD_URLS=https://….ngrok-free.dev` | `GHAL_BOL_COORD_URLS=https://coord.ghalbol.com` |

> **Relay is raw libp2p TCP** — not carried by nginx `:443` or ngrok `http`. Clients dial it for WAN chat, voice, and video. HTTP API (register / lookup / `GET /v1/relay`) is separate.

Production has used `coord.ghalbol.com` since the first GCP deploy. Bore was added only for **local** dev; the cloud VM was **not** updated for bore and does not need it.

---

## Local dev (laptop)

WAN needs **two** tunnels:

1. **Coord HTTP** — ngrok `http` (register, heartbeat, peer lookup, `/v1/relay` JSON).
2. **Relay TCP** — **bore** (libp2p Circuit Relay v2 on port `4002`). ngrok free `tcp` breaks libp2p Noise; bore is the supported dev path.

### One-time

```bash
cargo install bore-cli
```

### Every session

**Terminal 1 — server** (builds if needed, starts bore, advertises relay at `/v1/relay`):

```bash
./ghal_bol_server/deploy/run_server.sh
```

**Terminal 2 — coord HTTP**:

```bash
ngrok http 8765
# or: ngrok start coord_http
```

**App** — set `GHAL_BOL_COORD_URLS` in `ghal_bol_ui/env/.env.development` to the ngrok **https** URL (not `coord.ghalbol.com` unless you intentionally use prod relay).

### Verify

```bash
curl -s http://127.0.0.1:8765/health
curl -s http://127.0.0.1:8765/v1/relay | jq
# expect: "enabled": true, "addrs": ["/ip4/…/tcp/<bore-port>", …]
```

If `addrs` is empty: install `bore-cli`, check `run_server.sh` output, or you set `GHAL_BOL_RELAY_BORE=0` / `GHAL_BOL_RELAY_PUBLIC_*` which skips bore. The script prints **why** bore was skipped on stderr when it does not start.

After bore starts, the script probes relay TCP and warns if the port is not reachable yet.

### Verify relay TCP (required for WAN)

Coord HTTP and relay TCP are **separate**. ngrok carrying `:8765` does **not** expose libp2p relay.

```bash
PORT=$(curl -s http://127.0.0.1:8765/v1/relay | jq -r '.addrs[0]' | sed -n 's|.*/tcp/\([0-9]*\).*|\1|p')
IP=$(curl -s http://127.0.0.1:8765/v1/relay | jq -r '.addrs[0]' | sed -n 's|.*/ip4/\([0-9.]*\)/.*|\1|p')
nc -zv "${IP}" "${PORT}"    # must succeed while server+bore are running
```

Optional — reservation with a real secp256k1 client key:

```bash
PROBE_SECP256K1=1 cargo run -p ghal_bol_server --example relay_probe -- \
  "$(curl -s http://127.0.0.1:8765/v1/relay | jq -r '.addrs[0]')/$(curl -s http://127.0.0.1:8765/v1/relay | jq -r '.peer_id')/p2p-circuit"
```

Expect `RESERVATION ACCEPTED`.

### Local opt-outs

| Env | Effect |
|-----|--------|
| `GHAL_BOL_RELAY_BORE=0` | Do not start bore (WAN across NAT will not work unless you set relay addrs another way) |
| `GHAL_BOL_RELAY_PUBLIC_ADDRS=…` | Use your own public multiaddrs; bore skipped |
| `GHAL_BOL_RELAY_ENABLE=0` | Disable relay node entirely |

**Alternative:** point dev `GHAL_BOL_COORD_URLS` at `https://coord.ghalbol.com` and use the **production** relay (no local server). Handy for app-only work; not for server changes.

**Paid ngrok only:** `run_server_ngrok.sh` sets relay addrs from an ngrok **tcp** tunnel — not viable on ngrok free.

---

## Regression prevention (WAN dev)

These failures looked like “client bugs” but were **server/tunnel** issues. Do not patch the app until this checklist passes.

### Two tunnels, two protocols

| | Coord HTTP | Relay TCP |
|---|------------|-----------|
| **Carries** | register, heartbeat, lookup, `/v1/relay` JSON | libp2p Noise + relay v2 |
| **Dev** | ngrok `http 8765` | bore (started by `run_server.sh`) |
| **Not interchangeable** | ngrok does **not** replace bore | bore does **not** replace ngrok |

### `run_server.sh` + bore lifecycle

- **Default:** bore starts automatically (`cargo install bore-cli` required).
- **Bore skipped when:** `GHAL_BOL_RELAY_BORE=0`, `GHAL_BOL_RELAY_ENABLE=0`, or non-empty `GHAL_BOL_RELAY_PUBLIC_ADDRS` / `GHAL_BOL_RELAY_PUBLIC_HOST` (whitespace-only counts as unset).
- **New remote port every run** — bore.pub assigns e.g. `:7660` then `:22234`. `GET /v1/relay` updates; clients must refetch after each server start.
- **Ctrl+C stops server and bore** — relay TCP dies immediately. ngrok may still answer HTTP while relay is down → coord logs show `GET /v1/relay` 200 but endless `GET /v1/peers/…` 404 (peers never register).
- **First run without bore output** — if you see `relay has no public address advertised` and `advertised=[]`, bore did not run. Read stderr from `run_server.sh` for the skip reason.

### Stable relay identity

Relay PeerId comes from:

`~/.local/share/com.ghalbol/ghalbol_server/relay_ed25519.key`

**Do not delete** this file. If it is regenerated, clients with old `ghalbol_relay.json` must refetch `/v1/relay`.

### Client cache after server restart

Apps cache relay coords at `<app_data>/ghalbol_relay.json`. After bore port or relay PeerId changes:

- Restart apps (or rebuild native with cache-invalidation on refused dial), **or**
- Delete `ghalbol_relay.json` under the app data dir.

### Healthy server log pattern

```
Starting bore: local 4002 -> bore.pub ...
bore relay endpoint  : bore.pub:<port>
advertising via /v1/relay: /ip4/159.223.110.159/tcp/<port>
relay v2 node started ... advertised=["/ip4/…/tcp/<port>"]
peer registered public_key=… endpoints=1    ← must appear after apps unlock
```

### Unhealthy patterns

| Server / coord log | Problem |
|--------------------|---------|
| `advertised=[]`, warn about `GHAL_BOL_RELAY_PUBLIC_HOST` | No public relay — WAN impossible |
| Only `GET /v1/peers/…` 404, no `peer registered` | Relay TCP dead or clients never reserved+registered |
| `GET /v1/relay` 200 from ngrok but `nc` refused on advertised port | Stale bore port or server stopped |

See [TRANSPORT.md](../../docs/TRANSPORT.md) § “WAN prerequisites” and [COORDINATION_SERVER.md](../../docs/COORDINATION_SERVER.md) § “Troubleshooting”.

## Production (Google Cloud — `coord.ghalbol.com`)

Existing VM layout (unchanged by local bore work):

```text
Phones  →  https://coord.ghalbol.com     →  nginx :443  →  127.0.0.1:8765  →  ghal_bol_server (HTTP)
Phones  →  coord.ghalbol.com:4002 (TCP)  ─────────────────────────────────→  ghal_bol_server (relay)
```

- **No bore** on the VM.
- systemd unit sets `GHAL_BOL_RELAY_PUBLIC_HOST=coord.ghalbol.com` → `/v1/relay` advertises `/dns4/coord.ghalbol.com/tcp/4002`.
- Relay identity: `~/.local/share/com.ghalbol/ghalbol_server/relay_ed25519.key` (stable PeerId across restarts).

### Deploy / update binary on the VM

On the VM (or build elsewhere and copy the binary):

```bash
cargo build --release -p ghal_bol_server
# install binary where ExecStart points (see ghal-bol-server.service)
sudo systemctl daemon-reload
sudo systemctl restart ghal-bol-server
```

First-time or unit changes:

```bash
sudo cp ghal_bol_server/deploy/ghal-bol-server.service /etc/systemd/system/
# Edit User, ExecStart, GHAL_BOL_SERVER_DB if paths differ
sudo systemctl daemon-reload
sudo systemctl enable --now ghal-bol-server
```

Unit env (production — no bore):

| Variable | Production value |
|----------|------------------|
| `GHAL_BOL_SERVER_LISTEN` | `127.0.0.1:8765` (nginx fronts HTTPS) |
| `GHAL_BOL_RELAY_LISTEN` | `0.0.0.0:4002` |
| `GHAL_BOL_RELAY_PUBLIC_HOST` | `coord.ghalbol.com` |

### Firewall (GCP)

Relay TCP must be open (often done once at first deploy):

```bash
gcloud compute firewall-rules create ghalbol-relay \
  --direction=INGRESS --action=ALLOW --rules=tcp:4002 \
  --network=default --source-ranges=0.0.0.0/0
```

Also allow `4002/tcp` in the VM OS firewall (`ufw`, etc.) if enabled.

### nginx + TLS

Copy and edit [nginx-coord.conf](nginx-coord.conf) — `server_name coord.ghalbol.com`, certbot paths. HTTP API only; relay stays on `:4002` direct.

### Verify production (from laptop)

```bash
curl -s https://coord.ghalbol.com/health
curl -s https://coord.ghalbol.com/v1/relay | jq
# {"enabled":true,"peer_id":"12D3…","addrs":["/dns4/coord.ghalbol.com/tcp/4002"]}

nc -vz coord.ghalbol.com 4002

COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

On the VM:

```bash
sudo journalctl -u ghal-bol-server -f | grep -E 'relay v2|relay listening|listening'
```

### When to redeploy production

Redeploy the **binary** when server code changes (API, relay limits, SQLite, etc.). You do **not** need bore, `run_server.sh`, or ngrok on the VM. Local bore changes do not require a production deploy unless you also changed `ghal_bol_server` Rust code you want live.

---

## Reference

| What | Default |
|------|---------|
| SQLite | `~/.local/share/com.ghalbol/ghalbol_server/coord.db` |
| Local listen (`run_server.sh`) | `0.0.0.0:8765` |
| Relay listen | `0.0.0.0:4002` |
| Binary (local build) | `build/ghal_bol_server-target/release/ghal_bol_server` |

| Env (server) | Local dev | Production |
|--------------|-----------|------------|
| `GHAL_BOL_RELAY_ENABLE` | `1` | `1` |
| `GHAL_BOL_RELAY_PUBLIC_HOST` | unset (bore sets addrs) | `coord.ghalbol.com` |
| `GHAL_BOL_RELAY_PUBLIC_ADDRS` | set by bore | unset |
| bore | auto via `run_server.sh` | not used |

## Optional: user systemd (dev laptop)

```bash
WS="$(pwd)"
mkdir -p ~/.config/systemd/user
sed "s|@WORKSPACE@|${WS}|g" ghal_bol_server/deploy/ghal-bol-server.user.service \
  > ~/.config/systemd/user/ghal-bol-server.service
systemctl --user daemon-reload
systemctl --user enable --now ghal-bol-server
```

For daily dev, `run_server.sh` + ngrok is simpler (includes bore).

## Smoke (no Flutter)

```bash
./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

See [../../docs/COORDINATION_SERVER.md](../../docs/COORDINATION_SERVER.md).
