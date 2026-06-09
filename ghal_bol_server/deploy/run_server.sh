#!/usr/bin/env bash
# Run ghal_bol_server from the workspace (no global install paths).
#
# WAN relay (chat/voice/video across NAT): by default starts bore.pub → local relay port 4002
# and advertises the tunnel at GET /v1/relay. ngrok free TCP cannot carry libp2p Noise; use
# ngrok http for coord only (GHAL_BOL_COORD_URLS in the app).
#
# Opt out: GHAL_BOL_RELAY_BORE=0, or set GHAL_BOL_RELAY_PUBLIC_ADDRS / GHAL_BOL_RELAY_PUBLIC_HOST
# (production VM / paid ngrok TCP).
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/build/ghal_bol_server-target}"
BIN="${CARGO_TARGET_DIR}/release/ghal_bol_server"

cd "${WORKSPACE_ROOT}"

need_build=0
if [[ ! -x "${BIN}" ]]; then
  need_build=1
elif [[ "${WORKSPACE_ROOT}/Cargo.toml" -nt "${BIN}" ]] \
  || [[ "${WORKSPACE_ROOT}/ghal_bol_server/Cargo.toml" -nt "${BIN}" ]]; then
  need_build=1
elif find "${WORKSPACE_ROOT}/ghal_bol_server/src" -name '*.rs' -newer "${BIN}" -print -quit | grep -q .; then
  need_build=1
fi

if [[ "${need_build}" -eq 1 ]]; then
  # Default shell SIGINT during build (do not trap globally — that steals Ctrl+C from `exec`'d server).
  cargo build --release -p ghal_bol_server --bin ghal_bol_server
fi

if [[ ! -x "${BIN}" ]]; then
  echo "error: ${BIN} missing after build" >&2
  exit 1
fi

# Phones on Wi‑Fi need a routable bind (override with 127.0.0.1:8765 for loopback-only).
export GHAL_BOL_SERVER_LISTEN="${GHAL_BOL_SERVER_LISTEN:-0.0.0.0:8765}"
# Same default as the binary: ~/.local/share/com.ghalbol/ghalbol_server/coord.db
# Override only if you need a different dir:
# export GHAL_BOL_SERVER_DB="${HOME}/.local/share/com.ghalbol/ghalbol_server"

BORE_PID=""
BORE_LOG=""
cleanup_bore() {
  [[ -n "${BORE_PID}" ]] && kill "${BORE_PID}" 2>/dev/null || true
  [[ -n "${BORE_LOG}" ]] && rm -f "${BORE_LOG}" 2>/dev/null || true
}

# Default on for dev: without a public relay addr, WAN chat/calls cannot traverse NAT.
should_start_bore() {
  local bore_flag="${GHAL_BOL_RELAY_BORE:-}"
  if [[ "${bore_flag}" == "0" || "${bore_flag}" == "false" || "${bore_flag}" == "no" ]]; then
    return 1
  fi
  local relay_enable="${GHAL_BOL_RELAY_ENABLE:-}"
  if [[ "${relay_enable}" == "0" || "${relay_enable}" == "false" || "${relay_enable}" == "no" ]]; then
    return 1
  fi
  if [[ -n "${GHAL_BOL_RELAY_PUBLIC_ADDRS:-}" ]]; then
    return 1
  fi
  if [[ -n "${GHAL_BOL_RELAY_PUBLIC_HOST:-}" ]]; then
    return 1
  fi
  return 0
}

if should_start_bore; then
  RELAY_LOCAL_PORT="${RELAY_LOCAL_PORT:-4002}"
  BORE_HOST="${BORE_HOST:-bore.pub}"
  BORE_LOG="$(mktemp -t ghalbol_bore.XXXXXX.log)"

  command -v bore >/dev/null 2>&1 || {
    echo "error: bore not installed (cargo install bore-cli)" >&2
    exit 1
  }

  trap cleanup_bore EXIT INT TERM

  echo "Starting bore: local ${RELAY_LOCAL_PORT} -> ${BORE_HOST} ..."
  bore local "${RELAY_LOCAL_PORT}" --to "${BORE_HOST}" >"${BORE_LOG}" 2>&1 &
  BORE_PID=$!

  # bore logs e.g. "listening at bore.pub:38421" — capture the assigned remote port.
  REMOTE_PORT=""
  for _ in $(seq 1 30); do
    if ! kill -0 "${BORE_PID}" 2>/dev/null; then
      echo "error: bore exited early:" >&2
      cat "${BORE_LOG}" >&2
      exit 1
    fi
    # grep returns 1 while bore is still connecting — must not trip `set -e`.
    REMOTE_PORT="$(grep -oE "${BORE_HOST}:[0-9]+" "${BORE_LOG}" 2>/dev/null | head -n1 | sed 's/.*://' || true)"
    [[ -n "${REMOTE_PORT}" ]] && break
    sleep 1
  done

  if [[ -z "${REMOTE_PORT}" ]]; then
    echo "error: could not determine bore remote port. bore output:" >&2
    cat "${BORE_LOG}" >&2
    exit 1
  fi

  # Advertise resolved IPv4(s) only — bore.pub geo-DNS returns region-specific IPs that do not
  # route to this tunnel; IP-only keeps all clients dialing the same server-side resolved address.
  IPS="$(getent ahosts "${BORE_HOST}" 2>/dev/null | awk '{print $1}' | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' | sort -u)"
  if [[ -z "${IPS}" ]]; then
    echo "error: could not resolve ${BORE_HOST} to IPv4 — WAN relay cannot be advertised" >&2
    exit 1
  fi
  ADDRS=""
  for ip in ${IPS}; do ADDRS="${ADDRS:+${ADDRS},}/ip4/${ip}/tcp/${REMOTE_PORT}"; done
  export GHAL_BOL_RELAY_PUBLIC_ADDRS="${ADDRS}"

  echo "bore relay endpoint  : ${BORE_HOST}:${REMOTE_PORT}"
  echo "resolved IPv4(s)     : ${IPS:-<none>}"
  echo "advertising via /v1/relay: ${GHAL_BOL_RELAY_PUBLIC_ADDRS}"
  echo "(verify after start:  curl -s http://127.0.0.1:8765/v1/relay | jq)"
  echo

  # Foreground server (not exec) so the trap can stop bore when the server exits.
  "${BIN}"
else
  exec "${BIN}"
fi
