#!/usr/bin/env bash
# Build the HTML, print it to PDF with headless Chromium, and cut PNG previews.
set -euo pipefail
cd "$(dirname "$0")"
node build.js
out="${1:-out}"
mkdir -p "$out"
chromium --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
  --no-pdf-header-footer --run-all-compositor-stages-before-draw --virtual-time-budget=8000 \
  --print-to-pdf="$out/face-id.pdf" "file://$PWD/face-id.html" 2>/dev/null
rm -f "$out"/p-*.png
pdftoppm -r 72 -png "$out/face-id.pdf" "$out/p"
pdfinfo "$out/face-id.pdf" | grep -E 'Pages|Page size'
