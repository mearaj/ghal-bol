#!/usr/bin/env bash
# Build libghal_bol.so for Android → workspace/build/android-native-ndk/
# Gradle: android/app/build.gradle.kts jniLibs.srcDirs. Does not use adb.
#
# Default: all four standard Android ABIs (required for Play, emulators, and 32-bit ARM devices):
#   armeabi-v7a, arm64-v8a, x86, x86_64
# Dev fast path (one phone, arm64 only): PACK_ANDROID_ARM64_ONLY=1
#
set -euo pipefail

# Some sub-tools (cargo/cmake/ninja/clang progress + diagnostics) can leave a
# dangling SGR attribute on the TTY, so the shell stays dim/gray afterwards.
# Always restore default colors on exit when stdout is a real terminal.
trap '[ -t 1 ] && printf "\033[0m"' EXIT

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
  # :p2p JNI entry points must be exported on every ABI we ship.
  local jni_count
  jni_count=$(nm -D "$so" 2>/dev/null | rg -c 'Java_com_ghalbol' || true)
  if [[ "${jni_count:-0}" -lt 6 ]]; then
    echo "ERROR: $so missing P2pDaemonNative JNI exports (found ${jni_count:-0}, need >= 6)."
    exit 1
  fi
}

# cargo-ndk may leave hashed helper .so files from prior builds; only ship our libs.
prune_jni_abi_dir() {
  local abi="$1"
  local dir="$OUT_DIR/$abi"
  [[ -d "$dir" ]] || return 0
  find "$dir" -maxdepth 1 -type f -name '*.so' \
    ! -name 'libghal_bol.so' ! -name 'libc++_shared.so' -delete 2>/dev/null || true
}

# NDK sysroot lib triple (differs from the Rust triple for 32-bit ARM).
ndk_lib_triple() {
  case "$1" in
    armeabi-v7a) echo "arm-linux-androideabi" ;;
    arm64-v8a) echo "aarch64-linux-android" ;;
    x86) echo "i686-linux-android" ;;
    x86_64) echo "x86_64-linux-android" ;;
    *) echo "" ;;
  esac
}

