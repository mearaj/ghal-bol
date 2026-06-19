#!/usr/bin/env bash
# Deploy ghal_bol_server to the production GCP VM (coord.ghalbol.com).
#
# Requires ghal_bol_server/deploy/gcp.env.local (gitignored) with:
#   GCP_PROJECT, GCP_ZONE, GCP_INSTANCE, GCP_USER
# Copy gcp.env.example if missing, then fill in real values.
#
#   ./ghal_bol_server/deploy/deploy_server.sh
#
# Optional:
#   SKIP_VERIFY=1     — deploy only, no curl / smoke_coord
#   COORD_URL=...     — default https://coord.ghalbol.com
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"
ENV_FILE="${DEPLOY_DIR}/gcp.env.local"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/build/ghal_bol_server-target}"
BIN="${CARGO_TARGET_DIR}/release/ghal_bol_server"
COORD_URL="${COORD_URL:-https://coord.ghalbol.com}"
RELAY_HOST="${RELAY_HOST:-coord.ghalbol.com}"
RELAY_PORT="${RELAY_PORT:-4002}"

cd "${WORKSPACE_ROOT}"

if [[ ! -f "${ENV_FILE}" ]]; then
  if [[ -f "${DEPLOY_DIR}/gcp.env.example" ]]; then
    cp "${DEPLOY_DIR}/gcp.env.example" "${ENV_FILE}"
    echo "created ${ENV_FILE} from gcp.env.example — edit it, then re-run." >&2
  else
    echo "error: missing ${ENV_FILE}" >&2
  fi
  exit 1
fi

# shellcheck source=/dev/null
source "${ENV_FILE}"

for var in GCP_PROJECT GCP_ZONE GCP_INSTANCE GCP_USER; do
  if [[ -z "${!var:-}" ]]; then
    echo "error: ${var} is empty in ${ENV_FILE}" >&2
    exit 1
  fi
  case "${!var}" in
    your-*|REPLACE_*|your-gcp-project-id|your-vm-instance-name|your-linux-username-on-vm)
      echo "error: ${var} still has placeholder value in ${ENV_FILE}" >&2
      echo "  edit ${ENV_FILE} with your GCP project, zone, instance name, and VM username" >&2
      exit 1
      ;;
  esac
done

if ! command -v gcloud >/dev/null 2>&1; then
  echo "error: gcloud not found — install Google Cloud SDK" >&2
  exit 1
fi

GCLOUD_PROJECT=(--project="${GCP_PROJECT}")
GCLOUD_ZONE=(--zone="${GCP_ZONE}")
REMOTE="${GCP_USER}@${GCP_INSTANCE}"

echo "== build ghal_bol_server (release) =="
cargo build --release -p ghal_bol_server

if [[ ! -x "${BIN}" ]]; then
  echo "error: ${BIN} missing after build" >&2
  exit 1
fi

echo "== copy binary to ${REMOTE} =="
gcloud compute scp "${BIN}" "${REMOTE}:~/ghal_bol_server.new" \
  "${GCLOUD_PROJECT[@]}" "${GCLOUD_ZONE[@]}"

echo "== install + restart ghal-bol-server on VM =="
gcloud compute ssh "${REMOTE}" \
  "${GCLOUD_PROJECT[@]}" "${GCLOUD_ZONE[@]}" -- \
  'mv ~/ghal_bol_server.new ~/ghal_bol_server && chmod +x ~/ghal_bol_server && sudo systemctl restart ghal-bol-server && sudo systemctl status ghal-bol-server --no-pager'

if [[ "${SKIP_VERIFY:-0}" == "1" ]]; then
  echo "SKIP_VERIFY=1 — done (no live checks)."
  exit 0
fi

echo "== verify ${COORD_URL} =="
health="$(curl -fsS "${COORD_URL}/health")"
echo "health: ${health}"

if command -v jq >/dev/null 2>&1; then
  relay="$(curl -fsS "${COORD_URL}/v1/relay")"
  echo "relay: $(echo "${relay}" | jq -c '{enabled, peer_id, addrs}')"
else
  curl -fsS "${COORD_URL}/v1/relay"
  echo
fi

if command -v nc >/dev/null 2>&1; then
  echo "== relay TCP ${RELAY_HOST}:${RELAY_PORT} =="
  nc -vz "${RELAY_HOST}" "${RELAY_PORT}"
else
  echo "warn: nc not found — skipping relay TCP probe" >&2
fi

echo "== smoke_coord (live only) =="
LIVE_ONLY=1 COORD_URL="${COORD_URL}" "${DEPLOY_DIR}/smoke_coord.sh"

echo "OK — deployed to ${REMOTE}, verified ${COORD_URL}"
