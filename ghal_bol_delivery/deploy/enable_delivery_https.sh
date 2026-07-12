#!/usr/bin/env bash
# nginx :55003 (WAN WSS) -> 127.0.0.1:8770 for delivery.ghalbol.com
#
# Same flow as enable_coord1_https.sh:
#   1) preflight as your user (no sudo)
#   2) one sudo password for nginx
#
#   ./ghal_bol_delivery/deploy/enable_delivery_https.sh
set -euo pipefail

DEPLOY_DIR="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
HOST="${DELIVERY_HOST:-delivery.ghalbol.com}"
HTTPS_PORT="${DELIVERY_HTTPS_PORT:-55003}"
NGINX_CONF="/etc/nginx/conf.d/ghalbol-delivery.conf"
LE_DIR="/etc/letsencrypt/live/${HOST}"
LE_FULLCHAIN="${LE_DIR}/fullchain.pem"
LE_PRIVKEY="${LE_DIR}/privkey.pem"
SSL_DIR="/etc/nginx/ssl/${HOST}"
WEBROOT="/var/www/certbot"

fail() {
  echo "FAILED: $*" >&2
  exit 1
}

# --- user checks first (no sudo, no background jobs) ---
echo ""
echo "== enable_delivery_https.sh =="
echo "== preflight: delivery on 127.0.0.1:8770 =="
curl -fsS "http://127.0.0.1:8770/health" >/dev/null \
  || fail "delivery not on 127.0.0.1:8770 - run install_delivery_home.sh first"

if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
  echo "== sudo once for nginx (delivery.ghalbol.com :${HTTPS_PORT}) =="
  # Do not use sudo -E: a cargo/rust env from install in the same shell can break auth.
  exec sudo /usr/bin/env bash "$(readlink -f "${BASH_SOURCE[0]}")" --as-root
fi

if [[ "${1:-}" == "--as-root" ]]; then
  shift
fi

mkdir -p /etc/nginx/conf.d "${WEBROOT}/.well-known/acme-challenge"

USE_LE=0
if [[ -f "${LE_FULLCHAIN}" && -f "${LE_PRIVKEY}" ]]; then
  USE_LE=1
  echo "== Let's Encrypt found: ${LE_DIR} =="
  cp "${DEPLOY_DIR}/nginx-delivery.conf" "${NGINX_CONF}" || fail "cp nginx-delivery.conf"
elif [[ "${DELIVERY_SELF_SIGNED:-0}" == "1" ]]; then
  echo "== no LE cert - self-signed (DELIVERY_SELF_SIGNED=1) =="
  mkdir -p "${SSL_DIR}"
  if [[ ! -f "${SSL_DIR}/privkey.pem" ]]; then
    openssl req -x509 -nodes -days 825 -newkey rsa:2048 \
      -keyout "${SSL_DIR}/privkey.pem" \
      -out "${SSL_DIR}/fullchain.pem" \
      -subj "/CN=${HOST}" 2>/dev/null || fail "openssl"
    chmod 600 "${SSL_DIR}/privkey.pem"
  fi
  cp "${DEPLOY_DIR}/nginx-delivery-selfsigned.conf" "${NGINX_CONF}" || fail "cp nginx-delivery-selfsigned.conf"
else
  fail "no Let's Encrypt at ${LE_DIR} - run ./ghal_bol_delivery/deploy/certbot_delivery.sh --issue first"
fi

echo "== nginx -t =="
nginx -t || fail "nginx -t"

systemctl enable nginx || fail "systemctl enable nginx"
systemctl start nginx || fail "systemctl start nginx"
systemctl reload nginx || fail "systemctl reload nginx"

sleep 1
ss -tln | grep -q ":${HTTPS_PORT}" || fail ":${HTTPS_PORT} not listening after reload"

if [[ "${USE_LE}" -eq 1 ]]; then
  curl -fsS --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" "https://${HOST}:${HTTPS_PORT}/health" >/dev/null \
    || fail "https://${HOST}:${HTTPS_PORT}/health (LE cert + nginx)"
  echo ""
  echo "OK - wss://${HOST}:${HTTPS_PORT}/v1/ws (Let's Encrypt)"
  echo "     App: GHAL_BOL_DELIVERY_URL=wss://${HOST}:${HTTPS_PORT}"
else
  curl -fsSk --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" "https://${HOST}:${HTTPS_PORT}/health" >/dev/null \
    || fail "https://${HOST}:${HTTPS_PORT}/health (self-signed + nginx)"
  echo ""
  echo "OK - wss://${HOST}:${HTTPS_PORT}/v1/ws (self-signed)"
fi

echo "     ./ghal_bol_delivery/deploy/verify_delivery.sh"
