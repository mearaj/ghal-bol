#!/usr/bin/env bash
# Operator check for delivery.ghalbol.com (run on the delivery host).
# Same checks as enable_delivery_https.sh: local delivery + nginx --resolve.
#
#   ./ghal_bol_delivery/deploy/verify_delivery.sh
set -euo pipefail

HOST="delivery.ghalbol.com"
HTTPS_PORT="${DELIVERY_HTTPS_PORT:-55003}"
DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DDNS_LAST_IP="${DEPLOY_DIR}/.godaddy-ddns-delivery.last_ip"

failures=0
note() { echo ">> $*"; }
pass() { echo "PASS - $*"; }
fail() { echo "FAIL - $*" >&2; failures=$((failures + 1)); }

PUBLIC_IP=""
PUBLIC_IP="$(getent ahostsv4 "${HOST}" 2>/dev/null | awk '{print $1; exit}' || true)"
CURRENT_IP=""
CURRENT_IP="$(curl -4 -fsS --connect-timeout 8 https://api.ipify.org 2>/dev/null || true)"
if [[ -z "${CURRENT_IP}" && -f "${DDNS_LAST_IP}" ]]; then
  CURRENT_IP="$(tr -d '[:space:]' < "${DDNS_LAST_IP}")"
fi

note "GoDaddy DNS"
if [[ -z "${PUBLIC_IP}" ]]; then
  fail "DNS: no A record for ${HOST} - restart ghal-bol-delivery"
elif [[ -n "${CURRENT_IP}" && "${PUBLIC_IP}" != "${CURRENT_IP}" ]]; then
  fail "DNS stale: ${HOST} -> ${PUBLIC_IP}, this PC is ${CURRENT_IP}"
else
  pass "${HOST} -> ${PUBLIC_IP}"
fi

note "delivery on 127.0.0.1:8770"
if curl -fsS --connect-timeout 5 "http://127.0.0.1:8770/health" >/dev/null; then
  pass "http://127.0.0.1:8770/health"
else
  fail "http://127.0.0.1:8770/health - run install_delivery_home.sh"
fi

note "delivery HTTPS /health (nginx --resolve ${HOST}:${HTTPS_PORT}:127.0.0.1)"
if HEALTH_JSON="$(curl -fsS --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" --connect-timeout 10 "https://${HOST}:${HTTPS_PORT}/health")"; then
  pass "https://${HOST}:${HTTPS_PORT}/health"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${HEALTH_JSON}" | jq -c '{ok, instance_id, schema_version, pending_messages}')"
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/health - run enable_delivery_https.sh"
fi

note "nginx listen :${HTTPS_PORT}"
if ss -tln 2>/dev/null | grep -qE ":${HTTPS_PORT}\\b"; then
  pass "nginx listening on :${HTTPS_PORT}"
else
  fail ":${HTTPS_PORT} not listening - check nginx"
fi

echo ""
if [[ "${failures}" -eq 0 ]]; then
  echo "OK - ${HOST} delivery ready (WSS wss://${HOST}:${HTTPS_PORT}/v1/ws)."
  exit 0
fi

echo "FAILED - ${failures} check(s)." >&2
exit 1
