#!/usr/bin/env bash
# GhalBol Windows cross-compile helpers.
# Based on Kumkaj's windows-env.sh pattern.
#
# Makepad desktop packaging (same model as Makepad Builder):
#   MAKEPAD_PACKAGE_DIR=.
#   resources live beside the .exe as <rust_crate_name>/resources/...
#   Cargo package `ghalbol_app` → rustc module `ghalbol_app` (underscores).
#   Packaged dep paths are therefore `ghalbol_app/resources/...`.

windows_default_target() {
  echo "x86_64-pc-windows-gnu"
}

windows_ensure_toolchain() {
  local target
  target="$(windows_default_target)"
  if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
    echo "Installing Rust target $target …"
    rustup target add "$target"
  fi
  if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "Missing x86_64-w64-mingw32-gcc (Arch: pacman -S mingw-w64-gcc)" >&2
    return 1
  fi
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CC_x86_64_pc_windows_gnu:-x86_64-w64-mingw32-gcc}"
  export CC_x86_64_pc_windows_gnu="${CC_x86_64_pc_windows_gnu:-x86_64-w64-mingw32-gcc}"
  export CXX_x86_64_pc_windows_gnu="${CXX_x86_64_pc_windows_gnu:-x86_64-w64-mingw32-g++}"
  echo "Windows cross: target=$target linker=$CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER"
}

# Compile-time package root: load resources relative to the .exe / cwd.
makepad_enable_package_dir() {
  export MAKEPAD_PACKAGE_DIR=.
}

# Stage desktop resources next to ghalbol_app.exe.
# One tree only: ghalbol_app/resources/ (fonts + icons). Underscores required.
makepad_copy_desktop_resources() {
  local out="$1"
  local root="${GHALBOL_ROOT:-}"
  if [[ -z "$root" ]]; then
    echo "makepad_copy_desktop_resources: GHALBOL_ROOT not set" >&2
    return 1
  fi

  # Derive crate resources dir from app's Cargo.toml package name (hyphens → underscores).
  local crate_dir="${GHALBOL_CRATE_DIR:-ghalbol_app}"

  # Remove old trees.
  rm -rf "$out/ghalbol" "$out/ghalbol_app" "$out/makepad_widgets"
  mkdir -p "$out/$crate_dir/resources/fonts"

  local app_dir="${GHALBOL_APP_DIR:-$root/apps/ghalbol}"
  rsync -a --exclude 'android/' \
    "$app_dir/resources/" "$out/$crate_dir/resources/"

  # CJK + colour emoji: live in vendor Makepad.
  local widgets_res="$root/vendor/makepad/widgets/resources"
  local fonts_out="$out/$crate_dir/resources/fonts"
  local f
  for f in LXGWWenKaiRegular.ttf LXGWWenKaiBold.ttf NotoColorEmoji.ttf; do
    if [[ -f "$fonts_out/$f" ]]; then
      continue
    fi
    if [[ -f "$widgets_res/$f" ]]; then
      cp -f "$widgets_res/$f" "$fonts_out/$f"
    else
      echo "warn: missing $f (run ./scripts/setup-makepad.sh + ./scripts/fetch-ui-fonts.sh)" >&2
    fi
  done

  if [[ ! -f "$fonts_out/NotoSans-Regular.ttf" ]]; then
    echo "error: missing NotoSans-Regular.ttf under $fonts_out" >&2
    echo "Run ./scripts/fetch-ui-fonts.sh before packaging." >&2
    return 1
  fi
}
