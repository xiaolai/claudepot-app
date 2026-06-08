//! End-to-end test of the fatal-signal crash handler
//! (`diagnostic_logging::install_signal_handler`).
//!
//! A fatal signal kills the process, so the install → record → re-raise
//! path can't be exercised in-process. This test re-execs the test
//! binary as a child: the child installs the handler against a temp log
//! dir and raises `SIGABRT`; the parent asserts the child was terminated
//! by the re-raised `SIGABRT` and that `crash.log` carries the record.
//!
//! Unix-only — the signal handler is `#[cfg(unix)]`. (On macOS the
//! aborting child also produces a `.ips` under DiagnosticReports, but
//! its process name is the test binary's, not `claudepot-tauri`, so it
//! never matches the harvest filter.)
#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::path::Path;

/// When set, the test runs in "child" mode: install the handler against
/// this dir and abort.
const CHILD_ENV: &str = "CLAUDEPOT_SIGNAL_HANDLER_CHILD_DIR";

#[test]
fn signal_handler_records_crash_and_reraises() {
    if let Ok(dir) = std::env::var(CHILD_ENV) {
        // ---- child ----
        claudepot_core::diagnostic_logging::install_signal_handler(Path::new(&dir));
        // SAFETY: deliberately raising a fatal signal to drive the
        // handler. The process is expected to die on the re-raise.
        unsafe {
            libc::raise(libc::SIGABRT);
        }
        // Unreachable unless the handler swallowed the signal — which
        // would itself be a bug, so make the child exit cleanly and let
        // the parent's signal assertion fail loudly.
        std::process::exit(0);
    }

    // ---- parent ----
    let tmp = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().expect("test exe path");
    let output = std::process::Command::new(exe)
        .args([
            "signal_handler_records_crash_and_reraises",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, tmp.path())
        .output()
        .expect("spawn child test process");

    assert_eq!(
        output.status.signal(),
        Some(libc::SIGABRT),
        "child must be terminated by the re-raised SIGABRT, not exit cleanly; \
         stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let crash_log = std::fs::read_to_string(tmp.path().join("crash.log"))
        .expect("handler should have written crash.log before re-raising");
    assert!(
        crash_log.contains("=== claudepot crash ==="),
        "crash.log must carry the record marker: {crash_log:?}"
    );
    assert!(
        crash_log.contains("SIGABRT"),
        "crash.log must name the signal: {crash_log:?}"
    );
}
