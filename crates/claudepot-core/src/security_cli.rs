//! The `/usr/bin/security` verified-write pattern for Claudepot-owned
//! macOS secrets that both the GUI and the CLI must read
//! (`rules/architecture.md`, surface 2).
//!
//! Write via `security -i` with the value hex-encoded over stdin, never
//! argv, then read it back: a zero exit from `security -i` is not proof
//! the item landed.
//!
//! **A deliberate exception to `rules/rust-conventions.md`'s "all I/O
//! async, subprocesses via `tokio::process`".** This module is
//! synchronous (`std::process` plus a polled deadline) because its
//! callers that cannot await are synchronous by contract:
//! `routes::keychain` sits under the route-lifecycle `Effects` trait
//! (`routes::lifecycle`, sync), and `relay::route_secret::KeychainReader`
//! is a sync trait. An async version would force those callers to block
//! on a runtime they may already be inside. Async callers never call it
//! on a runtime thread: `relay::token_store` and
//! `relay::route_secret::read_for_serving` go through
//! `tokio::task::spawn_blocking`. A new async caller must do the same.
//!
//! Every call is bounded by a timeout, because a locked keychain in a GUI
//! session can hold `security` on an unlock dialog indefinitely.
//!
//! `cli_backend::storage` and `session::export_delivery` keep their own
//! async copies of the pattern; they are not migrated here.

use std::io::{Read as _, Write as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub use crate::relay::secret::store_key_ok as key_ok;
use crate::relay::secret::Secret;

const TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);
const SECURITY: &str = "/usr/bin/security";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecurityCliError {
    #[error("the keychain is locked — unlock it and retry")]
    Locked,
    #[error("keychain: {0}")]
    Backend(String),
    #[error("the keychain write did not take effect")]
    NotVerified,
    #[error("unusable keychain key")]
    BadKey,
    #[error("the value is too long for the keychain")]
    TooLong,
}

struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
}

/// Wait for `child` with a deadline, killing it on timeout. Output is a
/// password or nothing, far below a pipe buffer, so reading after exit
/// cannot deadlock.
fn wait(child: Child) -> Result<Output, SecurityCliError> {
    wait_within(child, TIMEOUT)
}

fn wait_within(mut child: Child, timeout: Duration) -> Result<Output, SecurityCliError> {
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SecurityCliError::Backend("security timed out".into()));
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(e) => return Err(SecurityCliError::Backend(format!("wait: {e}"))),
        }
    };
    let mut stdout = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut stdout)
            .map_err(|e| SecurityCliError::Backend(format!("read: {e}")))?;
    }
    Ok(Output { status, stdout })
}

