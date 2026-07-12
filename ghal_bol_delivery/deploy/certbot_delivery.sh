#!/usr/bin/env bash
# Issue or renew Let's Encrypt for delivery.ghalbol.com (manual DNS-01 - same as coord1).
#
#   ./ghal_bol_delivery/deploy/certbot_delivery.sh          # check + nginx only
#   ./ghal_bol_delivery/deploy/certbot_delivery.sh --issue   # new cert (needs DNS TXT at GoDaddy)
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOST="${DELIVERY_HOST:-delivery.ghalbol.com}"
EMAIL="${CERTBOT_EMAIL:-mearajbhagad@gmail.com}"
LE_FULLCHAIN="/etc/letsencrypt/live/${HOST}/fullchain.pem"

if [[ "${1:-}" == "--issue" ]]; then
  if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
    echo "== certbot_delivery.sh: sudo once for certbot + nginx =="
    exec sudo /usr/bin/env bash "$(readlink -f "${BASH_SOURCE[0]}")" --issue
  fi
  echo "== bootstrap nginx :80 for ACME =="
  mkdir -p /etc/nginx/conf.d /var/www/certbot/.well-known/acme-challenge
  cp "${DEPLOY_DIR}/nginx-delivery-bootstrap.conf" /etc/nginx/conf.d/ghalbol-delivery.conf
  nginx -t || { echo "nginx -t failed" >&2; exit 1; }
  systemctl reload nginx
  echo "== certbot manual DNS (add TXT record at GoDaddy when prompted) =="
  certbot certonly --manual --preferred-challenges dns \
    -d "${HOST}" -m "${EMAIL}" --agree-tos --no-eff-email
  /usr/bin/env bash "${DEPLOY_DIR}/enable_delivery_https.sh"
  exit $?
fi

if [[ -f "${LE_FULLCHAIN}" ]]; then
  echo "Let's Encrypt already present: ${LE_FULLCHAIN}"
  exec "${DEPLOY_DIR}/enable_delivery_https.sh"
fi

echo "No cert at ${LE_FULLCHAIN}" >&2
echo "Run: ./ghal_bol_delivery/deploy/certbot_delivery.sh --issue" >&2
exit 1
