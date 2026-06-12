#!/usr/bin/env bash
# generate-menu-icons.sh — Rasterize tray-menu glyphs from Lucide SVGs.
#
# Output: src-tauri/icons/menu/{name}.png + {name}-light.png — 144x144
# RGBA bitmaps, stroke at 1.5 weight, glyph centered in transparent
# padding so AppKit's auto-scale leaves the glyph noticeably smaller
# than the menu row text. Two stroke variants per glyph:
#   {name}.png       — dark stroke, rendered on LIGHT dropdowns
#   {name}-light.png — light stroke, rendered on DARK dropdowns
# `tray_icons.rs` selects per system appearance at menu-build time;
# muda 0.19 still doesn't call setTemplate:YES on custom bitmaps, so
# AppKit never auto-tints these — the paired sets are the fix.
#
# Knobs (intentional, set once and don't drift):
#   STROKE_WIDTH=1.5   — Lucide ships at 2; 1.5 matches the paper-mono
#                        register used in the webview (`<Glyph>` runs
#                        1.75 but at smaller pixel sizes).
#   STROKE_DARK=#3a3a3a — dark gray (~22% luminance), close to native
#                        macOS Light-mode label color. macOS NSMenu
#                        Vibrant Light material renders as a mid-gray
#                        (≈ rgb(160,160,160)); a #888 stroke (53%) sat
#                        nearly on top of that bg and read as invisible.
#   STROKE_LIGHT=#d0d0d0 — light gray (~81% luminance), close to the
#                        native Dark-mode label color. NSMenu Vibrant
#                        Dark material is a dark gray (≈ rgb(45,45,50));
#                        the dark stroke disappears on it.
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
STROKE_DARK="#3a3a3a"
STROKE_LIGHT="#d0d0d0"
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
    "shield:shield"
    "sliders:sliders-horizontal"
    "terminal:terminal"
    "user-plus:user-plus"
)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$OUT_DIR"

render() {
    # render <src.svg> <stroke-color> <out.png>
    local src="$1" stroke="$2" out="$3"
    local work glyph
    work="$TMP/$(basename "$out" .png).svg"
    glyph="$TMP/$(basename "$out" .png).glyph.png"
    # Two-step sed: paint Lucide's `currentColor` strokes a fixed gray
    # (no CSS context at rasterize time), and slim the default 2-unit
    # stroke to our 1.5 weight.
    sed -e "s|stroke=\"currentColor\"|stroke=\"$stroke\"|g" \
        -e "s|stroke-width=\"2\"|stroke-width=\"$STROKE_WIDTH\"|g" \
        "$src" > "$work"
    rsvg-convert -w "$CONTENT_PX" -h "$CONTENT_PX" "$work" -o "$glyph"
    magick "$glyph" -gravity center -background none \
        -extent "${CANVAS_PX}x${CANVAS_PX}" "$out"
}

for entry in "${MAP[@]}"; do
    out_name="${entry%%:*}"
    src_name="${entry##*:}"
    src="$LUCIDE_DIR/$src_name.svg"

    [ -f "$src" ] || {
        echo "ERROR: $src not found" >&2
        exit 1
    }

    render "$src" "$STROKE_DARK" "$OUT_DIR/$out_name.png"
    render "$src" "$STROKE_LIGHT" "$OUT_DIR/$out_name-light.png"

    printf "✓ %-14s + %-20s ← %s.svg\n" "$out_name.png" "$out_name-light.png" "$src_name"
done

echo ""
echo "${#MAP[@]} glyphs (×2 stroke variants) written to $OUT_DIR/"
echo "Stroke ${STROKE_WIDTH} @ ${STROKE_DARK} (light menus) / ${STROKE_LIGHT} (dark menus),"
echo "content ${CONTENT_PX}px in ${CANVAS_PX}px canvas."
