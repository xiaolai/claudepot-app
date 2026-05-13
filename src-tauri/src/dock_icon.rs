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

use objc2::{rc::Retained, AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

/// Embedded 512×512 source. Big enough that every Dock-relevant
/// size (96, 128, 144, 160, 256) is a downsample with plenty of
/// pixels to filter from, not an upscale.
const DOCK_ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

/// Override the application icon on the main thread. Call once
/// during `setup()`.
pub fn override_application_icon() {
    // SAFETY: `setup()` callbacks run on the main thread. Same
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
