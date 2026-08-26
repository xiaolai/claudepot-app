//! Onboarding: add a new account via `claude auth login` scaffold (Mode B).
//!
//! Uses a temp CLAUDE_CONFIG_DIR so the current active account isn't clobbered.
//! After login, imports the credential from the hashed keychain item or file.

use crate::proc_utils::NoWindowExt;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Boundary error for onboarding. Historically lived in the
/// crate-root `error.rs`; relocated next to its boundary per
/// rust-conventions ("one enum per module boundary").
/// `crate::error::OnboardError` remains a re-export.
#[derive(thiserror::Error, Debug)]
pub enum OnboardError {
    #[error("claude CLI not found at {0}")]
    CliBinaryNotFound(String),

    // Second tuple field is the captured stderr tail from the
    // `claude auth login` subprocess (last N lines, redacted of
    // `sk-ant-*` tokens). When present, it's appended to the rendered
    // error so the GUI dialog shows the actual failure reason —
    // network error, keychain perm denied, OAuth state mismatch, etc.
    // — instead of just the exit code. Issue #16.
    #[error("{}", match (*.0, .1.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty())) {
        (-2, _) => "login timed out — close the Claudepot window and try again, or complete the browser flow faster".to_string(),
        (code, Some(tail)) => format!(
            "`claude auth login` exited with code {code}\n\nclaude stderr (last lines):\n{tail}"
        ),
        (code, None) => format!("`claude auth login` exited with code {code}"),
    })]
    AuthLoginFailed(i32, Option<String>),

    #[error("login cancelled")]
    AuthLoginCancelled,

    #[error("import failed: no credentials at hashed service name for {0}")]
    ImportFailed(String),

    #[error("{0}")]
    Swap(#[from] crate::cli_backend::SwapError),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// `Swap` **delegates** to the inner [`SwapError`]. Its `#[error("{0}")]`
/// adds no words of its own, so an `onboard.swap` code would name a
/// wrapper the user never sees, and its only honest catalog sentence was
/// the bare `{{detail}}` — a "translation" that re-emits English. The
/// inner error already carries a code and structured params; forwarding
/// them is what makes the sentence translatable at all.
///
/// `Io` keeps its own code: `std::io::Error` has no code to forward to.
impl crate::error_code::ErrorCode for OnboardError {
    fn code(&self) -> &'static str {
        match self {
            OnboardError::CliBinaryNotFound(_) => "onboard.cli_binary_not_found",
            OnboardError::AuthLoginFailed(..) => "onboard.auth_login_failed",
            OnboardError::AuthLoginCancelled => "onboard.auth_login_cancelled",
            OnboardError::ImportFailed(_) => "onboard.import_failed",
            OnboardError::Swap(e) => e.code(),
            OnboardError::Io(_) => "onboard.io",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            OnboardError::CliBinaryNotFound(path) => serde_json::json!({ "path": path }),
            // One code, three English branches (timeout, exit code
            // alone, exit code + stderr tail) — the Display impl picks
            // between them and a catalog entry must do the same, which
            // is why both values are carried unconditionally.
            // `exit_code == -2` is the timeout sentinel; `stderr_tail`
            // is `""` when the subprocess produced none.
            //
            // The tail is already run through
            // `session_export::redact_secrets` at capture time, which
            // is what makes it safe to put in `params` at all — do not
            // widen this to the raw stream.
            OnboardError::AuthLoginFailed(code, tail) => serde_json::json!({
                "exit_code": code,
                "stderr_tail": tail
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(""),
            }),
            OnboardError::AuthLoginCancelled => serde_json::json!({}),
            // The temp `CLAUDE_CONFIG_DIR` the import looked in, not an
            // email — see `import_credential`'s only constructor.
            OnboardError::ImportFailed(path) => serde_json::json!({ "path": path }),
            // Delegated above, so the inner error's own params travel
            // with its own code — no frozen English in `detail`.
            // A caller that needs the inner identity matches on
            // `SwapError` before it is wrapped.
            OnboardError::Swap(e) => e.params(),
            OnboardError::Io(e) => serde_json::json!({ "detail": e.to_string() }),
        }
    }
}

