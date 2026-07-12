#!/usr/bin/env bash
# Verify Rust DaemonMethod::ALL matches Dart DaemonMethod.all (integrator SDK parity).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

rust_all=$(python3 <<'PY'
import re
text = open("ghal_bol_core/src/daemon/client_api.rs").read()
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

# flutter test bundles pubspec assets; env/*.env.* are gitignored (see ghal_bol_ui/env/README.md).
if [[ ! -f ghal_bol_ui/env/.env.development ]]; then
  cp ghal_bol_ui/env/.env.development.example ghal_bol_ui/env/.env.development
fi
if [[ ! -f ghal_bol_ui/env/.env.production ]]; then
  cp ghal_bol_ui/env/.env.production.example ghal_bol_ui/env/.env.production
fi

cd ghal_bol_ui && flutter test test/daemon_client_api_test.dart

# Daemon Rust unit tests (client_api, paths, …) run in CI job "Rust tests"
# (`cargo test -p ghal_bol_core --lib`) which installs ALSA/opus; do not compile ghal_bol here.
