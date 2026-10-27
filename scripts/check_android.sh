#!/usr/bin/env bash
# Type-check the Android build of ghalbol_app without the Android SDK.
#
# cargo-makepad compiles the app's src/main.rs as a cdylib library through a generated
# wrapper crate (`cargo rustc --lib --crate-type=cdylib`). A plain `cargo check` of the
# bin target always fails on Android with "`main` function not found", so this script
# builds the same kind of wrapper and checks it. Only the NDK is needed (C deps).
#
# Usage: ./scripts/check_android.sh [target-triple]   (default aarch64-linux-android)
# Extra cargo flags via CARGO_CHECK_FLAGS, e.g. CARGO_CHECK_FLAGS="--release".
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
TARGET="${1:-aarch64-linux-android}"
APP="$ROOT/apps/ghalbol"
WRAP="$ROOT/target/android-check-wrapper"

mkdir -p "$WRAP"
{
  sed -e "s#path = \"../../ghalbol_core\"#path = \"$ROOT/ghalbol_core\"#" \
      -e "s#^build = \"build.rs\"#build = \"$APP/build.rs\"#" \
      "$APP/Cargo.toml"
  cat <<EOF

[lib]
path = "$APP/src/main.rs"
crate-type = ["cdylib"]

[workspace]
EOF
} >"$WRAP/Cargo.toml"
cp -f "$ROOT/Cargo.lock" "$WRAP/Cargo.lock"

GHALBOL_ANDROID_NDK_ENV_QUIET=1 source "$ROOT/scripts/android-ndk-env.sh"

# Link the same prebuilt static Opus the real Android build uses (see build_android_app.sh).
if [[ "$TARGET" == "aarch64-linux-android" ]]; then
  OPUS_LIB="$ROOT/build/android-opus/arm64-v8a/lib"
  if [[ ! -f "$OPUS_LIB/libopus.a" ]]; then
    PACK_ANDROID_ARM64_ONLY=1 "$ROOT/scripts/build_android_opus.sh"
  fi
  export LIBOPUS_LIB_DIR="$OPUS_LIB" LIBOPUS_STATIC=1 LIBOPUS_NO_PKG=1
fi
# shellcheck disable=SC2086
cargo check --manifest-path "$WRAP/Cargo.toml" --lib --target "$TARGET" \
  --target-dir "$ROOT/target" ${CARGO_CHECK_FLAGS:-}
cargo check -p ghalbol_core --target "$TARGET" ${CARGO_CHECK_FLAGS:-}
