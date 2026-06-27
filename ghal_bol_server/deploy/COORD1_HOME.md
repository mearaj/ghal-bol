# Home coord (`coord1.ghalbol.com`)

Same layout as GCP `coord.ghalbol.com`: **GoDaddy DDNS**, **nginx :443 HTTPS** → `127.0.0.1:8765`, **relay TCP via UPnP** (dynamic WAN port).

| Path | Port |
|------|------|
| Coord HTTPS (register, lookup, `/v1/relay`) | nginx **8443** → loopback **8765** |
| libp2p relay | **dynamic** TCP — router UPnP maps WAN port → local ephemeral port (see `GET /v1/relay`) |

No manual router rule for relay port. `install_coord1_home.sh` sets `GHAL_BOL_RELAY_DYNAMIC=1`, `GHAL_BOL_RELAY_LISTEN=0.0.0.0:0`, `GHAL_BOL_RELAY_UPNP=1`. Clients refetch `/v1/relay` for the current port.

---

## 1 — GoDaddy DDNS

```bash
cp ghal_bol_server/deploy/godaddy-ddns-coord1.credentials.example \
   ghal_bol_server/deploy/godaddy-ddns-coord1.credentials
# edit GODADDY_API_KEY / GODADDY_API_SECRET
chmod 600 ghal_bol_server/deploy/godaddy-ddns-coord1.credentials
./ghal_bol_server/deploy/godaddy-ddns.sh
```

Timer is installed with `install_coord1_home.sh` (`godaddy-ddns.timer`, user systemd).

---

## 2 — Coord + relay

```bash
./ghal_bol_server/deploy/install_coord1_home.sh
```

Coord: `127.0.0.1:8765`. Relay: dynamic UPnP (`GHAL_BOL_RELAY_DYNAMIC=1`, listen `0.0.0.0:0`), `GHAL_BOL_RELAY_PUBLIC_HOST=coord1.ghalbol.com`.

---

## 3 — nginx HTTPS :443

Uses **existing Let's Encrypt** at `/etc/letsencrypt/live/coord1.ghalbol.com/` when present (do not regenerate self-signed).

```bash
./ghal_bol_server/deploy/enable_coord1_https.sh
```

First issue or renewal (manual DNS at GoDaddy — same as before):

```bash
./ghal_bol_server/deploy/certbot_coord1.sh --issue
```

Self-signed fallback only if no LE: `COORD1_SELF_SIGNED=1 ./ghal_bol_server/deploy/enable_coord1_https.sh`

---

## 4 — Verify WAN

```bash
./ghal_bol_server/deploy/verify_coord1.sh
```

Parses the relay port from `GET /v1/relay` (UPnP — changes after restart). **Do not hardcode 4002** for coord1.

**Automatic recovery (no app-user action):** clients react to bootstrap TCP failure → refetch `GET /v1/relay` → server remaps UPnP (event-driven, storm-throttled). Clients replace stale dial addrs on that forced refetch. No periodic UPnP poll — see `docs/TRANSPORT.md` § Event-driven async.

App (Let's Encrypt — no insecure flag):

```bash
GHAL_BOL_COORD_URLS=["https://coord1.ghalbol.com:8443"]
```

---

## 5 — Certificate renewal

Cert was issued with **manual DNS-01** (`pref_challs = dns-01` in `/etc/letsencrypt/renewal/coord1.ghalbol.com.conf`). Renew the same way when Let's Encrypt emails you — not automatic HTTP-01 unless you switch challenge type.

---

## nginx configs

| File | Use |
|------|-----|
| `nginx-coord1-selfsigned.conf` | `:443` with local cert + `:80` ACME |
| `nginx-coord1.conf` | `:443` with Let's Encrypt paths |

GCP production (`coord.ghalbol.com`) is unchanged.
