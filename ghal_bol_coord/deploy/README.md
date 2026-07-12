# Deploy `ghal_bol_coord`

| | **Home** (`coord1.ghalbol.com`) | **GCP** (`coord.ghalbol.com`) | **Local dev** |
|---|----------------------------------|--------------------------------|---------------|
| **Coord HTTP** | nginx `:8443` → `127.0.0.1:8765` | nginx `:443` → `127.0.0.1:8765` | `run_server.sh` |
| **DDNS** | In-process GoDaddy API (`GHAL_BOL_DDNS_CREDENTIALS`) | static | — |
| **WAN relay** | fixed `:55002` | fixed `:4002` | bore or `PUBLIC_HOST` |
| **Install** | `install_coord1_home.sh` | `deploy_server.sh` | `run_server.sh` |

> Home coord1: forward **8443** + **55002** on the router. Then `install_coord1_home.sh` and `verify_coord1.sh`.

Full home steps: **[COORD1_HOME.md](COORD1_HOME.md)**.

```bash
# credentials once, then:
./ghal_bol_coord/deploy/install_coord1_home.sh
./ghal_bol_coord/deploy/enable_coord1_https.sh
./ghal_bol_coord/deploy/verify_coord1.sh
```

---

## Production GCP

```bash
./ghal_bol_coord/deploy/deploy_server.sh
```

See [GCP.md](GCP.md). **Do not change GCP when editing home deploy.**

---

## Smoke

```bash
COORD_URL=https://coord.ghalbol.com ./ghal_bol_coord/deploy/smoke_coord.sh
COORD_URL=https://coord1.ghalbol.com ./ghal_bol_coord/deploy/smoke_coord.sh
```
