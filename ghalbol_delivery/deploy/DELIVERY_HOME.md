# Home delivery (`delivery.ghalbol.com`)

## Home port map (gotigin / coord1 host)

| Service | WAN port | Loopback |
|---------|----------|----------|
| coord1 HTTPS + bridge WSS | **8443** | 8765 |
| **delivery WSS** | **55003** | **8770** |

| Path | Port |
|------|------|
| Delivery WSS | nginx **55003** -> loopback **8770** |

**Router:** forward **55003** (TCP) to the delivery host. Do not expose raw **8770** on WAN.

```bash
./ghalbol_delivery/deploy/install_delivery_home.sh
./ghalbol_delivery/deploy/enable_delivery_https.sh
./ghalbol_delivery/deploy/verify_delivery.sh
```

App delivery URL: `wss://delivery.ghalbol.com:55003`

---

## GoDaddy DDNS (once)

Uses GoDaddy REST **v3** with a [Personal Access Token](https://developer.godaddy.com/) (`GODADDY_PAT`). Required scopes: `domains.domain:read`, `domains.dns:update`.

```bash
cp ghalbol_delivery/deploy/godaddy-ddns-delivery.credentials.example \
   ghalbol_delivery/deploy/godaddy-ddns-delivery.credentials
# edit GODADDY_PAT, then:
chmod 600 ghalbol_delivery/deploy/godaddy-ddns-delivery.credentials
./ghalbol_delivery/deploy/install_delivery_home.sh
```

## HTTPS (once)

```bash
./ghalbol_delivery/deploy/enable_delivery_https.sh
```

Cert renewal: `./ghalbol_delivery/deploy/certbot_delivery.sh --issue` (manual DNS-01 at GoDaddy).

Override WAN port (dev only): `DELIVERY_HTTPS_PORT=8444 ./ghalbol_delivery/deploy/enable_delivery_https.sh`
