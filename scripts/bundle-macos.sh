#!/usr/bin/env bash
# Builds target/release/Telelogs.app around the telelogs binary, with the app icon.
# Usage: scripts/bundle-macos.sh [--debug]
set -euo pipefail
cd "$(dirname "$0")/.."

profile=release
if [[ "${1:-}" == "--debug" ]]; then profile=debug; fi
cargo build -p telelog-app $([[ $profile == release ]] && echo --release)

version=$(cargo metadata --no-deps --format-version 1 |
  python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "telelog-app"))')
app=target/$profile/Telelogs.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$profile/telelogs" "$app/Contents/MacOS/telelogs"
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
