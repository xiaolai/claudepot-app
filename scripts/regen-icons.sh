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
if ! python3 -c "from PIL import Image" 2>/dev/null; then
  echo "error: PIL/Pillow not found. Install via 'pip3 install Pillow' or 'brew install python-pillow'." >&2
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

# 3. Windows .ico — Microsoft spec sizes (16, 24, 32, 48, 64, 256).
#    PIL writes each ICO layer as a PNG-compressed sub-image, which is
#    what Vista+ Windows expects (ImageMagick's `magick *.png foo.ico`
#    falls back to raw BMP per layer and bloats the file ~100×).
ICO_PNGS=$(mktemp -d -t claudepot-ico)
for size in 16 24 32 48 64 256; do
  rsvg-convert -w "$size" -h "$size" "$SVG" -o "$ICO_PNGS/$size.png"
done
python3 - "$ICO_PNGS" "$ICONS_DIR/icon.ico" <<'PY'
"""Build a multi-layer .ico whose every layer is a PNG-compressed
copy of the matching pre-rendered file. Embedding raw PNG bytes
avoids any decoder/resizer round-trip, so the bitmap content is
exactly what rsvg produced. PIL's ICO writer would re-encode and
resize from a single source — the wrong shape for this task.

Format reference: ICONDIR + 6 ICONDIRENTRYs + 6 PNG payloads."""
import struct, sys
from pathlib import Path

src_dir, out_path = Path(sys.argv[1]), Path(sys.argv[2])
sizes = [16, 24, 32, 48, 64, 256]
payloads = [(src_dir / f"{s}.png").read_bytes() for s in sizes]

# ICONDIR = reserved (2) + type (2, 1=ICO) + count (2)
header = struct.pack("<HHH", 0, 1, len(sizes))
# Each ICONDIRENTRY = 16 bytes; payloads start after header + N entries.
entries_start = len(header) + 16 * len(sizes)
offset = entries_start
entries, blob = b"", b""
for size, payload in zip(sizes, payloads):
    # Width/height of 0 means 256+ in the ICO format.
    w = h = 0 if size >= 256 else size
    # ICONDIRENTRY: w(1) h(1) colors(1) reserved(1) planes(2) bpp(2) size(4) offset(4)
    entries += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(payload), offset)
    blob += payload
    offset += len(payload)

out_path.write_bytes(header + entries + blob)
PY
echo "  wrote $ICONS_DIR/icon.ico"

echo ""
echo "Done. Verify with:"
echo "  file $ICONS_DIR/icon.ico"
echo "  iconutil -c iconset $ICONS_DIR/icon.icns -o /tmp/check.iconset && ls /tmp/check.iconset"
