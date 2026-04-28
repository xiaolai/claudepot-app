//! PID registry poller — reads `~/.claude/sessions/<pid>.json`.
//!
//! Claude Code writes one JSON file per top-level running process
//! (interactive, bg, daemon, daemon-worker). The writer is
//! `concurrentSessions.ts::registerSession` — subagents and
//! teammates deliberately skip registration so `claude ps` doesn't
//! conflate swarm usage with concurrent sessions.
//!
//! ### Strict filename guard
//!
//! The filename MUST match `^\d+\.json$`. `parseInt` in CC is lenient
//! and would treat `2026-03-14_notes.md` as pid 2026, causing the
//! sweep path to delete user notes — see
//! `anthropics/claude-code#34210`. We mirror CC's guard exactly.
//!
//! ### Staleness sweep
//!
//! When a registered pid is not running, CC deletes the file — except
//! on WSL, where a PID written inside WSL may not be probeable from
//! the Windows side (or vice versa). The plan retains that carve-out:
//! we sweep on macOS and Linux, observe-only on WSL.
//!
//! ### Malformed files
//!
//! A partial or unreadable file is not an error — CC might be
//! mid-write. The poller logs via `tracing` and skips the file; on
//! the next tick (default 2s) it retries.

use once_cell::sync::Lazy;
use regex::Regex;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::paths;
use crate::session_live::types::PidRecord;

/// `readdir` filename guard. Accepts only pure-integer stems with
/// `.json` extension. Every other name — `.DS_Store`, `2026-03-14.md`,
/// `1234.json.tmp` — is rejected.
static PID_FILENAME_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\d+\.json$").expect("static regex"));

/// One poll cycle produces this shape: the live records + paths
/// that looked stale on this tick. The caller decides whether to
/// sweep (see `sweep_stale`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PollOutcome {
    /// PIDs whose process is alive AND whose file parsed cleanly.
    pub live: Vec<PidRecord>,
    /// Files whose pid-in-filename is dead. Only populated outside
    /// WSL — on WSL the list is always empty (observe-only).
    pub stale_paths: Vec<PathBuf>,
    /// Files we encountered but couldn't parse. Kept separate from
    /// `stale_paths` so we don't sweep them — they may be mid-write.
    pub unparseable_paths: Vec<PathBuf>,
}

/// Trait injected into the poller so tests can feed synthetic
/// process lists without touching real PIDs.
///
/// The production impl (`SysinfoCheck`) caches a `sysinfo::System`
/// across calls; `prime` refreshes the cache with the exact PIDs
/// the caller is about to check, turning each `is_running` into an
/// O(1) membership check. Fake checks can ignore `prime` (default
/// no-op) since they don't read the live process table.
pub trait ProcessCheck: Send + Sync {
    fn is_running(&self, pid: u32) -> bool;

    /// Prime the check with the full set of PIDs about to be
    /// probed. Default is a no-op for fakes.
    fn prime(&self, _pids: &[u32]) {}
}

/// Production check via `sysinfo`. Caches the `System` behind a
/// `Mutex` so consecutive calls from the same poll tick reuse one
/// process list instead of rebuilding it per probe. Call
/// [`SysinfoCheck::refresh_for`] once per tick to prime the cache
/// with the exact PIDs the caller is about to check — that turns
/// the N-probe cost into a single system call.
pub struct SysinfoCheck {
    sys: std::sync::Mutex<sysinfo::System>,
}

impl Default for SysinfoCheck {
    fn default() -> Self {
        Self::new()
    }
}

impl SysinfoCheck {
    pub fn new() -> Self {
        Self {
            sys: std::sync::Mutex::new(sysinfo::System::new()),
        }
    }

    /// Prime the cached `System` with the given PIDs. Callers should
    /// invoke this once per poll tick before running `poll_dir` so
    /// each `is_running` call is a cheap HashMap lookup.
    pub fn refresh_for(&self, pids: &[u32]) {
        use sysinfo::{Pid, ProcessesToUpdate};
        let handles: Vec<Pid> = pids.iter().map(|p| Pid::from_u32(*p)).collect();
        let Ok(mut sys) = self.sys.lock() else { return };
        sys.refresh_processes(ProcessesToUpdate::Some(&handles), true);
    }
}

impl ProcessCheck for SysinfoCheck {
    fn is_running(&self, pid: u32) -> bool {
        use sysinfo::Pid;
        let Ok(sys) = self.sys.lock() else {
            return false;
        };
        sys.process(Pid::from_u32(pid)).is_some()
    }

    fn prime(&self, pids: &[u32]) {
        self.refresh_for(pids);
    }
}

