#!/bin/bash
# Rasterize the transcript's tool and file icons (the iOS asset catalog's
# SVGs — one source for both apps) into PNG assets for the Android painter.
#
#   scripts/android/gen-icons.sh <out_dir>
#
# Needs rsvg-convert (`brew install librsvg`); without it the painter falls
# back to Material symbols.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$1"
mkdir -p "$OUT"
if ! command -v rsvg-convert >/dev/null; then
  echo "note: rsvg-convert not found — skipped icon rasterization"
  exit 0
fi
for svg in "$ROOT"/apps/ios/Zeron/Assets.xcassets/{ToolIcons,FileIcons}/*.imageset/*.svg; do
  name="$(basename "$(dirname "$svg")" .imageset)"
  png="$OUT/$name.png"
  [[ "$png" -nt "$svg" ]] && continue
  rsvg-convert -w 72 -h 72 --keep-aspect-ratio "$svg" -o "$png"
done
