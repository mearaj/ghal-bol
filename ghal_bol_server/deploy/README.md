# Deploy `ghal_bol_server`

| | **Home** (`coord1.ghalbol.com`) | **GCP** (`coord.ghalbol.com`) | **Local dev** |
|---|----------------------------------|--------------------------------|---------------|
| **Coord HTTP** | nginx `:443` → `127.0.0.1:8765` | nginx `:443` → `127.0.0.1:8765` | `run_server.sh` |
| **DDNS** | GoDaddy API (`godaddy-ddns.sh`) | static | — |
| **WAN relay** | `coord1.ghalbol.com:4002` | `coord.ghalbol.com:4002` | bore or `PUBLIC_HOST` |
| **Install** | `install_coord1_home.sh` | `deploy_server.sh` | `run_server.sh` |

> Relay is raw libp2p TCP — not HTTP. Forward **443** and **4002** on the home router.

Full home steps: **[COORD1_HOME.md](COORD1_HOME.md)**.

```bash
# credentials once, then:
./ghal_bol_server/deploy/install_coord1_home.sh
./ghal_bol_server/deploy/enable_coord1_https.sh
./ghal_bol_server/deploy/verify_coord1.sh
```

---

## Production GCP

```bash
./ghal_bol_server/deploy/deploy_server.sh
```

See [GCP.md](GCP.md). **Do not change GCP when editing home deploy.**

---

## Smoke

```bash
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=https://coord1.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```
