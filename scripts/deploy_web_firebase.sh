#!/usr/bin/env bash
# Publish web/ to Firebase Hosting (ghalbol.com).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if ! command -v firebase >/dev/null 2>&1; then
  echo "Install firebase-tools and run firebase login first." >&2
  exit 1
fi
firebase deploy --only hosting
