#!/usr/bin/env bash
# Load Android cross-build environment into the **current shell**:
#     source scripts/android-ndk-env.sh
#
# Designed for newcomers: if ANDROID_NDK_HOME / ANDROID_NDK_ROOT / SDK vars are unset,
# the script picks a recent NDK from usual install locations (Android Studio, env SDK).
#
# Overrides (optional): ANDROID_NDK_HOST_TAG, GHAL_BOL_ANDROID_API_LEVEL (default 21).
# Silence status lines: GHAL_BOL_ANDROID_NDK_ENV_QUIET=1
#
# One-shot without sourcing (CI / scripts):  scripts/with-android-env.sh cargo check …

set -euo pipefail

_ghalbol_log() {
  [[ -n "${GHAL_BOL_ANDROID_NDK_ENV_QUIET:-}" ]] && return 0
  printf '%s\n' "$*" >&2
}

is_ndk_root() {
  [[ -n "$1" && -d "$1/toolchains/llvm/prebuilt" ]]
}

ndk_version_dirs() {
  local parent="$1"
  [[ -d "$parent" ]] || return 1
  find "$parent" -mindepth 1 -maxdepth 1 -type d 2>/dev/null \
    | sed 's|.*/||' \
    | grep -E '^[[:digit:]]+\.[[:digit:]]+' \
    | sort -V
}

pick_latest_under() {
  local parent="$1"
  ndk_version_dirs "$parent" | tail -n 1
}

resolve_android_ndk_home() {
  local r ver parent seen="|"

  for r in "${ANDROID_NDK_HOME:-}" "${ANDROID_NDK_ROOT:-}"; do
    [[ -z "$r" ]] && continue
    [[ "$seen" == *"|${r}|"* ]] && continue
    seen+="${r}|"
    if is_ndk_root "$r"; then
      printf '%s\n' "$r"
      return 0
    fi
  done

  for parent in "${ANDROID_SDK_ROOT:-}/ndk" "${ANDROID_HOME:-}/ndk" "${HOME}/Android/Sdk/ndk"; do
    [[ "$parent" == "/ndk" || ! -d "$parent" ]] && continue
    ver="$(pick_latest_under "$parent")"
    [[ -z "$ver" ]] && continue
    r="${parent%/}/${ver}"
    [[ "$seen" == *"|${r}|"* ]] && continue
    seen+="${r}|"
    if is_ndk_root "$r"; then
      printf '%s\n' "$r"
      return 0
    fi
  done

  return 1
}

default_host_tag_candidates() {
  local os m
  os="$(uname -s)"
  m="$(uname -m)"
  case "${os}/${m}" in
    Linux/x86_64 | Linux/amd64) printf '%s\n' linux-x86_64 ;;
    Linux/aarch64 | Linux/arm64)
      printf '%s\n' linux-aarch64
      printf '%s\n' linux-x86_64
      ;;
    Darwin/arm64 | Darwin/aarch64) printf '%s\n' darwin-arm64 ;;
    Darwin/x86_64 | Darwin/i386) printf '%s\n' darwin-x86_64 ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT*)
      printf '%s\n' windows-x86_64
      ;;
    *)
      printf '%s\n' linux-x86_64
      ;;
  esac
  printf '%s\n' linux-x86_64 darwin-arm64 darwin-x86_64 windows-x86_64
}

pick_prebuilt_host_tag() {
  local ndk="$1" api="$2" tag bin seen="|"
  local -a tries=()

  if [[ -n "${ANDROID_NDK_HOST_TAG:-}" ]]; then
    tries+=("$ANDROID_NDK_HOST_TAG")
  fi
  local t
  while IFS= read -r t; do
    tries+=("$t")
  done < <(default_host_tag_candidates)

  for tag in "${tries[@]}"; do
    [[ -z "$tag" ]] && continue
    [[ "$seen" == *"|${tag}|"* ]] && continue
    seen+="${tag}|"
    bin="${ndk}/toolchains/llvm/prebuilt/${tag}/bin"
    if [[ -x "${bin}/aarch64-linux-android${api}-clang" ]]; then
      printf '%s\n' "$tag"
      return 0
    fi
  done
  return 1
}

