#!/usr/bin/env bash
# Run coordination server on all interfaces (phones / other PCs on LAN).
set -euo pipefail
export GHAL_BOL_SERVER_LISTEN="${GHAL_BOL_SERVER_LISTEN:-0.0.0.0:8765}"
exec "$(dirname "${BASH_SOURCE[0]}")/run_server.sh"
