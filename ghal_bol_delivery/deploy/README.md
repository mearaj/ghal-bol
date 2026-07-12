# Ghal Bol delivery — deploy

| | **Home** (`delivery.ghalbol.com`) |
|--|-----------------------------------|
| **DDNS** | In-process GoDaddy API (`GHAL_BOL_DDNS_CREDENTIALS`) |
| **WAN** | nginx **55003** → loopback **8770** (WSS) |
| **Install** | `install_delivery_home.sh` |

> Home: forward **55003** on the router (high port, like coord1 relay **55002**). Then `install_delivery_home.sh`, `enable_delivery_https.sh`, `verify_delivery.sh`.

Full home steps: **[DELIVERY_HOME.md](DELIVERY_HOME.md)**.

```bash
./ghal_bol_delivery/deploy/install_delivery_home.sh
./ghal_bol_delivery/deploy/enable_delivery_https.sh
./ghal_bol_delivery/deploy/verify_delivery.sh
```

Production app URL: `GHAL_BOL_DELIVERY_URL=wss://delivery.ghalbol.com:55003`
