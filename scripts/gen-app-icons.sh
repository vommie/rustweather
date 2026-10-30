#!/usr/bin/env bash
# Renders the app icon in every size the desktop needs from packaging/rustweather.svg. The PNGs are committed, so a normal
# build does not need this; run it after changing the SVG. Needs `rsvg-convert` (package librsvg).
set -euo pipefail
cd "$(dirname "$0")/.."
SVG=packaging/rustweather.svg
command -v rsvg-convert >/dev/null || { echo "rsvg-convert is missing: sudo pacman -S librsvg" >&2; exit 1; }
mkdir -p packaging/icons
for s in 16 22 24 32 48 64 128 256 512; do
  rsvg-convert -w "$s" -h "$s" "$SVG" -o "packaging/icons/rustweather-$s.png"
done
echo "icons written to packaging/icons"