/// Hard timeout for `claude auth login` — generous enough that slow
/// readers completing OAuth in the browser finish in time, tight enough
/// that a user who closed the browser or walked away doesn't leave the
/// GUI stuck on a spinner forever. Matches Kannon's 10-minute window.
pub const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// How many trailing stderr lines from `claude auth login` to keep
/// for inclusion in the user-facing error message. The whole stream
/// still goes to `tracing` for log inspection; this cap just limits
/// what gets stuffed into the dialog so a runaway child can't bloat
/// memory or render unreadable. Issue #16: the previous behaviour
/// dropped stderr entirely on failure, leaving users with only the
/// exit code.
/// Where the browser is sent to drop its `claude.ai` session before a
/// login begins. See [`clear_browser_session`].
pub const LOGOUT_URL: &str = "https://claude.ai/logout";

/// How long to let the logout land before `claude auth login` opens the
/// authorize URL in the same browser.
///
/// This is a fixed wait because there is nothing to observe: the cookie
/// lives in the user's browser and Claudepot drives it through the OS
/// handler, with no channel back. Too short and the authorize page may
/// load against a session the logout has not dropped yet — which is the
/// state this exists to prevent; too long and every login pays for it.
/// Three seconds covers a redirect on a normal connection.
const LOGOUT_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);

/// The platform command that hands a URL to the user's default browser.
///
/// Split from [`clear_browser_session`] so the platform choice is
/// testable without spawning anything — the judgement never needs a
/// browser, only the I/O does.
fn browser_open_command(url: &str) -> (&'static str, Vec<String>) {
    #[cfg(target_os = "macos")]
    {
        ("/usr/bin/open", vec![url.to_string()])
    }
    #[cfg(target_os = "linux")]
    {
        ("xdg-open", vec![url.to_string()])
    }
    #[cfg(target_os = "windows")]
    {
        // `start` needs an empty title argument first, or it treats the
        // URL as the window title and opens nothing.
        (
            "cmd",
            vec![
                "/C".to_string(),
                "start".to_string(),
                String::new(),
                url.to_string(),
            ],
        )
    }
}

/// Send the browser to `claude.ai/logout` and give it a moment to land.
///
/// **Why this runs at all.** `claude auth login` opens an authorize URL
/// in the user's default browser. The Google session and the `claude.ai`
/// session are different cookies, so a browser already signed in to
/// `claude.ai` authorizes *that* account — the account chooser either
/// never appears or is ignored. Adding a second account then silently
/// re-authorizes the first, and the result looks like success: a
/// credential blob lands, `/profile` agrees with it, and the only sign
/// anything went wrong is the email.
///
/// **The side effect is real and intended.** This signs the user out of
/// `claude.ai` in their everyday browser. That is a visible cost, taken
/// deliberately: signing back in is one click, whereas a silently wrong
/// account propagates into a stored credential.
///
/// **Best-effort, never fatal.** Every failure — no handler, a spawn
/// error, a browser that ignores the URL — leaves the previous
/// behaviour, which is exactly what shipped before this existed. A login
/// must not fail because a logout page did not open.
pub async fn clear_browser_session() {
    let (program, args) = browser_open_command(LOGOUT_URL);
    tracing::info!(
        url = LOGOUT_URL,
        "clearing browser claude.ai session before login"
    );

    match tokio::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .no_window()
        .spawn()
    {
        Ok(mut child) => {
            // Reap it. The handler exits immediately once it has handed
            // the URL off; not waiting leaves a zombie on Unix.
            let _ = child.wait().await;
        }
        Err(e) => {
            // Deliberately not an error return — see the contract above.
            tracing::warn!(
                error = %e,
                "could not open the logout URL; continuing. If this login \
                 authorizes the wrong account, a stale claude.ai session is why"
            );
            return;
        }
    }

    tokio::time::sleep(LOGOUT_SETTLE).await;
}

const STDERR_TAIL_LINES: usize = 12;

