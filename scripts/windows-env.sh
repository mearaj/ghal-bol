#!/usr/bin/env bash
# GhalBol Windows cross-compile helpers.
# Resource staging (MAKEPAD_PACKAGE_DIR layout) lives in desktop-resources.sh.

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
