#!/usr/bin/env bash
# Build ghal_bol and copy the desktop library into ghal_bol_ui (Linux / Windows / macOS).
#
# Phone (Android): ./scripts/pack_android_workspace_jni_libs.sh
#
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if command -v pkill >/dev/null 2>&1; then
  # Linux comm names are limited to 15 bytes, so `pkill -x
  # ghal_bol_core_daemon` never matches. Match the executable path/argv and
  # ensure a rebuilt daemon cannot leave an old deleted process serving the
  # socket.
  pkill -f '[/]ghal_bol_core_daemon([[:space:]]|$)' 2>/dev/null || true
fi
runtime="${XDG_RUNTIME_DIR:-}"
if [[ -n "$runtime" ]]; then
  rm -f "$runtime/ghal_bol/p2p.sock" 2>/dev/null || true
fi
rm -f "${HOME:-}/.local/share/com.ghalbol/ghal_bol/p2p.sock" /tmp/ghal_bol/p2p.sock 2>/dev/null || true

PROFILE=debug
if [[ "${RELEASE:-}" == "1" ]]; then
  PROFILE=release
fi

TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin)['target_directory'])" 2>/dev/null || true)"
if [[ -z "${TARGET_DIR:-}" || ! -d "$TARGET_DIR" ]]; then
  TARGET_DIR="$ROOT/target"
fi

# Cargo places the cdylib under target/<profile>/deps/ (not target/<profile>/).
# Building only --bin ghal_bol_core_daemon does not refresh the cdylib artifact.
resolve_native_artifact() {
  local profile="$1"
  shift
  local name base="$TARGET_DIR/$profile"
  for name in "$@"; do
    if [[ -f "$base/deps/$name" ]]; then
      echo "$base/deps/$name"
      return 0
    fi
    if [[ -f "$base/$name" ]]; then
      echo "$base/$name"
      return 0
    fi
  done
  return 1
}

if [[ "$PROFILE" == "release" ]]; then
  echo "==> cargo build --release -p ghal_bol_core --lib --bin ghal_bol_core_daemon"
  cargo build --release -p ghal_bol_core --lib --bin ghal_bol_core_daemon
else
  echo "==> cargo build -p ghal_bol_core --lib --bin ghal_bol_core_daemon"
  cargo build -p ghal_bol_core --lib --bin ghal_bol_core_daemon
fi

LINUX_SO="$(resolve_native_artifact "$PROFILE" libghal_bol_core.so || resolve_native_artifact "$PROFILE" lib_ghal_bol_core.so || true)"
WIN_DLL="$(resolve_native_artifact "$PROFILE" ghal_bol_core.dll || resolve_native_artifact "$PROFILE" _ghal_bol_core.dll || true)"
MAC_DYLIB="$(resolve_native_artifact "$PROFILE" libghal_bol_core.dylib || resolve_native_artifact "$PROFILE" lib_ghal_bol_core.dylib || true)"
LINUX_DAEMON="$TARGET_DIR/$PROFILE/ghal_bol_core_daemon"

verify_linux_lib() {
  local so="$1"
  if strings "$so" 2>/dev/null | rg -q 'project_dirs_from_application_id'; then
    echo "ERROR: $so still contains legacy storage path code (project_dirs_from_application_id)."
    exit 1
  fi
}

mkdir -p "$ROOT/ghal_bol_ui/linux/native/lib"
if [[ -n "$LINUX_SO" ]]; then
  verify_linux_lib "$LINUX_SO"
  cp -f "$LINUX_SO" "$ROOT/ghal_bol_ui/linux/native/lib/lib_ghal_bol_core.so"
  echo "OK: $LINUX_SO -> ghal_bol_ui/linux/native/lib/lib_ghal_bol_core.so"
else
  echo "ERROR: no libghal_bol_core.so under $TARGET_DIR/$PROFILE (deps/ or root). Build failed?"
  exit 1
fi

mkdir -p "$ROOT/ghal_bol_ui/linux/native/libexec"
if [[ -f "$LINUX_DAEMON" ]]; then
  cp -f "$LINUX_DAEMON" "$ROOT/ghal_bol_ui/linux/native/libexec/"
  chmod +x "$ROOT/ghal_bol_ui/linux/native/libexec/ghal_bol_core_daemon"
  echo "OK: $LINUX_DAEMON -> ghal_bol_ui/linux/native/libexec/"
else
  echo "ERROR: no $LINUX_DAEMON"
  exit 1
fi

mkdir -p "$ROOT/ghal_bol_ui/windows/native"
if [[ -n "$WIN_DLL" ]]; then
  cp -f "$WIN_DLL" "$ROOT/ghal_bol_ui/windows/native/_ghal_bol_core.dll"
  echo "OK: $WIN_DLL -> ghal_bol_ui/windows/native/_ghal_bol_core.dll"
else
  echo "skip: no ghal_bol_core.dll (build on Windows or cross-compile to stage it)"
fi

mkdir -p "$ROOT/ghal_bol_ui/macos/native"
if [[ -n "$MAC_DYLIB" ]]; then
  cp -f "$MAC_DYLIB" "$ROOT/ghal_bol_ui/macos/native/lib_ghal_bol_core.dylib"
  echo "OK: $MAC_DYLIB -> ghal_bol_ui/macos/native/lib_ghal_bol_core.dylib"
else
  echo "skip: no libghal_bol_core.dylib (build on macOS to stage it)"
fi

echo "Done. Android: ./scripts/pack_android_workspace_jni_libs.sh  then flutter run (all ABIs by default)"
