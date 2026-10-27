#!/usr/bin/env bash
# Ghal Bol — per-user Linux install from an extracted package (no root).
# Copies the package to ~/.local/lib/<slug>/, links ~/.local/bin/<bin>, and adds the
# menu entry + icon. The app itself keeps the menu entry current and adds login
# autostart each time it starts.
set -euo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SRC/desktop-ids.sh"
desktop_load_manifest "$SRC/install-manifest.env"

DEST="$(desktop_install_dir_linux)"
BIN_DIR="$(desktop_bin_dir_linux)"
APPS_DIR="$(desktop_applications_dir_linux)"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/128x128/apps"

if [[ "$SRC" != "$DEST" ]]; then
  rm -rf "$DEST"
  mkdir -p "$DEST"
  cp -R "$SRC/." "$DEST/"
fi
mkdir -p "$BIN_DIR" "$APPS_DIR" "$ICON_DIR" "${XDG_CONFIG_HOME:-$HOME/.config}"
ln -sfn "$DEST/$BIN_NAME" "$BIN_DIR/$BIN_NAME"
cp -f "$DEST/ghalbol_app/resources/icon_128.png" "$ICON_DIR/$ENTRY_ID.png"
cat >"$APPS_DIR/$ENTRY_ID.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$GHALBOL_APP_LABEL
Exec=$DEST/$BIN_NAME %u
Icon=$ENTRY_ID
StartupNotify=true
MimeType=x-scheme-handler/ghalbol;
EOF
command -v xdg-mime >/dev/null 2>&1 \
  && xdg-mime default "$ENTRY_ID.desktop" x-scheme-handler/ghalbol || true
command -v update-desktop-database >/dev/null 2>&1 \
  && update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true

echo "$GHALBOL_APP_LABEL installed to $DEST"
echo "Start it from the app menu or run: $BIN_NAME"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "Note: $BIN_DIR is not on your PATH." ;;
esac
