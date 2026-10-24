#!/usr/bin/env bash
# Install home delivery server for delivery.ghalbol.com (user systemd, no root for server).
#
#   ./ghalbol_delivery/deploy/install_delivery_home.sh
#
# See DELIVERY_HOME.md.
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"

DELIVERY_USER="$(id -un)"
DELIVERY_BIN="/home/${DELIVERY_USER}/bin/ghalbol_delivery"
GHALBOL_DELIVERY_LISTEN="127.0.0.1:8770"
GHALBOL_DDNS_CREDENTIALS="${DEPLOY_DIR}/godaddy-ddns-delivery.credentials"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/build/ghalbol_delivery-target}"
BIN="${CARGO_TARGET_DIR}/release/ghalbol_delivery"
DEST="${DELIVERY_BIN}"
UNIT_NAME="ghalbol-delivery"
USER_UNIT_DIR="${HOME}/.config/systemd/user"

render_unit() {
  sed -e "s|REPLACE_DELIVERY_USER|${DELIVERY_USER}|g" \
    -e "s|REPLACE_DELIVERY_BIN|${DELIVERY_BIN}|g" \
    -e "s|REPLACE_GHALBOL_DELIVERY_LISTEN|${GHALBOL_DELIVERY_LISTEN}|g" \
    -e "s|REPLACE_GHALBOL_DDNS_CREDENTIALS|${GHALBOL_DDNS_CREDENTIALS}|g" \
    "${DEPLOY_DIR}/ghalbol-delivery.user.service"
}

cd "${WORKSPACE_ROOT}"

echo "== build ghalbol_delivery (release) =="
cargo build --release -p ghalbol_delivery --bin ghalbol_delivery

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

if [[ ! -f "${GHALBOL_DDNS_CREDENTIALS}" ]]; then
  echo "warn: missing ${GHALBOL_DDNS_CREDENTIALS} - copy godaddy-ddns-delivery.credentials.example and set GODADDY_PAT (v3)" >&2
  echo "      DDNS disabled until credentials exist; restart ${UNIT_NAME} after adding them." >&2
elif grep -Eq '^(GODADDY_PAT)=($|paste_)' "${GHALBOL_DDNS_CREDENTIALS}" \
  || grep -Eq '^(GODADDY_API_KEY|GODADDY_API_SECRET)=' "${GHALBOL_DDNS_CREDENTIALS}"; then
  echo "warn: ${GHALBOL_DDNS_CREDENTIALS} needs GODADDY_PAT (GoDaddy API v3), not legacy API key/secret." >&2
  echo "      Edit credentials, then rerun this installer or restart ${UNIT_NAME}." >&2
  chmod 600 "${GHALBOL_DDNS_CREDENTIALS}"
else
  chmod 600 "${GHALBOL_DDNS_CREDENTIALS}"
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
echo "OK - delivery on ${GHALBOL_DELIVERY_LISTEN}"
echo ""
echo "Next (separate command, wait for shell prompt first):"
echo "     ./ghalbol_delivery/deploy/enable_delivery_https.sh"
echo "     ./ghalbol_delivery/deploy/verify_delivery.sh"
