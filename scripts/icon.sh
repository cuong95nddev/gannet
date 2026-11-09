#!/bin/sh
# Make the app icon, crates/gannet/resources/AppIcon.icns, from the
# website's icon (site/assets/icon.svg), shrunk onto macOS's icon grid (the
# body 824 of 1024 px, centred), and the menu bar icon, MenuBarIcon.pdf
# beside it: the viewfinder and the bird alone, in black, which macOS tints
# as a template. Run it after changing the icon; both are checked in, so
# builds need neither tool.
#   needs rsvg-convert (brew install librsvg) and iconutil (macOS)
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SVG="$ROOT/site/assets/icon.svg"
OUT="$ROOT/crates/gannet/resources/AppIcon.icns"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
# The icon's body is 240 of 256 units from (8, 8); the grid wants 206.
inner=$(sed -e '1d' -e '$d' "$SVG")
cat > "$STAGE/icon.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <g transform="translate(18.13 18.13) scale(0.8583)">
$inner
  </g>
</svg>
SVG

set_dir="$STAGE/AppIcon.iconset"
mkdir "$set_dir"
for size in 16 32 128 256 512; do
    rsvg-convert -w $size -h $size "$STAGE/icon.svg" -o "$set_dir/icon_${size}x${size}.png"
    rsvg-convert -w $((size * 2)) -h $((size * 2)) "$STAGE/icon.svg" -o "$set_dir/icon_${size}x${size}@2x.png"
done
mkdir -p "$(dirname "$OUT")"
iconutil -c icns "$set_dir" -o "$OUT"
echo "$OUT"

# The menu bar's: the art without its tile, 18 pt square, strokes a
# little bolder for the small size.
cat > "$STAGE/menu.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="40 40 176 176">
  <g fill="none" stroke="#000" stroke-width="16" stroke-linecap="round" stroke-linejoin="round">
    <path d="M52 92V60h32"/>
    <path d="M172 60h32v32"/>
    <path d="M204 164v32h-32"/>
    <path d="M84 196H52v-32"/>
  </g>
  <path d="M128 182 116 136 88 92l30 18 10 6 10-6 30-18-28 44z" fill="#000"/>
</svg>
SVG
rsvg-convert -f pdf "$STAGE/menu.svg" -o "$(dirname "$OUT")/MenuBarIcon.pdf"
echo "$(dirname "$OUT")/MenuBarIcon.pdf"
