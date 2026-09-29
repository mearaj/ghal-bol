#!/usr/bin/env bash
# Compatibility wrapper — Android packages are built with cargo-makepad.
set -euo pipefail
echo "Android packages are built with ./scripts/build_android_app.sh"
exec "$(cd "$(dirname "$0")" && pwd)/build_android_app.sh" "$@"
