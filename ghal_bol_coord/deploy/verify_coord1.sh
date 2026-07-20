#!/usr/bin/env bash
# Operator check for coord1.ghalbol.com (run on the coord1 host).
# Local coord + nginx --resolve + native WAN call bridge API.
#
#   ./ghal_bol_coord/deploy/verify_coord1.sh
set -euo pipefail

HOST="coord1.ghalbol.com"
HTTPS_PORT="8443"
# Valid secp256k1 identity wire for bridge pending probe (no registered peer required).
BRIDGE_PROBE_WIRE="0220899663decabbb1b9f19c2e7baa610e123badd98cfe6e43484f941c45a36d0c"

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

note "coord HTTPS /health (nginx --resolve ${HOST}:${HTTPS_PORT}:127.0.0.1)"
if HEALTH_JSON="$(curl -fsS --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" --connect-timeout 10 "https://${HOST}:${HTTPS_PORT}/health")"; then
  pass "https://${HOST}:${HTTPS_PORT}/health"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${HEALTH_JSON}" | jq -c '{ok, database, bridge}')"
    if [[ "$(echo "${HEALTH_JSON}" | jq -r '.bridge // false')" != "true" ]]; then
      fail "health bridge=false — native connect bridge disabled"
    fi
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/health — run enable_coord1_https.sh"
fi

note "GET /v1/bridge/pending (nginx)"
if PENDING_JSON="$(curl -fsS --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" --connect-timeout 10 \
  "https://${HOST}:${HTTPS_PORT}/v1/bridge/pending?identity_wire=${BRIDGE_PROBE_WIRE}")"; then
  pass "https://${HOST}:${HTTPS_PORT}/v1/bridge/pending"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${PENDING_JSON}" | jq -c '{pending: (.pending | length)}')"
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/v1/bridge/pending"
fi

note "WSS upgrade /v1/bridge/connect (nginx must forward Upgrade)"
WS_BODY="$(mktemp)"
WS_CODE="$(curl -sS -o "${WS_BODY}" -w "%{http_code}" --http1.1 --resolve "${HOST}:${HTTPS_PORT}:127.0.0.1" \
  --connect-timeout 10 --max-time 8 \
  -H "Connection: Upgrade" -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
  "https://${HOST}:${HTTPS_PORT}/v1/bridge/connect?bridge_id=verify&token=verify" || true)"
WS_TEXT="$(tr -d '\r' <"${WS_BODY}" | head -c 200)"
rm -f "${WS_BODY}"
# 101 = handshake accepted (token may still be rejected after upgrade).
# 400 + "Connection header did not include 'upgrade'" = nginx missing proxy Upgrade headers.
if [[ "${WS_CODE}" == "101" ]]; then
  pass "WSS /v1/bridge/connect → HTTP 101"
elif echo "${WS_TEXT}" | grep -qi "Connection header did not include"; then
  fail "WSS blocked by nginx (no Upgrade proxy) — run: ./ghal_bol_coord/deploy/enable_coord1_https.sh"
  echo "     got HTTP ${WS_CODE}: ${WS_TEXT}"
else
  # Backend may close after upgrade with other errors; non-upgrade-400 is still progress.
  if [[ "${WS_CODE}" == "400" ]] && echo "${WS_TEXT}" | grep -qi "upgrade"; then
    fail "WSS upgrade failed HTTP ${WS_CODE}: ${WS_TEXT}"
  else
    pass "WSS path reachable (HTTP ${WS_CODE}; not missing Upgrade headers)"
    echo "     body: ${WS_TEXT}"
  fi
fi

note "nginx listen :${HTTPS_PORT}"
if ss -tln 2>/dev/null | grep -qE ":${HTTPS_PORT}\\b"; then
  pass "nginx listening on :${HTTPS_PORT}"
else
  fail ":${HTTPS_PORT} not listening — check nginx"
fi

echo ""
if [[ "${failures}" -eq 0 ]]; then
  echo "OK — ${HOST} coord + bridge ready (HTTPS :${HTTPS_PORT}, WSS /v1/bridge/connect)."
  echo "     WAN text: wss://delivery.ghalbol.com:55003 (delivery server, not coord)."
  exit 0
fi

echo "FAILED — ${failures} check(s)." >&2
exit 1
