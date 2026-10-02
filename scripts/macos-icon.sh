#!/usr/bin/env bash
# Renders assets/brand/icon.svg into crates/telelog-app/resources/Telelogs.icns.
# Needs rsvg-convert (brew install librsvg) and macOS's iconutil. Rerun after changing the icon.
set -euo pipefail
cd "$(dirname "$0")/.."

iconset=$(mktemp -d)/Telelogs.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  rsvg-convert -w "$size" -h "$size" assets/brand/icon.svg -o "$iconset/icon_${size}x${size}.png"
  rsvg-convert -w $((size * 2)) -h $((size * 2)) assets/brand/icon.svg -o "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o crates/telelog-app/resources/Telelogs.icns
echo "wrote crates/telelog-app/resources/Telelogs.icns"
