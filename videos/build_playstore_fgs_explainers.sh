#!/usr/bin/env bash
# Build Play Console foreground-service explainer videos.
set -euo pipefail
cd "$(dirname "$0")"
python3 build_playstore_fgs_explainers.py
rm -rf _work
ls -lh *.mp4
