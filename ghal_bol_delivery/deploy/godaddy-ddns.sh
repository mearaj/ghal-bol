#!/usr/bin/env bash
# One-shot GoDaddy A record update for delivery.ghalbol.com (manual / emergency).
#
# Normal path: DDNS runs inside ghal_bol_delivery (see install_delivery_home.sh).
#
#   ./ghal_bol_delivery/deploy/godaddy-ddns.sh
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CREDS="${DEPLOY_DIR}/godaddy-ddns-delivery.credentials"
STATE="${DEPLOY_DIR}/.godaddy-ddns-delivery.last_ip"

print_credentials_template() {
  cat <<'EOF'
GODADDY_API_KEY=paste_key_here
GODADDY_API_SECRET=paste_secret_here
GODADDY_DOMAIN=ghalbol.com
GODADDY_HOST=delivery
GODADDY_TTL=600
EOF
}

if [[ ! -f "$CREDS" ]]; then
  echo "missing ${CREDS}" >&2
  echo "create that file with:" >&2
  print_credentials_template >&2
  exit 1
fi

# shellcheck source=/dev/null
source "$CREDS"

for var in GODADDY_API_KEY GODADDY_API_SECRET GODADDY_DOMAIN GODADDY_HOST; do
  if [[ -z "${!var:-}" ]] || [[ "${!var}" == *paste_* ]]; then
    echo "error: set ${var} in ${CREDS}" >&2
    exit 1
  fi
done

TTL="${GODADDY_TTL:-600}"
AUTH="Authorization: sso-key ${GODADDY_API_KEY}:${GODADDY_API_SECRET}"
BASE="https://api.godaddy.com/v1/domains/${GODADDY_DOMAIN}/records/A/${GODADDY_HOST}"
FQDN="${GODADDY_HOST}.${GODADDY_DOMAIN}"

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq required (pacman -S jq)" >&2
  exit 1
fi

CURRENT="$(curl -fsS -4 --no-keepalive -H 'Cache-Control: no-cache' "https://api.ipify.org?$(date +%s)" 2>/dev/null \
  || curl -fsS -4 --no-keepalive ifconfig.me)"
if [[ -z "$CURRENT" ]]; then
  echo "error: could not detect public IPv4" >&2
  exit 1
fi

REMOTE="$(curl -fsS -H "$AUTH" -H 'Cache-Control: no-cache' "$BASE" 2>/dev/null | jq -r '.[0].data // empty' || true)"
if [[ "$REMOTE" == "$CURRENT" ]]; then
  echo "$CURRENT" > "$STATE"
  exit 0
fi

curl -fsS -X PUT \
  -H "$AUTH" \
  -H "Content-Type: application/json" \
  "$BASE" \
  -d "[{\"data\":\"${CURRENT}\",\"ttl\":${TTL}}]"

echo "$CURRENT" > "$STATE"
if command -v logger >/dev/null 2>&1; then
  logger -t godaddy-ddns "${FQDN} -> ${CURRENT}"
fi
echo "${FQDN} -> ${CURRENT}"
