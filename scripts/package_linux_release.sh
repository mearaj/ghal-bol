#!/usr/bin/env bash
# Release binary of ghalbol_app, packed for https://ghalbol.com/download/linux.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p ghalbol_app --release
STAGE="$(mktemp -d)"
mkdir -p "$STAGE/ghalbol" "$ROOT/web/downloads"
cp "$ROOT/target/release/ghalbol_app" "$STAGE/ghalbol/ghalbol_app"
cat > "$STAGE/ghalbol/run.sh" << 'EOF'
#!/bin/sh
cd "$(dirname "$0")"
exec ./ghalbol_app "$@"
EOF
chmod +x "$STAGE/ghalbol/run.sh" "$STAGE/ghalbol/ghalbol_app"
tar -C "$STAGE" -czf "$ROOT/web/downloads/ghalbol-linux-x64.tar.gz" ghalbol
rm -rf "$STAGE"
echo "Wrote web/downloads/ghalbol-linux-x64.tar.gz"
