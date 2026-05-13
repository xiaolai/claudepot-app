//! Direct filesystem + subprocess probes that produce the same fields
//! the pty scrape extracts — without depending on Ink's TUI render.
//!
//! ## Why
//!
//! The pty-based [`scrape`](super::scrape) replays an Ink render to
//! pluck out fields like `cc_version`, `install_type`, and
//! `install_path`. That replay is load-bearing on Ink's exact redraw
//! strategy; the next CC release that nudges its cursor-up cadence
//! can leave the parser holding a clobbered grid. When that happens,
//! the Health pane today shows "claude version unknown" with a red
//! dot — even though `claude --version` would have told us the
//! version in one fork-exec.
//!
//! These probes are the deterministic fallback. Each is a cheap,
//! testable function with one source of truth: a subprocess exit,
//! a file read, a `readlink`. The scrape stays in place — it still
//! provides the residue (auto-update channel, latest-version,
//! background-server status, plugin errors) that has no direct
//! filesystem source today — but the load-bearing identity fields
//! are sourced here.
//!
//! ## Scope rule
//!
//! Each function returns `Option<T>` and never panics. A probe that
//! fails returns `None`; callers compose probes with the scraped
//! snapshot at [`super::compose`]. Probe functions do not log
//! `tracing::warn!` at the failure path — silent failure is the
//! contract, and the [`super::parse_failures`] log already covers
//! the forensic ask for scrape failures. Adding warn logs here
//! would spam the dev console on every refresh in the (legitimate)
//! "claude not installed yet" boot state.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Result of [`probe_version`] — the identity triple for the running
/// `claude` binary, sourced directly from disk + a 50-ms subprocess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionProbe {
    /// Bare semver string as printed by `claude --version`. e.g.
    /// `"2.1.140"`. Matches the format CC's own doctor uses in the
    /// `Currently running:` row's parenthesized prefix.
    pub version: String,
    /// Resolved (symlink-followed, Windows-verbatim-stripped) path
    /// to the binary. e.g. `/Users/joker/.local/share/claude/versions/2.1.140`
    /// rather than `/Users/joker/.local/bin/claude`.
    pub binary_path: PathBuf,
    /// Heuristic classification of the install — `"native"`,
    /// `"npm-global"`, `"homebrew"`, `"volta"`, or `None` when no
    /// rule matched (callers fall back to whatever the scrape said).
    pub install_type: Option<String>,
}

/// Run `claude --version`, parse the bare semver out, follow the
/// binary's symlink for the canonical install path, and classify
/// the install type.
///
/// All three steps are best-effort and independent: a failure to
/// canonicalize doesn't void the version we already parsed. The
/// function returns `None` only when we can't locate a binary at
/// all OR the version line doesn't parse — both cases the caller
/// should treat as "unknown, fall back to scrape".
///
/// Wall-clock budget: <100 ms in steady state. Higher on a cold
/// disk cache, still bounded by [`SUBPROCESS_TIMEOUT`].
pub fn probe_version() -> Option<VersionProbe> {
    let binary = resolve_claude_binary()?;
    let version = run_version_subprocess(&binary)?;
    let canonical = resolve_install_path(&binary).unwrap_or_else(|| binary.clone());
    let install_type = classify_install_path(&canonical);
    Some(VersionProbe {
        version,
        binary_path: canonical,
        install_type,
    })
}

