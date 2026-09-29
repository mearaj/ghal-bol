#!/usr/bin/env bash
# Local dev — run ghal_bol_coord on this machine.
#
# Config: ghal_bol_coord/.env.development (copy from .env.development.example once)
#
#   ./ghal_bol_coord/deploy/run_server.sh
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="$(cd "${DEPLOY_DIR}/.." && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"
# shellcheck source=load_env.sh
source "${DEPLOY_DIR}/load_env.sh"
load_server_env "${SERVER_DIR}" ".env.development"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/build/ghal_bol_coord-target}"
BIN="${CARGO_TARGET_DIR}/release/ghal_bol_coord"

cd "${WORKSPACE_ROOT}"

need_build=0
if [[ ! -x "${BIN}" ]]; then
  need_build=1
elif [[ "${WORKSPACE_ROOT}/Cargo.toml" -nt "${BIN}" ]] \
  || [[ "${WORKSPACE_ROOT}/ghal_bol_coord/Cargo.toml" -nt "${BIN}" ]]; then
  need_build=1
elif find "${WORKSPACE_ROOT}/ghal_bol_coord/src" -name '*.rs' -newer "${BIN}" -print -quit | grep -q .; then
  need_build=1
fi

if [[ "${need_build}" -eq 1 ]]; then
  cargo build --release -p ghal_bol_coord --bin ghal_bol_coord
fi

if [[ ! -x "${BIN}" ]]; then
  echo "error: ${BIN} missing after build" >&2
  exit 1
fi

# Phones on Wi‑Fi need a routable bind (override with 127.0.0.1:8765 for loopback-only).
export GHAL_BOL_COORD_LISTEN="${GHAL_BOL_COORD_LISTEN:-0.0.0.0:8765}"

if [[ -n "${GHAL_BOL_COORD_PUBLIC_BASE_URL:-}" ]]; then
  echo "coord public base: ${GHAL_BOL_COORD_PUBLIC_BASE_URL}" >&2
else
  echo "NOTE: set GHAL_BOL_COORD_PUBLIC_BASE_URL for bridge connect_url in responses (see COORDINATION_SERVER.md)." >&2
fi

exec "${BIN}"
