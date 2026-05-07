#!/usr/bin/env bash
# Regenerate Claudepot's icon assets from src-tauri/icons/icon.svg.
#
# Why this script exists rather than `pnpm tauri icon`:
# `pnpm tauri icon` uses bilinear resampling internally for some
# layers — most visibly the macOS .icns 128/256-pixel layers, which
# are exactly what the Dock displays at default size on Retina. The
# result looks visibly soft. Regenerating those layers directly from
# SVG via rsvg-convert and packing with `iconutil` produces
# pixel-perfect crisp output. See dev-docs/network-detection-panel.md
# (audit notes) and the v0.1.15 CHANGELOG entry.
#
# This script outputs only the assets we actually ship:
#   - icon.svg          — source of truth (untouched)
#   - icon.icns         — macOS .app
#   - icon.ico          — Windows NSIS + MSI
#   - icon.png (512)    — fallback / Linux 512x512
#   - 32/48/64/128/256x.png — Linux hicolor sizes (per tauri.conf.json bundle.icon)
#
# Skipped on purpose (we don't ship these targets):
#   - Square*Logo.png + StoreLogo.png (MSIX/UWP)
#   - ios/ + android/ (mobile)

set -euo pipefail

cd "$(dirname "$0")/.."

SVG=src-tauri/icons/icon.svg
ICONS_DIR=src-tauri/icons

if ! command -v rsvg-convert >/dev/null; then
  echo "error: rsvg-convert not found. Install via 'brew install librsvg'." >&2
  exit 1
fi
if ! command -v iconutil >/dev/null; then
  echo "error: iconutil not found. macOS-only tool — run this on a Mac." >&2
  exit 1
fi
if ! command -v magick >/dev/null; then
  echo "error: ImageMagick (magick) not found. Install via 'brew install imagemagick'." >&2
  exit 1
fi

echo "Source: $SVG"

# 1. Standalone Linux + master PNGs.
for size in 32 48 64 128 256; do
  out="$ICONS_DIR/${size}x${size}.png"
  rsvg-convert -w "$size" -h "$size" "$SVG" -o "$out"
  echo "  wrote $out"
done
rsvg-convert -w 512 -h 512 "$SVG" -o "$ICONS_DIR/icon.png"
echo "  wrote $ICONS_DIR/icon.png (512x512)"

# 2. macOS .icns — full Apple-spec ladder, packed via iconutil.
SET=$(mktemp -d -t claudepot-iconset)/Claudepot.iconset
mkdir -p "$SET"
for spec in 16:icon_16x16 32:icon_16x16@2x 32:icon_32x32 64:icon_32x32@2x \
            128:icon_128x128 256:icon_128x128@2x 256:icon_256x256 \
            512:icon_256x256@2x 512:icon_512x512 1024:icon_512x512@2x; do
  size="${spec%%:*}"
  name="${spec##*:}"
  rsvg-convert -w "$size" -h "$size" "$SVG" -o "$SET/${name}.png"
done
iconutil -c icns "$SET" -o "$ICONS_DIR/icon.icns"
echo "  wrote $ICONS_DIR/icon.icns"

# 3. Windows .ico — Microsoft spec sizes (16, 24, 32, 48, 64, 256),
#    PNG-compressed inside .ico via magick. Each layer rendered fresh
#    from SVG so pixel content is crisp.
ICO_PNGS=$(mktemp -d -t claudepot-ico)
for size in 16 24 32 48 64 256; do
  rsvg-convert -w "$size" -h "$size" "$SVG" -o "$ICO_PNGS/$size.png"
done
# `magick` stores layers ≥256 as PNG-compressed automatically; for
# smaller layers it defaults to BMP which bloats the file. Force PNG
# for all by using the per-layer PNG-format directive.
magick "$ICO_PNGS/16.png" "$ICO_PNGS/24.png" "$ICO_PNGS/32.png" \
       "$ICO_PNGS/48.png" "$ICO_PNGS/64.png" "$ICO_PNGS/256.png" \
       -define ico:format=png "$ICONS_DIR/icon.ico"
echo "  wrote $ICONS_DIR/icon.ico"

echo ""
echo "Done. Verify with:"
echo "  file $ICONS_DIR/icon.ico"
echo "  iconutil -c iconset $ICONS_DIR/icon.icns -o /tmp/check.iconset && ls /tmp/check.iconset"
