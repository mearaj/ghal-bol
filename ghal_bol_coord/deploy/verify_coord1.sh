#!/usr/bin/env bash
# Operator check for coord1.ghalbol.com (run on the coord1 host).
# Same checks as enable_coord1_https.sh: local coord + nginx --resolve + relay bind.
#
#   ./ghal_bol_coord/deploy/verify_coord1.sh
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
  fail "DNS: no A record for ${HOST} — restart ghal-bol-coord1"
elif [[ -n "${CURRENT_IP}" && "${PUBLIC_IP}" != "${CURRENT_IP}" ]]; then
  fail "DNS stale: ${HOST} → ${PUBLIC_IP}, this PC is ${CURRENT_IP}"
else
  pass "${HOST} → ${PUBLIC_IP}"
fi

note "coord on 127.0.0.1:8765"
if curl -fsS --connect-timeout 5 "http://127.0.0.1:8765/health" >/dev/null; then
  pass "http://127.0.0.1:8765/health"
else
  fail "http://127.0.0.1:8765/health — run install_coord1_home.sh"
fi

note "coord HTTPS /health (nginx —resolve ${HOST}:${HTTPS_PORT}:127.0.0.1)"
if HEALTH_JSON="$(curl -fsS --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" --connect-timeout 10 "https://${HOST}:${HTTPS_PORT}/health")"; then
  pass "https://${HOST}:${HTTPS_PORT}/health"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${HEALTH_JSON}" | jq -c '{ok, database, relay: .relay.wan_ready}')"
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/health — run enable_coord1_https.sh"
fi

note "GET /v1/relay (nginx)"
if RELAY_JSON="$(curl -fsS --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" --connect-timeout 10 "https://${HOST}:${HTTPS_PORT}/v1/relay")"; then
  pass "https://${HOST}:${HTTPS_PORT}/v1/relay"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${RELAY_JSON}" | jq -c '{enabled, addrs}')"
    PARSED="$(echo "${RELAY_JSON}" | jq -r '.addrs[0] // empty' | sed -n 's|.*/tcp/\([0-9]*\)$|\1|p')"
    [[ -n "${PARSED}" ]] && RELAY_PORT="${PARSED}"
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/v1/relay"
fi

note "nginx listen :${HTTPS_PORT}"
if ss -tln 2>/dev/null | grep -qE ":${HTTPS_PORT}\\b"; then
  pass "nginx listening on :${HTTPS_PORT}"
else
  fail ":${HTTPS_PORT} not listening — check nginx"
fi

note "relay TCP on 127.0.0.1:${RELAY_PORT}"
if timeout 5 bash -c "echo >/dev/tcp/127.0.0.1/${RELAY_PORT}" 2>/dev/null; then
  pass "127.0.0.1:${RELAY_PORT} TCP open"
else
  fail "127.0.0.1:${RELAY_PORT} TCP closed — check ghal-bol-coord1"
fi

echo ""
if [[ "${failures}" -eq 0 ]]; then
  echo "OK — ${HOST} coord + relay ready (HTTPS :${HTTPS_PORT}, relay TCP :${RELAY_PORT})."
  exit 0
fi

echo "FAILED — ${failures} check(s)." >&2
exit 1
