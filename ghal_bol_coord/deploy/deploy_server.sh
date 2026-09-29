#!/usr/bin/env bash
# Deploy ghal_bol_coord to the production GCP VM.
#
#   ./ghal_bol_coord/deploy/deploy_server.sh
#
# Edit the config block below. Optional: SKIP_VERIFY=1
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"

# --- deploy config (edit here) ---
GCP_PROJECT=ghalbolcoord
GCP_ZONE=us-central1-a
GCP_INSTANCE=instance-20260531-113442
GCP_USER=mearajbhagad
COORD_URL=https://coord.ghalbol.com
GHAL_BOL_COORD_LISTEN=127.0.0.1:8765
# --- end deploy config ---

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/build/ghal_bol_coord-target}"
BIN="${CARGO_TARGET_DIR}/release/ghal_bol_coord"

render_deploy_unit() {
  local src="$1" dst="$2"
  sed -e "s|REPLACE_VM_USER|${GCP_USER}|g" \
    -e "s|REPLACE_GHAL_BOL_COORD_LISTEN|${GHAL_BOL_COORD_LISTEN}|g" \
    "${src}" > "${dst}"
}

cd "${WORKSPACE_ROOT}"

for var in GCP_PROJECT GCP_ZONE GCP_INSTANCE GCP_USER; do
  if [[ -z "${!var:-}" ]]; then
    echo "error: ${var} is empty — edit deploy config in deploy_server.sh" >&2
    exit 1
  fi
  case "${!var}" in
    your-*|REPLACE_*|your-gcp-project-id|your-vm-instance-name|your-linux-username-on-vm)
      echo "error: ${var} still has placeholder value — edit deploy config in deploy_server.sh" >&2
      exit 1
      ;;
  esac
done

if ! command -v gcloud >/dev/null 2>&1; then
  echo "error: gcloud not found — install Google Cloud SDK" >&2
  exit 1
fi

GCLOUD=(--project="${GCP_PROJECT}" --zone="${GCP_ZONE}")
REMOTE="${GCP_USER}@${GCP_INSTANCE}"

STAGING="$(mktemp -d)"
trap 'rm -rf "${STAGING}"' EXIT

render_deploy_unit "${DEPLOY_DIR}/ghal-bol-coord.service" "${STAGING}/ghal-bol-coord.service"
render_deploy_unit "${DEPLOY_DIR}/coord-stats-logrotate" "${STAGING}/coord-stats-logrotate"
render_deploy_unit "${DEPLOY_DIR}/coord-vm-monitor.service" "${STAGING}/coord-vm-monitor.service"

echo "== deploy config =="
echo "  coord verify     : ${COORD_URL}"
echo "  coord listen     : ${GHAL_BOL_COORD_LISTEN}"

echo "== build ghal_bol_coord (release) =="
cargo build --release -p ghal_bol_coord

if [[ ! -x "${BIN}" ]]; then
  echo "error: ${BIN} missing after build" >&2
  exit 1
fi

echo "== copy to ${REMOTE} =="
gcloud compute scp "${BIN}" "${REMOTE}:~/ghal_bol_coord.new" "${GCLOUD[@]}"
gcloud compute scp \
  "${STAGING}/ghal-bol-coord.service" \
  "${STAGING}/coord-stats-logrotate" \
  "${STAGING}/coord-vm-monitor.service" \
  "${DEPLOY_DIR}/coord-vm-stats.sh" \
  "${DEPLOY_DIR}/journald-ghalbol.conf" \
  "${DEPLOY_DIR}/nginx-logrotate-ghalbol" \
  "${DEPLOY_DIR}/coord-vm-monitor.timer" \
  "${REMOTE}:~/" \
  "${GCLOUD[@]}"

echo "== install on VM + restart =="
gcloud compute ssh "${REMOTE}" "${GCLOUD[@]}" -- bash -s <<'REMOTE_EOF'
set -euo pipefail

chmod +x "${HOME}/coord-vm-stats.sh"

sudo cp "${HOME}/ghal-bol-coord.service" /etc/systemd/system/ghal-bol-coord.service

sudo mkdir -p /etc/systemd/journald.conf.d
sudo cp "${HOME}/journald-ghalbol.conf" /etc/systemd/journald.conf.d/ghalbol.conf
sudo systemctl restart systemd-journald

sudo cp "${HOME}/nginx-logrotate-ghalbol" /etc/logrotate.d/ghalbol-coord
sudo cp "${HOME}/coord-stats-logrotate" /etc/logrotate.d/ghalbol-coord-stats

sudo cp "${HOME}/coord-vm-monitor.service" /etc/systemd/system/coord-vm-monitor.service
sudo cp "${HOME}/coord-vm-monitor.timer" /etc/systemd/system/coord-vm-monitor.timer

mv "${HOME}/ghal_bol_coord.new" "${HOME}/ghal_bol_coord"
chmod +x "${HOME}/ghal_bol_coord"

sudo systemctl daemon-reload
sudo systemctl enable ghal-bol-coord coord-vm-monitor.timer
sudo systemctl disable --now relay-egress-cap 2>/dev/null || true
sudo systemctl restart ghal-bol-coord
sudo systemctl status ghal-bol-coord --no-pager
REMOTE_EOF

if [[ "${SKIP_VERIFY:-0}" == "1" ]]; then
  echo "SKIP_VERIFY=1 — done."
  exit 0
fi

echo "== verify ${COORD_URL} =="
health="$(curl -fsS "${COORD_URL}/health")"
echo "health: ${health}"

LIVE_ONLY=1 COORD_URL="${COORD_URL}" "${DEPLOY_DIR}/smoke_coord.sh"

echo "OK — ${REMOTE} @ ${COORD_URL}"
