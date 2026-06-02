#!/usr/bin/env bash
# Build libghal_bol.so for Android → workspace/build/android-native-ndk/
# Gradle: android/app/build.gradle.kts jniLibs.srcDirs. Does not use adb.
#
# Default: all standard Android ABIs (armeabi-v7a, arm64-v8a, x86, x86_64).
# Fast path (phone only): PACK_ANDROID_ARM64_ONLY=1
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
OUT_DIR="$WORKSPACE_DIR/build/android-native-ndk"

mkdir -p "$OUT_DIR"
cd "$WORKSPACE_DIR"

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "ERROR: cargo-ndk not found.  cargo install cargo-ndk"
  exit 1
fi

if [[ -z "${ANDROID_NDK_HOME:-}" && -z "${NDK_HOME:-}" ]]; then
  echo "ERROR: set ANDROID_NDK_HOME or NDK_HOME to your Android NDK path."
  exit 1
fi

verify_android_lib() {
  local so="$1"
  if [[ ! -f "$so" ]]; then
    echo "ERROR: missing $so"
    exit 1
  fi
  if strings "$so" 2>/dev/null | rg -q 'project_dirs_from_application_id'; then
    echo "ERROR: $so still contains legacy storage path code (project_dirs_from_application_id)."
    exit 1
  fi
}

build_abi() {
  local abi="$1"
  local triple="$2"
  echo ""
  echo "==> [$abi] rustup target: $triple (release --lib)"
  rustup target add "$triple" 2>/dev/null || true
  cargo ndk -t "$abi" -o "$OUT_DIR" build -p ghal_bol --release --lib
  verify_android_lib "$OUT_DIR/$abi/libghal_bol.so"
  echo "==> [$abi] OK: $OUT_DIR/$abi/libghal_bol.so"
}

echo "==> Output: $OUT_DIR"

if [[ "${PACK_ANDROID_ARM64_ONLY:-}" == "1" ]]; then
  echo "==> PACK_ANDROID_ARM64_ONLY=1 — building arm64-v8a only"
  build_abi "arm64-v8a" "aarch64-linux-android"
else
  echo "==> Building all Android ABIs (armeabi-v7a, arm64-v8a, x86, x86_64)"
  build_abi "armeabi-v7a" "armv7-linux-androideabi"
  build_abi "arm64-v8a" "aarch64-linux-android"
  build_abi "x86" "i686-linux-android"
  build_abi "x86_64" "x86_64-linux-android"
fi

echo ""
echo "OK. Run: cd ghal_bol_ui && flutter run"
