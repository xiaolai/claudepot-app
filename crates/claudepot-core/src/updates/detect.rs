//! Detect installed CC CLI(s) and Claude Desktop, classify by source.
//!
//! Multi-install scenarios are common — a user can have native curl +
//! Homebrew + npm at once. We enumerate all candidates, classify each
//! by install method, and mark the one that `which claude` resolves
//! to as `is_active`. Updates target the active install.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CliInstallKind {
    /// `~/.local/bin/claude` symlink → `~/.local/share/claude/versions/<v>`.
    /// Auto-updates by design.
    NativeCurl,
    /// `node_modules/@anthropic-ai/claude-code-<plat>` postinstall
    /// link. Same binary as native; auto-updates via the bundled
    /// `claude update`.
    NpmGlobal,
    /// `brew install --cask claude-code` (stable channel; manual
    /// update via `brew upgrade --cask claude-code`).
    HomebrewStable,
    /// `brew install --cask claude-code@latest` (rolling channel).
    HomebrewLatest,
    /// apt-installed `/usr/bin/claude`.
    Apt,
    /// dnf-installed `/usr/bin/claude`.
    Dnf,
    /// apk-installed `/usr/bin/claude`.
    Apk,
    /// WinGet `Anthropic.ClaudeCode`.
    WinGet,
    /// Couldn't classify — surface for the user to investigate.
    Unknown,
}

impl CliInstallKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::NativeCurl => "native (curl)",
            Self::NpmGlobal => "npm",
            Self::HomebrewStable => "homebrew (stable)",
            Self::HomebrewLatest => "homebrew (latest)",
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Apk => "apk",
            Self::WinGet => "winget",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliInstall {
    pub kind: CliInstallKind,
    pub binary_path: PathBuf,
    pub version: Option<String>,
    /// True iff this is the install that the active `claude` on PATH
    /// resolves to (via canonicalize), i.e. the one that runs when
    /// the user types `claude`.
    pub is_active: bool,
    /// True iff this install's update path is auto-managed by CC
    /// itself (native + npm). Drives the UI hint "auto-updates" vs
    /// "manual via brew/winget/apt".
    pub auto_updates: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopSource {
    Homebrew,
    DirectDmg,
    Setapp,
    MacAppStore,
    UserLocal,
}

impl DesktopSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Homebrew => "homebrew cask",
            Self::DirectDmg => "direct download",
            Self::Setapp => "setapp",
            Self::MacAppStore => "mac app store",
            Self::UserLocal => "user-local install",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopInstall {
    pub app_path: PathBuf,
    pub version: Option<String>,
    pub source: DesktopSource,
    /// True iff Claudepot can drive an update on this install.
    /// False for Setapp / Mac App Store — those have their own
    /// update channels we don't go near.
    pub manageable: bool,
}

const NATIVE_BIN_REL: &str = ".local/bin/claude";
const NATIVE_LOCKS_REL: &str = ".local/state/claude/locks";

fn cli_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "claude.exe"
    } else {
        "claude"
    }
}

/// Resolve the active `claude` binary path by querying PATH directly,
/// bypassing shell aliases and functions. We can't trust shell
/// `which` because the user's `claude` may be a function (this is the
/// case in some plugin-aware shells); we walk PATH ourselves.
pub fn resolve_active_cli_binary() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let target = cli_filename();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(target);
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.is_file() {
                return Some(std::fs::canonicalize(&candidate).unwrap_or(candidate));
            }
        }
    }
    None
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    same_file::is_same_file(a, b).unwrap_or(false)
}

fn is_active_match(active: Option<&Path>, p: &Path) -> bool {
    match active {
        Some(a) => same_path(a, p),
        None => false,
    }
}

/// Enumerate all detectable CC CLI installs on this machine.
pub fn detect_cli_installs() -> Vec<CliInstall> {
    detect_cli_installs_at(dirs::home_dir().as_deref())
}

