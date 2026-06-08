//! Main-thread tripwire for AppKit-touching code.
//!
//! macOS asserts the main thread for `NSStatusItem` / `NSWindow`
//! mutation and aborts the process (`SIGTRAP` / `SIGABRT`) when the
//! rule is violated. The abort fires *deferred* — on a later run-loop
//! turn, with misleading frames: the v0.1.4x tray self-quit arc
//! crashed in `-[NSStatusBar _removeStatusItem:]` even though the real
//! culprit was a `set_menu` / `set_icon` on a tokio worker elsewhere.
//!
//! [`warn_if_off_main_thread`] names the real culprit at the call site
//! the moment it runs off-thread, *before* AppKit defers the abort. It
//! logs and never panics — it is a diagnostic, not an enforcement
//! gate, so it can never turn a would-be-logged event into a crash
//! (including in `pnpm tauri dev` debug builds).
//!
//! Call it at the top of every AppKit-touching function (tray apply,
//! `traffic_light::emit`, `dock_icon` override). Routed call sites
//! (`run_on_main_thread` closures) stay silent; a future regression
//! that forgets to route lights up in `claudepot.log` with a
//! backtrace.

/// True when the caller is on the process main thread.
///
/// macOS: `pthread_main_np() != 0` — a cheap thread-local query with no
/// AppKit / objc dependency, fine to call on every AppKit entry.
///
/// **Not for use inside a signal handler.** `pthread_main_np` is a
/// BSD/macOS extension and is NOT on the POSIX async-signal-safe list.
/// The fatal-signal handler in [`crate::diagnostic_logging`] needs a
/// main-thread test too, and deliberately avoids this function —
/// it compares a `pthread_self()` value (which *is* async-signal-safe)
/// captured at install time instead.
#[cfg(target_os = "macos")]
#[inline]
pub fn is_main_thread() -> bool {
    // SAFETY: `pthread_main_np` is a thread-local query with no
    // preconditions.
    unsafe { libc::pthread_main_np() != 0 }
}

/// True when the caller is on the process main thread.
///
/// Off macOS there is no AppKit main-thread rule, so every thread is
/// treated as "main" and the tripwire compiles to a no-op.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn is_main_thread() -> bool {
    true
}

/// Log (at `error`, with a backtrace) if AppKit-touching code is
/// running off the main thread. No-op when already on the main thread,
/// and on every non-macOS platform.
///
/// `tag` identifies the call site (`"tray::rebuild"`,
/// `"traffic_light::emit"`). It becomes a structured field so the
/// offending function is greppable in `claudepot.log`.
///
/// Never panics. The whole point is to surface a latent crash class in
/// the log without itself becoming a failure mode.
#[inline]
pub fn warn_if_off_main_thread(tag: &str) {
    if is_main_thread() {
        return;
    }
    let backtrace = std::backtrace::Backtrace::force_capture();
    let current = std::thread::current();
    let thread_name = current.name().unwrap_or("<unnamed>");
    tracing::error!(
        target: "claudepot_main_thread",
        tag,
        thread = thread_name,
        %backtrace,
        "AppKit-touching code invoked off the main thread — this aborts the process on macOS; route it through run_on_main_thread"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warn_if_off_main_thread_never_panics() {
        // Whatever thread the test harness runs on, the call must
        // return without panicking — it is a log-only diagnostic.
        // (On macOS this thread is not the main thread, so the error
        // branch executes; on other platforms `is_main_thread` is
        // const-true and the early return is taken. Both must be
        // panic-free.)
        warn_if_off_main_thread("test::probe");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn is_main_thread_is_true_off_macos() {
        assert!(is_main_thread());
    }
}
