#!/usr/bin/env bash
# generate-menu-icons.sh — Rasterize tray-menu glyphs from Lucide SVGs.
#
# Output: src-tauri/icons/menu/*.png — 144x144 RGBA bitmaps, mid-gray
# stroke at 1.5 weight, glyph centered in transparent padding so AppKit's
# auto-scale leaves the glyph noticeably smaller than the menu row text.
#
# Knobs (intentional, set once and don't drift):
#   STROKE_WIDTH=1.5   — Lucide ships at 2; 1.5 matches the paper-mono
#                        register used in the webview (`<Glyph>` runs
#                        1.75 but at smaller pixel sizes).
#   STROKE_COLOR=#888  — mid-gray reads on both Light and Dark menu
#                        backgrounds; muda doesn't call setTemplate:YES
#                        on custom bitmaps, so pure black/white would
#                        disappear in one mode.
#   CONTENT_PX=96      — the glyph's rendered size, ...
#   CANVAS_PX=144      — ...inside this transparent canvas. The 33%
#                        padding shrinks the visible glyph to roughly
#                        the optical size of native AppKit menu icons.
#
# Requires: rsvg-convert, ImageMagick (`magick`), node_modules/lucide-static.
# Run after pnpm install. Idempotent.

set -euo pipefail

cd "$(dirname "$0")/.."

LUCIDE_DIR="node_modules/lucide-static/icons"
OUT_DIR="src-tauri/icons/menu"

STROKE_WIDTH="1.5"
STROKE_COLOR="#888888"
CONTENT_PX=96
CANVAS_PX=144

[ -d "$LUCIDE_DIR" ] || {
    echo "ERROR: $LUCIDE_DIR not found — did you run pnpm install?" >&2
    exit 1
}
command -v rsvg-convert >/dev/null || {
    echo "ERROR: rsvg-convert not found (brew install librsvg)" >&2
    exit 1
}
command -v magick >/dev/null || {
    echo "ERROR: ImageMagick (magick) not found" >&2
    exit 1
}

# Output PNG basename → Lucide SVG basename. Output names match the
# constants in src-tauri/src/tray_icons.rs; Lucide names are pinned to
# the v1.8 catalogue. Update both sides together if either renames.
MAP=(
    "badge-check:badge-check"
    "bar-chart:bar-chart"
    "bolt:zap"
    "check:check"
    "circle-dot:circle-dot"
    "circle-pause:circle-pause"
    "circle-play:circle-play"
    "circle-user:circle-user"
    "desktop:monitor"
    "home:home"
    "layers:layers"
    "power:power"
    "refresh:refresh-cw"
    "sliders:sliders-horizontal"
    "terminal:terminal"
    "user-plus:user-plus"
)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$OUT_DIR"

for entry in "${MAP[@]}"; do
    out_name="${entry%%:*}"
    src_name="${entry##*:}"
    src="$LUCIDE_DIR/$src_name.svg"

    [ -f "$src" ] || {
        echo "ERROR: $src not found" >&2
        exit 1
    }

    work="$TMP/$out_name.svg"
    # Two-step sed: paint Lucide's `currentColor` strokes a fixed gray
    # (no CSS context at rasterize time), and slim the default 2-unit
    # stroke to our 1.5 weight.
    sed -e "s|stroke=\"currentColor\"|stroke=\"$STROKE_COLOR\"|g" \
        -e "s|stroke-width=\"2\"|stroke-width=\"$STROKE_WIDTH\"|g" \
        "$src" > "$work"

    glyph="$TMP/$out_name.glyph.png"
    rsvg-convert -w "$CONTENT_PX" -h "$CONTENT_PX" "$work" -o "$glyph"

    out="$OUT_DIR/$out_name.png"
    magick "$glyph" -gravity center -background none \
        -extent "${CANVAS_PX}x${CANVAS_PX}" "$out"

    printf "✓ %-14s ← %s.svg\n" "$out_name.png" "$src_name"
done

echo ""
echo "${#MAP[@]} icons written to $OUT_DIR/"
echo "Stroke ${STROKE_WIDTH} @ ${STROKE_COLOR}, content ${CONTENT_PX}px in ${CANVAS_PX}px canvas."
