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

use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};

/// Build the production `RollingFileAppender` that the Tauri shell
/// uses to write the rolled-daily diagnostic log. Daily rotation,
/// 7-day retention via `max_log_files` (replaces an earlier custom
/// startup-pass), and a `claudepot.log` symlink that always points
/// at today's dated file — the CLI's `claudepot logs --tail` and
/// any external `tail -f` rely on that symlink being stable across
/// midnight rollovers.
///
/// Returns `InitError` on a read-only `log_dir`, a permission
/// failure, or a symlink-creation failure. The caller chooses
/// whether that's fatal (panic) or recoverable (drop to
/// stderr-only logging); the Tauri shell takes the recoverable
/// path.
pub fn build_file_appender(log_dir: &Path) -> Result<RollingFileAppender, InitError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("claudepot.log")
        .max_log_files(7)
        .latest_symlink("claudepot.log")
        .build(log_dir)
}

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

    /// Direct test of the re-entry guard pattern used inside the
    /// real hook. The hook's `IN_HOOK` cell is private to its
    /// closure (no external way to call the body twice), so we
    /// exercise the same `Cell<bool>` mechanism here against a
    /// self-recursing body and assert the inner call is skipped.
    /// Loose binding to the hook's wording, tight binding to its
    /// semantics — if this test stops passing, the guard pattern
    /// in `install_panic_hook` is broken in the same way.
    #[test]
    fn reentry_guard_pattern_skips_nested_invocation() {
        thread_local! {
            static IN_HOOK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
            static CALLS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        }

        fn body() {
            if IN_HOOK.with(|f| f.replace(true)) {
                // Re-entry: defer.
                return;
            }
            CALLS.with(|c| c.set(c.get() + 1));
            body();
            IN_HOOK.with(|f| f.set(false));
        }

        CALLS.with(|c| c.set(0));
        IN_HOOK.with(|f| f.set(false));
        body();
        assert_eq!(
            CALLS.with(|c| c.get()),
            1,
            "the recursive call should have been blocked by the guard"
        );
        assert!(
            !IN_HOOK.with(|f| f.get()),
            "the guard must be released after the outer call returns"
        );
    }

    #[test]
    fn build_file_appender_creates_symlink_and_dated_file() {
        // Verify that our exact production builder args produce
        // the on-disk layout the rest of the surface depends on:
        // a `claudepot.log` symlink (followed by `claudepot logs
        // --tail` and any external `tail -f`) and at least one
        // `claudepot.log.YYYY-MM-DD` dated file. This catches a
        // typo in the builder args (wrong prefix, missing
        // `latest_symlink` call) without requiring a `pnpm tauri
        // dev` launch.
        let tmp = tempfile::tempdir().unwrap();
        let _appender = build_file_appender(tmp.path()).expect("builder should succeed");

        let symlink_path = tmp.path().join("claudepot.log");
        assert!(
            symlink_path.exists(),
            "claudepot.log symlink should exist immediately after builder build"
        );

        let names: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("claudepot.log.") && n.len() > "claudepot.log.".len()),
            "expected at least one dated file, got: {names:?}"
        );
    }
}
