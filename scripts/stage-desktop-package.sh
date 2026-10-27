#!/usr/bin/env bash
# Stage install/uninstall helpers + manifest into a desktop package folder.
# Usage (after the binary + resources are in $OUT — see desktop-resources.sh):
#   source scripts/desktop-ids.sh
#   stage_desktop_package "$OUT" linux|windows
#
# Linux package:   <bin>, run.sh, install.sh, uninstall.sh, desktop-ids.sh, install-manifest.env
# Windows package: ghalbol-app.exe, Setup.bat, Uninstall.bat, install.ps1, uninstall.ps1,
#                  install-manifest.env
stage_desktop_package() {
  local out="${1:?}"
  local os="${2:?}" # linux | windows
  local root="${GHALBOL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
  local scripts="$root/scripts"

  mkdir -p "$out"

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
    cp -f "$scripts/desktop-ids.sh" "$out/desktop-ids.sh"
    cp -f "$scripts/linux/install.sh" "$out/install.sh"
    cp -f "$scripts/linux/uninstall.sh" "$out/uninstall.sh"
    # Portable launcher without a full install. Resources resolve beside the binary.
    cat >"$out/run.sh" <<EOF
#!/usr/bin/env bash
exec "\$(cd "\$(dirname "\${BASH_SOURCE[0]}")" && pwd)/$(desktop_bin_name)" "\$@"
EOF
    chmod +x "$out/install.sh" "$out/uninstall.sh" "$out/run.sh" "$out/desktop-ids.sh"
  else
    cp -f "$scripts/windows/install.ps1" "$out/install.ps1"
    cp -f "$scripts/windows/uninstall.ps1" "$out/uninstall.ps1"
    # Double-click entry points (opening .ps1 in Explorer often launches Notepad).
    cp -f "$scripts/windows/Setup.bat" "$out/Setup.bat"
    cp -f "$scripts/windows/Uninstall.bat" "$out/Uninstall.bat"
  fi
}
