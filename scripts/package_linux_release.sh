#!/usr/bin/env bash
# Production Linux x86_64 package for https://ghalbol.com/download/linux.
# Writes web/downloads/ghalbol-linux-x64.tar.gz: extract, then ./install.sh (or ./run.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export GHALBOL_ROOT="$ROOT"
export GHALBOL_APP_DIR="$ROOT/apps/ghalbol"
export GHALBOL_ENV_DIR="$GHALBOL_APP_DIR/env"

source "$ROOT/scripts/load-ghalbol-env.sh"
source "$ROOT/scripts/desktop-resources.sh"
source "$ROOT/scripts/desktop-ids.sh"
source "$ROOT/scripts/stage-desktop-package.sh"
source "$ROOT/scripts/cargo-target-dir.sh"

load_ghalbol_env production
makepad_enable_package_dir

cargo build -p ghalbol_app --release

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
PKG="$STAGE/ghalbol"
mkdir -p "$PKG" "$ROOT/web/downloads"
cp "$(cargo_target_dir)/release/ghalbol_app" "$PKG/$(desktop_bin_name)"
makepad_copy_desktop_resources "$PKG"
stage_desktop_package "$PKG" linux

tar -C "$STAGE" -czf "$ROOT/web/downloads/ghalbol-linux-x64.tar.gz" ghalbol
echo "Wrote web/downloads/ghalbol-linux-x64.tar.gz ($(du -h "$ROOT/web/downloads/ghalbol-linux-x64.tar.gz" | awk '{print $1}'))"
