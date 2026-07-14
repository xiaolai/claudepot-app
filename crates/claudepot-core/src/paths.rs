use std::path::PathBuf;

/// CC CLI config directory. Honors `$CLAUDE_CONFIG_DIR`, defaults to `~/.claude`.
pub fn claude_config_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".claude")
        })
}

/// CC CLI credentials file path.
pub fn claude_credentials_file() -> PathBuf {
    claude_config_dir().join(".credentials.json")
}

/// CC's `.claude.json` state file. CC stores it at `$HOME/.claude.json`
/// — a sibling of `~/.claude/`, not inside it. Central accessor so the
/// CLI and the Tauri shell agree on the location. `None` when the home
/// directory can't be resolved.
pub fn claude_json_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

/// Claude Desktop data directory (macOS / Windows). Returns None on Linux.
pub fn claude_desktop_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::data_dir().map(|d| d.join("Claude"))
    }
    #[cfg(target_os = "windows")]
    {
        dirs::data_local_dir().map(|d| {
            d.join("Packages")
                .join("Claude_pzs8sxrjxfjjc")
                .join("LocalCache")
                .join("Roaming")
                .join("Claude")
        })
    }
    #[cfg(target_os = "linux")]
    {
        None
    }
}

/// Claudepot's own private data root. Honors `$CLAUDEPOT_DATA_DIR`,
/// defaults to `$HOME/.claudepot` per the repository contract. The
/// previous implementation used `dirs::data_dir()/Claudepot` (resolving
/// to `~/Library/Application Support/Claudepot` on macOS), which split
/// state across machines and violated every path reference elsewhere
/// in the codebase.
pub fn claudepot_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDEPOT_DATA_DIR") {
        return PathBuf::from(dir);
    }

    // ─── GUARD: a TEST can never reach the real ~/.claudepot ─────────
    //
    // Compiled only into test builds. Without this, falling through to
    // the default here from a test hands out the developer's LIVE data
    // dir.
    //
    // This is not hypothetical. On 2026-07-14 it silently destroyed a
    // real `sessions.db` (129 -> 1 sessions, 8131 -> 0 exchanges): a
    // test passed a temp *config* dir, but the call chain reached
    // `list_all_sessions()`, which resolves its *data* dir here — and
    // that path's `refresh()` prunes rows for transcript files it cannot
    // see. The temp config dir had none, so it pruned everything.
    //
    // Rather than merely DETECT the violation (a panic the next test
    // author must remember to appease), make it UNREPRESENTABLE: hand
    // each test thread its own private temp data dir. Rust runs every
    // test in its own thread, so this isolates tests from each other as
    // well as from the user. A test that wants a specific dir still sets
    // CLAUDEPOT_DATA_DIR — that check above takes precedence.
    //
    // SCOPE — read this before trusting it:
    // `cfg(test)` is per-crate. This guard therefore covers claudepot-core's
    // own UNIT tests (where the incident happened) and NOTHING ELSE.
    // Integration tests under `tests/`, and tests in claudepot-cli /
    // src-tauri, link this crate as a normal dependency with `cfg(test)`
    // OFF — they get the real `~/.claudepot` unless they set
    // CLAUDEPOT_DATA_DIR themselves. That requirement is enforced at CI
    // level by `scripts/repo-invariants.sh` ("test binaries isolate the
    // data root"), not by the type system. Do not claim universal
    // isolation on the strength of this block alone.
    #[cfg(test)]
    {
        thread_local! {
            /// Per-test-thread data root. Dropped (and deleted) when the
            /// test thread exits.
            static TEST_DATA_DIR: tempfile::TempDir =
                tempfile::tempdir().expect("tempdir for test data root");
        }
        TEST_DATA_DIR.with(|d| d.path().to_path_buf())
    }

    #[cfg(not(test))]
    {
        default_data_dir()
    }
}

/// The default data root, `$HOME/.claudepot`.
///
/// Split out of [`claudepot_data_dir`] so the test-build guard there can
/// refuse the fallback while this computation stays directly testable.
fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".claudepot")
}

/// Per-account Desktop profile snapshot directory.
pub fn desktop_profile_dir(account_id: uuid::Uuid) -> PathBuf {
    claudepot_data_dir()
        .join("desktop")
        .join(account_id.to_string())
}

/// Claudepot's repair tree — canonical home for rename journals, per-
/// project locks, and pre-rename / pre-clean snapshots.
///
/// Lived at `<claude_config_dir>/claudepot/` in early builds (co-located
/// with CC's project tree). Consolidated into `<claudepot_data_dir>/repair/`
/// so everything Claudepot writes lives under one root. See
/// `migrations::migrate_repair_tree` for the one-time move at startup.
pub fn claudepot_repair_dir() -> PathBuf {
    claudepot_data_dir().join("repair")
}

/// `(journals, locks, snapshots)` triple rooted at `claudepot_repair_dir()`.
/// Callers that need all three prefer this over re-deriving each subdir
/// to keep the layout single-sourced.
pub fn claudepot_repair_dirs() -> (PathBuf, PathBuf, PathBuf) {
    let base = claudepot_repair_dir();
    (
        base.join("journals"),
        base.join("locks"),
        base.join("snapshots"),
    )
}

