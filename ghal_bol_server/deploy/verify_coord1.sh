#!/usr/bin/env bash
# WAN check for coord1.ghalbol.com — GoDaddy DNS, HTTPS :8443, relay TCP (port from /v1/relay).
#
#   ./ghal_bol_server/deploy/verify_coord1.sh
set -euo pipefail

HOST="${COORD1_HOST:-coord1.ghalbol.com}"
HTTPS_PORT="${COORD1_HTTPS_PORT:-8443}"
CURL_TIMEOUT="${VERIFY_CURL_TIMEOUT:-10}"
TCP_TIMEOUT="${VERIFY_TCP_TIMEOUT:-8}"
COORD_INSECURE="${COORD_INSECURE_TLS:-0}"

CURL_TLS=()
[[ "${COORD_INSECURE}" == "1" ]] && CURL_TLS=(-k)

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
  fail "DNS: no A record for ${HOST} — check GHAL_BOL_DDNS_CREDENTIALS and journalctl --user -u ghal-bol-server-coord1"
elif [[ -n "${CURRENT_IP}" && "${PUBLIC_IP}" != "${CURRENT_IP}" ]]; then
  fail "DNS stale: ${HOST} → ${PUBLIC_IP}, this PC is ${CURRENT_IP} — restart ghal-bol-server-coord1 or run godaddy-ddns.sh once"
else
  pass "${HOST} → ${PUBLIC_IP}"
fi

note "coord HTTPS (app URL)"
if HEALTH_JSON="$(curl -fsS "${CURL_TLS[@]}" --connect-timeout "${CURL_TIMEOUT}" "https://${HOST}:${HTTPS_PORT}/health")"; then
  pass "https://${HOST}:${HTTPS_PORT}/health"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${HEALTH_JSON}" | jq -c '{ok, database, relay: .relay.wan_ready}')"
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/health — nginx :443 or router forward"
fi

RELAY_PORT=""
note "GET /v1/relay via HTTPS"
if RELAY_JSON="$(curl -fsS "${CURL_TLS[@]}" --connect-timeout "${CURL_TIMEOUT}" "https://${HOST}:${HTTPS_PORT}/v1/relay")"; then
  pass "https://${HOST}:${HTTPS_PORT}/v1/relay"
  if command -v jq >/dev/null 2>&1; then
    echo "     $(echo "${RELAY_JSON}" | jq -c '{enabled, addrs}')"
    RELAY_PORT="$(echo "${RELAY_JSON}" | jq -r '.addrs[0] // empty' | sed -n 's|.*/tcp/\([0-9]*\)$|\1|p')"
  fi
else
  fail "https://${HOST}:${HTTPS_PORT}/v1/relay"
fi

if [[ -z "${RELAY_PORT}" ]]; then
  RELAY_PORT="${GHAL_BOL_RELAY_PORT:-4002}"
  note "relay port fallback ${RELAY_PORT} (parse /v1/relay addrs for UPnP dynamic port)"
fi

note "relay libp2p TCP on public IP (not nginx)"
if [[ -n "${PUBLIC_IP}" ]]; then
  if timeout "${TCP_TIMEOUT}" bash -c "echo >/dev/tcp/${PUBLIC_IP}/${RELAY_PORT}" 2>/dev/null; then
    pass "${PUBLIC_IP}:${RELAY_PORT} TCP open"
  else
    fail "${PUBLIC_IP}:${RELAY_PORT} TCP timeout — check UPnP log in journalctl --user -u ghal-bol-server-coord1"
  fi
fi

echo ""
if [[ "${failures}" -eq 0 ]]; then
  echo "OK — ${HOST} WAN coord + relay reachable (relay TCP :${RELAY_PORT})."
  exit 0
fi

echo "FAILED — ${failures} WAN check(s)." >&2
exit 1
