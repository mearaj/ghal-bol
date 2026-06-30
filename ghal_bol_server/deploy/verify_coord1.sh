#!/usr/bin/env bash
# WAN check for coord1.ghalbol.com
#
#   ./ghal_bol_server/deploy/verify_coord1.sh
set -euo pipefail

HOST="coord1.ghalbol.com"
HTTPS_PORT="8443"
RELAY_PORT="55002"

failures=0
note() { echo ">> $*"; }
pass() { echo "PASS — $*"; }
fail() { echo "FAIL — $*" >&2; failures=$((failures + 1)); }

PUBLIC_IP=""
PUBLIC_IP="$(getent ahostsv4 "${HOST}" 2>/dev/null | awk '{print $1; exit}' || true)"
CURRENT_IP=""
CURRENT_IP="$(curl -4 -fsS --connect-timeout 8 https://api.ipify.org 2>/dev/null || true)"

note "GoDaddy DNS"
if [[ -z "${PUBLIC_IP}" ]]; then
  fail "DNS: no A record for ${HOST} — restart ghal-bol-server-coord1"
elif [[ -n "${CURRENT_IP}" && "${PUBLIC_IP}" != "${CURRENT_IP}" ]]; then
  fail "DNS stale: ${HOST} → ${PUBLIC_IP}, this PC is ${CURRENT_IP}"
else
  pass "${HOST} → ${PUBLIC_IP}"
fi

note "coord HTTPS"
if HEALTH_JSON="$(curl -fsS --connect-timeout 10 "https://${HOST}:${HTTPS_PORT}/health")"; then
  pass "https://${HOST}:${HTTPS_PORT}/health"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${HEALTH_JSON}" | jq -c '{ok, database, relay: .relay.wan_ready}')"
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/health — check nginx and router forward 8443"
fi

note "GET /v1/relay"
if RELAY_JSON="$(curl -fsS --connect-timeout 10 "https://${HOST}:${HTTPS_PORT}/v1/relay")"; then
  pass "https://${HOST}:${HTTPS_PORT}/v1/relay"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${RELAY_JSON}" | jq -c '{enabled, addrs}')"
    PARSED="$(echo "${RELAY_JSON}" | jq -r '.addrs[0] // empty' | sed -n 's|.*/tcp/\([0-9]*\)$|\1|p')"
    [[ -n "${PARSED}" ]] && RELAY_PORT="${PARSED}"
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/v1/relay"
fi

note "relay TCP on public IP"
if [[ -n "${PUBLIC_IP}" ]]; then
  if timeout 8 bash -c "echo >/dev/tcp/${PUBLIC_IP}/${RELAY_PORT}" 2>/dev/null; then
    pass "${PUBLIC_IP}:${RELAY_PORT} TCP open"
  else
    fail "${PUBLIC_IP}:${RELAY_PORT} TCP timeout — forward WAN TCP ${RELAY_PORT} on the router"
  fi
fi

echo ""
if [[ "${failures}" -eq 0 ]]; then
  echo "OK — ${HOST} coord + relay reachable (TCP :${RELAY_PORT})."
  exit 0
fi

echo "FAILED — ${failures} check(s)." >&2
exit 1
