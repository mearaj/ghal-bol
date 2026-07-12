#!/usr/bin/env bash
# Issue or renew Let's Encrypt for coord1.ghalbol.com (manual DNS-01 — same as first issue).
#
# Your existing cert was manual DNS (see /etc/letsencrypt/renewal/coord1.ghalbol.com.conf).
# Re-run certbot the same way when renewing; this script only installs nginx for an existing cert.
#
#   ./ghal_bol_coord/deploy/certbot_coord1.sh          # check + nginx only
#   ./ghal_bol_coord/deploy/certbot_coord1.sh --issue   # new cert (needs DNS TXT at GoDaddy)
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOST="${COORD1_HOST:-coord1.ghalbol.com}"
EMAIL="${CERTBOT_EMAIL:-mearajbhagad@gmail.com}"
LE_FULLCHAIN="/etc/letsencrypt/live/${HOST}/fullchain.pem"

if [[ "${1:-}" == "--issue" ]]; then
  if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
    exec sudo -E /usr/bin/env bash "$(readlink -f "${BASH_SOURCE[0]}")" --issue
  fi
  echo "== bootstrap nginx :80 for ACME =="
  mkdir -p /etc/nginx/conf.d /var/www/certbot/.well-known/acme-challenge
  cp "${DEPLOY_DIR}/nginx-coord1-bootstrap.conf" /etc/nginx/conf.d/ghalbol-coord1.conf
  nginx -t
  systemctl reload nginx
  echo "== certbot manual DNS (add TXT record at GoDaddy when prompted) =="
  certbot certonly --manual --preferred-challenges dns \
    -d "${HOST}" -m "${EMAIL}" --agree-tos --no-eff-email
  exec "${DEPLOY_DIR}/enable_coord1_https.sh"
fi

if [[ -f "${LE_FULLCHAIN}" ]]; then
  echo "Let's Encrypt already present: ${LE_FULLCHAIN}"
  exec "${DEPLOY_DIR}/enable_coord1_https.sh"
fi

echo "No cert at ${LE_FULLCHAIN}" >&2
echo "Run: ./ghal_bol_coord/deploy/certbot_coord1.sh --issue" >&2
exit 1
