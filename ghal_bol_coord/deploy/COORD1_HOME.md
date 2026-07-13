# Home coord (`coord1.ghalbol.com`)

| Path | Port |
|------|------|
| Coord HTTPS + WAN call bridge WSS | nginx **8443** → loopback **8765** |
| delivery WSS (same host) | TCP **55003** → nginx → loopback **8770** (see `ghal_bol_delivery/deploy/DELIVERY_HOME.md`) |

**Router:** forward **8443** and **55003** (TCP) to the coord1/delivery host. Port **55002** (legacy libp2p relay) is no longer used.

```bash
./ghal_bol_coord/deploy/install_coord1_home.sh
./ghal_bol_coord/deploy/verify_coord1.sh
```

App coord URL: `https://coord1.ghalbol.com:8443`  
Bridge WSS: `wss://coord1.ghalbol.com:8443/v1/bridge/connect`

---

## GoDaddy DDNS (once)

```bash
cp ghal_bol_coord/deploy/godaddy-ddns-coord1.credentials.example \
   ghal_bol_coord/deploy/godaddy-ddns-coord1.credentials
# edit API key / secret, then:
chmod 600 ghal_bol_coord/deploy/godaddy-ddns-coord1.credentials
./ghal_bol_coord/deploy/install_coord1_home.sh
```

## HTTPS (once)

```bash
./ghal_bol_coord/deploy/enable_coord1_https.sh
```

Cert renewal: `./ghal_bol_coord/deploy/certbot_coord1.sh --issue` (manual DNS-01 at GoDaddy).

GCP production (`coord.ghalbol.com`) is unchanged.
