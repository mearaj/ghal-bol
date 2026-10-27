# Ghal Bol delivery — deploy

| | **Home** (`delivery.ghalbol.com`) |
|--|-----------------------------------|
| **DDNS** | In-process GoDaddy API v3 PAT (`GHALBOL_DDNS_CREDENTIALS`) |
| **WAN** | nginx **55003** → loopback **8770** (WSS) |
| **Install** | `install_delivery_home.sh` |

> Home: forward **55003** on the router (high port, like coord1 relay **55002**). Then `install_delivery_home.sh`, `enable_delivery_https.sh`, `verify_delivery.sh`.

Full home steps: **[DELIVERY_HOME.md](DELIVERY_HOME.md)**.

> **Redeploy with the apps.** Commit `2dd0f51` renamed `ghal_bol` → `ghalbol` in the session / upload signing domains, the envelope `ghalbol.share` value, env names (`GHALBOL_DELIVERY_*`) and the data dir (`com.ghalbol.delivery/ghalbol_delivery/`). Old and new servers and clients do not interoperate. Re-run the install scripts from the same commit as the apps. Mail still pending in the old mailbox was sealed in the old format and cannot be opened by current apps, so starting with an empty mailbox loses nothing usable.

```bash
./ghalbol_delivery/deploy/install_delivery_home.sh
./ghalbol_delivery/deploy/enable_delivery_https.sh
./ghalbol_delivery/deploy/verify_delivery.sh
```

Production app URL: `GHALBOL_DELIVERY_URL=wss://delivery.ghalbol.com:55003`
