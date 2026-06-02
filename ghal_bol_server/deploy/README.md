# Deploy `ghal_bol_server`

Run the coordination API locally; expose it with **nginx + TLS** (production) or **ngrok** (dev).

```text
Production:  Phones  →  https://coord.ghalbol.com  →  nginx :443  →  127.0.0.1:8765  →  ghal_bol_server
Dev/staging: Phones  →  https://….ngrok-free.dev  →  ngrok  →  http://127.0.0.1:8765  →  ghal_bol_server
```

| What | Default |
|------|---------|
| SQLite | `~/.local/share/com.ghalbol/ghalbol_server/coord.db` |
| Listen | `0.0.0.0:8765` via `run_server.sh` (phones on Wi‑Fi) |
| Binary | `target/release/ghal_bol_server` |

## 1. Build + run

From workspace root:

```bash
cargo build --release -p ghal_bol_server
./ghal_bol_server/deploy/run_server.sh
curl -s http://127.0.0.1:8765/health
```

## 2. Production VM (nginx + TLS)

On the host running `ghal_bol_server`:

1. Build and install the binary (listen on loopback only):

   ```bash
   cargo build --release -p ghal_bol_server
   GHAL_BOL_SERVER_LISTEN=127.0.0.1:8765 ./target/release/ghal_bol_server
   ```

2. Copy and edit [nginx-coord.conf](nginx-coord.conf) — set `server_name` and certificate paths (certbot).

3. Enable systemd (user or system unit) so the server survives reboot — see section 3 below, or install a system unit pointing at your binary path.

4. Smoke from your laptop:

   ```bash
   COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
   ```

## 3. ngrok (dev)

```bash
ngrok http 8765
```

Set the app coord URL to the ngrok `https://…` URL (`GHAL_BOL_COORD_URL` in `ghal_bol_ui/env/.env.development` or `--dart-define`).

Smoke against the tunnel:

```bash
COORD_URL=https://YOUR_SUBDOMAIN.ngrok-free.dev ./ghal_bol_server/deploy/smoke_coord.sh
```

## 4. Production VM systemd

For `coord.ghalbol.com` (binary on the VM, nginx on :443):

```bash
sudo cp ghal_bol_server/deploy/ghal-bol-server.service /etc/systemd/system/
# Edit User, ExecStart, GHAL_BOL_SERVER_DB if paths differ
sudo systemctl daemon-reload
sudo systemctl enable --now ghal-bol-server
curl -s https://coord.ghalbol.com/health
```

Reboot test: `sudo reboot` → confirm `/health` after reconnect.

See [../../docs/PRODUCTION_RELEASE.md](../../docs/PRODUCTION_RELEASE.md) P1.2.

## 5. Optional: user systemd (dev laptop)

```bash
WS="$(pwd)"
mkdir -p ~/.config/systemd/user
sed "s|@WORKSPACE@|${WS}|g" ghal_bol_server/deploy/ghal-bol-server.user.service \
  > ~/.config/systemd/user/ghal-bol-server.service
systemctl --user daemon-reload
systemctl --user enable --now ghal-bol-server
```

## Smoke (no Flutter)

```bash
./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
```

See [../../docs/COORDINATION_SERVER.md](../../docs/COORDINATION_SERVER.md).
