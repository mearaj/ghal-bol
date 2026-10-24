# Home coord (`coord1.ghalbol.com`)

| Path | Port |
|------|------|
| Coord HTTPS + WAN call bridge WSS | nginx **8443** → loopback **8765** |
| delivery WSS (same host) | TCP **55003** → nginx → loopback **8770** (see `ghalbol_delivery/deploy/DELIVERY_HOME.md`) |

**Router:** forward **8443** and **55003** (TCP) to the coord1/delivery host.

```bash
./ghalbol_coord/deploy/install_coord1_home.sh
./ghalbol_coord/deploy/verify_coord1.sh
```

App coord URL: `https://coord1.ghalbol.com:8443`  
Bridge WSS: `wss://coord1.ghalbol.com:8443/v1/bridge/connect`

Unit `GHALBOL_COORD_PUBLIC_URL` and app `GHALBOL_COORD_URLS` must be that same URL.

**nginx must forward WebSocket Upgrade** (`proxy_set_header Upgrade` / `Connection`) for
`/v1/bridge/connect`. If clients log `bridge ws connect: HTTP error: 400 Bad Request` /
`Connection header did not include 'upgrade'`, re-apply HTTPS nginx config:

```bash
./ghalbol_coord/deploy/enable_coord1_https.sh
./ghalbol_coord/deploy/verify_coord1.sh
```

---

## GoDaddy DDNS (once)

Uses GoDaddy REST **v3** with a [Personal Access Token](https://developer.godaddy.com/) (`GODADDY_PAT`). Required scopes: `domains.domain:read`, `domains.dns:update`.

```bash
cp ghalbol_coord/deploy/godaddy-ddns-coord1.credentials.example \
   ghalbol_coord/deploy/godaddy-ddns-coord1.credentials
# edit GODADDY_PAT, then:
chmod 600 ghalbol_coord/deploy/godaddy-ddns-coord1.credentials
./ghalbol_coord/deploy/install_coord1_home.sh
```

## HTTPS (once)

```bash
./ghalbol_coord/deploy/enable_coord1_https.sh
```

Cert renewal: `./ghalbol_coord/deploy/certbot_coord1.sh --issue` (manual DNS-01 at GoDaddy).

GCP production (`coord.ghalbol.com`) is unchanged.