_ghalbol_fail_help() {
  cat >&2 <<'EOF'
android-ndk-env: could not configure the Android NDK LLVM toolchain for Rust.

Try one of these:

  1) Android Studio → Settings → Languages & Frameworks → Android SDK → SDK Tools
     → enable "NDK (Side by side)" → apply. A typical path is:
        ~/Android/Sdk/ndk/<version>

  2) Point the script at your SDK (Studio writes sdk.dir in local.properties):
        export ANDROID_SDK_ROOT="$HOME/Android/Sdk"
        source scripts/android-ndk-env.sh

     Or set the NDK root directly:
        export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/<version>"
        source scripts/android-ndk-env.sh

  3) Install Rust Android targets (once per machine):
        rustup target add aarch64-linux-android armv7-linux-androideabi \
                        i686-linux-android x86_64-linux-android

  4) Prefer cargo-ndk for APK jniLibs (it discovers the NDK too); see:
        ghal_bol_ui/README.md   # Flutter shell; optional JNI notes

One-shot (no sourcing):  scripts/with-android-env.sh cargo check -p ghal_bol_core --target aarch64-linux-android
EOF
}

API="${GHAL_BOL_ANDROID_API_LEVEL:-21}"

NDK="$(resolve_android_ndk_home || true)"
if ! is_ndk_root "$NDK"; then
  _ghalbol_fail_help
  return 1 2>/dev/null || exit 1
fi

export ANDROID_NDK_HOME="$NDK"

HOST_TAG="$(pick_prebuilt_host_tag "$NDK" "$API" || true)"
if [[ -z "${HOST_TAG:-}" ]]; then
  _ghalbol_log "android-ndk-env: no usable prebuilt/ host tag under: ${NDK}/toolchains/llvm/prebuilt/"
  if [[ -d "${NDK}/toolchains/llvm/prebuilt" ]]; then
    _ghalbol_log "  Found:" "$(ls -1 "${NDK}/toolchains/llvm/prebuilt" 2>/dev/null | tr '\n' ' ')"
  fi
  _ghalbol_fail_help
  return 1 2>/dev/null || exit 1
fi

export ANDROID_NDK_HOST_TAG="$HOST_TAG"

BIN="${NDK}/toolchains/llvm/prebuilt/${HOST_TAG}/bin"
if [[ ! -x "${BIN}/aarch64-linux-android${API}-clang" ]]; then
  _ghalbol_log "android-ndk-env: missing ${BIN}/aarch64-linux-android${API}-clang"
  _ghalbol_fail_help
  return 1 2>/dev/null || exit 1
fi

export ANDROID_NDK_TOOLCHAIN_BIN="${BIN}"

_ghalbol_log "android-ndk-env: ANDROID_NDK_HOME=${NDK}"
_ghalbol_log "android-ndk-env: using ${HOST_TAG} LLVM (API ${API})"

export CC_aarch64_linux_android="${BIN}/aarch64-linux-android${API}-clang"
export CXX_aarch64_linux_android="${BIN}/aarch64-linux-android${API}-clang++"
export CC_armv7_linux_androideabi="${BIN}/armv7a-linux-androideabi${API}-clang"
export CXX_armv7_linux_androideabi="${BIN}/armv7a-linux-androideabi${API}-clang++"
export CC_i686_linux_android="${BIN}/i686-linux-android${API}-clang"
export CXX_i686_linux_android="${BIN}/i686-linux-android${API}-clang++"
export CC_x86_64_linux_android="${BIN}/x86_64-linux-android${API}-clang"
export CXX_x86_64_linux_android="${BIN}/x86_64-linux-android${API}-clang++"

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${CC_aarch64_linux_android}"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="${CC_armv7_linux_androideabi}"
export CARGO_TARGET_I686_LINUX_ANDROID_LINKER="${CC_i686_linux_android}"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="${CC_x86_64_linux_android}"