# libghal_bol.so now needs libc++_shared.so (Oboe/C++ for native voice). The
# Flutter app does not otherwise ship it (flutter_webrtc static-links its own
# libc++), so dlopen of our lib fails with "libc++_shared.so not found" in BOTH
# the UI and `:p2p` processes. Bundle the NDK's copy into jniLibs per ABI.
copy_libcxx_shared() {
  local abi="$1"
  local ndk="${ANDROID_NDK_HOME:-${NDK_HOME:-}}"
  local llvm
  llvm=$(echo "$ndk"/toolchains/llvm/prebuilt/*)
  local t
  t="$(ndk_lib_triple "$abi")"
  local src="$llvm/sysroot/usr/lib/$t/libc++_shared.so"
  if [[ ! -f "$src" ]]; then
    echo "ERROR: [$abi] libc++_shared.so not found at $src"
    exit 1
  fi
  cp -f "$src" "$OUT_DIR/$abi/libc++_shared.so"
  echo "==> [$abi] bundled libc++_shared.so"
}

build_abi() {
  local abi="$1"
  local triple="$2"
  local opus_lib="$WORKSPACE_DIR/build/android-opus/$abi/lib"
  echo ""
  echo "==> [$abi] rustup target: $triple (release --lib)"
  rustup target add "$triple" 2>/dev/null || true
  if [[ ! -f "$opus_lib/libopus.a" ]]; then
    echo "ERROR: missing prebuilt $opus_lib/libopus.a — run scripts/build_android_opus.sh first."
    exit 1
  fi
  # OpenH264 x86 NASM (.asm) objects use R_386_32 relocations that cannot link
  # into a PIC shared lib (libghal_bol.so) on Android x86/x86_64 emulators.
  # Fall back to C-only via OPENH264_NO_ASM (see openh264-sys2 build.rs). ARM ABIs
  # keep NASM/NEON asm — real devices build fine.
  local openh264_no_asm=()
  if [[ "$abi" == "x86" || "$abi" == "x86_64" ]]; then
    echo "==> [$abi] OPENH264_NO_ASM=1 (emulator ABI — NASM not PIC-safe for .so)"
    openh264_no_asm=(OPENH264_NO_ASM=1)
    rm -rf "$WORKSPACE_DIR/target/$triple"/release/build/openh264-sys2-* 2>/dev/null || true
  fi
  # audiopus_sys can't cross-compile Opus itself; link our NDK-built static lib (P6 native voice).
  prune_jni_abi_dir "$abi"
  env "${openh264_no_asm[@]}" LIBOPUS_NO_PKG=1 LIBOPUS_STATIC=1 LIBOPUS_LIB_DIR="$opus_lib" \
    cargo ndk -t "$abi" -o "$OUT_DIR" build -p ghal_bol --release --lib
  prune_jni_abi_dir "$abi"
  verify_android_lib "$OUT_DIR/$abi/libghal_bol.so"
  copy_libcxx_shared "$abi"
  prune_jni_abi_dir "$abi"
  echo "==> [$abi] OK: $OUT_DIR/$abi/libghal_bol.so"
}

# All ABIs Gradle/jniLibs expect when not using PACK_ANDROID_ARM64_ONLY.
ANDROID_ABIS_ALL=(armeabi-v7a arm64-v8a x86 x86_64)

verify_android_pack_complete() {
  local -a abis=()
  if [[ "${PACK_ANDROID_ARM64_ONLY:-}" == "1" ]]; then
    abis=(arm64-v8a)
  else
    abis=("${ANDROID_ABIS_ALL[@]}")
  fi
  local missing=0
  for abi in "${abis[@]}"; do
    for lib in libghal_bol.so libc++_shared.so; do
      if [[ ! -f "$OUT_DIR/$abi/$lib" ]]; then
        echo "ERROR: missing $OUT_DIR/$abi/$lib"
        missing=1
      fi
    done
  done
  if (( missing != 0 )); then
    exit 1
  fi
  echo ""
  echo "OK. Packaged ${#abis[@]} Android ABI(s): ${abis[*]}"
  echo "    $OUT_DIR/<abi>/libghal_bol.so + libc++_shared.so"
}

# Native voice (P6) needs a static libopus.a per ABI built with the NDK.
echo "==> Building Opus for Android (static, per ABI)"
"$SCRIPT_DIR/build_android_opus.sh"

# audiopus_sys (0.1.8) does NOT emit `rerun-if-env-changed`, so a previously cached
# build (e.g. one that compiled its own host libopus.so, or a direct `cargo ndk`
# run without our LIBOPUS_* env) gets reused and ignores the prebuilt static lib →
# "libopus.so is incompatible with <android arch>". Force-remove its artifacts for
# every Android target so build.rs MUST re-run with the env set below. `cargo clean
# -p` alone is unreliable for build-script `out` dirs, so we rm them directly too.
echo "==> Clearing stale audiopus_sys artifacts (force re-link of prebuilt opus)"
cargo clean -p audiopus_sys 2>/dev/null || true
for triple in armv7-linux-androideabi aarch64-linux-android i686-linux-android x86_64-linux-android; do
  rm -rf \
    "$WORKSPACE_DIR/target/$triple"/*/build/audiopus_sys-* \
    "$WORKSPACE_DIR/target/$triple"/*/deps/*audiopus_sys* 2>/dev/null || true
  # x86 OpenH264 may have been built with NASM before OPENH264_NO_ASM was set.
  case "$triple" in
    i686-linux-android|x86_64-linux-android)
      rm -rf "$WORKSPACE_DIR/target/$triple"/*/build/openh264-sys2-* 2>/dev/null || true
      ;;
  esac
done

echo "==> Output: $OUT_DIR"

if [[ "${PACK_ANDROID_ARM64_ONLY:-}" == "1" ]]; then
  echo "==> PACK_ANDROID_ARM64_ONLY=1 — building arm64-v8a only"
  build_abi "arm64-v8a" "aarch64-linux-android"
  verify_android_pack_complete
else
  echo "==> Building all Android ABIs (armeabi-v7a, arm64-v8a, x86, x86_64)"
  build_abi "armeabi-v7a" "armv7-linux-androideabi"
  build_abi "arm64-v8a" "aarch64-linux-android"
  build_abi "x86" "i686-linux-android"
  build_abi "x86_64" "x86_64-linux-android"
  verify_android_pack_complete
fi

echo ""
echo "Run: cd ghal_bol_ui && flutter run"
