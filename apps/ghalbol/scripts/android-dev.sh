#!/usr/bin/env bash
# Android dev APK build for THIS app (GhalBol Chat): loads
# apps/ghalbol/env/development.env then builds the debug APK.
# Usage: ./apps/ghalbol/scripts/android-dev.sh   or: cargo ghalbol-android-dev
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
source "$ROOT/scripts/load-ghalbol-env.sh"
export GHALBOL_ROOT="$ROOT"
export GHALBOL_APP_DIR="$ROOT/apps/ghalbol"
export GHALBOL_ENV_DIR="$GHALBOL_APP_DIR/env"
load_ghalbol_env development

echo "Building GhalBol debug APK..."
echo "  App ID:   ${GHALBOL_APP_ID:-com.ghalbol.debug}"
echo "  Label:    ${GHALBOL_APP_LABEL:-Ghal Bol Dev}"

# Existing workspace Android flow (cargo-ndk based).
"$ROOT/scripts/build_android_app.sh" "$@"
