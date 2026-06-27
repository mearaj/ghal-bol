#!/usr/bin/env bash
# Install home coord + relay for coord1.ghalbol.com (user systemd, no root for server/DDNS).
#
#   ./ghal_bol_server/deploy/install_coord1_home.sh
#
# Stack: GoDaddy DDNS → ghal_bol_server (127.0.0.1:8765) + relay (UPnP dynamic) → nginx :443 HTTPS.
# See COORD1_HOME.md. Does not touch GCP coord.ghalbol.com.
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"

# --- home coord config (edit here) ---
COORD1_USER="${COORD1_USER:-$(id -un)}"
COORD1_BIN="${COORD1_BIN:-/home/${COORD1_USER}/bin/ghal_bol_server}"
COORD1_HOST="${COORD1_HOST:-coord1.ghalbol.com}"
COORD1_HTTPS_URL="${COORD1_HTTPS_URL:-https://coord1.ghalbol.com:8443}"
GHAL_BOL_SERVER_LISTEN="${GHAL_BOL_SERVER_LISTEN:-127.0.0.1:8765}"
GHAL_BOL_RELAY_LISTEN="${GHAL_BOL_RELAY_LISTEN:-0.0.0.0:0}"
GHAL_BOL_RELAY_DYNAMIC="${GHAL_BOL_RELAY_DYNAMIC:-1}"
GHAL_BOL_RELAY_UPNP="${GHAL_BOL_RELAY_UPNP:-1}"
GHAL_BOL_RELAY_PUBLIC_HOST="${GHAL_BOL_RELAY_PUBLIC_HOST:-coord1.ghalbol.com}"
GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES="${GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES:-0}"
GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER="${GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER:-16}"
# --- end config ---

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/build/ghal_bol_server-target}"
BIN="${CARGO_TARGET_DIR}/release/ghal_bol_server"
DEST="${COORD1_BIN}"
UNIT_NAME="ghal-bol-server-coord1"
DDNS_UNIT="godaddy-ddns"
USER_UNIT_DIR="${HOME}/.config/systemd/user"

render_unit() {
  sed -e "s|REPLACE_COORD1_USER|${COORD1_USER}|g" \
    -e "s|REPLACE_COORD1_BIN|${COORD1_BIN}|g" \
    -e "s|REPLACE_GHAL_BOL_SERVER_LISTEN|${GHAL_BOL_SERVER_LISTEN}|g" \
    -e "s|REPLACE_GHAL_BOL_RELAY_LISTEN|${GHAL_BOL_RELAY_LISTEN}|g" \
    -e "s|REPLACE_GHAL_BOL_RELAY_DYNAMIC|${GHAL_BOL_RELAY_DYNAMIC}|g" \
    -e "s|REPLACE_GHAL_BOL_RELAY_UPNP|${GHAL_BOL_RELAY_UPNP}|g" \
    -e "s|REPLACE_GHAL_BOL_RELAY_PUBLIC_HOST|${GHAL_BOL_RELAY_PUBLIC_HOST}|g" \
    -e "s|REPLACE_GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES|${GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES}|g" \
    -e "s|REPLACE_GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER|${GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER}|g" \
    "${DEPLOY_DIR}/ghal-bol-server-coord1.user.service"
}

render_godaddy_unit() {
  sed -e "s|REPLACE_GODADDY_DDNS_SCRIPT|${DEPLOY_DIR}/godaddy-ddns.sh|g" \
    "${DEPLOY_DIR}/godaddy-ddns.user.service"
}

cd "${WORKSPACE_ROOT}"

echo "== build ghal_bol_server (release) =="
cargo build --release -p ghal_bol_server --bin ghal_bol_server

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

chmod +x "${DEPLOY_DIR}/godaddy-ddns.sh"
render_godaddy_unit > "${USER_UNIT_DIR}/${DDNS_UNIT}.service"
cp "${DEPLOY_DIR}/godaddy-ddns.user.timer" "${USER_UNIT_DIR}/${DDNS_UNIT}.timer"

echo "== restart ${UNIT_NAME} =="
systemctl --user daemon-reload
systemctl --user enable "${UNIT_NAME}.service"
systemctl --user restart "${UNIT_NAME}.service"
sleep 1
if ! systemctl --user is-active --quiet "${UNIT_NAME}.service"; then
  echo "error: ${UNIT_NAME} failed to start" >&2
  systemctl --user status "${UNIT_NAME}.service" --no-pager >&2 || true
  exit 1
fi

echo "== enable GoDaddy DDNS timer (user) =="
systemctl --user enable --now "${DDNS_UNIT}.timer"
if [[ -f "${DEPLOY_DIR}/godaddy-ddns-coord1.credentials" ]]; then
  "${DEPLOY_DIR}/godaddy-ddns.sh" || echo "warn: godaddy-ddns.sh failed — check credentials" >&2
else
  echo "warn: missing ${DEPLOY_DIR}/godaddy-ddns-coord1.credentials — copy .example and fill API key" >&2
fi

echo ""
echo "OK — ${UNIT_NAME} on ${GHAL_BOL_SERVER_LISTEN}  relay dynamic (UPnP) listen ${GHAL_BOL_RELAY_LISTEN}"
echo "     DDNS: godaddy-ddns.timer (user)"
echo "     Next: ./ghal_bol_server/deploy/enable_coord1_https.sh   (uses LE if present)"
echo "     App:  GHAL_BOL_COORD_URLS=[\"${COORD1_HTTPS_URL}\"]"
echo "     WAN:  ./ghal_bol_server/deploy/verify_coord1.sh"
