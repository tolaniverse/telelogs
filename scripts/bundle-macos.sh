#!/usr/bin/env bash
# Builds target/<profile>/Telelogs.app around the telelogs binary, with the app icon.
# Usage: scripts/bundle-macos.sh [--debug] [--binary PATH]
#   --binary PATH  bundle an already-built binary (e.g. a universal one) instead of building
set -euo pipefail
cd "$(dirname "$0")/.."

profile=release
binary=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug) profile=debug ;;
    --binary) binary="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
if [[ -z "$binary" ]]; then
  cargo build -p telelog-app $([[ $profile == release ]] && echo --release)
  binary="target/$profile/telelogs"
fi

version=$(cargo metadata --no-deps --format-version 1 |
  python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "telelog-app"))')
app=target/$profile/Telelogs.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/telelogs"
cp crates/telelog-app/resources/Telelogs.icns "$app/Contents/Resources/Telelogs.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Telelogs</string>
  <key>CFBundleDisplayName</key><string>Telelogs</string>
  <key>CFBundleIdentifier</key><string>dev.telelogs.app</string>
  <key>CFBundleExecutable</key><string>telelogs</string>
  <key>CFBundleIconFile</key><string>Telelogs</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
echo "built $app"
