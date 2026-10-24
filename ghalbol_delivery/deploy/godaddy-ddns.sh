#!/usr/bin/env bash
# One-shot GoDaddy A record update for delivery.ghalbol.com (manual / emergency).
#
# Normal path: DDNS runs inside ghalbol_delivery (see install_delivery_home.sh).
#
#   ./ghalbol_delivery/deploy/godaddy-ddns.sh
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CREDS="${DEPLOY_DIR}/godaddy-ddns-delivery.credentials"
STATE="${DEPLOY_DIR}/.godaddy-ddns-delivery.last_ip"
V3_BASE="https://api.godaddy.com/v3"

print_credentials_template() {
  cat <<'EOF'
GODADDY_PAT=paste_pat_here
GODADDY_DOMAIN=ghalbol.com
GODADDY_HOST=delivery
GODADDY_TTL=600
EOF
}

if [[ ! -f "$CREDS" ]]; then
  echo "missing ${CREDS}" >&2
  print_credentials_template >&2
  exit 1
fi

# shellcheck source=/dev/null
source "$CREDS"

for var in GODADDY_PAT GODADDY_DOMAIN GODADDY_HOST; do
  if [[ -z "${!var:-}" ]] || [[ "${!var}" == *paste_* ]]; then
    echo "error: set ${var} in ${CREDS}" >&2
    exit 1
  fi
done

TTL="${GODADDY_TTL:-600}"
if [[ "$TTL" -lt 600 ]]; then
  TTL=600
fi
AUTH="Authorization: Bearer ${GODADDY_PAT}"
LIST_URL="${V3_BASE}/domains/zones/${GODADDY_DOMAIN}/dns-records?type=A&name=${GODADDY_HOST}"
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

LIST_JSON="$(curl -fsS -H "$AUTH" -H 'Cache-Control: no-cache' "$LIST_URL")"
REMOTE="$(jq -r '.items[0].data // empty' <<< "$LIST_JSON")"
RECORD_ID="$(jq -r '.items[0].recordId // empty' <<< "$LIST_JSON")"

if [[ "$REMOTE" == "$CURRENT" ]]; then
  echo "$CURRENT" > "$STATE"
  exit 0
fi

BODY="$(jq -nc --arg name "$GODADDY_HOST" --arg data "$CURRENT" --argjson ttl "$TTL" \
  '{name: $name, type: "A", data: $data, ttl: $ttl}')"

if [[ -n "$RECORD_ID" ]]; then
  curl -fsS -X PUT \
    -H "$AUTH" \
    -H "Content-Type: application/json" \
    "${V3_BASE}/domains/zones/${GODADDY_DOMAIN}/dns-records/${RECORD_ID}" \
    -d "$BODY"
else
  curl -fsS -X POST \
    -H "$AUTH" \
    -H "Content-Type: application/json" \
    "${V3_BASE}/domains/zones/${GODADDY_DOMAIN}/dns-records" \
    -d "$BODY"
fi

echo "$CURRENT" > "$STATE"
if command -v logger >/dev/null 2>&1; then
  logger -t godaddy-ddns "${FQDN} -> ${CURRENT}"
fi
echo "${FQDN} -> ${CURRENT}"
