//! Override the macOS Dock icon at runtime so Cocoa's NSImage
//! pipeline handles all downsampling instead of the legacy .icns
//! IconServices path.
//!
//! ## Why this exists
//!
//! macOS Dock at default size renders icons at 48pt = 96 raster
//! pixels on Retina. The `.icns` format only stores layers at the
//! standard ladder (16/32/64/128/256/512/1024), so macOS picks the
//! 128 layer and downscales 128 → 96 with bilinear filtering. For
//! pixel-art icons this softens the edges visibly — no amount of
//! making the .icns layers crisper can fix it because the
//! downsample step always runs.
//!
//! Routing the icon through `NSImage.initWithData` of our 512×512
//! PNG lets Cocoa pick Lanczos (or equivalent) for any Dock size's
//! downsample step, preserving crispness at non-128 render sizes.
//!
//! Tauri's runtime makes the same call in dev mode (the
//! `#[cfg(all(dev, target_os = "macos"))]` block in
//! `crates/tauri/src/app.rs`) but not in prod. We replicate it
//! ourselves in `setup()` so the prod Dock matches dev's quality.
//!
//! Earlier history: v0.1.16 tried the same trick with the 32×32
//! source that Tauri's codegen embeds into `app_handle.manager.app_icon`
//! — that produced visibly blocky output because 32 → 96 is an
//! UPSCALE that loses too much. Using the 512 source means every
//! Dock size is a downsample (or 1:1), which is the regime where
//! Cocoa's filtering looks best.
//!
//! The `cfg(target_os = "macos")` gate lives on the `mod dock_icon`
//! declaration in `lib.rs`; the duplicate inner attribute that used
//! to sit here was redundant (clippy's `duplicated_attributes` lint
//! caught it in Rust 1.92).

// Everything below the release no-op is debug-only, imports and the
// embedded PNG included. Without these gates a release build carries a
// 728 KB `include_bytes!` it can never reach, and warns about five
// unused imports on top.
#[cfg(debug_assertions)]
use objc2::{rc::Retained, AllocAnyThread, MainThreadMarker};
#[cfg(debug_assertions)]
use objc2_app_kit::{NSApplication, NSImage};
#[cfg(debug_assertions)]
use objc2_foundation::NSData;

/// Embedded 1024×1024 source, and deliberately **not** the bundle's
/// `icon.png`.
///
/// The two paths are inverses of each other, which is the classic way
/// to get a macOS icon wrong:
///
/// | Path | What it wants |
/// |---|---|
/// | bundle `.icns` | **full bleed** — the system applies the squircle mask, the inset and the shadow |
/// | `setApplicationIconImage` (here) | **everything already applied** — drawn verbatim at the slot size |
///
/// So this file carries the squircle and the inset itself: the artwork
/// occupies 824/1024 = 0.805 of the canvas, Apple's measured tile
/// fraction, with a superellipse corner rather than a circular arc.
/// `scripts/regen-icons.sh` builds it; `scripts/verify-icons.py`
/// asserts the geometry.
///
/// The previous artwork hid the distinction by baking a squircle into
/// the SVG at 416/512 of the canvas, so the same file happened to work
/// in both roles. The current icon set is full-bleed by design — right
/// for the bundle — and handing it here unmodified renders a
/// hard-edged square about 22% larger than every neighbouring Dock
/// icon.
///
/// 1024 rather than 512 because every Dock-relevant size (96, 128,
/// 144, 160, 256) is then a downsample with plenty of pixels to filter
/// from, never an upscale.
#[cfg(debug_assertions)]
const DOCK_ICON_PNG: &[u8] = include_bytes!("../icons/icon-dock.png");

/// Override the application icon on the main thread.
///
/// # Debug builds only — and that is the whole point
///
/// A packaged app gets its Dock icon from the bundle: macOS 26 from
/// `Contents/Resources/Assets.car` (the compiled `Claudepot.icon`,
/// which is what earns the Liquid Glass treatment), and macOS 25 and
/// earlier from `icon.icns`. `setApplicationIconImage` **replaces**
/// whatever that produced with a flat bitmap, so leaving it on in
/// release would throw away the glass rendering the layered icon
/// exists to get — the system can only light an icon that still has
/// separable layers at composite time, and a PNG handed to this API
/// has none.
///
/// It stays in debug because an unbundled `cargo run` binary has no
/// bundle at all: no `Assets.car`, no `.icns`, no `Info.plist`. The
/// Dock shows the generic `exec` tile, and this is the only thing that
/// replaces it.
///
/// The cost is real and worth naming: pre-26 release builds lose the
/// crispness fix this function was originally written for (the `.icns`
/// 128 layer downscaled bilinearly at the Dock's default 96 px). They
/// fall back to the full `.icns` ladder, which is good but not
/// Lanczos-from-512. Restoring it means gating on the OS version
/// rather than the build profile — do that if pre-26 softness is worth
/// the extra branch, but never re-enable it unconditionally.
#[cfg(debug_assertions)]
pub fn override_application_icon() {
    // `MainThreadMarker::new_unchecked` + `setApplicationIconImage` are
    // main-thread-only; off the main thread they are UB. `setup()` runs
    // on the main thread, so this guard never trips in practice — but if
    // that assumption ever breaks, log loudly and skip rather than
    // corrupt or abort silently. The early return is what makes the
    // `new_unchecked` below sound.
    if !claudepot_core::main_thread::is_main_thread() {
        claudepot_core::main_thread::warn_if_off_main_thread(
            "dock_icon::override_application_icon",
        );
        return;
    }
    // SAFETY: established above that we are on the main thread; same
    // unchecked-constructor pattern as Tauri's dev-mode path.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let app = NSApplication::sharedApplication(mtm);

    let data = NSData::with_bytes(DOCK_ICON_PNG);
    let Some(image): Option<Retained<NSImage>> = NSImage::initWithData(NSImage::alloc(), &data)
    else {
        // Fall through silently — the .icns fallback still renders
        // *something*, and a panic on cold launch is much worse
        // than a blurry icon.
        tracing::warn!("dock_icon: failed to decode embedded icon bytes");
        return;
    };

    // SAFETY: valid `NSApplication` handle on main thread; passing
    // an `NSImage` is the documented contract.
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

/// Release builds: the bundle owns the Dock icon. See the debug
/// variant above for why overriding it would be a regression rather
/// than an improvement.
///
/// A no-op function rather than `#[cfg]` at each call site, so callers
/// stay readable and a new one cannot accidentally reintroduce the
/// override in release by forgetting the attribute.
#[cfg(not(debug_assertions))]
pub fn override_application_icon() {}
