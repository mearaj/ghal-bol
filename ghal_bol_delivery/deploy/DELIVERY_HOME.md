# Home delivery (`delivery.ghalbol.com`)

## Home port map (gotigin / coord1 host)

| Service | WAN port | Loopback |
|---------|----------|----------|
| coord1 HTTPS | **8443** | 8765 |
| coord1 libp2p relay | **55002** | (in-process) |
| **delivery WSS** | **55003** | **8770** |

Delivery uses a **high WAN port** like coord1 relay (**55002**). Many home routers forward **8443** and **55002** but not adjacent ports such as **8444** — same class of issue as GCP relay **4002** vs home **55002**.

| Path | Port |
|------|------|
| Delivery WSS | nginx **55003** -> loopback **8770** |

**Router:** forward **55003** (TCP) to the delivery host. Do not expose raw **8770** on WAN.

```bash
./ghal_bol_delivery/deploy/install_delivery_home.sh
./ghal_bol_delivery/deploy/enable_delivery_https.sh
./ghal_bol_delivery/deploy/verify_delivery.sh
```

App delivery URL: `wss://delivery.ghalbol.com:55003`

---

## GoDaddy DDNS (once)

```bash
cp ghal_bol_delivery/deploy/godaddy-ddns-delivery.credentials.example \
   ghal_bol_delivery/deploy/godaddy-ddns-delivery.credentials
# edit API key / secret, then:
chmod 600 ghal_bol_delivery/deploy/godaddy-ddns-delivery.credentials
./ghal_bol_delivery/deploy/install_delivery_home.sh
```

## HTTPS (once)

```bash
./ghal_bol_delivery/deploy/enable_delivery_https.sh
```

Cert renewal: `./ghal_bol_delivery/deploy/certbot_delivery.sh --issue` (manual DNS-01 at GoDaddy).

Override WAN port (dev only): `DELIVERY_HTTPS_PORT=8444 ./ghal_bol_delivery/deploy/enable_delivery_https.sh`
