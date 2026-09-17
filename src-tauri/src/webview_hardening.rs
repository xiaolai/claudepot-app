//! Release-only WebView2 hardening the renderer cannot do for itself.
//!
//! `useWebviewChromeGuard` cancels the reload keys from the page, and on
//! macOS that is the whole story. On WebView2, F5 / Ctrl+R / Ctrl+Shift+R
//! are **browser accelerator keys**: handled by the browser, on by default,
//! and not something a page is guaranteed to be able to cancel. WebView2's
//! own switch for them is `ICoreWebView2Settings3::
//! SetAreBrowserAcceleratorKeysEnabled`. wry calls it when asked
//! (`with_browser_accelerator_keys(false)`); Tauri 2.11 never asks, so this
//! reaches the settings through `with_webview`.
//!
//! What the switch turns off, per Microsoft's documentation: reload, find,
//! print, zoom, dev tools and history navigation. It leaves editing keys
//! (copy, paste, undo, select all) alone — and the app's own ⌘F / ⌘R
//! handlers keep working, because the keys now reach the page instead of
//! the browser.
//!
//! Development builds are left alone: reload and dev tools are the loop.
//! A failure is logged, not fatal — the renderer guard still stands.

use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller, ICoreWebView2Settings3,
};
use windows_core::Interface;

/// Turn off WebView2's browser accelerator keys for `window`.
pub fn disable_browser_accelerator_keys(window: &tauri::WebviewWindow) {
    let scheduled = window.with_webview(|webview| {
        if let Err(e) = apply(&webview.controller()) {
            tracing::warn!(error = %e, "could not disable WebView2 browser accelerator keys");
        }
    });
    if let Err(e) = scheduled {
        tracing::warn!(error = %e, "could not reach the WebView2 controller");
    }
}

fn apply(controller: &ICoreWebView2Controller) -> windows_core::Result<()> {
    // SAFETY: plain COM calls on a live controller, made from the closure
    // Tauri runs on the thread that owns the webview.
    unsafe {
        controller
            .CoreWebView2()?
            .Settings()?
            .cast::<ICoreWebView2Settings3>()?
            .SetAreBrowserAcceleratorKeysEnabled(false)
    }
}