fn spawn(args: &[&str], stdin: bool) -> Result<Child, SecurityCliError> {
    Command::new(SECURITY)
        .args(args)
        .stdin(if stdin { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| SecurityCliError::Backend(format!("spawn {SECURITY}: {e}")))
}

pub fn read(service: &str, account: &str) -> Result<Option<Secret>, SecurityCliError> {
    if !key_ok(service) || !key_ok(account) {
        return Err(SecurityCliError::BadKey);
    }
    let child = spawn(
        &["find-generic-password", "-a", account, "-s", service, "-w"],
        false,
    )?;
    let mut out = wait(child)?;
    if out.status.success() {
        let v = String::from_utf8_lossy(&out.stdout).trim_end().to_owned();
        zeroize::Zeroize::zeroize(&mut out.stdout);
        return Ok((!v.is_empty()).then(|| Secret::new(v)));
    }
    // 44 = not found, a clean miss; 36 = locked. Anything else is an
    // error — never report a present-but-unreadable secret as absent.
    match out.status.code() {
        Some(44) => Ok(None),
        Some(36) => Err(SecurityCliError::Locked),
        code => Err(SecurityCliError::Backend(format!(
            "security find-generic-password failed (code {})",
            code.unwrap_or(-1)
        ))),
    }
}

pub fn write(service: &str, account: &str, secret: &Secret) -> Result<(), SecurityCliError> {
    if !key_ok(service) || !key_ok(account) {
        return Err(SecurityCliError::BadKey);
    }
    // Both the hex form and the command line hold the secret, so both
    // are wrapped to be zeroized on drop.
    let hex_value = Secret::new(hex::encode(secret.expose().as_bytes()));
    let line = Secret::new(format!(
        "add-generic-password -U -a \"{account}\" -s \"{service}\" -X \"{}\"\n",
        hex_value.expose()
    ));
    // `security -i` reads one line into a fixed buffer and splits a
    // longer one into separate commands; refuse it rather than send it.
    if !crate::cli_backend::security_output::fits_stdin(line.expose()) {
        return Err(SecurityCliError::TooLong);
    }
    let out = run_with_stdin(spawn(&["-i"], true)?, line, TIMEOUT)?;
    if !out.status.success() {
        return Err(SecurityCliError::Backend(format!(
            "security add-generic-password failed (exit {})",
            out.status.code().unwrap_or(-1)
        )));
    }
    match read(service, account)? {
        Some(back) if back == *secret => Ok(()),
        _ => Err(SecurityCliError::NotVerified),
    }
}

/// Feed `line` to the child on its own thread, so the deadline covers the
/// write as well as the exit: a child that stops reading is killed at the
/// deadline, which fails the blocked write with a broken pipe.
fn run_with_stdin(
    mut child: Child,
    line: Secret,
    timeout: Duration,
) -> Result<Output, SecurityCliError> {
    let (tx, rx) = mpsc::channel();
    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            // Dropping stdin closes it, which ends `security -i`.
            let _ = tx.send(stdin.write_all(line.expose().as_bytes()));
        });
    }
    let out = wait_within(child, timeout)?;
    match rx.recv() {
        Ok(Err(e)) => Err(SecurityCliError::Backend(format!("stdin write: {e}"))),
        _ => Ok(out),
    }
}

/// Idempotent: a missing item is success.
pub fn delete(service: &str, account: &str) -> Result<(), SecurityCliError> {
    if !key_ok(service) || !key_ok(account) {
        return Err(SecurityCliError::BadKey);
    }
    let child = spawn(
        &["delete-generic-password", "-a", account, "-s", service],
        false,
    )?;
    let out = wait(child)?;
    match out.status.code() {
        Some(0) | Some(44) => Ok(()),
        Some(36) => Err(SecurityCliError::Locked),
        code => Err(SecurityCliError::Backend(format!(
            "security delete-generic-password failed (code {})",
            code.unwrap_or(-1)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_that_could_break_the_security_command_line_are_refused() {
        assert!(key_ok("2f0c8a6e-1d0b-4f5a-9b1e-7c3d2a1b0c9d-api_key"));
        assert!(key_ok("claudepot-routes"));
        for bad in ["", "a\"b", "a b", "a\nb", "a\\b", "a\rb"] {
            assert!(!key_ok(bad), "{bad:?} must be refused");
        }
        assert!(!key_ok(&"a".repeat(129)));
    }

    #[test]
    fn a_value_too_long_for_the_command_buffer_is_refused_before_spawning() {
        let long = Secret::new("x".repeat(2100));
        assert_eq!(write("svc", "a", &long), Err(SecurityCliError::TooLong));
    }

    #[test]
    fn a_child_that_stops_reading_stdin_is_killed_at_the_deadline() {
        // `sleep` never reads stdin, so a write larger than the pipe
        // buffer blocks until the deadline kills the child.
        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let r = run_with_stdin(
            child,
            Secret::new("x".repeat(1 << 20)),
            Duration::from_millis(200),
        );
        assert!(matches!(r, Err(SecurityCliError::Backend(_))));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_bad_key_is_refused_before_spawning() {
        assert_eq!(read("svc", "a b"), Err(SecurityCliError::BadKey));
        assert_eq!(
            write("svc\"", "a", &Secret::new("x")),
            Err(SecurityCliError::BadKey)
        );
        assert_eq!(delete("", "a"), Err(SecurityCliError::BadKey));
    }
}
