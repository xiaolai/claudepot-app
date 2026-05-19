//! Runtime introspection of the macOS traffic-light position so the
//! webview chrome can align text to where the OS actually placed the
//! buttons — not where we *think* `trafficLightPosition.y` puts them.
//!
//! ## Why this exists
//!
//! AppKit does NOT center the close/zoom/minimize buttons at the
//! chrome's geometric center. The actual visible center depends on
//! the macOS version, the button's reported height (14px on modern
//! macOS, 16px on older), the configured `trafficLightPosition.y`,
//! and AppKit's autoresizing of the standard window-button row.
//!
//! Hardcoded compensation breaks every time any of those inputs
//! shifts. History on this repo:
//! - April 20: y=14 was "optical centre" for a 38px chrome.
//! - April 20 later: y=21 was the "measured centre" via screenshot.
//! - 0.1.9 release: bumped to y=22.
//! - May 2026 (Tauri 2.11 era): the user observes misalignment again,
//!   meaning the magic number drifted yet again.
//!
//! Fix: read the real `NSWindow.standardWindowButton(.closeButton)`
//! frame at runtime, emit `traffic-light-metrics`, and let the
//! frontend translate chrome content onto that line via a CSS
//! custom property. The webview no longer encodes any AppKit
//! geometry assumption.
//!
//! Detailed recipe lives in `~/.claude/skills/tauri/SKILL.md` under
//! "Vertical alignment: introspect at runtime, don't hardcode".

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSWindow, NSWindowButton};
use serde::Serialize;
use tauri::{Emitter, Runtime, WebviewWindow};

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TrafficLightMetrics {
    /// Center of the close button, logical CSS px, top-left origin.
    pub center_x: f64,
    pub center_y: f64,
    /// Right edge of the traffic-light cluster (close → zoom span)
    /// in logical CSS px from the window's left edge. The chrome's
    /// `--chrome-inset-left` reads this to size the gap before the
    /// breadcrumb.
    pub right: f64,
    pub width: f64,
    pub height: f64,
}

#[cfg(target_os = "macos")]
fn compute<R: Runtime>(win: &WebviewWindow<R>) -> Option<TrafficLightMetrics> {
    let raw = win.ns_window().ok()?;
    if raw.is_null() {
        return None;
    }
    // SAFETY: Tauri owns the NSWindow retain; the pointer is valid
    // for this synchronous call. The reference does not outlive this
    // function.
    let ns_window: &NSWindow = unsafe { &*(raw as *const NSWindow) };

    // objc2-app-kit 0.3 marks these accessors as safe; the binding
    // wraps msg_send! with the appropriate retain semantics.
    let close = ns_window.standardWindowButton(NSWindowButton::CloseButton)?;
    let zoom = ns_window.standardWindowButton(NSWindowButton::ZoomButton);

    // `close.frame()` is in its superview's coordinate system (the
    // titlebar container view), NOT the window's. Convert through
    // `convertRect:toView:nil` so we get window-base coords in
    // NS bottom-left orientation, then flip to top-left for CSS.
    let close_bounds = close.bounds();
    let in_window = close.convertRect_toView(close_bounds, None);
    let win_h = ns_window.frame().size.height;
    let center_y = win_h - in_window.origin.y - in_window.size.height / 2.0;

    let right = if let Some(z) = zoom {
        let zb = z.bounds();
        let zf = z.convertRect_toView(zb, None);
        zf.origin.x + zf.size.width
    } else {
        in_window.origin.x + in_window.size.width
    };

    Some(TrafficLightMetrics {
        center_x: in_window.origin.x + in_window.size.width / 2.0,
        center_y,
        right,
        width: in_window.size.width,
        height: in_window.size.height,
    })
}

#[cfg(not(target_os = "macos"))]
fn compute<R: Runtime>(_win: &WebviewWindow<R>) -> Option<TrafficLightMetrics> {
    // Non-macOS platforms draw their own chrome decorations or use a
    // custom titlebar without traffic lights. The frontend keeps
    // using its CSS fallbacks (chrome geometric center) when the
    // metrics are absent.
    None
}

/// IPC command for cold-mount pulls. The renderer subscribes to the
/// `traffic-light-metrics` event first, then invokes this once in case
/// the boot-time emit fired before the listener was attached.
#[tauri::command]
pub fn traffic_light_metrics<R: Runtime>(window: WebviewWindow<R>) -> Option<TrafficLightMetrics> {
    compute(&window)
}

/// Emit the current metrics to the renderer if they can be computed.
/// No-op on non-macOS and on platforms where the NSWindow pointer is
/// not available (e.g. headless test runners).
///
/// **Must be called from the main thread on macOS.** `compute` calls
/// into AppKit (`NSWindow.standardWindowButton`, `convertRect:toView:`,
/// `frame`); those methods assert the main thread and crash the process
/// with `*** Assertion failure ... must be called from the main thread`
/// if invoked from a tokio worker or any other secondary thread. Call
/// sites that may not be on the main thread should use
/// [`emit_on_main_thread`] instead, which routes through
/// `WebviewWindow::run_on_main_thread`.
pub fn emit<R: Runtime>(window: &WebviewWindow<R>) {
    if let Some(m) = compute(window) {
        let _ = window.emit("traffic-light-metrics", m);
    }
}

/// Main-thread-safe wrapper around [`emit`]. Schedules the AppKit-
/// touching body via `WebviewWindow::run_on_main_thread`, so callers in
/// async / tokio-worker contexts can dispatch a metrics emit without
/// crashing on the AppKit main-thread assertion.
///
/// Fires-and-forgets: any error from `run_on_main_thread` is logged at
/// `warn` and otherwise ignored, mirroring `emit`'s `let _ = …` for the
/// emit-error path.
pub fn emit_on_main_thread<R: Runtime>(window: &WebviewWindow<R>) {
    let win = window.clone();
    if let Err(e) = window.run_on_main_thread(move || emit(&win)) {
        tracing::warn!("traffic_light::emit_on_main_thread: dispatch failed: {e}");
    }
}
