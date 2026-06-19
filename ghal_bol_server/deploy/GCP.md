# Production deploy — Google Cloud (`coord.ghalbol.com`)

**Canonical guide:** [README.md](README.md) § Production.  
**API / troubleshooting:** [../../docs/COORDINATION_SERVER.md](../../docs/COORDINATION_SERVER.md).

Keep **project id, instance name, VM user, and IPs** in a local file only — not in git. Copy [gcp.env.example](gcp.env.example) → `gcp.env.local` (gitignored).

```bash
source ghal_bol_server/deploy/gcp.env.local   # sets GCP_PROJECT, GCP_ZONE, GCP_INSTANCE, GCP_USER
```

---

## Routine deploy (from repo root)

```bash
cp ghal_bol_server/deploy/gcp.env.example ghal_bol_server/deploy/gcp.env.local
# edit gcp.env.local once, then:
./ghal_bol_server/deploy/deploy_server.sh
```

`SKIP_VERIFY=1` skips curl / `smoke_coord.sh` after restart.

---

## Verify

```bash
curl -s https://coord.ghalbol.com/health
curl -s https://coord.ghalbol.com/v1/relay | jq
nc -vz coord.ghalbol.com 4002
```

Logs on VM:

```bash
source ghal_bol_server/deploy/gcp.env.local
gcloud compute ssh "${GCP_USER}@${GCP_INSTANCE}" \
  --project="${GCP_PROJECT}" --zone="${GCP_ZONE}" -- \
  'sudo journalctl -u ghal-bol-server -n 50 --no-pager'
```

---

## First-time VM setup (rare)

1. Edit [ghal-bol-server.service](ghal-bol-server.service) — set `User`, `ExecStart`, `GHAL_BOL_SERVER_DB`.
2. Install unit: `sudo cp ghal-bol-server.service /etc/systemd/system/ && sudo systemctl enable --now ghal-bol-server`
3. nginx + certbot — [nginx-coord.conf](nginx-coord.conf) (HTTP API on `:443` only; relay stays TCP `:4002`).
4. Firewall relay if missing — see [README.md](README.md) (`ghalbol-relay`, `tcp:4002`).

**Do not** run `run_server.sh`, bore, or ngrok on the production VM.
