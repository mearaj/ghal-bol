#!/usr/bin/env bash
# Verify Rust DaemonMethod::ALL matches Dart DaemonMethod.all (integrator SDK parity).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

rust_all=$(python3 <<'PY'
import re
text = open("ghal_bol/src/daemon/client_api.rs").read()
block = re.search(r"pub const ALL.*?&\[(.*?)\];", text, re.S)
items = re.findall(r"Self::(\w+)", block.group(1) if block else "")
print(len(items))
PY
)
dart_count=$(python3 <<'PY'
import re
text = open("ghal_bol_ui/lib/daemon_client_api.dart").read()
block = re.search(r"static const all = <String>\[(.*?)\];", text, re.S)
names = re.findall(r"(\w+),", block.group(1) if block else "")
print(len(names))
PY
)

if [[ "$rust_all" != "$dart_count" ]]; then
  echo "SDK parity FAILED: Rust ALL=$rust_all Dart all=$dart_count" >&2
  exit 1
fi

echo "SDK parity OK: $rust_all daemon RPC methods (Rust + Dart)"

cd ghal_bol_ui && flutter test test/daemon_client_api_test.dart
cd "$root" && cargo test -p ghal_bol daemon:: --quiet
