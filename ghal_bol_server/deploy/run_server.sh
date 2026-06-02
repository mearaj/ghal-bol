#!/usr/bin/env bash
# Run ghal_bol_server from the workspace (no global install paths).
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

exec "${BIN}"