/// Bounded grace window for the stderr drain task to flush remaining
/// buffered lines after the child process exits or is killed. Long
/// enough to catch the final line a healthy child writes before
/// exiting; short enough that a stuck pipe can't hang the login flow.
const STDERR_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_millis(150);

type StderrTail = Arc<Mutex<VecDeque<String>>>;

fn new_stderr_tail() -> StderrTail {
    Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES + 1)))
}

async fn drain_stderr_tail(tail: &StderrTail) -> Option<String> {
    let buf = tail.lock().await;
    if buf.is_empty() {
        return None;
    }
    // Fold without the intermediate cloned Vec — join over &str works
    // straight off the deque iterator.
    Some(
        buf.iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Cancellable variant: pass a shared `Notify`; when another task calls
/// `notify.notify_one()`, the subprocess is killed and this function
/// returns `AuthLoginCancelled`. Used by the GUI's Cancel button.
///
/// Error cases:
/// - `AuthLoginCancelled` — user clicked Cancel
/// - `AuthLoginFailed(-2)` — hit LOGIN_TIMEOUT
/// - `AuthLoginFailed(code)` — subprocess exited with failure
pub async fn run_auth_login_in_place_cancellable(
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), OnboardError> {
    let claude_path = which_claude()?;
    // Before the browser opens, not after — see `clear_browser_session`.
    // Resolving the binary first means a missing `claude` fails without
    // having signed the user out of anything.
    clear_browser_session().await;
    run_auth_login_in_place_cancellable_with_binary(&claude_path, cancel).await
}

/// Internal seam: accepts an explicit binary path so tests can point
/// the loop at a controllable stub process. Mirrors
/// [`run_auth_login_cancellable_with_binary`] — kept private to the
/// crate so the public surface stays tied to the auto-discovered
/// `claude` binary.
pub(crate) async fn run_auth_login_in_place_cancellable_with_binary(
    claude_path: &std::path::Path,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), OnboardError> {
    tracing::info!(
        binary = %claude_path.display(),
        timeout_secs = LOGIN_TIMEOUT.as_secs(),
        "spawning `claude auth login` in place"
    );

    let mut child = tokio::process::Command::new(claude_path)
        .arg("auth")
        .arg("login")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_window()
        .spawn()
        .map_err(OnboardError::Io)?;

    // Drain stdout / stderr into tracing so logs from the child surface
    // when the GUI is launched without a terminal. Tasks are aborted
    // automatically when the parent `child` is dropped. The stderr
    // pipe additionally feeds a bounded ring buffer so the failure
    // path can include the actual reason in `OnboardError::AuthLoginFailed`
    // rather than just the exit code (issue #16).
    let stderr_tail = new_stderr_tail();
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(pipe_to_tracing(stdout, "claude-stdout", None));
    }
    let stderr_handle = child.stderr.take().map(|stderr| {
        tokio::spawn(pipe_to_tracing(
            stderr,
            "claude-stderr",
            Some(stderr_tail.clone()),
        ))
    });

    let cancel_fut = async {
        match cancel.as_ref() {
            Some(n) => n.notified().await,
            // Never-resolving future when no cancel channel was provided.
            None => std::future::pending::<()>().await,
        }
    };

    let outcome = tokio::select! {
        exit = child.wait() => {
            match exit {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(OnboardError::AuthLoginFailed(
                    status.code().unwrap_or(-1),
                    None,
                )),
                Err(e) => Err(OnboardError::Io(e)),
            }
        }
        _ = tokio::time::sleep(LOGIN_TIMEOUT) => {
            tracing::warn!(
                "`claude auth login` exceeded {}s — killing child",
                LOGIN_TIMEOUT.as_secs()
            );
            let _ = child.kill().await;
            Err(OnboardError::AuthLoginFailed(-2, None))
        }
        _ = cancel_fut => {
            tracing::info!("login cancelled by user — killing child");
            let _ = child.kill().await;
            Err(OnboardError::AuthLoginCancelled)
        }
    };

    // On failure paths only, give the stderr drain task a small grace
    // window to flush whatever's still buffered before we read the
    // tail. The successful and cancel paths don't need the tail, so
    // we skip the wait and let the task wind down on its own when the
    // pipe FD closes.
    match outcome {
        Ok(()) => Ok(()),
        Err(OnboardError::AuthLoginCancelled) => Err(OnboardError::AuthLoginCancelled),
        Err(OnboardError::AuthLoginFailed(code, _)) => {
            if let Some(handle) = stderr_handle {
                let _ = tokio::time::timeout(STDERR_DRAIN_GRACE, handle).await;
            }
            Err(OnboardError::AuthLoginFailed(
                code,
                drain_stderr_tail(&stderr_tail).await,
            ))
        }
        Err(other) => Err(other),
    }
}

async fn pipe_to_tracing<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: R,
    stream_name: &'static str,
    tail: Option<StderrTail>,
) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        // `claude auth login` can occasionally echo OAuth artifacts into
        // its diagnostic output. Run every line through the same
        // `sk-ant-*` masker the rest of the codebase uses before it
        // reaches `tracing` — otherwise any sink (file, syslog, console)
        // becomes a credential disclosure surface.
        // See `.claude/rules/rust-conventions.md` §Security.
        let safe = crate::session_export::redact_secrets(&line);
        tracing::info!(target: "claudepot::onboard", stream = stream_name, "{}", safe);
        if let Some(tail) = tail.as_ref() {
            // Same redacted line goes into the bounded ring buffer that
            // backs the user-facing error tail. Lock-and-push happens
            // per line — the lock is uncontended in practice (only this
            // task and the failure-path drainer touch it) and lines
            // arrive at most as fast as `claude auth login` writes,
            // which is well below contention threshold.
            let mut buf = tail.lock().await;
            buf.push_back(safe);
            while buf.len() > STDERR_TAIL_LINES {
                buf.pop_front();
            }
        }
    }
}