/// Locate the `claude` binary across every install method the
/// Health pane needs to support. Shared between [`probe_version`]
/// and [`super::scrape::spawn_and_capture`] so both surfaces see
/// the same candidate set; without sharing, probes would silently
/// fail on Homebrew installs where the binary is `claude-code`,
/// even though the pty scrape happily finds them.
///
/// Three tiers:
///
/// 1. Shared [`crate::fs_utils::find_claude_binary`] (covers
///    `~/.local/bin`, `/usr/local/bin`, `/usr/bin`, Windows
///    AppData) — same surface `services::doctor_service` uses.
/// 2. Doctor-specific extras: brew-cask `claude-code` paths,
///    npm-global via `~/.npm-global/bin` or system `node` prefix,
///    Volta shim. Deliberately kept here, not pushed into the
///    shared helper, because widening `find_claude_binary` changes
///    behavior for every other caller (CLI's `claudepot doctor`,
///    `currentCcIdentity`, etc.). Adding paths here is a
///    unidirectional change — easy to widen later if real install
///    distribution data justifies it.
/// 3. None — caller treats this as a missing-CC failure.
pub(super) fn resolve_claude_binary() -> Option<PathBuf> {
    if let Some(p) = crate::fs_utils::find_claude_binary() {
        return Some(p);
    }
    // The Homebrew cask `claude-code` installs the binary as
    // `claude-code` (not `claude`) — see
    // `crate::updates::detect::detect_cli_installs` which probes
    // exactly these paths. The doctor subcommand works against
    // either name; CC dispatches on argv[1] regardless of argv[0].
    let mut candidates: Vec<PathBuf> = vec![
        // Apple Silicon Homebrew (cask `claude-code`).
        PathBuf::from("/opt/homebrew/bin/claude-code"),
        // Intel Homebrew (cask `claude-code`).
        PathBuf::from("/usr/local/bin/claude-code"),
        // Same brew-cask binary under the Cellar shim path on Intel.
        PathBuf::from("/usr/local/Homebrew/bin/claude-code"),
        // Apple Silicon Homebrew formula installing `claude`
        // directly (less common; future-proofed).
        PathBuf::from("/opt/homebrew/bin/claude"),
        // Common Linux distro npm-global prefix.
        PathBuf::from("/usr/local/lib/node_modules/.bin/claude"),
    ];
    if let Some(home) = dirs::home_dir() {
        // User-local npm prefix; common when users `npm config set
        // prefix ~/.npm-global` to avoid `sudo npm install -g`.
        candidates.push(home.join(".npm-global/bin/claude"));
        // Volta shim.
        candidates.push(home.join(".volta/bin/claude"));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Wall-clock cap on the `claude --version` subprocess. Real runs
/// land in 30–80 ms; the cap exists so a half-installed claude that
/// hangs on stdin-read doesn't freeze the IPC handler.
const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(3);

fn run_version_subprocess(binary: &Path) -> Option<String> {
    // `--version` is non-interactive, exits in tens of milliseconds,
    // and does not need a tty. Synchronous + std::process is fine
    // here; the subprocess timeout below uses `wait_timeout`-style
    // polling rather than tokio, keeping this module free of an
    // async dependency.
    let mut child = Command::new(binary)
        .arg("--version")
        // Inherit nothing — no stdin, no shell pipes. The subprocess
        // must not adopt a parent's pty or env quirks.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    // Poll for exit within SUBPROCESS_TIMEOUT. The 25 ms slice keeps
    // total wall-clock close to the real exit time without burning
    // CPU on a tight loop.
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if start.elapsed() >= SUBPROCESS_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    };

    if !status.success() {
        return None;
    }

    let output = child.wait_with_output().ok()?;
    parse_version_line(&String::from_utf8_lossy(&output.stdout))
}

/// Extract the semver from `claude --version` stdout.
///
/// CC's current format is `"2.1.140 (Claude Code)\n"`. We take the
/// first whitespace-separated token and validate it as
/// `<digit>+ (. <digit>+)+`. Anything else returns `None` rather
/// than handing the scrape an invalid version string.
fn parse_version_line(stdout: &str) -> Option<String> {
    let first = stdout.split_whitespace().next()?;
    if !first.contains('.') {
        return None;
    }
    // Reject anything other than digits + dots so we don't mistake
    // `unknown` or an error message for a version.
    if !first.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    Some(first.to_string())
}

/// Resolve the binary's symlink chain and strip Windows verbatim
/// prefix. `~/.local/bin/claude` is typically a symlink to
/// `~/.local/share/claude/versions/<ver>`; the canonical path is
/// what makes [`classify_install_path`] work.
fn resolve_install_path(binary: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(binary).ok()?;
    // Per `.claude/rules/paths.md`: any canonicalize result must
    // pass through simplify_windows_path before being handed
    // outward. On non-Windows this is a no-op.
    let s = canonical.to_string_lossy().to_string();
    let simplified = crate::path_utils::simplify_windows_path(&s);
    Some(PathBuf::from(simplified))
}

/// Map a resolved binary path to CC's install_type vocabulary.
///
/// The order matters: native's `.local/share/claude/versions/`
/// prefix is the strongest signal and is checked first. Homebrew
/// before npm-global because brew-cask installs land in `Cellar/`
/// (apple-silicon) or under `/usr/local/Homebrew/`, both of which
/// could otherwise look generic. Volta is last because it shims
/// under `~/.volta/` regardless of upstream packaging.
///
/// ### Separator normalization
///
/// Per `.claude/rules/paths.md`, never hardcode `/` as the
/// separator. After `fs::canonicalize` on Windows, paths use `\`,
/// which means a literal `"/.local/share/claude/"` substring check
/// would never match a Windows-shaped path. We normalize `\` to
/// `/` before matching so the same rules fire on both hosts. (The
/// CC native-install layout is currently Unix-only, but Homebrew
/// patterns and the `node_modules` token are reachable from
/// cross-platform installs.)
pub fn classify_install_path(path: &Path) -> Option<String> {
    let raw = path.to_string_lossy();
    let s: String = raw.replace('\\', "/");

    // Native install — CC's official self-update layout.
    // `/.local/share/claude/versions/<ver>` is the canonical form.
    if s.contains("/.local/share/claude/versions/") {
        return Some("native".to_string());
    }

    // Homebrew, both architectures.
    if s.contains("/opt/homebrew/") || s.contains("/usr/local/Homebrew/") {
        return Some("homebrew".to_string());
    }

    // Volta — shims under `~/.volta/`.
    if s.contains("/.volta/") {
        return Some("volta".to_string());
    }

    // npm-global — either a user-prefix install or the system
    // node_modules tree. The `node_modules` token catches both.
    if s.contains("/.npm-global/") || s.contains("/node_modules/") {
        return Some("npm-global".to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_line_extracts_bare_semver() {
        assert_eq!(
            parse_version_line("2.1.140 (Claude Code)\n"),
            Some("2.1.140".to_string())
        );
    }

    #[test]
    fn parse_version_line_extracts_when_no_trailing_label() {
        assert_eq!(
            parse_version_line("2.0.0\n"),
            Some("2.0.0".to_string())
        );
    }

    #[test]
    fn parse_version_line_rejects_non_numeric() {
        assert_eq!(parse_version_line("unknown\n"), None);
        // No dot at all → not a semver.
        assert_eq!(parse_version_line("42\n"), None);
        // Contains a letter where we'd expect digits.
        assert_eq!(parse_version_line("2.x.140\n"), None);
    }

    #[test]
    fn parse_version_line_rejects_empty() {
        assert_eq!(parse_version_line(""), None);
        assert_eq!(parse_version_line("\n"), None);
    }

    #[test]
    fn classify_native_install_path() {
        let p = PathBuf::from("/Users/joker/.local/share/claude/versions/2.1.140");
        assert_eq!(classify_install_path(&p), Some("native".to_string()));
    }

    #[test]
    fn classify_homebrew_apple_silicon() {
        let p = PathBuf::from("/opt/homebrew/bin/claude-code");
        assert_eq!(classify_install_path(&p), Some("homebrew".to_string()));
    }

    #[test]
    fn classify_homebrew_intel_cellar() {
        let p = PathBuf::from("/usr/local/Homebrew/Cellar/claude-code/2.1.140/bin/claude-code");
        assert_eq!(classify_install_path(&p), Some("homebrew".to_string()));
    }

    #[test]
    fn classify_npm_global_user_prefix() {
        let p = PathBuf::from("/Users/joker/.npm-global/bin/claude");
        assert_eq!(classify_install_path(&p), Some("npm-global".to_string()));
    }

    #[test]
    fn classify_npm_global_system_nodemodules() {
        let p = PathBuf::from("/usr/local/lib/node_modules/@anthropic-ai/claude-code/bin/claude");
        assert_eq!(classify_install_path(&p), Some("npm-global".to_string()));
    }

    #[test]
    fn classify_volta() {
        let p = PathBuf::from("/Users/joker/.volta/bin/claude");
        assert_eq!(classify_install_path(&p), Some("volta".to_string()));
    }

    #[test]
    fn classify_unknown_returns_none() {
        let p = PathBuf::from("/some/random/bin/claude");
        assert_eq!(classify_install_path(&p), None);
    }

    #[test]
    fn classify_native_beats_node_modules_if_both_present() {
        // Hypothetical: someone has a native install whose path
        // also contains `node_modules` somewhere upstream. The
        // native check must run first because it's the more
        // specific signal.
        let p = PathBuf::from("/Users/me/.local/share/claude/versions/2.1.140/node_modules/.bin/claude");
        assert_eq!(classify_install_path(&p), Some("native".to_string()));
    }

    #[test]
    fn classify_normalizes_backslash_separator() {
        // After canonicalize on Windows, paths use `\`. The
        // separator normalization step in `classify_install_path`
        // converts to `/` before matching, so the same rules fire
        // regardless of host. The npm-global token (`node_modules`)
        // is the cross-platform classification with the strongest
        // signal — Homebrew and Volta both have host-specific
        // install layouts that the substring rules don't (yet)
        // canonicalize across, but `node_modules` is identical on
        // every platform CC supports.
        let p = PathBuf::from(
            r"C:\Users\me\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.cmd",
        );
        assert_eq!(classify_install_path(&p), Some("npm-global".to_string()));
    }
}
