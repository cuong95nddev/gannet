#!/bin/sh
# Build Gannet.app and install it into /Applications (or $GANNET_INSTALL_DIR),
# replacing and relaunching a running copy.
#
#   scripts/install.sh            build, install, launch
#   scripts/install.sh --no-open  build and install only
#
# Signs with the first "Apple Development" identity in the keychain, if any,
# so Screen Recording stays allowed across reinstalls; set GANNET_DEV_SIGN_ID
# to pick another, or GANNET_SIGN_ID for a release signature.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)

open_after=1
for arg in "$@"; do
    case "$arg" in
        --no-open) open_after=0 ;;
        -h | --help)
            sed -n '2,10s/^# \{0,1\}//p' "$0"
            exit 0
            ;;
        *)
            echo "unknown option: $arg" >&2
            exit 2
            ;;
    esac
done

if [ -z "${GANNET_SIGN_ID:-}" ] && [ -z "${GANNET_DEV_SIGN_ID:-}" ]; then
    GANNET_DEV_SIGN_ID=$(security find-identity -v -p codesigning 2>/dev/null |
        sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development: .*/\1/p' | head -1)
    export GANNET_DEV_SIGN_ID
    [ -n "$GANNET_DEV_SIGN_ID" ] ||
        echo "warning: no Apple Development identity; ad hoc signed, so macOS asks for Screen Recording after each build" >&2
fi

dir=${GANNET_INSTALL_DIR:-/Applications}
dest="$dir/Gannet.app"
bin="$dest/Contents/MacOS/gannet"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
# Its status, not a pipe's: a failed build must stop here, before the
# installed app is touched.
out=$("$ROOT/scripts/bundle.sh" "$STAGE")
app=$(printf '%s\n' "$out" | tail -1)
if [ ! -d "$app" ]; then
    echo "error: the build made no app (got: $app)" >&2
    exit 1
fi

# macOS keeps privacy permissions per signing identity: a different one
# (say Apple Development after Developer ID) means granting them again.
# Turning a permission back on in System Settings keeps the old identity's
# record, so it only holds until the next build: reset it, then grant anew.
req() { codesign -d -r- "$1" 2>/dev/null | sed -n 's/^designated => //p'; }
old_req=$(req "$dest")
if [ -n "$old_req" ] && [ "$old_req" != "$(req "$app")" ]; then
    echo "warning: signed by a different identity than the installed Gannet; macOS will ask for Screen Recording again" >&2
    echo "  clear the old grant first, or it never sticks: tccutil reset ScreenCapture dev.gannet" >&2
fi

# Quit the installed copy only, not a `cargo run` build.
if pgrep -f "^$bin" >/dev/null; then
    echo "Quitting the running Gannet"
    pkill -f "^$bin" || true
    i=0
    while pgrep -f "^$bin" >/dev/null && [ $i -lt 20 ]; do
        sleep 0.25
        i=$((i + 1))
    done
    pkill -9 -f "^$bin" 2>/dev/null || true
fi

mkdir -p "$dir"
rm -rf "$dest"
ditto "$app" "$dest"
codesign --verify "$dest"
echo "Installed $dest"

if [ "$open_after" = 1 ]; then
    open "$dest"
    echo "Launched Gannet (menu bar)"
fi
