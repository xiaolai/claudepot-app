//! Force the macOS Dock icon to render via Cocoa's `NSImage` pipeline
//! instead of the legacy IconServices `.icns` path.
//!
//! ## Why this exists
//!
//! Tauri's runtime calls `NSApplication::setApplicationIconImage` in
//! dev mode (see the `#[cfg(all(dev, target_os = "macos"))]` block in
//! `crates/tauri/src/app.rs`) but **not** in prod. In prod, macOS
//! falls back to picking a baked-in layer from `Contents/Resources/icon.icns`
//! and rendering it through the legacy IconServices pipeline.
//!
//! For pixel-art icons whose grid doesn't divide evenly into the
//! Apple icon-size ladder (16/32/64/128/256/512/1024), the .icns
//! layers have anti-aliased rect edges at sub-pixel positions, which
//! IconServices renders softly at the Dock's display size. NSImage
//! rendering of a single source PNG via Cocoa picks integer-ratio
//! upscaling more aggressively, preserving the crisp pixel-art look.
//!
//! Calling `setApplicationIconImage` ourselves at startup mirrors
//! Tauri's dev-mode behavior in prod, fixing the visible blur. The
//! source PNG bytes are baked in at compile time via `include_bytes!`.

#![cfg(target_os = "macos")]

use objc2::{rc::Retained, AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

/// Embedded copy of the application icon. We deliberately use the
/// 32×32 PNG (the same one Tauri's codegen embeds into
/// `app_handle.manager.app_icon` from the first `bundle.icon` entry
/// for non-Windows targets) so the pipeline matches dev mode
/// exactly — Cocoa upscales 32→{Dock size} with integer-ratio
/// scaling that preserves the pixel-art crispness.
const DOCK_ICON_PNG: &[u8] = include_bytes!("../icons/32x32.png");

/// Override the application icon on the main thread. Call once
/// during `setup()` while you still hold the main-thread context.
///
/// Safe no-op if the icon bytes fail to decode (returns silently
/// rather than panicking — losing the override is much better than
/// a crash on cold launch).
pub fn override_application_icon() {
    // SAFETY: `setup()` callbacks run on the main thread. The
    // unchecked constructor is the same pattern Tauri's dev-mode
    // path uses (see `crates/tauri/src/app.rs`).
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let app = NSApplication::sharedApplication(mtm);

    let data = NSData::with_bytes(DOCK_ICON_PNG);
    let Some(image): Option<Retained<NSImage>> =
        NSImage::initWithData(NSImage::alloc(), &data)
    else {
        // PNG bytes failed to decode. Fall through silently — the
        // .icns fallback will still render *something*.
        tracing::warn!("dock_icon: failed to decode embedded icon bytes");
        return;
    };

    // SAFETY: `app` is a valid `NSApplication` handle obtained on
    // the main thread; passing an `NSImage` is the documented
    // contract for this method.
    unsafe { app.setApplicationIconImage(Some(&image)) };
}
