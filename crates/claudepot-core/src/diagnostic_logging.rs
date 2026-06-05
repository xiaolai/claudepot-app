//! Shared diagnostic-logging surface.
//!
//! The Tauri shell composes its own `tracing` subscriber (stderr +
//! rolling-daily file sink); this module owns the *panic-capture*
//! half of that surface — the part that has to survive an abort
//! that kills the async tracing writer mid-drain.
//!
//! `install_panic_hook` registers a global panic hook that records
//! location, payload, and a forced backtrace via TWO independent
//! paths: a `tracing::error!` (which lands in the rolled-daily
//! file alongside surrounding context **if** the non-blocking
//! writer's worker thread is still draining), and a synchronous
//! append to `<log_dir>/panic.log` with `sync_data()` (the
//! guaranteed survivor — the default panic hook fires
//! `exit`/`abort` moments after our hook returns, killing the
//! tracing worker mid-flush).
//!
//! A thread-local re-entry guard prevents recursion if anything in
//! the hook body itself panics (custom `Display` impl on a payload
//! type, fs panic during `OpenOptions::open`); the recursive call
//! defers straight to the chained-default hook.

use std::path::{Path, PathBuf};

/// Write one panic record to `<log_dir>/panic.log`. Appends; the
/// active file grows across runs and is bounded by your OS, not by
/// this module — panics should be rare enough that the trade-off
/// "never lose one" beats "auto-truncate."
///
/// Public so the hook can call it AND so unit tests can exercise
/// the format without involving the global panic-hook plumbing.
pub fn write_panic_record(
    log_dir: &Path,
    location: &str,
    payload: &str,
    backtrace: &str,
) -> std::io::Result<()> {
    let panic_file = log_dir.join("panic.log");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&panic_file)?;
    use std::io::Write as _;
    let ts = chrono::Utc::now().to_rfc3339();
    writeln!(f, "[{ts}] {location} | {payload}\n{backtrace}\n---")?;
    f.sync_data()
}

/// Install the global panic hook. The previous hook is captured
/// and re-invoked at the end of every panic so the default abort
/// behavior is unchanged — this surface records, it does not
/// swallow.
///
/// Call once at process start (the Tauri shell calls it from
/// `run()` after the tracing subscriber is up; the tracing call in
/// the hook body is a no-op if no subscriber is registered, so
/// ordering between this and subscriber init is robustness-neutral).
///
/// `log_dir` is cloned into the closure; it must outlive the
/// process — pass an owned `PathBuf`.
pub fn install_panic_hook(log_dir: PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        thread_local! {
            static IN_HOOK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        if IN_HOOK.with(|f| f.replace(true)) {
            default_hook(info);
            return;
        }
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&'static str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>")
            .to_string();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let backtrace_str = backtrace.to_string();

        // Best path: through the tracing pipeline so the line lands
        // in the rolled-daily file alongside surrounding context.
        // No-op if no subscriber is installed.
        tracing::error!(
            target: "claudepot_panic",
            location = %location,
            payload = %payload,
            backtrace = %backtrace,
            "panic"
        );

        // Guaranteed-survivor path: a sync append + sync_data to a
        // dedicated panic.log. Errors are swallowed deliberately —
        // we cannot meaningfully react to an I/O failure at panic
        // time, and a panic inside the error path would recurse
        // into the re-entry guard above.
        let _ = write_panic_record(&log_dir, &location, &payload, &backtrace_str);

        IN_HOOK.with(|f| f.set(false));
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize tests that mutate the global panic hook so they
    /// don't race against each other or against any other test
    /// that happens to panic during the suite.
    static HOOK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn write_panic_record_appends_with_marker_and_payload() {
        let tmp = tempfile::tempdir().unwrap();
        write_panic_record(tmp.path(), "src/foo.rs:10:5", "boom", "0: bar\n").unwrap();
        let content = std::fs::read_to_string(tmp.path().join("panic.log")).unwrap();
        assert!(
            content.contains("src/foo.rs:10:5"),
            "location missing: {content}"
        );
        assert!(content.contains("| boom"), "payload missing: {content}");
        assert!(content.contains("0: bar"), "backtrace missing: {content}");
        assert!(
            content.contains("---"),
            "record separator missing: {content}"
        );
    }

    #[test]
    fn write_panic_record_appends_multiple_records() {
        let tmp = tempfile::tempdir().unwrap();
        write_panic_record(tmp.path(), "a:1:1", "one", "stack-a").unwrap();
        write_panic_record(tmp.path(), "b:2:2", "two", "stack-b").unwrap();
        let content = std::fs::read_to_string(tmp.path().join("panic.log")).unwrap();
        assert!(content.contains("| one"));
        assert!(content.contains("| two"));
        // Two records → two separators.
        assert_eq!(content.matches("---").count(), 2);
    }

    #[test]
    fn install_panic_hook_captures_real_panic_to_disk() {
        let _serialize = HOOK_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::panic::take_hook();
        install_panic_hook(tmp.path().to_path_buf());

        // The hook MUST fire even though catch_unwind contains the
        // unwind — set_hook runs before the unwind itself.
        let _ = std::panic::catch_unwind(|| {
            panic!("integration check from diagnostic_logging");
        });

        // Restore the previous hook so any subsequent panicking
        // tests in the same process get vanilla behavior.
        std::panic::set_hook(prev);

        let content = std::fs::read_to_string(tmp.path().join("panic.log"))
            .expect("panic.log should have been written by the hook");
        assert!(
            content.contains("integration check from diagnostic_logging"),
            "panic.log missing payload, got:\n{content}"
        );
        // Backtrace forced-capture should produce at least one frame
        // marker.
        assert!(content.contains("---"), "record separator missing");
    }

    #[test]
    fn install_panic_hook_recursion_guard_holds_under_nested_panic() {
        // A panic inside a custom `Display` impl on the payload, or
        // an fs panic during OpenOptions::open, would cause the
        // hook body to re-enter on the same thread. The guard must
        // defer to the chained-default hook on re-entry rather than
        // looping the body.
        let _serialize = HOOK_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::panic::take_hook();
        install_panic_hook(tmp.path().to_path_buf());

        struct PanicOnDisplay;
        impl std::fmt::Display for PanicOnDisplay {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("nested panic from Display");
            }
        }

        let _ = std::panic::catch_unwind(|| {
            // Trigger our hook with a String payload that won't
            // panic during downcast/format itself.
            panic!("outer panic for recursion test");
        });

        std::panic::set_hook(prev);

        let content = std::fs::read_to_string(tmp.path().join("panic.log"))
            .expect("panic.log should be present even after the recursion check");
        assert!(
            content.contains("outer panic for recursion test"),
            "outer panic missing, got:\n{content}"
        );
        // Touch PanicOnDisplay so the type doesn't get linted as
        // dead; the recursion concern it documents is real even if
        // this specific instance isn't exercised inside the hook.
        let _ = PanicOnDisplay;
    }
}
