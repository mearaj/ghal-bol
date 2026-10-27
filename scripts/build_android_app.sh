#!/usr/bin/env bash
# Android install+run. **Default is DEBUG** (apps/ghalbol/env/development.env → home coord1 bridge).
# Production GCP is opt-in via --release only — bare runs must not overwrite debug with GCP.
#
#   ./scripts/build_android_app.sh           # debug   → development.env
#   ./scripts/build_android_app.sh --debug   # same
#   ./scripts/build_android_app.sh --release # release → production.env
#   ./scripts/build_android_app.sh --aab     # Play App Bundle (release → production.env)
#
# apps/ghalbol/build.rs bakes the env file named by GHALBOL_ENV. This script sets it from the
# flag unless the caller already exported it (cargo ghalbol-android-dev exports development).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REV="1f60033ed3b1158cb7bf93accf1c91e0166c9373"
# cargo-makepad defaults to CARGO_MANIFEST_DIR (baked at install time). Always
# pass --sdk-path so a path-install from another machine cannot poison the NDK.
# Prefer a cache dir we own (some machines have a root-owned ~/.makepad).
SDK_DIR="${GHALBOL_MAKEPAD_ANDROID_SDK:-$HOME/.cache/ghalbol/makepad-android_33_linux_x64}"
NDK_DIR="$SDK_DIR/ndk/28.2.13676358"
MODE="run"
EXTRA=( )
case "${1:-}" in
  ""|--debug)
    MODE="run"
    EXTRA=( )
    ;;
  --release)
    MODE="run"
    EXTRA=(--release)
    ;;
  --aab|--apk)
    MODE="build-aab"
    EXTRA=(--release)
    if [[ "${1}" == "--apk" ]]; then
      MODE="build"
    fi
    ;;
  *)
    echo "usage: $0 [--debug|--release|--aab]" >&2
    exit 1
    ;;
esac
if [[ -z "${GHALBOL_ENV:-}" ]]; then
  if [[ "${#EXTRA[@]}" -eq 0 ]]; then
    export GHALBOL_ENV=development
  else
    export GHALBOL_ENV=production
  fi
fi

if ! command -v cargo-makepad >/dev/null 2>&1; then
  cargo install --git https://github.com/makepad/makepad --rev "$REV" cargo-makepad --locked
fi

# Stripped (default) NDK lacks cmake toolchain files; aws-lc-sys needs --full-ndk.
if [[ ! -f "$NDK_DIR/source.properties" || ! -f "$NDK_DIR/build/cmake/android.toolchain.cmake" ]]; then
  echo "Makepad full Android NDK missing under $SDK_DIR — running install-toolchain --full-ndk (once)." >&2
  mkdir -p "$(dirname "$SDK_DIR")"
  rm -rf "$SDK_DIR/ndk"
  cargo makepad android --sdk-path="$SDK_DIR" install-toolchain --full-ndk
fi

# audiopus_sys cannot cross-compile Opus; prebuild static libopus.a for arm64.
OPUS_LIB="$ROOT/build/android-opus/arm64-v8a/lib"
if [[ ! -f "$OPUS_LIB/libopus.a" ]]; then
  echo "Building static libopus.a for Android arm64-v8a…" >&2
  ANDROID_NDK_HOME="$NDK_DIR" PACK_ANDROID_ARM64_ONLY=1 "$ROOT/scripts/build_android_opus.sh"
fi

# Do NOT source android-ndk-env.sh here — it points CC_* at Studio's NDK and
# fights cargo-makepad's ANDROID_NDK_ROOT / clang (breaks aws-lc-sys cmake).
unset ANDROID_NDK_HOME ANDROID_NDK_ROOT ANDROID_NDK_TOOLCHAIN_BIN ANDROID_NDK_HOST_TAG || true
unset CC_aarch64_linux_android CXX_aarch64_linux_android \
  CC_armv7_linux_androideabi CXX_armv7_linux_androideabi \
  CC_i686_linux_android CXX_i686_linux_android \
  CC_x86_64_linux_android CXX_x86_64_linux_android \
  CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER \
  CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER \
  CARGO_TARGET_I686_LINUX_ANDROID_LINKER \
  CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER || true

# Point audiopus_sys at the prebuilt static Opus (host-arch .so would fail link).
export LIBOPUS_LIB_DIR="$OPUS_LIB"
export LIBOPUS_STATIC=1
export LIBOPUS_NO_PKG=1
export OPUS_LIB_DIR="$OPUS_LIB"
export OPUS_STATIC=1

cd "$ROOT"
# Agent shells may set CARGO_TARGET_DIR to a sandbox cache; keep the repo target/.
unset CARGO_TARGET_DIR || true

echo "Android build: ${EXTRA[*]:-debug} · env=$GHALBOL_ENV (apps/ghalbol/env/$GHALBOL_ENV.env)." >&2
if [[ "$GHALBOL_ENV" == "production" ]]; then
  echo "WAN call bridge for home testing needs the development env (coord1)." >&2
fi

# Default mode is install+run. If nothing is plugged in, build the APK only —
# do not fail the whole script after a successful compile.
ADB_BIN="$SDK_DIR/platform-tools/adb"
if [[ "$MODE" == "run" ]]; then
  has_device=0
  if [[ -x "$ADB_BIN" ]] && "$ADB_BIN" devices 2>/dev/null | awk 'NR>1 && $2=="device" { found=1 } END { exit !found }'; then
    has_device=1
  fi
  if [[ "$has_device" -eq 0 ]]; then
    echo "No Android device/emulator connected — building APK only (skipping adb install)." >&2
    MODE="build"
  fi
fi

cargo makepad android --sdk-path="$SDK_DIR" "$MODE" -p ghalbol_app "${EXTRA[@]}"
if [[ "$MODE" == "build" || "$MODE" == "build-aab" ]]; then
  echo "Finished. Artifacts under target/makepad-android-apk/."
fi
