//! Test utilities — only compiled when `#[cfg(test)]`.
//!
//! Provides a global mutex for tests that modify `CLAUDEPOT_DATA_DIR`.

use std::sync::Mutex;

/// Global lock for tests that modify the `CLAUDEPOT_DATA_DIR` env var.
/// All tests across all modules that call `setup_test_data_dir()` share
/// this lock to prevent env var races.
///
/// Use `lock_data_dir()` instead of `DATA_DIR_LOCK.lock()` directly
/// to handle mutex poisoning from earlier test panics.
pub static DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the DATA_DIR_LOCK, recovering from poison if a prior test panicked.
pub fn lock_data_dir() -> std::sync::MutexGuard<'static, ()> {
    DATA_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Set `CLAUDEPOT_DATA_DIR` to a fresh temp dir and return it.
/// Caller MUST hold `DATA_DIR_LOCK` for the duration of the test.
///
/// Also forces file-based credential storage via `CLAUDEPOT_CREDENTIAL_BACKEND=file`
/// so tests don't touch the real Keychain on macOS.
pub fn setup_test_data_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());
    std::env::set_var("CLAUDEPOT_CREDENTIAL_BACKEND", "file");
    dir
}

/// Create a test AccountStore backed by a temp SQLite DB.
pub fn test_store() -> (crate::account::AccountStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let store = crate::account::AccountStore::open(&db).unwrap();
    (store, dir)
}

/// Build a sample credential blob JSON string with a given `expires_at` (epoch millis).
pub fn sample_blob_json(expires_at: i64) -> String {
    format!(
        r#"{{"claudeAiOauth":{{"accessToken":"sk-ant-oat01-test","refreshToken":"sk-ant-ort01-test","expiresAt":{},"scopes":["user:inference","user:profile"],"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}}}"#,
        expires_at
    )
}

/// A blob that expires 1 hour from now (fresh).
pub fn fresh_blob_json() -> String {
    sample_blob_json(chrono::Utc::now().timestamp_millis() + 3_600_000)
}

/// A blob that expired 1 hour ago.
pub fn expired_blob_json() -> String {
    sample_blob_json(chrono::Utc::now().timestamp_millis() - 3_600_000)
}

/// A blob that expires in 2 minutes (within 5-minute margin).
pub fn expiring_soon_blob_json() -> String {
    sample_blob_json(chrono::Utc::now().timestamp_millis() + 120_000)
}

/// The **half-state**: a present-but-stale access token with no refresh
/// token to renew it.
///
/// Distinct from [`signed_out_blob_json`] on purpose. `is_signed_out()`
/// is FALSE here (the access token is non-empty), so this blob reaches
/// the 401 branch and must come back `Rejected` — the server was asked
/// and refused. Reporting it as signed-out would assert two things this
/// state does not establish: that Claude Code cleared its own
/// credentials, and that the account is otherwise fine.
pub fn no_refresh_token_blob_json() -> String {
    r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-stale","refreshToken":"","expiresAt":0,"scopes":["user:inference"],"subscriptionType":"max","rateLimitTier":"default_claude_max_20x"}}"#
        .to_string()
}

/// Claude Code's **cleared-credentials sentinel**: valid JSON, every key
/// present, both tokens emptied and `expiresAt` zeroed, while `scopes` /
/// `subscriptionType` / `rateLimitTier` survive intact.
///
/// Shaped from a real observation (issue #74) rather than invented — the
/// surviving metadata fields are what make it parse cleanly and read as
/// an ordinary expired blob. Every layer that must treat this as
/// terminal tests against these exact bytes, so a fix at one layer and a
/// different assumption at another cannot both look correct.
pub fn signed_out_blob_json() -> String {
    r#"{"claudeAiOauth":{"accessToken":"","refreshToken":"","expiresAt":0,"scopes":["user:inference","user:profile"],"subscriptionType":"max","rateLimitTier":"default_claude_max_20x"}}"#
        .to_string()
}

/// Cross-platform absolute path safe to use as an `Agent::cwd` value
/// in tests. Returns `/tmp` on Unix, `C:\Users\<user>\AppData\Local\Temp`
/// on Windows — both pass `Path::is_absolute()`, which is the only
/// thing `crate::agent::draft::validate_cwd` checks beyond `..`
/// component rejection.
///
/// Hardcoding `"/tmp"` in test fixtures Windows-fails because
/// `/tmp` has no drive letter; this helper keeps tests platform-
/// portable without leaking any `cfg!(windows)` branches into the
/// call sites.
pub fn test_cwd() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

/// [`test_cwd`] with a sub-component joined, returning a platform-
/// correct absolute string. Useful when a test needs two distinct
/// cwds (e.g. `test_cwd_sub("proj")` and `test_cwd_sub("repo")`) so
/// it can verify they're handled distinctly.
pub fn test_cwd_sub(name: &str) -> String {
    std::env::temp_dir()
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// Create a test Account with sensible defaults.
pub fn make_account(email: &str) -> crate::account::Account {
    crate::account::Account {
        uuid: uuid::Uuid::new_v4(),
        email: email.to_string(),
        org_uuid: Some("org-test".to_string()),
        org_name: Some("Test Org".to_string()),
        subscription_type: Some("pro".to_string()),
        rate_limit_tier: None,
        created_at: chrono::Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
        verified_email: None,
        verified_at: None,
        verify_status: "never".to_string(),
    }
}

/// Names the test a process spawned by [`run_in_child`] is for. A
/// child-only test body gates on [`in_child`] and returns immediately in
/// the ordinary parallel run.
const CHILD_MARKER: &str = "CLAUDEPOT_TEST_CHILD";

/// Run the test at `test` (its full path, e.g.
/// `agent::install::tests::x`) in a fresh copy of this test binary with
/// `set` added to and `remove` taken out of its environment, and assert
/// that exactly that test ran and passed.
///
/// For a test that needs a different process-wide environment — `PATH`,
/// `HOME`. Tests run as parallel threads of one process, so setting such
/// a variable in-process sets it for every test running at that moment,
/// and a lock only serializes the tests that take it. Clearing `PATH` that
/// way made `shared_memory::git`'s HEAD test fail whenever it spawned
/// `git` inside the window.
pub fn run_in_child(test: &str, set: &[(&str, &std::ffi::OsStr)], remove: &[&str]) {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD_MARKER, test);
    for (k, v) in set {
        cmd.env(k, v);
    }
    for k in remove {
        cmd.env_remove(k);
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "child run of {test} failed:\n{stdout}\n{stderr}"
    );
    // Guard against a vacuous pass: a filter that matched nothing
    // "succeeds" too.
    assert!(
        stdout.contains("1 passed"),
        "child run matched no test named {test}:\n{stdout}"
    );
}

/// Is this process the child [`run_in_child`] spawned for `test`?
pub fn in_child(test: &str) -> bool {
    std::env::var(CHILD_MARKER).as_deref() == Ok(test)
}
