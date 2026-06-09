#!/usr/bin/env bash
# DEV: start ghal_bol_server with the relay's public address auto-detected from a
# running ngrok agent, so GET /v1/relay advertises the correct ngrok TCP endpoint
# and clients can actually dial + reserve a circuit on the relay.
#
# Order of operations:
#   1) Start ngrok FIRST (both tunnels) in another terminal:
#        ngrok start --all      # needs ngrok.yml.example merged into ~/.config/ngrok/ngrok.yml
#      ngrok TCP (free) requires a verified payment method on your account (still $0).
#   2) Run this script. It reads ngrok's local API (http://127.0.0.1:4040), finds the
#      tcp:// tunnel, and starts the server advertising /dns4/<host>/tcp/<port>.
#
# Re-run this whenever ngrok restarts (the free TCP host:port changes each run).
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NGROK_API="${NGROK_API:-http://127.0.0.1:4040/api/tunnels}"

command -v jq >/dev/null 2>&1 || { echo "error: jq is required (sudo pacman -S jq / apt install jq)" >&2; exit 1; }

echo "Looking for an ngrok tcp tunnel on ${NGROK_API} ..."
TCP_URL=""
for _ in $(seq 1 30); do
  TCP_URL="$(curl -s --max-time 3 "${NGROK_API}" 2>/dev/null \
    | jq -r '.tunnels[]? | select(.proto=="tcp") | .public_url' 2>/dev/null | head -n1 || true)"
  [[ -n "${TCP_URL}" && "${TCP_URL}" != "null" ]] && break
  sleep 1
done

if [[ -z "${TCP_URL}" || "${TCP_URL}" == "null" ]]; then
  echo "error: no ngrok tcp tunnel found." >&2
  echo "       Start ngrok first:  ngrok start --all" >&2
  echo "       (and confirm a tcp:// endpoint is listed at http://127.0.0.1:4040)" >&2
  exit 1
fi

HOSTPORT="${TCP_URL#tcp://}"
RELAY_HOST="${HOSTPORT%:*}"
RELAY_PORT="${HOSTPORT##*:}"

# Resolve the ngrok host to IPv4(s) and advertise /ip4/<ip>/tcp/<port> directly. This avoids
# the client having to resolve the ngrok hostname — ngrok sometimes hands out a regional host
# (e.g. *.tcp.ap.ngrok.io) that does NOT resolve, which silently kills the relay on clients.
IPS="$(getent ahosts "${RELAY_HOST}" 2>/dev/null | awk '{print $1}' | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' | sort -u)"
if [[ -z "${IPS}" ]]; then
  echo "WARNING: '${RELAY_HOST}' does not resolve to any IPv4 — this ngrok tunnel is unreachable." >&2
  echo "         ngrok gave a dead regional host. Restart ngrok (ngrok start --all) to get a new" >&2
  echo "         endpoint, or set the agent region. Falling back to advertising the hostname." >&2
  export GHAL_BOL_RELAY_PUBLIC_ADDRS="/dns4/${RELAY_HOST}/tcp/${RELAY_PORT}"
else
  ADDRS=""
  for ip in ${IPS}; do
    ADDRS="${ADDRS:+${ADDRS},}/ip4/${ip}/tcp/${RELAY_PORT}"
  done
  # Keep the hostname too, as a fallback for clients that resolve it fine.
  export GHAL_BOL_RELAY_PUBLIC_ADDRS="${ADDRS},/dns4/${RELAY_HOST}/tcp/${RELAY_PORT}"
fi

echo "ngrok relay endpoint : ${TCP_URL}"
echo "resolved IPv4(s)     : ${IPS:-<none>}"
echo "advertising via /v1/relay: ${GHAL_BOL_RELAY_PUBLIC_ADDRS}"
echo "(verify after start:  curl -s http://127.0.0.1:8765/v1/relay | jq)"
echo

exec "${DEPLOY_DIR}/run_server.sh"
