#!/usr/bin/env bash
# Cross-build a static libopus.a for each Android ABI using the NDK + CMake.
#
# audiopus_sys (pulled by the `audiopus` crate) builds Opus with autotools and
# does NOT pass `--host`, so under cargo-ndk it would compile Opus for the build
# host (wrong arch → "incompatible with aarch64linux" link error). Instead we
# build Opus ourselves here and let audiopus_sys link the prebuilt static lib via
# LIBOPUS_LIB_DIR / LIBOPUS_STATIC / LIBOPUS_NO_PKG (see pack_android_workspace_jni_libs.sh).
#
# Output: build/android-opus/<abi>/lib/libopus.a
#
# Env:
#   ANDROID_NDK_HOME or NDK_HOME   path to the Android NDK (required)
#   OPUS_VERSION                   Opus release to build (default 1.5.2)
#   ANDROID_API                    min platform level (default 21)
# Default: all four standard Android ABIs (armeabi-v7a, arm64-v8a, x86, x86_64).
# Dev fast path: PACK_ANDROID_ARM64_ONLY=1
set -euo pipefail

# cmake/ninja/clang can leave a dangling SGR color on the TTY; restore on exit.
trap '[ -t 1 ] && printf "\033[0m"' EXIT

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(dirname "$SCRIPT_DIR")"
OPUS_VERSION="${OPUS_VERSION:-1.5.2}"
ANDROID_API="${ANDROID_API:-21}"
OUT_DIR="$WORKSPACE_DIR/build/android-opus"
SRC_DIR="$OUT_DIR/src"
TARBALL="$SRC_DIR/opus-$OPUS_VERSION.tar.gz"
OPUS_SRC="$SRC_DIR/opus-$OPUS_VERSION"

NDK="${ANDROID_NDK_HOME:-${NDK_HOME:-}}"
if [[ -z "$NDK" ]]; then
  echo "ERROR: set ANDROID_NDK_HOME or NDK_HOME to your Android NDK path."
  exit 1
fi
TOOLCHAIN="$NDK/build/cmake/android.toolchain.cmake"
if [[ ! -f "$TOOLCHAIN" ]]; then
  echo "ERROR: NDK CMake toolchain not found: $TOOLCHAIN"
  exit 1
fi
for tool in cmake; do
  command -v "$tool" >/dev/null 2>&1 || { echo "ERROR: $tool not found."; exit 1; }
done
GENERATOR=()
if command -v ninja >/dev/null 2>&1; then
  GENERATOR=(-G Ninja)
fi

mkdir -p "$SRC_DIR"

if [[ ! -d "$OPUS_SRC" ]]; then
  if [[ ! -f "$TARBALL" ]]; then
    echo "==> Downloading Opus $OPUS_VERSION"
    URL="https://github.com/xiph/opus/releases/download/v$OPUS_VERSION/opus-$OPUS_VERSION.tar.gz"
    if command -v curl >/dev/null 2>&1; then
      curl -fL "$URL" -o "$TARBALL"
    else
      wget -O "$TARBALL" "$URL"
    fi
  fi
  echo "==> Extracting Opus $OPUS_VERSION"
  tar --no-same-owner -xzf "$TARBALL" -C "$SRC_DIR"
fi

build_abi() {
  local abi="$1"
  local build="$OUT_DIR/build/$abi"
  local prefix="$OUT_DIR/$abi"
  echo ""
  echo "==> [$abi] CMake configure Opus (static, PIC, android-$ANDROID_API)"
  rm -rf "$build"
  cmake "${GENERATOR[@]}" \
    -DCMAKE_TOOLCHAIN_FILE="$TOOLCHAIN" \
    -DANDROID_ABI="$abi" \
    -DANDROID_PLATFORM="android-$ANDROID_API" \
    -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=OFF \
    -DOPUS_BUILD_SHARED_LIBRARY=OFF \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DCMAKE_INSTALL_PREFIX="$prefix" \
    -DOPUS_BUILD_PROGRAMS=OFF \
    -DOPUS_BUILD_TESTING=OFF \
    -S "$OPUS_SRC" -B "$build"
  cmake --build "$build" --target install --config Release
  if [[ ! -f "$prefix/lib/libopus.a" ]]; then
    echo "ERROR: [$abi] missing $prefix/lib/libopus.a after build."
    exit 1
  fi
  echo "==> [$abi] OK: $prefix/lib/libopus.a"
}

if [[ "${PACK_ANDROID_ARM64_ONLY:-}" == "1" ]]; then
  echo "==> PACK_ANDROID_ARM64_ONLY=1 — building arm64-v8a only"
  build_abi "arm64-v8a"
else
  echo "==> Building Opus for all Android ABIs"
  build_abi "armeabi-v7a"
  build_abi "arm64-v8a"
  build_abi "x86"
  build_abi "x86_64"
fi

echo ""
echo "OK. libopus.a per ABI under: $OUT_DIR/<abi>/lib"
