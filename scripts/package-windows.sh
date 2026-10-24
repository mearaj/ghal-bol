#!/usr/bin/env bash
# Production Windows x86_64 package for GhalBol.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export GHALBOL_APP_DIR="$ROOT/apps/ghalbol"
export GHALBOL_ENV_DIR="$GHALBOL_APP_DIR/env"

source "$ROOT/scripts/load-ghalbol-env.sh"
source "$ROOT/scripts/windows-env.sh"
source "$ROOT/scripts/desktop-ids.sh"
source "$ROOT/scripts/stage-desktop-package.sh"

export GHALBOL_ROOT="$ROOT"
load_ghalbol_env production
makepad_enable_package_dir

OUT="$ROOT/dist/windows"
rm -rf "$OUT"
mkdir -p "$OUT"
TARGET="$(windows_default_target)"

if [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* || "$(uname -s)" == CYGWIN* ]]; then
  if rustup target list --installed 2>/dev/null | grep -qx 'x86_64-pc-windows-msvc'; then
    TARGET=x86_64-pc-windows-msvc
  fi
else
  windows_ensure_toolchain
fi

cargo build -p ghalbol_app --release --target "$TARGET"

source "$ROOT/scripts/cargo-target-dir.sh"
TARGET_DIR="$(cargo_target_dir)"
EXE=""
for cand in \
  "$TARGET_DIR/$TARGET/release/ghalbol_app.exe" \
  "$TARGET_DIR/$TARGET/release/ghalbol_app" \
  "$TARGET_DIR/release/ghalbol_app.exe"
do
  if [[ -f "$cand" ]]; then
    EXE="$cand"
    break
  fi
done
if [[ -z "$EXE" ]]; then
  echo "Built binary not found under $TARGET_DIR/$TARGET/release/" >&2
  exit 1
fi

cp -f "$EXE" "$OUT/ghalbol-app.exe"
makepad_copy_desktop_resources "$OUT"
stage_desktop_package "$OUT" windows

SIZE="$(du -h "$OUT/ghalbol-app.exe" | awk '{print $1}')"

cat >"$OUT/README.txt" <<EOF
GhalBol Chat Windows (production)
=================================
App id:   ${GHALBOL_APP_ID}
Label:    ${GHALBOL_APP_LABEL}
Exe size: ${SIZE}
Target:   $TARGET (x86_64 — Windows 10 and 11)

Install (recommended):
  Double-click Setup.bat
→ %LOCALAPPDATA%\\Programs\\Ghal Bol\\

Portable run (no install):
  .\\ghalbol-app.exe

Uninstall:
  Start Menu → Uninstall Ghal Bol
  or double-click Uninstall.bat
EOF

echo "Windows production package: $OUT/"
