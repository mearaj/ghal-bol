#!/usr/bin/env bash
# Build the Makepad Android package for ghal_bol_app (APK, or AAB with --aab).
# Same UI as the desktop app. Requires the Android SDK/NDK and cargo-makepad
# from the pinned Makepad revision.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REV="1f60033ed3b1158cb7bf93accf1c91e0166c9373"
MODE="build"
if [[ "${1:-}" == "--aab" ]]; then
  MODE="build-aab"
fi
if ! command -v cargo-makepad >/dev/null 2>&1; then
  cargo install --git https://github.com/makepad/makepad --rev "$REV" cargo-makepad --locked
fi
if [[ -f "$ROOT/scripts/android-ndk-env.sh" ]]; then
  # shellcheck disable=SC1091
  source "$ROOT/scripts/android-ndk-env.sh" || true
fi
cd "$ROOT"
cargo makepad android "$MODE" -p ghal_bol_app --release
echo "Android $MODE finished. APK/AAB is under target/makepad-android-apk/."