/// Diagnostic log directory. Honors `$CLAUDEPOT_LOG_DIR` for tests
/// and overrides; otherwise resolves per platform convention:
///
/// - macOS: `$HOME/Library/Logs/com.claudepot.app/`
/// - Windows: `%LOCALAPPDATA%\com.claudepot.app\logs\`
/// - Linux: `$XDG_STATE_HOME/com.claudepot.app/logs/`
///   (falls back to `$HOME/.local/state/com.claudepot.app/logs/`)
///
/// Used by the `tracing-appender` file sink + `claudepot logs`
/// CLI subcommand + the Settings → Cleanup "Reveal logs" button.
pub fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDEPOT_LOG_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("Library")
            .join("Logs")
            .join("com.claudepot.app")
    }
    #[cfg(target_os = "windows")]
    {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.claudepot.app")
            .join("logs")
    }
    #[cfg(target_os = "linux")]
    {
        dirs::state_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".local")
                    .join("state")
            })
            .join("com.claudepot.app")
            .join("logs")
    }
}

/// macOS crash-report directory — `$HOME/Library/Logs/DiagnosticReports/`.
///
/// Where the OS writes per-process `.ips` crash dumps. `None` off
/// macOS (no equivalent location; Linux/Windows crash capture goes
/// through the synchronous signal handler instead). Honors
/// `$CLAUDEPOT_DIAGNOSTIC_REPORTS_DIR` so the harvest path is testable
/// without a real crash on disk. See `crate::crash_reports`.
#[cfg(target_os = "macos")]
pub fn diagnostic_reports_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDEPOT_DIAGNOSTIC_REPORTS_DIR") {
        return Some(PathBuf::from(dir));
    }
    dirs::home_dir().map(|h| h.join("Library").join("Logs").join("DiagnosticReports"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::lock_data_dir;

    #[test]
    fn test_claude_config_dir_honors_env() {
        let _lock = lock_data_dir();
        std::env::set_var("CLAUDE_CONFIG_DIR", "/custom/config");
        let result = claude_config_dir();
        assert_eq!(result, PathBuf::from("/custom/config"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn test_claude_config_dir_default_fallback() {
        let _lock = lock_data_dir();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let result = claude_config_dir();
        // Should end with .claude (either ~/.claude or /tmp/.claude)
        assert!(result.ends_with(".claude"), "got: {}", result.display());
    }

    #[test]
    fn test_claude_credentials_file_is_under_config() {
        let _lock = lock_data_dir();
        std::env::set_var("CLAUDE_CONFIG_DIR", "/test/config");
        let result = claude_credentials_file();
        assert_eq!(result, PathBuf::from("/test/config/.credentials.json"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn test_claude_json_path_is_home_sibling() {
        let _lock = lock_data_dir();
        let result = claude_json_path();
        if let Some(home) = dirs::home_dir() {
            assert_eq!(result, Some(home.join(".claude.json")));
        } else {
            assert!(result.is_none());
        }
    }

    #[test]
    fn test_claudepot_data_dir_honors_env() {
        let _lock = lock_data_dir();
        std::env::set_var("CLAUDEPOT_DATA_DIR", "/custom/data");
        let result = claudepot_data_dir();
        assert_eq!(result, PathBuf::from("/custom/data"));
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    /// THE GUARD. Reproduces the 2026-07-14 data-loss incident at its
    /// root: with no `CLAUDEPOT_DATA_DIR` set, a test used to receive the
    /// developer's live `~/.claudepot`, and a downstream `refresh()`
    /// pruned a real `sessions.db` (129 -> 1 sessions, 8131 -> 0
    /// exchanges). A test must now be structurally incapable of seeing
    /// the real data root.
    #[test]
    fn a_test_can_never_reach_the_real_data_dir() {
        let _lock = lock_data_dir();
        std::env::remove_var("CLAUDEPOT_DATA_DIR");

        let got = claudepot_data_dir();
        let real = default_data_dir();

        assert_ne!(
            got, real,
            "claudepot_data_dir() handed a test the REAL data root — this is \
             the bug that destroyed a live sessions.db"
        );
        // And it must not be anywhere under the real root either.
        assert!(
            !got.starts_with(&real),
            "test data dir {} is inside the real data root {}",
            got.display(),
            real.display()
        );
    }

    /// Isolation is per-test-thread, so two tests can't clobber each
    /// other's data root (the reason we don't share one process-wide
    /// temp dir).
    #[test]
    fn test_data_dir_is_stable_within_a_thread() {
        let _lock = lock_data_dir();
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
        assert_eq!(claudepot_data_dir(), claudepot_data_dir());
    }

    #[test]
    fn test_claudepot_data_dir_default_is_home_dot_claudepot() {
        // Tests the default COMPUTATION directly. The public accessor is
        // guarded in test builds (see the test above), so it can't be
        // used to exercise the fallback.
        let result = default_data_dir();
        // Must be exactly $HOME/.claudepot per the repo contract —
        // not dirs::data_dir()/Claudepot, which was the prior default
        // and diverges from every other path reference in the codebase.
        assert!(result.ends_with(".claudepot"), "got: {}", result.display());
        // Verify it's in the home tree, not Library/Application Support
        // or similar platform-specific app-data location.
        if let Some(home) = dirs::home_dir() {
            assert!(
                result.starts_with(&home),
                "expected under {}, got {}",
                home.display(),
                result.display()
            );
        }
    }

    #[test]
    fn test_desktop_profile_dir_includes_uuid() {
        let _lock = lock_data_dir();
        std::env::set_var("CLAUDEPOT_DATA_DIR", "/data");
        let id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let result = desktop_profile_dir(id);
        assert_eq!(
            result,
            PathBuf::from("/data/desktop/550e8400-e29b-41d4-a716-446655440000")
        );
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }
}