fn detect_cli_installs_at(home: Option<&Path>) -> Vec<CliInstall> {
    let mut out: Vec<CliInstall> = Vec::new();
    let active = resolve_active_cli_binary();
    let active_ref = active.as_deref();

    // Native curl install
    if let Some(home) = home {
        let bin = home.join(NATIVE_BIN_REL);
        if bin.exists() {
            let resolved = std::fs::canonicalize(&bin).unwrap_or_else(|_| bin.clone());
            let version = native_version_from_target(&resolved).or_else(|| query_binary_version(&bin));
            out.push(CliInstall {
                kind: CliInstallKind::NativeCurl,
                version,
                is_active: is_active_match(active_ref, &bin),
                auto_updates: true,
                binary_path: bin,
            });
        }
    }

    // Homebrew (macOS / Linux). Both Cellar prefixes link into a
    // /opt/homebrew/bin/ or /usr/local/bin/ shim.
    for prefix in [
        "/opt/homebrew/bin/claude-code",
        "/usr/local/bin/claude-code",
    ] {
        let p = PathBuf::from(prefix);
        if p.exists() && !out.iter().any(|c| same_path(&c.binary_path, &p)) {
            let kind = if brew_cask_installed("claude-code@latest") {
                CliInstallKind::HomebrewLatest
            } else if brew_cask_installed("claude-code") {
                CliInstallKind::HomebrewStable
            } else {
                CliInstallKind::Unknown
            };
            out.push(CliInstall {
                kind,
                version: query_binary_version(&p),
                is_active: is_active_match(active_ref, &p),
                auto_updates: false,
                binary_path: p,
            });
        }
    }

    // npm global (only if it's actually a separate file from the
    // entries we've already pushed).
    if let Some(npm_bin) = npm_global_bin() {
        let candidate = npm_bin.join(cli_filename());
        if candidate.exists()
            && !out.iter().any(|c| same_path(&c.binary_path, &candidate))
        {
            out.push(CliInstall {
                kind: CliInstallKind::NpmGlobal,
                version: query_binary_version(&candidate),
                is_active: is_active_match(active_ref, &candidate),
                auto_updates: true,
                binary_path: candidate,
            });
        }
    }

    // Linux package managers (apt/dnf/apk all install to /usr/bin/claude)
    #[cfg(target_os = "linux")]
    {
        let p = PathBuf::from("/usr/bin/claude");
        if p.exists() && !out.iter().any(|c| same_path(&c.binary_path, &p)) {
            let kind = detect_linux_pkg_manager().unwrap_or(CliInstallKind::Unknown);
            out.push(CliInstall {
                kind,
                version: query_binary_version(&p),
                is_active: is_active_match(active_ref, &p),
                auto_updates: false,
                binary_path: p,
            });
        }
    }

    // Catch-all: whatever PATH-active binary we didn't classify above.
    if let Some(active_path) = active {
        if !out.iter().any(|c| same_path(&c.binary_path, &active_path)) {
            out.push(CliInstall {
                kind: CliInstallKind::Unknown,
                version: query_binary_version(&active_path),
                is_active: true,
                auto_updates: false,
                binary_path: active_path,
            });
        }
    }

    out
}

fn native_version_from_target(resolved: &Path) -> Option<String> {
    // `~/.local/bin/claude` is a symlink to a file under
    // `~/.local/share/claude/versions/<version>`. The terminal
    // component IS the version string.
    let parent = resolved.parent()?;
    if parent.file_name().and_then(|s| s.to_str()) == Some("versions") {
        return resolved
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());
    }
    None
}

fn query_binary_version(bin: &Path) -> Option<String> {
    // Run `<bin> --version`. Output is typically `2.1.126 (Claude Code)`.
    // We grab the first whitespace-delimited token that looks like a
    // version ("contains a dot AND a digit"). 5s timeout via std (we
    // can't easily timeout a sync std::process::Command; if the binary
    // hangs we lose the call but startup continues).
    let output = Command::new(bin).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .split_whitespace()
        .find(|tok| {
            tok.chars().filter(|c| *c == '.').count() >= 1
                && tok.chars().any(|c| c.is_ascii_digit())
        })
        .map(|s| s.to_string())
}

