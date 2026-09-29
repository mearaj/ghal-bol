# Production — `coord.ghalbol.com`

Edit the config block at the top of `deploy/deploy_server.sh` (GCP target), then from repo root:

```bash
./ghal_bol_coord/deploy/deploy_server.sh
```

`deploy_server.sh` renders the systemd unit with:

| Variable | Role |
|----------|------|
| `GCP_*` | `gcloud` target |
| `COORD_URL` | Post-deploy verify |
| `GHAL_BOL_COORD_LISTEN` | Loopback HTTP listen behind nginx |

Presence + WAN call bridge WSS share the same HTTPS vhost. See [README.md](README.md).
