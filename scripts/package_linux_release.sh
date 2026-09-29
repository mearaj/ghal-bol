#!/usr/bin/env bash
# Release binary of ghal_bol_app, packed for https://ghalbol.com/download/linux.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p ghal_bol_app --release
STAGE="$(mktemp -d)"
mkdir -p "$STAGE/ghal-bol" "$ROOT/web/downloads"
cp "$ROOT/target/release/ghal_bol_app" "$STAGE/ghal-bol/ghal_bol_app"
cat > "$STAGE/ghal-bol/run.sh" << 'EOF'
#!/bin/sh
cd "$(dirname "$0")"
exec ./ghal_bol_app "$@"
EOF
chmod +x "$STAGE/ghal-bol/run.sh" "$STAGE/ghal-bol/ghal_bol_app"
tar -C "$STAGE" -czf "$ROOT/web/downloads/ghal-bol-linux-x64.tar.gz" ghal-bol
rm -rf "$STAGE"
echo "Wrote web/downloads/ghal-bol-linux-x64.tar.gz"
