#!/usr/bin/env bash
# Run a command with the same Android NDK env as `source scripts/android-ndk-env.sh`.
# Example:
#   scripts/with-android-env.sh cargo check -p ghal_bol_core --target aarch64-linux-android

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "${ROOT}/scripts/android-ndk-env.sh"
exec "$@"
