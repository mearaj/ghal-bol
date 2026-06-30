# Home coord (`coord1.ghalbol.com`)

| Path | Port |
|------|------|
| Coord HTTPS | nginx **8443** → loopback **8765** |
| libp2p relay | TCP **55002** (GCP uses 4002; home routers often block 4002) |

**Router:** forward **8443** and **55002** (TCP) to the coord1 host.

```bash
./ghal_bol_server/deploy/install_coord1_home.sh
./ghal_bol_server/deploy/verify_coord1.sh
```

App coord URL: `https://coord1.ghalbol.com:8443`

---

## GoDaddy DDNS (once)

```bash
cp ghal_bol_server/deploy/godaddy-ddns-coord1.credentials.example \
   ghal_bol_server/deploy/godaddy-ddns-coord1.credentials
# edit API key / secret, then:
chmod 600 ghal_bol_server/deploy/godaddy-ddns-coord1.credentials
./ghal_bol_server/deploy/install_coord1_home.sh
```

## HTTPS (once)

```bash
./ghal_bol_server/deploy/enable_coord1_https.sh
```

Cert renewal: `./ghal_bol_server/deploy/certbot_coord1.sh --issue` (manual DNS-01 at GoDaddy).

GCP production (`coord.ghalbol.com`) is unchanged.
