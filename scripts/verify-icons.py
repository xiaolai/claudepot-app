#!/usr/bin/env python3
"""Structural checks on the generated icon assets.

# Why this exists

`regen-icons.sh` can succeed and still produce something wrong: an ICO
whose layers are raw BMP instead of PNG, an `.icns` missing the exact
layer the Dock reaches for, a tray icon at the wrong size, or a small
raster carrying the grain filter that is supposed to be flat below
48 px. Every one of those looks like a normal file on disk and only
shows up as "the icon looks off" much later.

None of this needs a GUI, so it runs anywhere. What it deliberately
does NOT check is how the artwork *looks* — that is what launching the
app is for.

Exit 0 = all checks pass, 1 = at least one failed.
"""
from __future__ import annotations

import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"

failures: list[str] = []
checks = 0


def check(ok: bool, label: str, detail: str = "") -> None:
    global checks
    checks += 1
    if ok:
        print(f"  ok   {label}")
    else:
        print(f"  FAIL {label}{(' — ' + detail) if detail else ''}")
        failures.append(label)


def png_size(p: Path) -> tuple[int, int] | None:
    """Width/height straight from the IHDR, so this needs no Pillow."""
    data = p.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    w, h = struct.unpack(">II", data[16:24])
    return w, h


# PNG IHDR colour-type byte. 6 is truecolour+alpha.
PNG_COLOUR_TYPES = {0: "grayscale", 2: "RGB", 3: "palette",
                    4: "gray+alpha", 6: "RGBA"}


def png_colour_type(p: Path) -> str:
    data = p.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        return "not-a-png"
    return PNG_COLOUR_TYPES.get(data[25], str(data[25]))


print("PNG ladder (tauri.conf.json bundle.icon)")
for size in (32, 48, 64, 128, 256):
    p = ICONS / f"{size}x{size}.png"
    check(p.is_file() and png_size(p) == (size, size),
          f"{p.name} is {size}x{size}",
          f"got {png_size(p) if p.is_file() else 'missing'}")

# Tauri's build script rejects a bundle icon that is not RGBA — it
# fails the whole crate compile, not just the bundling step. rsvg emits
# 3-channel RGB whenever the artwork is fully opaque, which this icon
# set is (the plate covers the canvas; the old squircle did not). This
# check exists because the build caught it and this script did not:
# dimensions were verified and colour type was assumed.
for name in ("32x32.png", "48x48.png", "64x64.png", "128x128.png",
             "256x256.png", "icon.png"):
    p = ICONS / name
    ct = png_colour_type(p) if p.is_file() else "missing"
    check(ct == "RGBA", f"{name} is RGBA (tauri build hard-requires it)",
          f"got {ct}")

# The Dock override in `dock_icon.rs` embeds this file and the module
# doc is explicit that 512 is what makes every Dock size a clean
# downsample. A smaller file here is an upscale at 96 px on Retina.
p = ICONS / "icon.png"
check(p.is_file() and png_size(p) == (512, 512),
      "icon.png is 512x512 (dock_icon.rs embeds it)",
      f"got {png_size(p) if p.is_file() else 'missing'}")

print("\nDock override asset (setApplicationIconImage)")
# The inverse of the bundle rule, and the easiest thing to get
# backwards: a bundle icon is full bleed and the system masks it, while
# `setApplicationIconImage` draws exactly what it is handed. A
# full-bleed image on that path renders as a hard square measured ~22%
# larger than every neighbour. Assert the two properties that
# distinguish this file from the bundle one.
dock = ICONS / "icon-dock.png"
if not dock.is_file():
    check(False, "icon-dock.png exists (dock_icon.rs embeds it)")
