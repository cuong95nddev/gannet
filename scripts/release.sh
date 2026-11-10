#!/bin/sh
# Build a universal Gannet.app and pack it in a DMG (with an Applications
# link to drag it onto) for a GitHub release.
#   scripts/release.sh [OUT_DIR]      default: dist
# Prints the DMG's path.
# For a DMG that opens without Gatekeeper warnings, set GANNET_SIGN_ID to a
# "Developer ID Application: ..." identity and GANNET_NOTARY_PROFILE to a
# notarytool keychain profile (`xcrun notarytool store-credentials`); the
# app and the DMG are then signed, and both notarized and stapled: the
# app's own ticket lets it open offline once copied out of the DMG
# (GANNET_NOTARY_KEYCHAIN: the keychain holding that profile, if not the
# default one).
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:-"$ROOT/dist"}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
GANNET_UNIVERSAL=1 "$ROOT/scripts/bundle.sh" "$STAGE/dmg" >/dev/null
notarize() {
    xcrun notarytool submit "$1" --keychain-profile "$GANNET_NOTARY_PROFILE" \
        ${GANNET_NOTARY_KEYCHAIN:+--keychain "$GANNET_NOTARY_KEYCHAIN"} --wait >&2
}
if [ -n "${GANNET_NOTARY_PROFILE:-}" ]; then
    ditto -c -k --keepParent "$STAGE/dmg/Gannet.app" "$STAGE/Gannet.zip"
    notarize "$STAGE/Gannet.zip"
    xcrun stapler staple "$STAGE/dmg/Gannet.app" >&2
fi
ln -s /Applications "$STAGE/dmg/Applications"

mkdir -p "$OUT"
DMG="$OUT/Gannet-$VERSION-macos-universal.dmg"
rm -f "$DMG"
hdiutil create -volname "Gannet $VERSION" -srcfolder "$STAGE/dmg" -fs HFS+ -format UDZO "$DMG" >/dev/null
if [ -n "${GANNET_SIGN_ID:-}" ]; then
    codesign --force --timestamp --sign "$GANNET_SIGN_ID" "$DMG" >&2
fi
if [ -n "${GANNET_NOTARY_PROFILE:-}" ]; then
    notarize "$DMG"
    xcrun stapler staple "$DMG" >&2
fi
echo "$DMG"
