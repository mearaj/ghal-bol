#!/usr/bin/env bash
# Ghal Bol — per-user Linux uninstall. Run from the install dir (~/.local/lib/<slug>/).
# Removes the install, launcher, menu entry, autostart and icon; asks before deleting data.
set -euo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SRC/desktop-ids.sh"
desktop_load_manifest "$SRC/install-manifest.env"

DEST="$(desktop_install_dir_linux)"
BIN_DIR="$(desktop_bin_dir_linux)"
APPS_DIR="$(desktop_applications_dir_linux)"
ICON="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps/$ENTRY_ID.png"
AUTOSTART="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/$ENTRY_ID.desktop"

pkill -f "^$DEST/$BIN_NAME" 2>/dev/null || true

DATA_DIR="$(desktop_data_dir_linux)"
if desktop_prompt_remove_data "$DATA_DIR"; then
  rm -rf "$DATA_DIR"
  echo "Data removed: $DATA_DIR"
fi

if [[ -L "$BIN_DIR/$BIN_NAME" ]]; then
  rm -f "$BIN_DIR/$BIN_NAME"
fi
rm -f "$APPS_DIR/$ENTRY_ID.desktop" "$AUTOSTART" "$ICON"
command -v update-desktop-database >/dev/null 2>&1 \
  && update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
if [[ -d "$DEST" && -f "$DEST/install-manifest.env" ]]; then
  rm -rf "$DEST"
fi
echo "$GHALBOL_APP_LABEL uninstalled."
