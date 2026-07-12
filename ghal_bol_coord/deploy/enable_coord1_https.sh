#!/usr/bin/env bash
# nginx :8443 (WAN) + :443 → 127.0.0.1:8765 for coord1.ghalbol.com
#
# Uses existing Let's Encrypt at /etc/letsencrypt/live/coord1.ghalbol.com/ when present.
# Self-signed only if LE missing and COORD1_SELF_SIGNED=1.
#
# Run as normal user — one sudo password:
#   ./ghal_bol_coord/deploy/enable_coord1_https.sh
if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
  exec sudo -E /usr/bin/env bash "$(readlink -f "${BASH_SOURCE[0]}")" "$@"
fi

DEPLOY_DIR="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
HOST="${COORD1_HOST:-coord1.ghalbol.com}"
HTTPS_PORT="${COORD1_HTTPS_PORT:-8443}"
NGINX_CONF="/etc/nginx/conf.d/ghalbol-coord1.conf"
LE_DIR="/etc/letsencrypt/live/${HOST}"
LE_FULLCHAIN="${LE_DIR}/fullchain.pem"
LE_PRIVKEY="${LE_DIR}/privkey.pem"
SSL_DIR="/etc/nginx/ssl/${HOST}"
WEBROOT="/var/www/certbot"

fail() {
  echo "FAILED: $*" >&2
  exit 1
}

echo "== preflight: coord on 127.0.0.1:8765 =="
curl -fsS "http://127.0.0.1:8765/health" >/dev/null || fail "coord not on 127.0.0.1:8765 — run install_coord1_home.sh first"

mkdir -p /etc/nginx/conf.d "${WEBROOT}/.well-known/acme-challenge"

USE_LE=0
if [[ -f "${LE_FULLCHAIN}" && -f "${LE_PRIVKEY}" ]]; then
  USE_LE=1
  echo "== Let's Encrypt found: ${LE_DIR} =="
  cp "${DEPLOY_DIR}/nginx-coord1.conf" "${NGINX_CONF}" || fail "cp nginx-coord1.conf"
elif [[ "${COORD1_SELF_SIGNED:-0}" == "1" ]]; then
  echo "== no LE cert — self-signed (COORD1_SELF_SIGNED=1) =="
  mkdir -p "${SSL_DIR}"
  if [[ ! -f "${SSL_DIR}/privkey.pem" ]]; then
    openssl req -x509 -nodes -days 825 -newkey rsa:2048 \
      -keyout "${SSL_DIR}/privkey.pem" \
      -out "${SSL_DIR}/fullchain.pem" \
      -subj "/CN=${HOST}" || fail "openssl"
    chmod 600 "${SSL_DIR}/privkey.pem"
  fi
  cp "${DEPLOY_DIR}/nginx-coord1-selfsigned.conf" "${NGINX_CONF}" || fail "cp nginx-coord1-selfsigned.conf"
else
  fail "no Let's Encrypt at ${LE_DIR} — issue cert first (see certbot_coord1.sh) or COORD1_SELF_SIGNED=1"
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
  echo "OK — https://${HOST}:${HTTPS_PORT} → coord (Let's Encrypt)"
  echo "     App: GHAL_BOL_COORD_URLS=[\"https://${HOST}:${HTTPS_PORT}\"]  (no INSECURE_TLS)"
else
  curl -fsSk --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" "https://${HOST}:${HTTPS_PORT}/health" >/dev/null \
    || fail "https://${HOST}:${HTTPS_PORT}/health (self-signed + nginx)"
  echo ""
  echo "OK — https://${HOST}:${HTTPS_PORT} → coord (self-signed)"
  echo "     App: GHAL_BOL_COORD_URLS=[\"https://${HOST}:${HTTPS_PORT}\"]  GHAL_BOL_COORD_INSECURE_TLS=1"
fi

echo "     ./ghal_bol_coord/deploy/verify_coord1.sh"
