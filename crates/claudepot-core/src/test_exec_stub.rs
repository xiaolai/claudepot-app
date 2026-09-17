//! Writing a shell script that a test is about to execute.
//!
//! Test-only, and deliberately free of crate dependencies: the unit
//! tests reach it as `crate::test_exec_stub`, and the integration tests
//! under `tests/` — which cannot see `#[cfg(test)]` items — include this
//! same file with `#[path]`. One implementation, no copies to drift.
//!
//! # Why a child process writes the file
//!
//! Exec'ing a file that any process holds open for writing fails with
//! `ETXTBSY` ("Text file busy") on Linux. Test threads fork
//! concurrently, and a fork that lands while *this* process has the
//! script open copies that descriptor into the child, where it stays
//! until the child execs. `O_CLOEXEC` does not help: it closes the copy
//! at exec, and the window is everything before it. So a script written
//! with `std::fs::write` and executed straight away sometimes cannot be
//! executed at all.
//!
//! `cc_capability`'s probe test failed on ubuntu-latest exactly that way
//! (2026-09-18): the stub never ran, and the probe reported "could not
//! read `claude --version`". A C reproduction in a Linux container —
//! sixteen threads each writing and exec'ing a script — gave 35 and 32
//! `ETXTBSY` out of 6,400 execs with in-process writes, and 0 out of
//! 12,800 with a child process doing the writing. That writer's
//! descriptor lives in another process, which has exited before this
//! function returns, so there is nothing left to inherit.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write `script` to `path` and make it executable with `mode`.
///
/// Panics on failure — this is a fixture, and a fixture that could not
/// be written must stop the test rather than let it pass over nothing.
pub fn write_exec_stub(path: &Path, script: &str, mode: u32) {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"cat > "$1" && chmod "$2" "$1""#)
        .arg("sh")
        .arg(path)
        .arg(format!("{mode:o}"))
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn the stub writer");
    child
        .stdin
        .take()
        .expect("stub writer stdin")
        .write_all(script.as_bytes())
        .expect("send the stub body");
    // The handle above is dropped at the end of that statement, which is
    // the EOF `cat` is waiting for.
    let status = child.wait().expect("wait for the stub writer");
    assert!(
        status.success(),
        "writing stub {} failed: {status}",
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_stub_runs_and_carries_the_requested_mode() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("stub");
        write_exec_stub(&p, "#!/bin/sh\necho 'it ran'\n", 0o700);
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let out = Command::new(&p).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "it ran\n");
    }

    #[test]
    fn a_path_it_cannot_write_is_a_panic_not_a_silent_skip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("no-such-dir").join("stub");
        let r = std::panic::catch_unwind(|| write_exec_stub(&p, "#!/bin/sh\n", 0o755));
        assert!(r.is_err());
    }
}
