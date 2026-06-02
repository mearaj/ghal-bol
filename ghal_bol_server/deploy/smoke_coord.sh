#!/usr/bin/env bash
# Server-focused smoke: unit/e2e tests, then optional live API against COORD_URL.
#
#   COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
#   COORD_URL=https://YOUR.ngrok-free.dev ./ghal_bol_server/deploy/smoke_coord.sh
#   LIVE_ONLY=1 COORD_URL=... COORD_INSECURE_TLS=1 ./ghal_bol_server/deploy/smoke_coord.sh  # skip cargo test
set -euo pipefail

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${DEPLOY_DIR}/../.." && pwd)"
cd "${WORKSPACE_ROOT}"

if [[ "${LIVE_ONLY:-0}" != "1" ]]; then
  echo "== ghal_bol_server: in-process API tests =="
  cargo test -p ghal_bol_server --test http_api

  echo "== ghal_bol_server: production E2E (subprocess + TCP + SQLite) =="
  cargo test -p ghal_bol_server --test e2e_production
fi

echo "== build server + coord_client =="
cargo build --release -p ghal_bol_server

COORD_URL="${COORD_URL:-}"
COORD_INSECURE_TLS="${COORD_INSECURE_TLS:-0}"

if [[ -z "${COORD_URL}" ]]; then
  echo "== live API smoke skipped (set COORD_URL to test a running server) =="
  echo "   Example: COORD_URL=http://127.0.0.1:8765 $0"
  exit 0
fi

CLIENT="${WORKSPACE_ROOT}/target/release/coord_client"
# -k must come before <base_url> (coord_client parse order).
CLIENT_ARGS=()
if [[ "${COORD_INSECURE_TLS}" == "1" ]]; then
  CLIENT_ARGS+=(-k)
fi
CLIENT_ARGS+=("${COORD_URL}")

run_client() {
  local subcmd="$1"
  echo ">> ${CLIENT} ${CLIENT_ARGS[*]} ${subcmd}"
  "${CLIENT}" "${CLIENT_ARGS[@]}" "${subcmd}"
}

echo "== live health: ${COORD_URL} =="
run_client health

echo "== live demo-two-peers: ${COORD_URL} =="
run_client demo-two-peers

echo "OK — coordination server smoke passed against ${COORD_URL}"
