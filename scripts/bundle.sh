#!/bin/sh
# Build a release Gannet.app (a menu bar app: no Dock icon) into OUT_DIR.
#   scripts/bundle.sh [OUT_DIR]          default: dist
# Prints the app's path.
# GANNET_UNIVERSAL=1 builds arm64 and x86_64 and joins them into one binary
# (needs `rustup target add aarch64-apple-darwin x86_64-apple-darwin`).
# GANNET_SIGN_ID signs it for distribution, GANNET_DEV_SIGN_ID for this Mac
# (see below).
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:-"$ROOT/dist"}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
# Bundle versions are numbers only: 0.2.1 for 0.2.1-rc1.
BUNDLE_VERSION=${VERSION%%[-+]*}

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
APP="$STAGE/Gannet.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/crates/gannet/resources/AppIcon.icns" "$APP/Contents/Resources/"

if [ "${GANNET_UNIVERSAL:-}" = 1 ]; then
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --locked --release -p gannet --target "$t" --manifest-path "$ROOT/Cargo.toml" >&2
    done
    lipo -create -output "$APP/Contents/MacOS/gannet" \
        "$ROOT/target/aarch64-apple-darwin/release/gannet" \
        "$ROOT/target/x86_64-apple-darwin/release/gannet"
else
    cargo build --release -p gannet --manifest-path "$ROOT/Cargo.toml" >&2
    cp "$ROOT/target/release/gannet" "$APP/Contents/MacOS/gannet"
fi

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Gannet</string>
    <key>CFBundleDisplayName</key><string>Gannet</string>
    <key>CFBundleIdentifier</key><string>dev.gannet</string>
    <key>CFBundleExecutable</key><string>gannet</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleShortVersionString</key><string>$BUNDLE_VERSION</string>
    <key>CFBundleVersion</key><string>$BUNDLE_VERSION</string>
    <key>LSMinimumSystemVersion</key><string>12.3</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# With GANNET_SIGN_ID (a "Developer ID Application: ..." identity) sign for
# distribution: hardened runtime and a secure timestamp, as notarization
# requires. GANNET_DEV_SIGN_ID (any identity, e.g. "Apple Development: ...")
# signs a build for this Mac plainly: a stable identity keeps Screen
# Recording allowed across rebuilds, which an ad hoc signature (keyed by the
# binary's hash) can't. Else ad hoc.
if [ -n "${GANNET_SIGN_ID:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$GANNET_SIGN_ID" "$APP" >&2
elif [ -n "${GANNET_DEV_SIGN_ID:-}" ]; then
    codesign --force --sign "$GANNET_DEV_SIGN_ID" "$APP" >&2
else
    codesign --force --sign - "$APP" >&2
fi

mkdir -p "$OUT"
rm -rf "$OUT/Gannet.app"
cp -R "$APP" "$OUT/Gannet.app"
echo "$OUT/Gannet.app"
