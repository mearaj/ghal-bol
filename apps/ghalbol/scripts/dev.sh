#!/usr/bin/env bash
# Desktop live-dev for THIS app (GhalBol Chat): loads
# apps/ghalbol/env/development.env then runs with --hot --remote.
# Usage: ./apps/ghalbol/scripts/dev.sh   or: cargo ghalbol-dev
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
source "$ROOT/scripts/load-ghalbol-env.sh"
export GHALBOL_ROOT="$ROOT"
export GHALBOL_APP_DIR="$ROOT/apps/ghalbol"
export GHALBOL_ENV_DIR="$GHALBOL_APP_DIR/env"
load_ghalbol_env development
exec cargo run -p ghalbol_app -- "$@" --hot --remote