/// Platform detector. On WSL the sweep is a no-op to mirror CC's own
/// `getPlatform() !== 'wsl'` carve-out at `concurrentSessions.ts:196`.
///
/// Three independent signals, any one enough to flip to WSL. The
/// reviews flagged that `/proc/version` alone misses custom kernels
/// and some distros — `osrelease` + `$WSL_DISTRO_NAME` fill those
/// gaps. On non-Linux platforms this is a compile-time no-op because
/// the sweep-on-WSL hazard (deleting a PID file visible only to the
/// other side of the WSL boundary) cannot exist when we're native.
#[cfg(target_os = "linux")]
pub(crate) fn is_wsl() -> bool {
    if std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some() {
        return true;
    }
    let check = |path: &str| -> bool {
        fs::read_to_string(path)
            .map(|s| {
                let l = s.to_lowercase();
                l.contains("microsoft") || l.contains("wsl")
            })
            .unwrap_or(false)
    };
    check("/proc/sys/kernel/osrelease") || check("/proc/version")
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn is_wsl() -> bool {
    false
}

/// Default sessions directory — `~/.claude/sessions`.
pub fn default_sessions_dir() -> PathBuf {
    paths::claude_config_dir().join("sessions")
}

/// Scan `dir` once and return the outcome. Does NOT delete stale
/// files on its own — the caller invokes `sweep_stale(&outcome)`
/// after they have acted on the live list.
pub fn poll_dir(dir: &Path, check: &dyn ProcessCheck) -> io::Result<PollOutcome> {
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // No registry dir → no sessions. Not an error.
            return Ok(PollOutcome::default());
        }
        Err(e) => return Err(e),
    };

    // First pass: enumerate candidate PIDs from filenames. Second
    // pass: probe. Splitting the passes lets the caller prime its
    // process table once with every PID we'll probe, turning the
    // N `is_running` calls into cheap membership checks. Without
    // this primer `SysinfoCheck::is_running` walks an empty cached
    // `System` — every probe returns false and live registry files
    // look stale.
    let mut candidates: Vec<(u32, std::path::PathBuf)> = Vec::new();
    for entry in read {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !PID_FILENAME_RE.is_match(&name_str) {
            continue;
        }
        let stem = &name_str[..name_str.len() - ".json".len()];
        let pid: u32 = match stem.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        candidates.push((pid, entry.path()));
    }
    // Prime the check with every PID we're about to probe.
    let pids: Vec<u32> = candidates.iter().map(|(p, _)| *p).collect();
    check.prime(&pids);

    let mut live = Vec::new();
    let mut stale_paths = Vec::new();
    let mut unparseable_paths = Vec::new();
    let on_wsl = is_wsl();

    for (pid, path) in candidates {
        if !check.is_running(pid) {
            if !on_wsl {
                stale_paths.push(path);
            }
            continue;
        }

        // Attempt to parse. Mid-write → JSON error → defer.
        match fs::read_to_string(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // Raced with a sweep — the file vanished between
                // readdir and read. Skip silently.
                continue;
            }
            Err(_) => {
                unparseable_paths.push(path);
                continue;
            }
            Ok(text) => match serde_json::from_str::<PidRecord>(&text) {
                Ok(mut rec) => {
                    // Trust the filename pid over the field in
                    // case of disagreement (CC always writes them
                    // equal, but be defensive).
                    rec.pid = pid;
                    live.push(rec);
                }
                Err(_) => {
                    unparseable_paths.push(path);
                }
            },
        }
    }

    Ok(PollOutcome {
        live,
        stale_paths,
        unparseable_paths,
    })
}