else:
    try:
        from PIL import Image as _I
    except ImportError:
        print("  skip Pillow not installed — dock geometry unchecked")
    else:
        im = _I.open(dock).convert("RGBA")
        alpha = im.getchannel("A")
        box = alpha.getbbox()
        check(im.size == (1024, 1024), "icon-dock.png is 1024x1024",
              f"got {im.size}")
        if box:
            frac = (box[2] - box[0]) / im.width
            # Apple's measured tile fraction, 824/1024. Debug-only
            # asset (see dock_icon.rs), so the bound is about catching
            # a regression to full bleed, not pixel parity.
            check(0.79 <= frac <= 0.82,
                  "artwork inset to ~0.805 of the canvas (Apple tile fraction)",
                  f"got {frac:.4f} — full bleed would be 1.0")
            # A square tile would leave the corner opaque. The
            # superellipse must cut it away.
            corner = alpha.getpixel((box[0], box[1]))
            check(corner == 0, "corner is masked (squircle, not a square)",
                  f"corner alpha {corner}; 255 means no mask was applied")
            check(alpha.getpixel((im.width // 2, im.height // 2)) == 255,
                  "centre is opaque")
        else:
            check(False, "icon-dock.png has any ink", "fully transparent")

    # And it must NOT be the same file as the bundle icon — that
    # equality is precisely the bug.
    bundle = ICONS / "icon.png"
    if bundle.is_file():
        check(dock.read_bytes() != bundle.read_bytes(),
              "dock asset differs from the full-bleed bundle icon")

print("\nmacOS 26 layered icon (Claudepot.icon → Assets.car)")
# THE check this file was missing when a black icon shipped in 0.4.12.
#
# Everything else here inspects files that are inputs. A layered icon
# is different: the picture is produced by Apple's renderer at
# composite time, so every input can be individually valid and the
# result still be wrong. Both 0.4.12 bugs were invisible to file
# inspection —
#
#   1. SVG layers carrying `filter`/`feTurbulence`/gradients rendered
#      near-BLACK. rsvg handles them; actool's SVG path evidently does
#      not, and it reports nothing.
#   2. Group order is TOP-FIRST. With the full-canvas plate listed
#      first it occluded the block entirely, giving a blank plate.
#
# So render it and look at the pixels.
dot_icon = ICONS / "Claudepot.icon"
car = ICONS / "Assets.car"
ICTOOL = Path("/Applications/Xcode.app/Contents/Applications/"
              "Icon Composer.app/Contents/Executables/ictool")

check(dot_icon.is_dir(), "Claudepot.icon exists")
check(car.is_file() and car.stat().st_size > 100_000,
      "Assets.car is present and non-trivial",
      f"{car.stat().st_size if car.is_file() else 'missing'} bytes")

# Layers must be PNG. An SVG layer is the black-icon bug.
if dot_icon.is_dir():
    svg_layers = sorted(p.name for p in (dot_icon / "Assets").glob("*.svg"))
    check(not svg_layers,
          "no SVG layers (actool renders filtered SVG as black)",
          f"found {svg_layers}")

if not ICTOOL.is_file():
    print("  skip ictool not installed — rendered check unavailable")
elif not dot_icon.is_dir():
    pass
else:
    try:
        from PIL import Image as _R
    except ImportError:
        print("  skip Pillow not installed — rendered check unavailable")
    else:
        out = Path("/tmp/cp-icon-verify.png")
        r = subprocess.run(
            [str(ICTOOL), str(dot_icon), "--export-image",
             "--output-file", str(out), "--platform", "macOS",
             "--rendition", "Default", "--width", "512", "--height", "512",
             "--scale", "1"],
            capture_output=True, text=True,
        )
        if r.returncode != 0 or not out.is_file():
            check(False, "ictool renders the layered icon", r.stderr.strip()[:200])
        else:
            im = _R.open(out).convert("RGB")
            px = list(im.getdata())
            # Not black. The 0.4.12 icon averaged ~(46,46,47).
            avg = tuple(sum(c[i] for c in px) // len(px) for i in range(3))
            check(sum(avg) > 200,
                  "rendered icon is not near-black",
                  f"mean RGB {avg} — a filtered SVG layer renders black")
            # The brand mark must actually be present. A plate with the
            # block occluded is light, uniform, and passes the test
            # above — so look for terracotta specifically.
            warm = sum(1 for r_, g_, b_ in px
                       if r_ > 120 and r_ - b_ > 45 and r_ - g_ > 18)
            frac = warm / len(px)
            check(frac > 0.05,
                  "the block is visible in the render (brand colour present)",
                  f"only {frac:.1%} warm pixels — group order is top-first; "
                  f"a full-canvas plate listed first occludes the block")

print("\nmacOS .icns")
icns = ICONS / "icon.icns"
if not icns.is_file():
    check(False, "icon.icns exists")
else:
    # NEVER pixel-check an .icns by unpacking it with
    # `iconutil -c iconset`. That direction decodes the legacy
    # `ic04`/`ic05` RLE members badly: a plate of #DCD8D2 comes back
    # as (220, 216, 0) in the trailing rows — blue zeroed — for an
    # icns whose own payloads are byte-perfect. Verified by parsing
    # the container directly (every PNG member clean) and by
    # rendering through ImageIO via `sips`, which is the path macOS
    # itself uses and which produces correct pixels at 16/32/64/128.
    #
    # This cost a real investigation: the corruption is vivid, it
    # appears only at the two smallest sizes, and it looks exactly
    # like a bad flat master. It is the TOOL. `iconutil` is still the
    # right way to enumerate which layers exist, which is all it is
    # used for below.
    _icns_payload_failures = []
    blob = icns.read_bytes()
    if blob[:4] != b"icns":
        check(False, "icon.icns has an icns magic")
    else:
        end = struct.unpack(">I", blob[4:8])[0]
        pos, png_members = 8, 0
        while pos < end:
            ln = struct.unpack(">I", blob[pos + 4:pos + 8])[0]
            if ln < 8:
                break
            member = blob[pos + 8:pos + ln]
            if member[:8] == b"\x89PNG\r\n\x1a\n":
                png_members += 1
                w, h = struct.unpack(">II", member[16:24])
                if w != h:
                    _icns_payload_failures.append(f"{w}x{h}")
            pos += ln
        check(png_members >= 6,
              f"icns carries {png_members} PNG members", "expected >= 6")
        check(not _icns_payload_failures,
              "every icns PNG member is square",
              ", ".join(_icns_payload_failures))
    # `iconutil -o` insists the output path end in `.iconset`; anything
    # else is rejected as an invalid argument.
    unpacked = Path("/tmp/cp-icns-check.iconset")
    subprocess.run(["rm", "-rf", str(unpacked)], check=False)
    out = subprocess.run(
        ["iconutil", "-c", "iconset", str(icns), "-o", str(unpacked)],
        capture_output=True, text=True,
    )
    check(out.returncode == 0, "icon.icns unpacks with iconutil", out.stderr.strip())
    got = {f.name for f in unpacked.glob("*.png")} if out.returncode == 0 else set()
    # The full Apple ladder. 128/256 matter most: those are what the
    # Dock renders at default size on Retina, and the reason this repo
    # does not use `pnpm tauri icon` at all.
    for name in ("icon_16x16.png", "icon_16x16@2x.png", "icon_32x32.png",
                 "icon_32x32@2x.png", "icon_128x128.png", "icon_128x128@2x.png",
                 "icon_256x256.png", "icon_256x256@2x.png", "icon_512x512.png",
                 "icon_512x512@2x.png"):
        check(name in got, f"icns carries {name}")

print("\nWindows .ico")
ico = ICONS / "icon.ico"
if not ico.is_file():
    check(False, "icon.ico exists")
else:
    blob = ico.read_bytes()
    reserved, kind, count = struct.unpack("<HHH", blob[:6])
    check(kind == 1, "ICO type field is 1")
    check(count == 6, f"ICO has 6 layers", f"got {count}")
    # Every layer must be a PNG payload. ImageMagick's default writes
    # raw BMP per layer, which Explorer renders but which bloats the
    # file ~100x — the trap named in regen-icons.sh.
    all_png = True
    for i in range(count):
        off = 6 + 16 * i
        length, data_off = struct.unpack("<II", blob[off + 8:off + 16])
        if blob[data_off:data_off + 8] != b"\x89PNG\r\n\x1a\n":
            all_png = False
    check(all_png, "every ICO layer is a PNG payload (not raw BMP)")
    check(len(blob) < 200_000, "icon.ico under 200 KB",
          f"{len(blob)} bytes — raw-BMP layers would blow past this")

print("\nTray (44x44 = @2x of 22pt)")
for name in ("tray-iconTemplate@2x.png", "tray-iconMono@2x.png",
             "tray-iconAlertTemplate@2x.png", "tray-iconAlertMono@2x.png"):
    p = ICONS / name
    check(p.is_file() and png_size(p) == (44, 44), f"{name} is 44x44",
          f"got {png_size(p) if p.is_file() else 'missing'}")

# How much of the tile the glyph actually INKS, which is what "the tray
# icon looks too small" means. A 44x44 file whose artwork occupies 60%
# of it passes every dimension check and still reads as undersized in
# the menubar — that is exactly how this shipped and had to be caught
# by eye. The window is anchored on the previous icon's measured
# 86%w x 75%h, which looked right for two years.
#
# The upper bound matters as much as the lower: ink touching the tile
# edge renders flush against the menubar with no optical padding.
try:
    from PIL import Image  # noqa: PLC0415 — optional, checked below
except ImportError:
    print("  skip Pillow not installed — tray coverage unchecked")
else:
    for name, lo, hi in (("tray-iconTemplate@2x.png", 0.70, 0.90),
                         ("tray-iconAlertTemplate@2x.png", 0.78, 0.95)):
        p = ICONS / name
        if not p.is_file():
            check(False, f"{name} coverage", "missing")
            continue
        im = Image.open(p).convert("RGBA")
        box = im.getchannel("A").getbbox()
        if box is None:
            check(False, f"{name} has any ink at all", "fully transparent")
            continue
        fw = (box[2] - box[0]) / im.width
        fh = (box[3] - box[1]) / im.height
        check(lo <= fw <= hi and lo <= fh <= hi,
              f"{name} inks {lo:.0%}-{hi:.0%} of the tile",
              f"got {fw:.0%}w x {fh:.0%}h")
        check(box[0] > 0 and box[1] > 0
              and box[2] < im.width and box[3] < im.height,
              f"{name} keeps a margin off the tile edge",
              f"bbox {box} in {im.size}")

print("\nGrain floor")
# Below 48 px the feTurbulence grain reads as dirt, so those rasters
# come from the flat master. Assert the masters exist and differ —
# a copy-paste that pointed both at the same file would silently undo
# the whole rule.
master, flat = ICONS / "icon.svg", ICONS / "icon-flat.svg"
check(master.is_file(), "icon.svg master exists")
check(flat.is_file(), "icon-flat.svg master exists")
if master.is_file() and flat.is_file():
    check(master.read_bytes() != flat.read_bytes(),
          "flat master is not a copy of the grained one")
    check(b"feTurbulence" in master.read_bytes(),
          "grained master actually carries the filter")
    check(b"feTurbulence" not in flat.read_bytes(),
          "flat master carries no filter")

print("\nNo stray `pnpm tauri icon` output")
# Those paths are gitignored precisely so a stray invocation cannot
# re-stage MSIX/iOS/Android dead bytes; if they are present the author
# ran the wrong command and may have overwritten the real assets too.
for stray in ("Square30x30Logo.png", "StoreLogo.png", "ios", "android"):
    check(not (ICONS / stray).exists(), f"no {stray} in src-tauri/icons")

print(f"\n{checks - len(failures)}/{checks} checks passed")
if failures:
    print("FAILED:", ", ".join(failures))
    sys.exit(1)
print("icons ok")