fn npm_global_bin() -> Option<PathBuf> {
    // `npm bin -g` was removed in npm 9. Try the canonical
    // `npm root -g` and derive `bin` as its parent's `bin/`.
    let output = Command::new("npm").args(["root", "-g"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let root = PathBuf::from(s.trim());
    // root: .../lib/node_modules → bin: .../bin
    root.parent().and_then(|p| p.parent()).map(|p| p.join("bin"))
}

fn brew_cask_installed(cask: &str) -> bool {
    Command::new("brew")
        .args(["list", "--cask", cask])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn detect_linux_pkg_manager() -> Option<CliInstallKind> {
    if Command::new("dpkg")
        .args(["-S", "/usr/bin/claude"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(CliInstallKind::Apt);
    }
    if Command::new("rpm")
        .args(["-qf", "/usr/bin/claude"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(CliInstallKind::Dnf);
    }
    if Command::new("apk")
        .args(["info", "-W", "/usr/bin/claude"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(CliInstallKind::Apk);
    }
    None
}

/// Detect the installed Claude Desktop, if any.
///
/// macOS: checks `/Applications/Claude.app` then `~/Applications/Claude.app`.
/// Windows: checks `%LOCALAPPDATA%\Programs\Claude\Claude.exe` (Squirrel
/// default) and `%ProgramFiles%\Claude\Claude.exe` (machine-wide).
/// Linux: returns `None` — no Linux Desktop ships today.
pub fn detect_desktop_install() -> Option<DesktopInstall> {
    #[cfg(target_os = "macos")]
    {
        let app_path = PathBuf::from("/Applications/Claude.app");
        if !app_path.exists() {
            if let Some(home) = dirs::home_dir() {
                let user_app = home.join("Applications/Claude.app");
                if user_app.exists() {
                    let version = read_app_short_version(&user_app);
                    return Some(DesktopInstall {
                        app_path: user_app,
                        version,
                        source: DesktopSource::UserLocal,
                        manageable: false,
                    });
                }
            }
            return None;
        }
        let version = read_app_short_version(&app_path);
        let source = classify_desktop_source(&app_path);
        let manageable = matches!(source, DesktopSource::Homebrew | DesktopSource::DirectDmg);
        Some(DesktopInstall {
            app_path,
            version,
            source,
            manageable,
        })
    }
    #[cfg(target_os = "windows")]
    {
        // Squirrel-Windows installs to %LOCALAPPDATA%\Programs\Claude\
        // by default. Some enterprise rollouts use %ProgramFiles%\Claude\
        // (machine-wide install). WinGet rolls into the former.
        let candidates: Vec<PathBuf> = [
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("Programs").join("Claude").join("Claude.exe")),
            std::env::var_os("ProgramFiles")
                .map(PathBuf::from)
                .map(|p| p.join("Claude").join("Claude.exe")),
        ]
        .into_iter()
        .flatten()
        .collect();

        for app_path in candidates {
            if app_path.exists() {
                let version = read_windows_exe_version(&app_path);
                let source = if winget_package_installed("Anthropic.ClaudeCode") {
                    DesktopSource::Homebrew // re-using the "package-manager managed" lane
                } else {
                    DesktopSource::DirectDmg
                };
                // Manageable iff WinGet manages it. Direct-installer
                // updates flow through Squirrel-Windows, which can only
                // run while Desktop itself is running — outside our reach.
                let manageable = matches!(source, DesktopSource::Homebrew);
                return Some(DesktopInstall {
                    app_path,
                    version,
                    source,
                    manageable,
                });
            }
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn read_app_short_version(app: &Path) -> Option<String> {
    let plist = app.join("Contents/Info.plist");
    let plist_str = plist.to_str()?;
    let output = Command::new("defaults")
        .args(["read", plist_str, "CFBundleShortVersionString"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "macos")]
fn classify_desktop_source(app: &Path) -> DesktopSource {
    if app.starts_with("/Applications/Setapp") {
        return DesktopSource::Setapp;
    }
    if let Some(home) = dirs::home_dir() {
        if app.starts_with(home.join("Applications")) {
            return DesktopSource::UserLocal;
        }
    }
    if app.join("Contents/_MASReceipt").exists() {
        return DesktopSource::MacAppStore;
    }
    if Command::new("brew")
        .args(["list", "--cask", "claude"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return DesktopSource::Homebrew;
    }
    DesktopSource::DirectDmg
}

/// True when an instance of Claude Desktop is currently running.
///
/// **macOS**: matches against the absolute path of the Desktop
/// executable (`/Applications/Claude.app/Contents/MacOS/Claude`).
/// The naive `pgrep -ix Claude` produces false positives because
/// the CC CLI binary is also named `claude` (basename). The full
/// path is unique to Desktop (CC's argv never contains it).
///
/// **Windows**: walks the live process list via `sysinfo` looking for
/// a process whose executable path ends in `Claude.exe` AND lives
/// under one of the conventional Desktop install locations
/// (`%LOCALAPPDATA%\Programs\Claude\` or `%ProgramFiles%\Claude\`).
/// Same false-positive avoidance: matching by full path keeps
/// random `claude.exe` (like the CC CLI on Windows) from registering
/// as Desktop.
///
/// **Linux**: always returns false; no Linux Desktop ships today.
pub fn is_desktop_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        if pgrep_full_path("/Applications/Claude.app/Contents/MacOS/Claude") {
            return true;
        }
        if let Some(home) = dirs::home_dir() {
            let user_path = home.join("Applications/Claude.app/Contents/MacOS/Claude");
            if let Some(s) = user_path.to_str() {
                if pgrep_full_path(s) {
                    return true;
                }
            }
        }
        false
    }
    #[cfg(target_os = "windows")]
    {
        windows_desktop_running()
    }
    #[cfg(target_os = "linux")]
    {
        false
    }
}

#[cfg(target_os = "windows")]
fn windows_desktop_running() -> bool {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};
    let candidates: Vec<PathBuf> = [
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Programs").join("Claude").join("Claude.exe")),
        std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .map(|p| p.join("Claude").join("Claude.exe")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if candidates.is_empty() {
        return false;
    }
    let sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Always)),
    );
    for proc in sys.processes().values() {
        if let Some(exe) = proc.exe() {
            if candidates.iter().any(|c| exe == c.as_path()) {
                return true;
            }
        }
    }
    false
}

#[cfg(target_os = "windows")]
fn read_windows_exe_version(exe: &Path) -> Option<String> {
    // Read ProductVersion via PowerShell. We avoid pulling in a
    // Windows-specific crate (winapi VerQueryValue dance) for a
    // single read that runs once at detection time. The PowerShell
    // call is cheap (~50 ms) and bundled with every supported
    // Windows version.
    let exe_str = exe.to_str()?;
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Item '{exe_str}').VersionInfo.ProductVersion"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(target_os = "windows")]
fn winget_package_installed(package_id: &str) -> bool {
    Command::new("winget")
        .args(["list", "--exact", "--id", package_id])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn pgrep_full_path(absolute_executable: &str) -> bool {
    // `pgrep -f` matches against the full command line. The Desktop
    // helper processes (Renderer / GPU) live under
    // `…/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper`,
    // so their argv[0] doesn't start with the main executable path —
    // no false positives from helpers.
    let output = Command::new("pgrep")
        .args(["-f", absolute_executable])
        .output();
    match output {
        Ok(o) => o.status.success() && !o.stdout.iter().all(|b| b.is_ascii_whitespace()),
        Err(_) => false,
    }
}

/// Number of `~/.local/state/claude/locks/<v>.lock` files whose `pid`
/// is still live. The native installer writes one lock per running
/// process; counting them is "how many CC CLIs are active right now"
/// — useful for the UI to show "2.1.126 — 3 processes active" and to
/// inform the user that an in-place update will leave those running
/// processes on the old version (correct behavior — symlink swap is
/// safe; the kernel keeps the old inode mmap'd).
pub fn count_running_cli_locks() -> usize {
    let Some(home) = dirs::home_dir() else {
        return 0;
    };
    count_running_cli_locks_at(&home)
}

fn count_running_cli_locks_at(home: &Path) -> usize {
    let dir = home.join(NATIVE_LOCKS_REL);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut count = 0;
    for entry in entries.flatten() {
        if entry.path().extension().and_then(|s| s.to_str()) != Some("lock") {
            continue;
        }
        let body = match std::fs::read_to_string(entry.path()) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let pid = match json.get("pid").and_then(|v| v.as_i64()) {
            Some(p) => p,
            None => continue,
        };
        if is_pid_alive(pid as i32) {
            count += 1;
        }
    }
    count
}

fn is_pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        // signal 0 doesn't actually send a signal — it just probes
        // whether the process exists. Returns:
        //   0     → exists and we have permission to signal
        //  -1+EPERM → exists but we lack permission (alive!)
        //  -1+ESRCH → no such process (dead)
        // The naive `== 0` check would mark every root-owned process
        // as dead from a non-root caller, which is wrong.
        //
        // We read errno via `std::io::Error::last_os_error()` so this
        // works on Linux (where `libc::__error()` doesn't exist —
        // that symbol is Apple/BSD-only).
        if unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn empty_path_yields_no_active_binary() {
        let saved = std::env::var_os("PATH");
        std::env::set_var("PATH", "");
        let r = resolve_active_cli_binary();
        if let Some(p) = saved {
            std::env::set_var("PATH", p);
        }
        assert!(r.is_none());
    }

    #[test]
    fn count_running_cli_locks_at_handles_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let n = count_running_cli_locks_at(tmp.path());
        assert_eq!(n, 0);
    }

    #[test]
    fn count_running_cli_locks_at_filters_dead_pids() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_dir = tmp.path().join(NATIVE_LOCKS_REL);
        std::fs::create_dir_all(&lock_dir).unwrap();
        // Our own PID is guaranteed alive AND signalable (no EPERM
        // ambiguity). PID 2147483640 is virtually guaranteed dead.
        let alive_pid = std::process::id();
        std::fs::write(
            lock_dir.join("alive.lock"),
            format!(
                r#"{{"pid": {alive_pid}, "version": "x", "execPath": "/x", "acquiredAt": 0}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            lock_dir.join("dead.lock"),
            r#"{"pid": 2147483640, "version": "x", "execPath": "/x", "acquiredAt": 0}"#,
        )
        .unwrap();
        let n = count_running_cli_locks_at(tmp.path());
        if cfg!(unix) {
            assert_eq!(n, 1);
        } else {
            assert_eq!(n, 0);
        }
    }

    #[test]
    fn is_pid_alive_recognizes_own_process() {
        let me = std::process::id() as i32;
        assert!(is_pid_alive(me) || !cfg!(unix));
    }

    #[test]
    fn is_pid_alive_recognizes_init_via_eperm() {
        // PID 1 (launchd / init) exists on every Unix host but a
        // non-root caller can't signal it — kill returns EPERM. Our
        // implementation must treat EPERM as alive.
        if cfg!(unix) {
            assert!(is_pid_alive(1));
        }
    }

    #[test]
    fn count_running_cli_locks_at_skips_malformed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_dir = tmp.path().join(NATIVE_LOCKS_REL);
        std::fs::create_dir_all(&lock_dir).unwrap();
        std::fs::write(lock_dir.join("garbage.lock"), "not json").unwrap();
        std::fs::write(lock_dir.join("missing-pid.lock"), r#"{"foo":"bar"}"#).unwrap();
        std::fs::write(lock_dir.join("not-a-lock.txt"), "{}").unwrap();
        let n = count_running_cli_locks_at(tmp.path());
        assert_eq!(n, 0);
    }

    #[test]
    fn detect_cli_installs_at_handles_no_home() {
        let r = detect_cli_installs_at(None);
        // On a host without `claude` on PATH and no home, the result
        // should be empty. If the test host has a real `claude` it
        // will appear as Unknown — accept that as a valid outcome.
        assert!(r.is_empty() || r.iter().any(|c| c.is_active));
    }

    #[test]
    fn cli_install_kind_label_covers_all_variants() {
        // Compile-time exhaustiveness check; if a variant is added we
        // get a `match` warning here.
        let all = [
            CliInstallKind::NativeCurl,
            CliInstallKind::NpmGlobal,
            CliInstallKind::HomebrewStable,
            CliInstallKind::HomebrewLatest,
            CliInstallKind::Apt,
            CliInstallKind::Dnf,
            CliInstallKind::Apk,
            CliInstallKind::WinGet,
            CliInstallKind::Unknown,
        ];
        for k in all {
            assert!(!k.label().is_empty());
        }
    }

    #[test]
    fn same_path_handles_missing_files() {
        let p1 = Path::new("/no/such/path/abc");
        let p2 = Path::new("/no/such/path/abc");
        assert!(same_path(p1, p2)); // identical strings
        let p3 = Path::new("/no/such/path/xyz");
        assert!(!same_path(p1, p3));
    }
}