/// Delete files flagged stale by a prior `poll_dir`. Errors on any
/// individual file are logged and swallowed so one permission glitch
/// doesn't abort the whole sweep.
pub fn sweep_stale(outcome: &PollOutcome) {
    for path in &outcome.stale_paths {
        if let Err(e) = fs::remove_file(path) {
            tracing::debug!(
                target = "session_live::registry",
                ?path,
                error = %e,
                "failed to sweep stale pid file (non-fatal)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Write;
    use tempfile::TempDir;

    /// Synthetic process set for deterministic tests. PIDs declared
    /// "alive" here are reported as running regardless of reality.
    struct FakeCheck(HashSet<u32>);
    impl ProcessCheck for FakeCheck {
        fn is_running(&self, pid: u32) -> bool {
            self.0.contains(&pid)
        }
    }

    fn copy_fixture(dir: &Path, fixture: &str, as_name: &str) {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/session_live/testdata/pid")
            .join(fixture);
        let body = fs::read_to_string(&src)
            .unwrap_or_else(|_| panic!("fixture missing: {}", src.display()));
        let dst = dir.join(as_name);
        let mut f = std::fs::File::create(&dst).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    fn dir_with(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
        let td = TempDir::new().unwrap();
        let root = td.path().to_path_buf();
        for (name, body) in files {
            let mut f = fs::File::create(root.join(name)).unwrap();
            f.write_all(body.as_bytes()).unwrap();
        }
        (td, root)
    }

    #[test]
    fn regex_accepts_only_pure_integer_stems() {
        assert!(PID_FILENAME_RE.is_match("1234.json"));
        assert!(PID_FILENAME_RE.is_match("99001.json"));
        assert!(!PID_FILENAME_RE.is_match("2026-03-14_notes.md"));
        assert!(!PID_FILENAME_RE.is_match("1234.json.tmp"));
        assert!(!PID_FILENAME_RE.is_match(".DS_Store"));
        assert!(!PID_FILENAME_RE.is_match("1234.JSON"));
        assert!(!PID_FILENAME_RE.is_match("a1234.json"));
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let td = TempDir::new().unwrap();
        let missing = td.path().join("no-such-dir");
        let check = FakeCheck(HashSet::new());
        let out = poll_dir(&missing, &check).unwrap();
        assert!(out.live.is_empty());
        assert!(out.stale_paths.is_empty());
    }

    #[test]
    fn live_records_parsed_from_fixtures() {
        let td = TempDir::new().unwrap();
        copy_fixture(td.path(), "99001-bg-busy.json", "99001.json");
        copy_fixture(td.path(), "99002-bg-waiting.json", "99002.json");
        let alive: HashSet<u32> = [99001, 99002].into_iter().collect();
        let out = poll_dir(td.path(), &FakeCheck(alive)).unwrap();
        assert_eq!(out.live.len(), 2);
        let mut by_pid: Vec<_> = out.live.into_iter().collect();
        by_pid.sort_by_key(|r| r.pid);
        assert_eq!(by_pid[0].pid, 99001);
        assert_eq!(by_pid[0].status.as_deref(), Some("busy"));
        assert_eq!(by_pid[1].pid, 99002);
        assert_eq!(by_pid[1].waiting_for.as_deref(), Some("approve Bash"));
    }

    #[test]
    fn strict_regex_rejects_user_files() {
        // The decoy fixture is `2026-03-14_notes.md`. It must be
        // skipped entirely — not parsed, not swept, not flagged.
        let td = TempDir::new().unwrap();
        copy_fixture(td.path(), "2026-03-14_notes.md", "2026-03-14_notes.md");
        copy_fixture(td.path(), "99001-bg-busy.json", "99001.json");
        let alive: HashSet<u32> = [99001].into_iter().collect();
        let out = poll_dir(td.path(), &FakeCheck(alive)).unwrap();
        assert_eq!(out.live.len(), 1);
        assert!(out.stale_paths.is_empty());
        assert!(out.unparseable_paths.is_empty());
    }

    #[test]
    fn dead_pid_is_flagged_stale_on_non_wsl() {
        if is_wsl() {
            // On WSL sweep is disabled; the guard below would fail.
            // We don't intend to test the WSL path here.
            return;
        }
        let td = TempDir::new().unwrap();
        copy_fixture(td.path(), "99001-bg-busy.json", "99001.json");
        // Mark no pids alive — 99001 is stale.
        let out = poll_dir(td.path(), &FakeCheck(HashSet::new())).unwrap();
        assert!(out.live.is_empty());
        assert_eq!(out.stale_paths.len(), 1);
        assert!(out.stale_paths[0].ends_with("99001.json"));
    }

    #[test]
    fn malformed_file_goes_to_unparseable_not_live() {
        let td = TempDir::new().unwrap();
        copy_fixture(td.path(), "99006-malformed.json", "99006.json");
        let out = poll_dir(td.path(), &FakeCheck([99006].into_iter().collect())).unwrap();
        assert!(out.live.is_empty());
        assert_eq!(out.unparseable_paths.len(), 1);
        assert!(
            out.stale_paths.is_empty(),
            "malformed must NEVER be swept — it may be mid-write"
        );
    }

    #[test]
    fn sweep_deletes_flagged_paths() {
        let (td, root) = dir_with(&[(
            "99009.json",
            r#"{"pid":99009,"sessionId":"s","cwd":"/tmp/x","startedAt":0}"#,
        )]);
        let _ = &td;
        // Inject PollOutcome manually.
        let out = PollOutcome {
            live: vec![],
            stale_paths: vec![root.join("99009.json")],
            unparseable_paths: vec![],
        };
        sweep_stale(&out);
        assert!(!root.join("99009.json").exists());
    }

    #[test]
    fn filename_pid_wins_over_field_pid_disagreement() {
        // Contrived: a file named `99100.json` whose internal `pid`
        // field is 42. CC never writes these disagreeing but we
        // defend against hand-edits / races anyway.
        let (td, root) = dir_with(&[(
            "99100.json",
            r#"{"pid":42,"sessionId":"s","cwd":"/tmp/x","startedAt":0}"#,
        )]);
        let _ = &td;
        let out = poll_dir(&root, &FakeCheck([99100].into_iter().collect())).unwrap();
        assert_eq!(out.live.len(), 1);
        assert_eq!(out.live[0].pid, 99100);
    }

    #[test]
    fn empty_directory_yields_empty_outcome() {
        let td = TempDir::new().unwrap();
        let out = poll_dir(td.path(), &FakeCheck(HashSet::new())).unwrap();
        assert_eq!(out, PollOutcome::default());
    }
}
