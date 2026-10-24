#!/usr/bin/env bash
# Stage install/uninstall helpers + manifest into a desktop package folder.
# Usage (after binary + resources are in $OUT):
#   source scripts/desktop-ids.sh
#   stage_desktop_package "$OUT" linux|windows
stage_desktop_package() {
  local out="${1:?}"
  local os="${2:?}" # linux | windows
  local root="${GHALBOL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
  local scripts="$root/scripts"

  mkdir -p "$out"
  cp -f "$scripts/desktop-ids.sh" "$out/desktop-ids.sh"

  # Manifest baked into the package (install copies it into the install dir).
  {
    echo "# Packaged with GhalBol — identifies prod vs debug install."
    echo "GHALBOL_ENV=${GHALBOL_ENV:?}"
    echo "GHALBOL_APP_ID=${GHALBOL_APP_ID:?}"
    echo "GHALBOL_APP_LABEL=${GHALBOL_APP_LABEL:?}"
    echo "INSTALL_SLUG=$(desktop_install_slug)"
    echo "BIN_NAME=$(desktop_bin_name)"
    echo "ENTRY_ID=$(desktop_entry_id)"
  } >"$out/install-manifest.env"

  if [[ "$os" == "linux" ]]; then
    cp -f "$scripts/install-linux.sh" "$out/install.sh"
    cp -f "$scripts/uninstall-linux.sh" "$out/uninstall.sh"
    chmod +x "$out/install.sh" "$out/uninstall.sh" "$out/desktop-ids.sh"
    # Portable launcher without full install.
    cat >"$out/run.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export MAKEPAD_PACKAGE_DIR=.
cd "$ROOT"
exec "$ROOT/bin/ghalbol" "$@"
EOF
    chmod +x "$out/run.sh"
  else
    cp -f "$scripts/install-windows.ps1" "$out/install.ps1"
    cp -f "$scripts/uninstall-windows.ps1" "$out/uninstall.ps1"
    # Double-click entry points (opening .ps1 in Explorer often launches Notepad).
    cp -f "$scripts/Setup.bat" "$out/Setup.bat"
    cp -f "$scripts/Uninstall.bat" "$out/Uninstall.bat"
  fi
}
