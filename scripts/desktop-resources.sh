#!/usr/bin/env bash
# Makepad desktop packaging helpers shared by the Linux and Windows packages.
#
# Packaged layout (Makepad resolves `<crate>:resources/…` under MAKEPAD_PACKAGE_DIR, searched
# from both the working directory and the executable's directory):
#   <package>/<binary>
#   <package>/ghalbol_app/resources/…        app fonts + icons (crate name, underscores)
#   <package>/makepad_widgets/resources/…    Font Awesome icons, CJK (LXGW) and emoji fonts
#
# Source from other scripts:  source "$ROOT/scripts/desktop-resources.sh"

# Compile-time package root: load resources relative to the binary / cwd.
makepad_enable_package_dir() {
  export MAKEPAD_PACKAGE_DIR=.
}

# Directory of the makepad-widgets crate this workspace builds against (git checkout).
makepad_widgets_dir() {
  local root="${GHALBOL_ROOT:?GHALBOL_ROOT not set}"
  local manifest
  manifest="$(cd "$root" && cargo metadata --format-version 1 --locked 2>/dev/null \
    | tr ',' '\n' \
    | grep -o '"manifest_path":"[^"]*/widgets/Cargo.toml"' \
    | head -n 1 \
    | sed -e 's/^"manifest_path":"//' -e 's/"$//')"
  if [[ -z "$manifest" ]]; then
    echo "makepad_widgets_dir: makepad-widgets not found in cargo metadata" >&2
    return 1
  fi
  dirname "$manifest"
}

# Stage app + makepad_widgets resources into a package folder.
makepad_copy_desktop_resources() {
  local out="${1:?}"
  local root="${GHALBOL_ROOT:?GHALBOL_ROOT not set}"
  local app_dir="${GHALBOL_APP_DIR:-$root/apps/ghalbol}"
  # Cargo package `ghalbol_app` → resource folder `ghalbol_app` (underscores).
  local crate_dir="${GHALBOL_CRATE_DIR:-ghalbol_app}"
  local widgets
  widgets="$(makepad_widgets_dir)" || return 1

  rm -rf "$out/$crate_dir" "$out/makepad_widgets"
  mkdir -p "$out/$crate_dir" "$out/makepad_widgets"
  cp -R "$app_dir/resources" "$out/$crate_dir/resources"
  rm -rf "$out/$crate_dir/resources/android"
  cp -R "$widgets/resources" "$out/makepad_widgets/resources"

  local f
  for f in "$crate_dir/resources/fonts/NotoSans-Regular.ttf" \
    makepad_widgets/resources/fa-solid-900.ttf \
    makepad_widgets/resources/LXGWWenKaiRegular.ttf \
    makepad_widgets/resources/NotoColorEmoji.ttf; do
    if [[ ! -f "$out/$f" ]]; then
      echo "error: packaged resource missing: $f" >&2
      return 1
    fi
  done
}
