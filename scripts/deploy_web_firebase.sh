#!/usr/bin/env bash
# Build Flutter web and deploy to Firebase Hosting (ghalbol.com / www.ghalbol.com).
# Before build: web/.well-known/assetlinks.json and web/downloads/ghal-bol-linux-x64.tar.gz
# (see docs/WEB_SITE.md).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/ghal_bol_ui"
flutter build web --release
cd "$ROOT"
if [[ ! -f .firebaserc ]]; then
  echo "Copy .firebaserc.example to .firebaserc and set your Firebase project id." >&2
  exit 1
fi
firebase deploy --only hosting
