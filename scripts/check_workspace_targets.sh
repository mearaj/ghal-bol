#!/usr/bin/env bash
# Cross-check the whole workspace for common host and cross targets.
#
# Installation:
#   rustup target add <triple>
# Android ABI checks need the Android NDK on PATH via cargo-ndk:
#   cargo install cargo-ndk
#   export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/<version>"   # or NDK_HOME
# Optional Gradle **jniLibs**: **./scripts/pack_android_workspace_jni_libs.sh** (all workspace **`cdylib`** **`.so`** files) — **PORTABILITY.md**.
#
# Darwin / MSVC targets typically require their host SDKs even for `cargo check`.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGETS=(
  x86_64-unknown-linux-gnu
  x86_64-pc-windows-gnu
  x86_64-apple-darwin
  aarch64-apple-darwin
  aarch64-unknown-linux-gnu
  aarch64-linux-android
  armv7-linux-androideabi
  i686-linux-android
  x86_64-linux-android
  aarch64-apple-ios
  x86_64-apple-ios
  wasm32-unknown-unknown
  wasm32-wasip1
  wasm32-wasip2
)

installed() {
  rustup target list --installed 2>/dev/null | grep -Fxq "$1"
}

is_apple_ios_triple() {
  case "$1" in
    aarch64-apple-ios | x86_64-apple-ios) return 0 ;;
    *) return 1 ;;
  esac
}

# Rust Android triple → `cargo-ndk -t` ABI name.
android_triple_to_ndk_abi() {
  case "$1" in
    aarch64-linux-android) echo arm64-v8a ;;
    armv7-linux-androideabi) echo armeabi-v7a ;;
    i686-linux-android) echo x86 ;;
    x86_64-linux-android) echo x86_64 ;;
    *) return 1 ;;
  esac
}

check_one_target() {
  local triple="$1"
  local abi
  if abi=$(android_triple_to_ndk_abi "$triple" 2>/dev/null); then
    if ! command -v cargo-ndk >/dev/null 2>&1; then
      echo >&2 ""
      echo >&2 "ERROR: ${triple}: Android target is installed but 'cargo-ndk' is not on PATH."
      echo >&2 "  Install with: cargo install cargo-ndk"
      echo >&2 "  Export NDK path: ANDROID_NDK_HOME (or NDK_HOME) must point at the NDK root."
      return 1
    fi
    echo "-- target: ${triple} (cargo-ndk ${abi}) --"
    cargo ndk -t "${abi}" check --workspace --all-targets
    return 0
  fi

  echo "-- target: ${triple} --"
  cargo check --workspace --all-targets --target "${triple}"
}

main() {
  local failed=0

  echo "== host (default rustc target) =="
  cargo check --workspace --all-targets

  echo ""
  echo "== cross targets ==="
  for t in "${TARGETS[@]}"; do
    if ! installed "${t}"; then
      echo "skip (not installed): ${t}  —  rustup target add ${t}"
      continue
    fi
    if is_apple_ios_triple "${t}"; then
      if [[ "$(uname -s)" != "Darwin" ]]; then
        echo "skip (iOS needs Apple host + Xcode): ${t}"
        continue
      fi
      if ! command -v xcrun >/dev/null 2>&1; then
        echo "skip (no xcrun / Xcode): ${t}"
        continue
      fi
    fi
    echo ""
    if ! ( check_one_target "${t}" ); then
      failed=1
    fi
  done

  echo ""
  if [[ "${failed}" -ne 0 ]]; then
    echo "== DONE (with failures) =="
    exit 1
  fi
  echo "== DONE =="
}

main "$@"
