#!/usr/bin/env bash
# Install home delivery server for delivery.ghalbol.com (user systemd, no root for server).
#
#   ./ghal_bol_delivery/deploy/install_delivery_home.sh
#
# See DELIVERY_HOME.md.
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"

DELIVERY_USER="$(id -un)"
DELIVERY_BIN="/home/${DELIVERY_USER}/bin/ghal_bol_delivery"
GHAL_BOL_DELIVERY_LISTEN="127.0.0.1:8770"
GHAL_BOL_DDNS_CREDENTIALS="${DEPLOY_DIR}/godaddy-ddns-delivery.credentials"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/build/ghal_bol_delivery-target}"
BIN="${CARGO_TARGET_DIR}/release/ghal_bol_delivery"
DEST="${DELIVERY_BIN}"
UNIT_NAME="ghal-bol-delivery"
USER_UNIT_DIR="${HOME}/.config/systemd/user"

render_unit() {
  sed -e "s|REPLACE_DELIVERY_USER|${DELIVERY_USER}|g" \
    -e "s|REPLACE_DELIVERY_BIN|${DELIVERY_BIN}|g" \
    -e "s|REPLACE_GHAL_BOL_DELIVERY_LISTEN|${GHAL_BOL_DELIVERY_LISTEN}|g" \
    -e "s|REPLACE_GHAL_BOL_DDNS_CREDENTIALS|${GHAL_BOL_DDNS_CREDENTIALS}|g" \
    "${DEPLOY_DIR}/ghal-bol-delivery.user.service"
}

cd "${WORKSPACE_ROOT}"

echo "== build ghal_bol_delivery (release) =="
cargo build --release -p ghal_bol_delivery --bin ghal_bol_delivery

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
echo "wrote ${USER_UNIT_DIR}/${UNIT_NAME}.service"

if [[ ! -f "${GHAL_BOL_DDNS_CREDENTIALS}" ]]; then
  echo "warn: missing ${GHAL_BOL_DDNS_CREDENTIALS} - copy godaddy-ddns-delivery.credentials.example and fill API key" >&2
  echo "      DDNS disabled until credentials exist; restart ${UNIT_NAME} after adding them." >&2
elif grep -Eq '^(GODADDY_API_KEY|GODADDY_API_SECRET)=($|paste_)' "${GHAL_BOL_DDNS_CREDENTIALS}"; then
  echo "warn: ${GHAL_BOL_DDNS_CREDENTIALS} still contains placeholder credentials." >&2
  echo "      Edit the API key/secret, then rerun this installer or restart ${UNIT_NAME}." >&2
  chmod 600 "${GHAL_BOL_DDNS_CREDENTIALS}"
else
  chmod 600 "${GHAL_BOL_DDNS_CREDENTIALS}"
  echo "DDNS credentials ready (mode 600; values not printed)"
fi

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
echo "OK - delivery on ${GHAL_BOL_DELIVERY_LISTEN}"
echo ""
echo "Next (separate command, wait for shell prompt first):"
echo "     ./ghal_bol_delivery/deploy/enable_delivery_https.sh"
echo "     ./ghal_bol_delivery/deploy/verify_delivery.sh"
