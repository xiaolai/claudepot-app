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
FLAT_SVG=src-tauri/icons/icon-flat.svg
WIN_SVG=assets/icon-set/windows/icon-glyph.svg
TRAY_SVG=assets/icon-set/tray/tray-template.svg
TRAY_ALERT_SVG=assets/icon-set/tray/tray-alert-template.svg
ICONS_DIR=src-tauri/icons

# Below this size the plate grain reads as dirt, not as a finish.
#
# The grain is `feTurbulence`, which is computed at RENDER size — so as
# the tile shrinks the noise coarsens relative to it instead of scaling
# with it. `icon-flat.svg` is the same artwork with a solid plate
# (#DCD8D2) and no filter. This is why the ladder cannot be generated
# from one file, and why `pnpm tauri icon` (single source, internal
# resampling) would be wrong here for a second, independent reason on
# top of the soft-.icns one.
GRAIN_FLOOR=48

# Echo the right master for a given pixel size.
src_for() {
  if [ "$1" -lt "$GRAIN_FLOOR" ]; then echo "$FLAT_SVG"; else echo "$SVG"; fi
}

# Rasterise to a size, and force the result to RGBA.
#
# `rsvg-convert` emits a 3-channel RGB PNG whenever the artwork is
# fully opaque, which this set is — the plate covers the whole canvas,
# unlike the old squircle-on-transparent icon. Tauri's build script
# hard-rejects that: "icon <path> is not RGBA", and it fails the whole
# crate compile rather than warning. `PNG32:` pins the 8-bit RGBA
# encoding regardless of what the source happened to need.
#
# Applied to EVERY raster, not just the bundle list, so the .icns and
# .ico ladders cannot pick up a stray 3-channel layer either.
render() {
  local size="$1" src="$2" out="$3"
  rsvg-convert -w "$size" -h "$size" "$src" -o "$out"
  magick "$out" -define png:color-type=6 "PNG32:$out"
}

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
for required in "$SVG" "$FLAT_SVG" "$WIN_SVG" "$TRAY_SVG" "$TRAY_ALERT_SVG"; do
  [ -f "$required" ] || { echo "error: missing master $required" >&2; exit 1; }
done

echo "Source: $SVG (grain), $FLAT_SVG (<${GRAIN_FLOOR}px)"

# 1. Standalone Linux + master PNGs.
for size in 32 48 64 128 256; do
  out="$ICONS_DIR/${size}x${size}.png"
  render "$size" "$(src_for "$size")" "$out"
  echo "  wrote $out"
done
render 512 "$SVG" "$ICONS_DIR/icon.png"
echo "  wrote $ICONS_DIR/icon.png (512x512)"

# 1b. Dock override asset — squircled and inset, unlike every other output.
#
# `dock_icon.rs` hands this to `NSApp.setApplicationIconImage`, and that
# path is the exact inverse of the bundle path:
#
#   bundle .icns  -> full bleed; the system applies mask, inset, shadow
#   setIcon()     -> drawn verbatim; NO mask, NO inset, NO shadow
#
# Getting those backwards is the classic macOS icon bug. The previous
# artwork hid it by baking a squircle into the SVG at 416/512 = 0.813 of
# the canvas, which is within a couple of pixels of Apple's measured
# tile fraction. This icon set is deliberately full-bleed (correct for
# the bundle), so handing it to setIcon() unmodified would render a
# hard-edged square measured at ~22% larger than every neighbour in the
# Dock.
#
# The corner is a superellipse (|x/a|^n + |y/a|^n = 1, n = 5) rather
# than a circular arc — a circular corner meets the straight edge with
# a curvature discontinuity, which is what makes a rounded rect read as
# boxy beside real icons.
#
# 824/1024 = 0.805, Apple's measured tile fraction
# (`~/.claude/agents/icon-smith/specs.md`).
#
# This asset feeds `dock_icon.rs`, which is now DEBUG-ONLY: a packaged
# app takes its Dock icon from the bundle (`Assets.car` on macOS 26,
# `icon.icns` before it), and overriding that at runtime would discard
# the Liquid Glass rendering the layered icon exists to earn. So this
# file only ever shows up in `cargo run`, where being a few pixels off
# the Apple grid costs nothing.
DOCK_TILE_FRACTION=0.805
render 1024 "$SVG" "$ICONS_DIR/icon-dock-src.png"
python3 - "$ICONS_DIR/icon-dock-src.png" "$ICONS_DIR/icon-dock.png" "$DOCK_TILE_FRACTION" <<'PY'
import sys
from PIL import Image, ImageDraw

CANVAS, N = 1024, 5

src_path, out_path = sys.argv[1], sys.argv[2]
TILE_FRACTION = float(sys.argv[3])
tile = int(round(CANVAS * TILE_FRACTION))

art = Image.open(src_path).convert("RGBA").resize((tile, tile), Image.LANCZOS)

# Superellipse alpha mask, supersampled 4x so the curve is smooth.
ss = 4
mask = Image.new("L", (tile * ss, tile * ss), 0)
a = tile * ss / 2
pts = []
STEPS = 2048
for i in range(STEPS):
    t = 2.0 * 3.141592653589793 * i / STEPS
    ct, st = __import__("math").cos(t), __import__("math").sin(t)
    # Parametric superellipse; sign-preserving fractional power.
    x = a * (abs(ct) ** (2.0 / N)) * (1 if ct >= 0 else -1)
    y = a * (abs(st) ** (2.0 / N)) * (1 if st >= 0 else -1)
    pts.append((a + x, a + y))
