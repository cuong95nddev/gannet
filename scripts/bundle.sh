#!/bin/sh
# Build dist/Gannet.app: a release build in a bundle with no Dock icon.
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release -p gannet
app=dist/Gannet.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/gannet "$app/Contents/MacOS/gannet"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Gannet</string>
  <key>CFBundleDisplayName</key><string>Gannet</string>
  <key>CFBundleIdentifier</key><string>dev.gannet</string>
  <key>CFBundleExecutable</key><string>gannet</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>12.3</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Ad hoc signature, so the Screen Recording permission sticks to the bundle.
codesign --force --sign - "$app"
echo "$app"
