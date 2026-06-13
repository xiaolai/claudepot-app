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

/// Name of the symlink [`build_file_appender`] maintains pointing at
/// today's dated file. The single owner of this contract — consumers
/// (`claudepot logs --tail`, the GUI's "Reveal logs") resolve through
/// [`resolve_active_log`] instead of hardcoding the name.
const ACTIVE_LOG_SYMLINK: &str = "claudepot.log";

/// Resolve [`crate::paths::log_dir`] and create it if missing, so
/// consumers (e.g. `claudepot logs --open`) land on a real directory
/// even before the GUI has ever booted. Deliberately does NOT
/// pre-create the log file or symlink — that would shadow the GUI's
/// later symlink creation if a CLI consumer ran first.
pub fn ensure_log_dir() -> std::io::Result<std::path::PathBuf> {
    let dir = crate::paths::log_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Find the active log file to follow. Preferred path: the
/// `claudepot.log` symlink the appender maintains. Fallback: if the
/// symlink hasn't been created yet (older GUI build, manual
/// deletion), pick the lexically-latest `claudepot.log.YYYY-MM-DD`
/// file in the directory. `None` means nothing exists to tail.
///
/// Lives here — next to [`build_file_appender`] — so the rotation
/// naming and the symlink contract have exactly one owner.
pub fn resolve_active_log(dir: &Path) -> Option<std::path::PathBuf> {
    let symlink = dir.join(ACTIVE_LOG_SYMLINK);
    // `exists()` follows symlinks, so this returns true when the
    // symlink points to a real file. We accept either a real file
    // or a working symlink.
    if symlink.exists() {
        return Some(symlink);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut candidates: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            // The rolled files are `claudepot.log.YYYY-MM-DD`.
            // Lexical sort on this prefix is also chronological,
            // so the lexically-latest entry is today's active file.
            if name.starts_with("claudepot.log.") {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    candidates.sort();
    candidates.pop()
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

/// Install handlers for the fatal signals that bypass the Rust panic
/// hook — `SIGSEGV`, `SIGABRT`, `SIGBUS`, `SIGILL`, `SIGFPE`,
/// `SIGTRAP`. These fire on foreign-code aborts (an AppKit assertion,
/// an Obj-C exception, a fault inside an FFI dependency) that never
/// reach `panic!`, so [`install_panic_hook`] is blind to them — the
/// v0.1.4x tray self-quits were an AppKit `abort()` and left
/// `panic.log` empty; only the OS `.ips` report caught them.
///
/// The handler is strictly async-signal-safe: it appends ONE
/// pre-formatted line — signal, main-thread-ness, pid, epoch — to a
/// pre-opened `<log_dir>/crash.log` via raw `write`/`fsync`, with no
/// heap allocation, lock, or backtrace. The authoritative symbolicated
/// backtrace comes from the OS `.ips` report on macOS (see
/// [`crate::crash_reports`]); the running `tracing` file sink holds the
/// breadcrumbs up to the crash. After writing, the handler restores the
/// default disposition and re-raises, so the OS still produces its
/// crash report and the process terminates normally.
///
/// Idempotent — safe to call once at process start. A re-entry guard
/// keeps a fault *inside* the handler from looping. The handler runs on
/// a dedicated alternate signal stack so a stack-overflow `SIGSEGV` is
/// still captured.
#[cfg(unix)]
pub fn install_signal_handler(log_dir: &Path) {
    signal_capture::install(log_dir);
}

/// No-op off Unix. Windows crash capture wants a Vectored Exception
/// Handler / `SetUnhandledExceptionFilter` — tracked as a follow-up. On
/// macOS the `.ips` harvest ([`crate::crash_reports`]) is the primary
/// crash record and this signal handler is the belt-and-suspenders; on
/// Linux the signal handler is the only crash record.
#[cfg(not(unix))]
pub fn install_signal_handler(_log_dir: &Path) {}

#[cfg(unix)]
mod signal_capture {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

    /// Fatal signals we capture: synchronous faults plus the
    /// abort/trap pair AppKit assertions raise.
    const FATAL: [libc::c_int; 6] = [
        libc::SIGSEGV,
        libc::SIGABRT,
        libc::SIGBUS,
        libc::SIGILL,
        libc::SIGFPE,
        libc::SIGTRAP,
    ];

    /// fd of the pre-opened `crash.log`, or `-1` before install.
    /// Opening a file inside the handler would not be
    /// async-signal-safe, so we open once and stash the descriptor.
    static CRASH_FD: AtomicI32 = AtomicI32::new(-1);
    /// Re-entry guard — a fault inside the handler must not recurse.
    static IN_HANDLER: AtomicBool = AtomicBool::new(false);
    /// Raw `pthread_self()` of the thread that called [`install`],
    /// captured once. The handler compares against this instead of
    /// calling `pthread_main_np` — that BSD/macOS extension is NOT on
    /// the POSIX async-signal-safe list, whereas `pthread_self` is. We
    /// install from the process main thread (Tauri `setup()` /
    /// `run()`), so "== this id" means "on the main thread". Stored as
    /// `usize` because `pthread_t` is a pointer on macOS and a
    /// `c_ulong` on Linux — both cast losslessly to `usize`. `0` is the
    /// not-yet-captured sentinel (no real `pthread_t` is null).
    static MAIN_THREAD_ID: AtomicUsize = AtomicUsize::new(0);

    pub(super) fn install(log_dir: &Path) {
        use std::os::unix::ffi::OsStrExt;

        if CRASH_FD.load(Ordering::SeqCst) >= 0 {
            return; // already installed
        }

        // Capture the installing thread's id up front (we run on the
        // main thread). The handler reads this — see MAIN_THREAD_ID.
        // SAFETY: `pthread_self` is a side-effect-free thread-local
        // query with no preconditions.
        MAIN_THREAD_ID.store(unsafe { libc::pthread_self() } as usize, Ordering::SeqCst);

        let path = log_dir.join("crash.log");
        let cpath = match std::ffi::CString::new(path.as_os_str().as_bytes()) {
            Ok(c) => c,
            Err(_) => {
                tracing::warn!("signal handler: crash.log path contains NUL; capture disabled");
                return;
            }
        };
        // SAFETY: open(2) with a valid C string and standard flags.
        let fd = unsafe {
            libc::open(
                cpath.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                0o600,
            )
        };
        if fd < 0 {
            tracing::warn!("signal handler: cannot open crash.log; capture disabled");
            return;
        }
        CRASH_FD.store(fd, Ordering::SeqCst);

        // Only set SA_ONSTACK if the alternate stack actually
        // registered — otherwise the kernel would try to run the
        // handler on a stack that doesn't exist.
        let on_alt_stack = install_alt_stack();

        let mut installed = 0u32;
        for &sig in &FATAL {
            // SAFETY: a well-formed sigaction with our extern "C"
            // handler. SA_ONSTACK runs it on the alternate stack (so a
            // stack-overflow SIGSEGV is still catchable) when that stack
            // registered; SA_RESTART avoids EINTR churn on interrupted
            // syscalls.
            let rc = unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                // `sighandler_t` is a `usize`; cast the fn item through
                // a thin pointer first (the `function_casts_as_integer`
                // lint rejects a direct fn-to-integer cast).
                sa.sa_sigaction = handler as *const () as libc::sighandler_t;
                libc::sigemptyset(&mut sa.sa_mask);
                sa.sa_flags = (if on_alt_stack { libc::SA_ONSTACK } else { 0 }) | libc::SA_RESTART;
                libc::sigaction(sig, &sa, std::ptr::null_mut())
            };
            if rc == 0 {
                installed += 1;
            } else {
                tracing::warn!("signal handler: sigaction for signal {sig} failed (rc={rc})");
            }
        }
        if installed == 0 {
            // Every registration failed — the descriptor is open but no
            // handler will ever fire. Say so rather than leave a false
            // sense of capture.
            tracing::warn!(
                "signal handler: no fatal-signal handlers installed; crash capture disabled"
            );
        }
    }

    /// Register a process-lifetime alternate signal stack so the
    /// handler can run even when the fault is a stack overflow (the
    /// normal stack is unusable in that case). Returns whether the
    /// stack registered.
    fn install_alt_stack() -> bool {
        const ALT_STACK_SIZE: usize = 64 * 1024;
        let stack: &'static mut [u8] = vec![0u8; ALT_STACK_SIZE].leak();
        // SAFETY: registering a valid, leaked (never-freed) stack of
        // the size we report.
        let rc = unsafe {
            let ss = libc::stack_t {
                ss_sp: stack.as_mut_ptr().cast(),
                ss_size: ALT_STACK_SIZE,
                ss_flags: 0,
            };
            libc::sigaltstack(&ss, std::ptr::null_mut())
        };
        if rc != 0 {
            tracing::warn!(
                "signal handler: sigaltstack failed (rc={rc}); stack-overflow crashes may be missed"
            );
        }
        rc == 0
    }

    extern "C" fn handler(sig: libc::c_int) {
        if IN_HANDLER.swap(true, Ordering::SeqCst) {
            // A fault while handling — don't loop; go straight to the
            // default disposition.
            reraise_default(sig);
            return;
        }
        let fd = CRASH_FD.load(Ordering::SeqCst);
        if fd >= 0 {
            write_record(fd, sig);
        }
        reraise_default(sig);
    }

    /// Append one record to `fd`. Every call here is on the POSIX
    /// async-signal-safe list: `pthread_self`, `getpid`,
    /// `clock_gettime`, `write`, `fsync`. No heap, no locks, no
    /// formatting that allocates.
    fn write_record(fd: libc::c_int, sig: libc::c_int) {
        // Async-signal-safe main-thread test: compare `pthread_self()`
        // (safe) against the id captured at install — NOT
        // `pthread_main_np`, which is not on the safe list.
        // SAFETY: side-effect-free thread-local query.
        let on_main =
            unsafe { libc::pthread_self() } as usize == MAIN_THREAD_ID.load(Ordering::SeqCst);
        // SAFETY: side-effect-free query with no preconditions.
        let pid = unsafe { libc::getpid() };
        let epoch: i64 = unsafe {
            let mut ts: libc::timespec = std::mem::zeroed();
            if libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) == 0 {
                // `time_t` is `i64` on every target we build (macOS and
                // Linux, both 64-bit), so this binds without a cast.
                ts.tv_sec
            } else {
                0
            }
        };

        let mut buf = [0u8; 160];
        let n = format_crash_record(&mut buf, sig, on_main, pid, epoch);
        // SAFETY: write up to `n` valid bytes from our stack buffer to
        // our own append-mode descriptor, advancing over short writes.
        // A non-positive return (a hard error, or a possible EINTR) is
        // retried a bounded number of times — no errno read needed, and
        // the cap rules out a spin on a persistent error while we are
        // mid-crash.
        unsafe {
            let mut off = 0usize;
            let mut retries = 0u32;
            while off < n && retries < 16 {
                let rc = libc::write(fd, buf.as_ptr().add(off).cast(), n - off);
                if rc > 0 {
                    off += rc as usize;
                } else {
                    retries += 1;
                }
            }
            let _ = libc::fsync(fd);
        }
    }

    /// Restore the default disposition for `sig` and re-raise it, so the
    /// OS performs its normal action (write the `.ips` crash report,
    /// dump core, terminate). Synchronous faults re-trigger when the
    /// handler returns; `raise` covers the asynchronous ones.
    fn reraise_default(sig: libc::c_int) {
        // SAFETY: resetting to SIG_DFL then re-raising is the standard
        // "log-and-die" tail; no allocation or unwinding.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut sa.sa_mask);
            sa.sa_flags = 0;
            libc::sigaction(sig, &sa, std::ptr::null_mut());
            libc::raise(sig);
        }
    }

    /// Fill `buf` with one crash line and return the byte count. Pure
    /// and allocation-free so the signal handler and the unit tests
    /// call the exact same code.
    ///
    /// `=== claudepot crash === signal=<NAME>(<n>) thread=<main|other> pid=<n> epoch=<n>\n`
    fn format_crash_record(
        buf: &mut [u8],
        sig: libc::c_int,
        on_main_thread: bool,
        pid: libc::c_int,
        epoch_secs: i64,
    ) -> usize {
        let mut w = SliceWriter::new(buf);
        w.put(b"=== claudepot crash === signal=");
        w.put(signal_name(sig).as_bytes());
        w.put(b"(");
        w.put_i64(i64::from(sig));
        w.put(b") thread=");
        w.put(if on_main_thread { b"main" } else { b"other" });
        w.put(b" pid=");
        w.put_i64(i64::from(pid));
        w.put(b" epoch=");
        w.put_i64(epoch_secs);
        w.put(b"\n");
        w.len()
    }

    fn signal_name(sig: libc::c_int) -> &'static str {
        match sig {
            libc::SIGSEGV => "SIGSEGV",
            libc::SIGABRT => "SIGABRT",
            libc::SIGBUS => "SIGBUS",
            libc::SIGILL => "SIGILL",
            libc::SIGFPE => "SIGFPE",
            libc::SIGTRAP => "SIGTRAP",
            _ => "SIG?",
        }
    }

    /// Bounded, allocation-free byte writer over a fixed stack buffer.
    /// Silently stops at capacity — a truncated record beats an
    /// overflow in a signal handler.
    struct SliceWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }

    impl<'a> SliceWriter<'a> {
        fn new(buf: &'a mut [u8]) -> Self {
            Self { buf, pos: 0 }
        }

        fn put(&mut self, bytes: &[u8]) {
            for &b in bytes {
                if self.pos < self.buf.len() {
                    self.buf[self.pos] = b;
                    self.pos += 1;
                }
            }
        }

        fn put_i64(&mut self, n: i64) {
            if n < 0 {
                self.put(b"-");
            }
            // Magnitude via i128 so i64::MIN doesn't overflow on negate.
            let mut mag = i128::from(n).unsigned_abs();
            let mut tmp = [0u8; 40];
            let mut i = tmp.len();
            if mag == 0 {
                i -= 1;
                tmp[i] = b'0';
            }
            while mag > 0 {
                i -= 1;
                tmp[i] = b'0' + (mag % 10) as u8;
                mag /= 10;
            }
            self.put(&tmp[i..]);
        }

        fn len(&self) -> usize {
            self.pos
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn format_crash_record_shape() {
            let mut buf = [0u8; 160];
            let n = format_crash_record(&mut buf, libc::SIGABRT, false, 12345, 1_733_000_000);
            let s = std::str::from_utf8(&buf[..n]).unwrap();
            assert_eq!(
                s,
                "=== claudepot crash === signal=SIGABRT(6) thread=other pid=12345 epoch=1733000000\n"
            );
        }

        #[test]
        fn format_crash_record_marks_main_thread() {
            let mut buf = [0u8; 160];
            let n = format_crash_record(&mut buf, libc::SIGSEGV, true, 1, 0);
            let s = std::str::from_utf8(&buf[..n]).unwrap();
            assert!(s.contains("signal=SIGSEGV(11)"), "got: {s}");
            assert!(s.contains("thread=main"), "got: {s}");
        }

        #[test]
        fn format_crash_record_never_overflows_small_buffer() {
            let mut buf = [0u8; 8];
            let n = format_crash_record(&mut buf, libc::SIGSEGV, true, 1, 0);
            assert!(n <= 8, "writer must clamp to capacity, wrote {n}");
        }

        #[test]
        fn put_i64_handles_zero_and_i64_min() {
            let mut buf = [0u8; 40];
            let mut w = SliceWriter::new(&mut buf);
            w.put_i64(0);
            assert_eq!(&w.buf[..w.pos], b"0");

            let mut buf2 = [0u8; 40];
            let mut w2 = SliceWriter::new(&mut buf2);
            w2.put_i64(i64::MIN);
            assert_eq!(
                std::str::from_utf8(&w2.buf[..w2.pos]).unwrap(),
                "-9223372036854775808"
            );
        }
    }
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
    fn test_resolve_active_log_empty_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(resolve_active_log(tmp.path()).is_none());
    }

    #[test]
    fn test_resolve_active_log_picks_lexically_latest_dated_file_when_no_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::File::create(tmp.path().join("claudepot.log.2026-06-03")).unwrap();
        std::fs::File::create(tmp.path().join("claudepot.log.2026-06-05")).unwrap();
        std::fs::File::create(tmp.path().join("claudepot.log.2026-06-04")).unwrap();
        std::fs::File::create(tmp.path().join("unrelated.txt")).unwrap();
        let active = resolve_active_log(tmp.path()).expect("should find a dated file");
        assert_eq!(active.file_name().unwrap(), "claudepot.log.2026-06-05");
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_active_log_prefers_symlink_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::File::create(tmp.path().join("claudepot.log.2026-06-04")).unwrap();
        let dated = tmp.path().join("claudepot.log.2026-06-05");
        std::fs::File::create(&dated).unwrap();
        let symlink = tmp.path().join("claudepot.log");
        std::os::unix::fs::symlink(&dated, &symlink).unwrap();
        let active = resolve_active_log(tmp.path()).expect("symlink should be picked");
        assert_eq!(active.file_name().unwrap(), "claudepot.log");
    }

    #[test]
    fn test_resolve_active_log_finds_appender_output() {
        // End-to-end coherence: whatever build_file_appender writes,
        // resolve_active_log must find — they own the same contract.
        let tmp = tempfile::tempdir().unwrap();
        let _appender = build_file_appender(tmp.path()).expect("builder should succeed");
        let active =
            resolve_active_log(tmp.path()).expect("resolver must find the appender's symlink/file");
        assert!(active.exists());
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