ImageDraw.Draw(mask).polygon(pts, fill=255)
mask = mask.resize((tile, tile), Image.LANCZOS)

art.putalpha(mask)
out = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
off = (CANVAS - tile) // 2
out.paste(art, (off, off), art)
out.save(out_path)
PY
rm -f "$ICONS_DIR/icon-dock-src.png"
echo "  wrote $ICONS_DIR/icon-dock.png (1024, squircled, inset $DOCK_TILE_FRACTION)"

# 2. macOS .icns — full Apple-spec ladder, packed via iconutil.
SET=$(mktemp -d -t claudepot-iconset)/Claudepot.iconset
mkdir -p "$SET"
for spec in 16:icon_16x16 32:icon_16x16@2x 32:icon_32x32 64:icon_32x32@2x \
            128:icon_128x128 256:icon_128x128@2x 256:icon_256x256 \
            512:icon_256x256@2x 512:icon_512x512 1024:icon_512x512@2x; do
  size="${spec%%:*}"
  name="${spec##*:}"
  render "$size" "$(src_for "$size")" "$SET/${name}.png"
done
iconutil -c icns "$SET" -o "$ICONS_DIR/icon.icns"
echo "  wrote $ICONS_DIR/icon.icns"

# 3. Windows .ico — Microsoft spec sizes (16, 24, 32, 48, 64, 256).
#    PIL writes each ICO layer as a PNG-compressed sub-image, which is
#    what Vista+ Windows expects (ImageMagick's `magick *.png foo.ico`
#    falls back to raw BMP per layer and bloats the file ~100×).
#
#    Built from the PLATELESS glyph, not the plated master. Windows
#    draws no enclosure of its own and shows the icon against taskbar
#    and Explorer chrome of every shade, so the anodised plate would
#    read as a grey card floating behind the block rather than as the
#    icon's surface. `windows/icon-glyph.svg` is the block alone,
#    transparent, filling the grid.
ICO_PNGS=$(mktemp -d -t claudepot-ico)
for size in 16 24 32 48 64 256; do
  render "$size" "$WIN_SVG" "$ICO_PNGS/$size.png"
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

# 4. Menu-bar tray icons, 44x44 (@2x of 22pt).
#
#    Two variants per state, and the reason is in `tray.rs`: macOS
#    inverts a *Template* image to match the menubar in Light and Dark,
#    while Windows and Linux have no such concept and would render a
#    pure-black glyph invisible on a dark taskbar. The Mono copy is the
#    same alpha filled #808080, which reads on both.
#
#    Generated here rather than hand-exported so the tray cannot drift
#    from the app icon: both now come from one authored set, and the
#    tray silhouette is deliberately the plain block — a single-colour
#    render cannot carry the four plane tones.
for pair in "tray-icon:$TRAY_SVG" "tray-iconAlert:$TRAY_ALERT_SVG"; do
  name="${pair%%:*}"
  src="${pair#*:}"
  render 44 "$src" "$ICONS_DIR/${name}Template@2x.png"
  magick "$ICONS_DIR/${name}Template@2x.png" \
    -fill '#808080' -colorize 100 "$ICONS_DIR/${name}Mono@2x.png"
  echo "  wrote $ICONS_DIR/${name}Template@2x.png + ${name}Mono@2x.png"
done

# 6. macOS 26 layered icon layers.
#
#    These are PNGs and not SVGs on purpose: `actool` renders a layer
#    SVG carrying a filter as near-black, which is how the 0.4.12 Dock
#    icon shipped black. The plate has `feTurbulence` grain and a
#    `radialGradient` sheen, so it is exactly the shape that fails.
#    rsvg draws both correctly, so rasterising here and handing actool
#    flat pixels sidesteps the whole class.
#
#    Regenerated HERE rather than by hand, because by hand is how they
#    went stale: the -48 centring shift moved every other raster and
#    left these two behind, so the Dock kept the old low block while
#    the tray, .icns and web mark had all moved. Nothing failed —
#    a stale layer is a valid PNG.
LAYERS_DIR="$ICONS_DIR/Claudepot.icon/Assets"
if [[ -d "$LAYERS_DIR" ]]; then
  render 1024 assets/icon-set/macos/layer-1-plate.svg "$LAYERS_DIR/plate.png"
  render 1024 assets/icon-set/macos/layer-2-glyph.svg "$LAYERS_DIR/block.png"
  echo "  wrote $LAYERS_DIR/plate.png + block.png"
  echo "  NOTE: run scripts/build-macos-glass-icon.sh to recompile Assets.car"
fi

echo ""
echo "Done. Verify with:"
echo "  file $ICONS_DIR/icon.ico"
echo "  iconutil -c iconset $ICONS_DIR/icon.icns -o /tmp/check.iconset && ls /tmp/check.iconset"
echo "  scripts/build-macos-glass-icon.sh   # recompile Assets.car from the layers"
echo "  scripts/verify-icons.py"
