#!/usr/bin/env bash
# Install home coord for coord1.ghalbol.com (user systemd, no root for server).
#
#   ./ghal_bol_coord/deploy/install_coord1_home.sh
#
# See COORD1_HOME.md. Does not touch GCP coord.ghalbol.com.
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"

# --- home coord config (edit here if needed) ---
COORD1_USER="$(id -un)"
COORD1_BIN="/home/${COORD1_USER}/bin/ghal_bol_coord"
GHAL_BOL_COORD_LISTEN="127.0.0.1:8765"
GHAL_BOL_COORD_PUBLIC_URL="https://coord1.ghalbol.com:8443"
GHAL_BOL_DDNS_CREDENTIALS="${DEPLOY_DIR}/godaddy-ddns-coord1.credentials"
# --- end config ---

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/build/ghal_bol_coord-target}"
BIN="${CARGO_TARGET_DIR}/release/ghal_bol_coord"
DEST="${COORD1_BIN}"
UNIT_NAME="ghal-bol-coord1"
LEGACY_UNIT_NAMES=("ghal-bol-coord-coord1" "ghal-bol-server-coord1")
USER_UNIT_DIR="${HOME}/.config/systemd/user"

render_unit() {
  sed -e "s|REPLACE_COORD1_USER|${COORD1_USER}|g" \
    -e "s|REPLACE_COORD1_BIN|${COORD1_BIN}|g" \
    -e "s|REPLACE_GHAL_BOL_COORD_LISTEN|${GHAL_BOL_COORD_LISTEN}|g" \
    -e "s|REPLACE_GHAL_BOL_COORD_PUBLIC_URL|${GHAL_BOL_COORD_PUBLIC_URL}|g" \
    -e "s|REPLACE_GHAL_BOL_DDNS_CREDENTIALS|${GHAL_BOL_DDNS_CREDENTIALS}|g" \
    "${DEPLOY_DIR}/ghal-bol-coord1.user.service"
}

cd "${WORKSPACE_ROOT}"

echo "== build ghal_bol_coord (release) =="
cargo build --release -p ghal_bol_coord --bin ghal_bol_coord

if [[ ! -x "${BIN}" ]]; then
  echo "error: ${BIN} missing after build" >&2
  exit 1
fi

echo "== install binary -> ${DEST} =="
mkdir -p "$(dirname "${DEST}")"
cp "${BIN}" "${DEST}.new"
chmod +x "${DEST}.new"
mv "${DEST}.new" "${DEST}"

mkdir -p "${USER_UNIT_DIR}"
render_unit > "${USER_UNIT_DIR}/${UNIT_NAME}.service"
render_unit > "${DEPLOY_DIR}/${UNIT_NAME}.user.service.rendered"

if [[ ! -f "${GHAL_BOL_DDNS_CREDENTIALS}" ]]; then
  echo "warn: missing ${GHAL_BOL_DDNS_CREDENTIALS} — copy godaddy-ddns-coord1.credentials.example and fill API key" >&2
  echo "      DDNS disabled until credentials exist; restart ${UNIT_NAME} after adding them." >&2
fi

for legacy in "${LEGACY_UNIT_NAMES[@]}"; do
  if systemctl --user is-enabled --quiet "${legacy}.service" 2>/dev/null; then
    echo "warn: legacy unit ${legacy}.service still enabled (same ports as ${UNIT_NAME})" >&2
    echo "      stop it manually: systemctl --user disable --now ${legacy}.service" >&2
  fi
done

echo "== restart ${UNIT_NAME} =="
systemctl --user daemon-reload
systemctl --user enable "${UNIT_NAME}.service"
systemctl --user restart "${UNIT_NAME}.service"
sleep 2
if ! systemctl --user is-active --quiet "${UNIT_NAME}.service"; then
  echo "error: ${UNIT_NAME} failed to start" >&2
  systemctl --user status "${UNIT_NAME}.service" --no-pager >&2 || true
  exit 1
fi

echo ""
echo "OK — coord1 on ${GHAL_BOL_COORD_LISTEN} (bridge WSS via ${GHAL_BOL_COORD_PUBLIC_URL})"
echo "     Verify: ./ghal_bol_coord/deploy/verify_coord1.sh"
