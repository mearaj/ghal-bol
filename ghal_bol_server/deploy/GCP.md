# Production — `coord.ghalbol.com`

**One-time:** copy `ghal_bol_server/.env.production.example` → `ghal_bol_server/.env.production`, fill in GCP settings and bandwidth limits.

**Deploy** (from repo root):

```bash
./ghal_bol_server/deploy/deploy_server.sh
```

`deploy_server.sh` reads `.env.production` and renders systemd units with:

| Variable | Role |
|----------|------|
| `GCP_*` | `gcloud` target |
| `COORD_URL` / `RELAY_HOST` / `RELAY_PORT` | Post-deploy verify |
| `GHAL_BOL_RELAY_EGRESS_MBIT` | Linux **tc** egress cap (`relay-egress-cap.service`) |
| `GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES` | Per-circuit byte cap in the relay binary |
| `GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER` | Concurrent circuits per peer |
| `GHAL_BOL_RELAY_LISTEN` / `GHAL_BOL_RELAY_PUBLIC_HOST` | Relay listen + advertised DNS |

See [README.md](README.md) § Production.