/// Run `claude auth login` with a temporary config dir.
/// Returns the path to the temp dir (caller is responsible for cleanup).
///
/// Pass a shared `Notify`; when another task calls `notify.notify_one()`,
/// the child `claude auth login` process is killed and this function
/// returns `AuthLoginCancelled`. The temp config dir is returned either
/// way so the caller can inspect it (credential read) or clean it up
/// (cancelled / failed).
///
/// Error cases mirror `run_auth_login_in_place_cancellable`:
/// * `AuthLoginCancelled` — user clicked Cancel
/// * `AuthLoginFailed(-2)` — hit `LOGIN_TIMEOUT`
/// * `AuthLoginFailed(code)` — subprocess exited with failure
pub async fn run_auth_login_cancellable(
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<PathBuf, OnboardError> {
    let claude_path = which_claude()?;
    // See the sibling call in `run_auth_login_in_place_cancellable`.
    clear_browser_session().await;
    run_auth_login_cancellable_with_binary(&claude_path, cancel).await
}

/// Internal seam: accepts an explicit binary path so tests can point
/// the loop at a controllable stub process. Not re-exported.
pub(crate) async fn run_auth_login_cancellable_with_binary(
    claude_path: &std::path::Path,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<PathBuf, OnboardError> {
    let temp_dir = tempfile::Builder::new()
        .prefix("claudepot-onboard-")
        .tempdir()
        .map_err(OnboardError::Io)?;
    let config_dir = temp_dir.path().to_path_buf();

    tracing::info!(
        binary = %claude_path.display(),
        config_dir = %config_dir.display(),
        timeout_secs = LOGIN_TIMEOUT.as_secs(),
        "spawning `claude auth login` in temp dir"
    );

    let mut child = tokio::process::Command::new(claude_path)
        .arg("auth")
        .arg("login")
        .env("CLAUDE_CONFIG_DIR", &config_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_window()
        .spawn()
        .map_err(OnboardError::Io)?;

    // Mirror run_auth_login_in_place_cancellable's drain-to-tracing
    // pattern. Stdout/stderr piped so a closed terminal doesn't lose
    // diagnostic output. Stderr also feeds a bounded ring buffer so
    // failures surface the actual reason in the user-facing error
    // (issue #16).
    let stderr_tail = new_stderr_tail();
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(pipe_to_tracing(stdout, "claude-stdout", None));
    }
    let stderr_handle = child.stderr.take().map(|stderr| {
        tokio::spawn(pipe_to_tracing(
            stderr,
            "claude-stderr",
            Some(stderr_tail.clone()),
        ))
    });

    let cancel_fut = async {
        match cancel.as_ref() {
            Some(n) => n.notified().await,
            None => std::future::pending::<()>().await,
        }
    };

    let outcome = tokio::select! {
        exit = child.wait() => {
            match exit {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(OnboardError::AuthLoginFailed(
                    status.code().unwrap_or(-1),
                    None,
                )),
                Err(e) => Err(OnboardError::Io(e)),
            }
        }
        _ = tokio::time::sleep(LOGIN_TIMEOUT) => {
            tracing::warn!(
                "`claude auth login` exceeded {}s — killing child",
                LOGIN_TIMEOUT.as_secs()
            );
            let _ = child.kill().await;
            Err(OnboardError::AuthLoginFailed(-2, None))
        }
        _ = cancel_fut => {
            tracing::info!("browser login cancelled by user — killing child");
            let _ = child.kill().await;
            Err(OnboardError::AuthLoginCancelled)
        }
    };

    // Resolve the tail BEFORE we hand back the result so the caller
    // sees a fully-rendered Display. See the parallel block in
    // `run_auth_login_in_place_cancellable_with_binary` for the
    // rationale; same grace window.
    let outcome = match outcome {
        Err(OnboardError::AuthLoginFailed(code, _)) => {
            if let Some(handle) = stderr_handle {
                let _ = tokio::time::timeout(STDERR_DRAIN_GRACE, handle).await;
            }
            Err(OnboardError::AuthLoginFailed(
                code,
                drain_stderr_tail(&stderr_tail).await,
            ))
        }
        other => other,
    };

    match outcome {
        Ok(()) => {
            // Keep the tempdir alive for the caller; they clean it up
            // after reading credentials.
            Ok(temp_dir.keep())
        }
        Err(e) => {
            // `claude auth login` can write credentials to two places:
            // the temp `.credentials.json` file *and*, on macOS, a
            // hashed Keychain entry keyed off the temp dir's path. If
            // OAuth completed in the browser milliseconds before the
            // user clicked Cancel (or the timeout elapsed), the child
            // may have already written into both. `TempDir::drop`
            // only removes the directory; the Keychain item would be
            // orphaned. Route cleanup through `cleanup()` which knows
            // about both surfaces.
            let config_dir = temp_dir.path().to_path_buf();
            // Release the TempDir handle first so its Drop doesn't
            // race with the explicit `remove_dir_all` inside cleanup.
            let _ = temp_dir.keep();
            cleanup(&config_dir).await;
            Err(e)
        }
    }
}

/// Read the credential blob from a temp config dir (file fallback).
pub async fn read_credentials_from_dir(
    config_dir: &std::path::Path,
) -> Result<String, OnboardError> {
    let cred_file = config_dir.join(".credentials.json");
    if cred_file.exists() {
        return std::fs::read_to_string(&cred_file).map_err(OnboardError::Io);
    }

    // Try the hashed keychain item (macOS)
    #[cfg(target_os = "macos")]
    {
        let hash = crate::cli_backend::keychain::hashed_service_name(&config_dir.to_string_lossy());
        if let Ok(Some(blob)) = crate::cli_backend::keychain::read(&hash).await {
            return Ok(blob);
        }
    }

    Err(OnboardError::ImportFailed(config_dir.display().to_string()))
}

/// Clean up after onboarding: remove temp dir and hashed keychain item.
pub async fn cleanup(config_dir: &std::path::Path) {
    // Remove temp directory
    let _ = std::fs::remove_dir_all(config_dir);

    // Remove hashed keychain item (macOS)
    #[cfg(target_os = "macos")]
    {
        let hash = crate::cli_backend::keychain::hashed_service_name(&config_dir.to_string_lossy());
        let _ = crate::cli_backend::keychain::delete(&hash).await;
    }
}

fn which_claude() -> Result<PathBuf, OnboardError> {
    crate::fs_utils::find_claude_binary().ok_or_else(|| {
        OnboardError::CliBinaryNotFound("claude not found in PATH or common locations".into())
    })
}

#[cfg(test)]
mod tests {
    /// The URL is the whole feature — a typo here silently does
    /// nothing, because the open is best-effort and a bad URL fails the
    /// same way as no handler.
    #[test]
    fn logout_url_is_the_claude_ai_logout_endpoint() {
        assert_eq!(super::LOGOUT_URL, "https://claude.ai/logout");
    }

    #[test]
    fn browser_open_command_passes_the_url_through() {
        let (program, args) = super::browser_open_command(super::LOGOUT_URL);
        assert!(!program.is_empty());
        assert!(
            args.iter().any(|a| a == super::LOGOUT_URL),
            "the URL must reach the handler; got {args:?}"
        );
    }

    /// On Windows `start` consumes the first quoted argument as the
    /// window title, so the empty placeholder is load-bearing: without
    /// it the URL becomes the title and no browser opens.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_start_keeps_its_empty_title_placeholder() {
        let (program, args) = super::browser_open_command(super::LOGOUT_URL);
        assert_eq!(program, "cmd");
        assert_eq!(args[0], "/C");
        assert_eq!(args[1], "start");
        assert_eq!(args[2], "", "empty title placeholder must precede the URL");
        assert_eq!(args[3], super::LOGOUT_URL);
    }

    /// The wait exists because nothing can be observed; if it is ever
    /// zeroed the authorize page can load against the session the
    /// logout has not dropped yet, which is the bug this prevents.
    #[test]
    fn logout_settle_is_nonzero() {
        assert!(super::LOGOUT_SETTLE > std::time::Duration::ZERO);
    }

    use super::*;

    /// The -2 sentinel for "timed out" must render a clear, actionable
    /// message — the GUI shows this verbatim in a toast, so it needs to
    /// guide the user instead of displaying a cryptic exit code.
    #[test]
    fn test_auth_login_timeout_error_message() {
        let err = OnboardError::AuthLoginFailed(-2, None);
        let msg = err.to_string();
        assert!(
            msg.contains("timed out"),
            "timeout message should say 'timed out'; got: {msg}"
        );
        assert!(
            msg.contains("try again"),
            "timeout message should include a recovery hint; got: {msg}"
        );
    }

    #[test]
    fn test_auth_login_non_timeout_exit_code_is_reported() {
        // Any non-(-2) exit code should include the actual code so the
        // user can diagnose (e.g. 1 = generic CC failure).
        let err = OnboardError::AuthLoginFailed(1, None);
        assert!(err.to_string().contains("1"));
    }

    /// Issue #16: when `claude auth login` exits non-zero, the captured
    /// stderr tail must be appended to the rendered error so the GUI
    /// dialog tells the user the actual reason (network error, OAuth
    /// state mismatch, keychain perm denied, …) instead of just the
    /// exit code.
    #[test]
    fn test_auth_login_stderr_tail_is_appended_to_error() {
        let tail = "Error: keychain item not found\nfailed at step 3".to_string();
        let err = OnboardError::AuthLoginFailed(1, Some(tail.clone()));
        let msg = err.to_string();
        assert!(
            msg.contains("exited with code 1"),
            "exit code still surfaces; got: {msg}"
        );
        assert!(
            msg.contains("claude stderr"),
            "stderr tail should be labelled; got: {msg}"
        );
        assert!(
            msg.contains("keychain item not found"),
            "actual stderr line should be in the message; got: {msg}"
        );
    }

    /// An empty / whitespace-only stderr tail must not produce a
    /// dangling "claude stderr (last lines):" header in the message.
    /// Otherwise a child that exits non-zero without writing to
    /// stderr renders an awkward two-line error with nothing under
    /// the label.
    #[test]
    fn test_auth_login_empty_stderr_tail_is_omitted() {
        let err = OnboardError::AuthLoginFailed(1, Some("   \n  ".into()));
        let msg = err.to_string();
        assert!(
            !msg.contains("claude stderr"),
            "blank tail should not render the label; got: {msg}"
        );
        assert_eq!(msg, "`claude auth login` exited with code 1");
    }

    /// Timeout sentinel takes precedence over any captured tail —
    /// the timeout message is the actionable one; a partial stderr
    /// tail from the killed child would just confuse.
    #[test]
    fn test_auth_login_timeout_ignores_stderr_tail() {
        let err = OnboardError::AuthLoginFailed(-2, Some("partial output before kill".into()));
        let msg = err.to_string();
        assert!(msg.contains("timed out"), "got: {msg}");
        assert!(
            !msg.contains("partial output"),
            "tail should be suppressed for the timeout sentinel; got: {msg}"
        );
    }

    /// Smoke test for the cancel path. The real `claude` binary isn't
    /// reliably installed in CI, so we stand in with a tiny shell
    /// stub that `exec`s into `sleep 30`. The stub ignores the
    /// `auth login` argv injected by the runner — it just needs to
    /// stay alive long enough for the test to fire the Notify. The
    /// test asserts that cancel resolves quickly with
    /// `AuthLoginCancelled` and that the temp dir no longer exists
    /// after the error-path cleanup.
    #[tokio::test]
    #[cfg(unix)]
    async fn cancel_kills_child_and_cleans_tempdir() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::sync::Notify;

        // Write a stub binary to a tempdir and make it executable.
        let stub_dir = tempfile::tempdir().expect("mk stub tempdir");
        let stub = stub_dir.path().join("claude-stub.sh");
        std::fs::write(&stub, "#!/bin/sh\nexec sleep 30\n").expect("write stub");
        let mut perms = std::fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).unwrap();

        // Snapshot any pre-existing `claudepot-onboard-*` directories
        // so stale state (from previous failed runs or parallel tests)
        // doesn't confuse the post-cleanup assertion below.
        let temp_root = std::env::temp_dir();
        let before: std::collections::HashSet<std::ffi::OsString> = std::fs::read_dir(&temp_root)
            .map(|it| {
                it.filter_map(|e| e.ok())
                    .map(|e| e.file_name())
                    .filter(|n| n.to_string_lossy().starts_with("claudepot-onboard-"))
                    .collect()
            })
            .unwrap_or_default();

        let notify = Arc::new(Notify::new());
        let notify_clone = notify.clone();
        let stub_path = stub.clone();

        let task = tokio::spawn(async move {
            run_auth_login_cancellable_with_binary(&stub_path, Some(notify_clone)).await
        });

        // Let the child actually spawn before we fire the Notify so
        // the tokio::select! is parked on child.wait() when the
        // cancel arm resolves.
        tokio::time::sleep(Duration::from_millis(150)).await;
        notify.notify_one();

        let outcome = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancel should complete within 5s")
            .expect("join handle should not panic");
        assert!(
            matches!(outcome, Err(OnboardError::AuthLoginCancelled)),
            "expected AuthLoginCancelled, got {outcome:?}"
        );

        // New tempdirs created by *this* test run must all be gone
        // after the awaited cleanup. Tempdirs from prior runs /
        // parallel tests are filtered out via the `before` snapshot.
        let after: std::collections::HashSet<std::ffi::OsString> = std::fs::read_dir(&temp_root)
            .map(|it| {
                it.filter_map(|e| e.ok())
                    .map(|e| e.file_name())
                    .filter(|n| n.to_string_lossy().starts_with("claudepot-onboard-"))
                    .collect()
            })
            .unwrap_or_default();
        let new_leftovers: Vec<_> = after.difference(&before).collect();
        assert!(
            new_leftovers.is_empty(),
            "cleanup should remove onboarding tempdirs created by this test run, still have: {new_leftovers:?}"
        );
    }
}
