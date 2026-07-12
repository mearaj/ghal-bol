# Production — `coord.ghalbol.com`

Edit the config block at the top of `deploy/deploy_server.sh` (GCP target + bandwidth limits), then from repo root:

```bash
./ghal_bol_coord/deploy/deploy_server.sh
```

`deploy_server.sh` renders systemd units with:

| Variable | Role |
|----------|------|
| `GCP_*` | `gcloud` target |
| `COORD_URL` / `RELAY_HOST` / `RELAY_PORT` | Post-deploy verify |
| `GHAL_BOL_RELAY_EGRESS_MBIT` | Linux **tc** egress cap (`relay-egress-cap.service`) |
| `GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES` | Per-circuit byte cap in the relay binary |
| `GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER` | Concurrent circuits per peer |
| `GHAL_BOL_RELAY_LISTEN` / `GHAL_BOL_RELAY_PUBLIC_HOST` | Relay listen + advertised DNS |

See [README.md](README.md) § Production.
