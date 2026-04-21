//! Claudepot rename-operation lockfile (spec §5.1).
//!
//! One lockfile per in-flight rename, keyed on the source CC project
//! dir's sanitized name. Stored at
//! `~/.claudepot/repair/locks/<old_san>.lock` (the legacy location was
//! `~/.claude/claudepot/locks/…`; `migrations::migrate_repair_tree`
//! moves it on first boot).
//!
//! Staleness rules (Q6):
//!   - Same hostname as current: `kill(pid, 0)` distinguishes
//!     ESRCH (dead) from EPERM / success (alive).
//!   - Cross-host: age-based, default 24h, env override
//!     `CLAUDEPOT_LOCK_STALE_HOURS`.

use crate::error::ProjectError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_STALE_HOURS: u64 = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lock {
    pub version: u32,
    pub pid: u32,
    pub hostname: String,
    pub start_iso8601: String,
    pub start_unix_secs: u64,
    pub claudepot_version: String,
}

/// RAII guard. Drop releases the lock if still held.
///
/// The guard stores a copy of the lock contents it wrote. Release
/// verifies the on-disk lock still matches (same pid + hostname +
/// start_unix_secs) before deleting, so a stale guard whose lock was
/// broken and re-acquired elsewhere will not delete the new holder's
/// file.
#[derive(Debug)]
pub struct LockGuard {
    pub path: PathBuf,
    owned: Lock,
    released: bool,
}

impl LockGuard {
    pub fn release(mut self) -> Result<(), ProjectError> {
        if self.still_own_lock() {
            fs::remove_file(&self.path).map_err(ProjectError::Io)?;
        }
        self.released = true;
        Ok(())
    }

    fn still_own_lock(&self) -> bool {
        if !self.path.exists() {
            return false;
        }
        match read_lock(&self.path) {
            Ok(cur) => {
                cur.pid == self.owned.pid
                    && cur.hostname == self.owned.hostname
                    && cur.start_unix_secs == self.owned.start_unix_secs
            }
            Err(_) => false,
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.released && self.still_own_lock() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Outcome of breaking a stale lock during acquisition. Used by
/// callers to audit-log the prior holder per spec §5.1.
#[derive(Debug, Clone)]
pub struct BrokenLockRecord {
    pub prior: Lock,
    pub reason: String,
}

/// Try to acquire the lock atomically. Uses `OpenOptions::create_new`
/// (O_EXCL under the hood) so two processes cannot both win the race.
/// A pre-existing lock is classified and either returned as-is (live)
/// or audit-logged + broken (stale). Returns the new guard plus an
/// optional record of a broken stale lock so callers can persist it
/// in their journal.
pub fn acquire(
    locks_dir: &Path,
    old_san: &str,
) -> Result<(LockGuard, Option<BrokenLockRecord>), ProjectError> {
    fs::create_dir_all(locks_dir).map_err(ProjectError::Io)?;
    let path = locks_dir.join(format!("{old_san}.lock"));

    let mut broken: Option<BrokenLockRecord> = None;

    loop {
        let now = SystemTime::now();
        let candidate = Lock {
            version: 1,
            pid: std::process::id(),
            hostname: whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string()),
            start_iso8601: chrono::DateTime::<chrono::Utc>::from(now)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            start_unix_secs: now
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        };

        match try_create_exclusive(&path, &candidate) {
            Ok(()) => {
                return Ok((
                    LockGuard {
                        path,
                        owned: candidate,
                        released: false,
                    },
                    broken,
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Another process (or our own past run) owns the path.
                // Classify and either wait-for-user or break.
                match read_lock(&path) {
                    Ok(existing) => {
                        if is_stale(&existing) {
                            let reason = stale_reason(&existing);
                            tracing::info!(
                                old = ?path,
                                pid = existing.pid,
                                reason = %reason,
                                "breaking stale claudepot lock"
                            );
                            broken = Some(BrokenLockRecord {
                                prior: existing.clone(),
                                reason,
                            });
                            // CAS-style removal: only delete if the lock
                            // on disk still matches the stale one we
                            // just classified. If another process has
                            // replaced it in the meantime, loop back and
                            // re-classify rather than clobbering theirs.
                            if let Ok(still) = read_lock(&path) {
                                if still.pid == existing.pid
                                    && still.hostname == existing.hostname
                                    && still.start_unix_secs == existing.start_unix_secs
                                {
                                    let _ = fs::remove_file(&path);
                                }
                                // else: someone replaced it; retry loop.
                            }
                            // File may or may not still exist now; let
                            // the next create_new tell us.
                            continue;
                        } else {
                            return Err(ProjectError::Ambiguous(format!(
                                "another claudepot rename is in progress \
                                 (pid={}, host={}). Run `claudepot project \
                                 repair --break-lock <path>` if you're sure \
                                 it's dead.",
                                existing.pid, existing.hostname
                            )));
                        }
                    }
                    Err(_) => {
                        // Unreadable: corrupt or partial write. Treat
                        // as stale and audit it. Use byte-wise CAS to
                        // avoid clobbering a fresh lock that may have
                        // replaced the corrupt one between our read
                        // and our remove.
                        let original_bytes = fs::read(&path).unwrap_or_default();
                        broken = Some(BrokenLockRecord {
                            prior: Lock {
                                version: 0,
                                pid: 0,
                                hostname: "?".to_string(),
                                start_iso8601: "?".to_string(),
                                start_unix_secs: 0,
                                claudepot_version: "?".to_string(),
                            },
                            reason: "unparseable lock file".to_string(),
                        });
                        if let Ok(current_bytes) = fs::read(&path) {
                            if current_bytes == original_bytes {
                                let _ = fs::remove_file(&path);
                            }
                            // else: someone replaced it; retry via loop.
                        }
                        continue;
                    }
                }
            }
            Err(e) => return Err(ProjectError::Io(e)),
        }
    }
}

/// Atomic-exclusive create: O_EXCL under the hood. Fails with
/// `AlreadyExists` if the file is already present.
fn try_create_exclusive(path: &Path, lock: &Lock) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    // Restrictive perms for lock file too — it contains host + pid.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
    }
    let json = serde_json::to_string_pretty(lock)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    f.write_all(json.as_bytes())?;
    f.sync_all()?;
    Ok(())
}

fn stale_reason(lock: &Lock) -> String {
    let current_host =
        whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    if lock.hostname == current_host {
        format!("same-host pid {} not alive (ESRCH)", lock.pid)
    } else {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let age = now_unix.saturating_sub(lock.start_unix_secs);
        format!(
            "cross-host ({}): age {}s exceeds staleness threshold",
            lock.hostname, age
        )
    }
}

/// Read a lock file. Used by staleness checks and `--break-lock`.
pub fn read_lock(path: &Path) -> Result<Lock, ProjectError> {
    let s = fs::read_to_string(path).map_err(ProjectError::Io)?;
    serde_json::from_str(&s).map_err(|e| ProjectError::Ambiguous(format!("lock read: {e}")))
}

/// Force-break a lock. Caller is responsible for user confirmation.
/// Returns the prior lock contents for audit.
pub fn break_lock(path: &Path) -> Result<Lock, ProjectError> {
    let lock = read_lock(path)?;
    fs::remove_file(path).map_err(ProjectError::Io)?;
    Ok(lock)
}

/// Composite staleness rule (spec §5.1).
pub fn is_stale(lock: &Lock) -> bool {
    let current_host =
        whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let age_based_stale = {
        let threshold_hours: u64 = std::env::var("CLAUDEPOT_LOCK_STALE_HOURS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_STALE_HOURS);
        let age = now_unix.saturating_sub(lock.start_unix_secs);
        age >= threshold_hours.saturating_mul(3600)
    };
    if lock.hostname != current_host {
        // Cross host: age-only.
        return age_based_stale;
    }
    // Same host: authoritative PID check on Unix; on non-Unix we
    // can't cheaply verify PID liveness, so we fall back to the
    // age-based rule to avoid permanently stranded locks.
    #[cfg(unix)]
    {
        pid_is_dead(lock.pid)
    }
    #[cfg(not(unix))]
    {
        age_based_stale
    }
}

/// Check whether a lock file's process is still alive (same-host only).
/// Returns true if the lock is live, false if dead or cross-host.
pub fn is_live(lock: &Lock) -> bool {
    let current_host =
        whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    if lock.hostname == current_host {
        !pid_is_dead(lock.pid)
    } else {
        // Cross host: we can't check liveness; pessimistically treat as
        // live so the journal nag suppression applies only when we
        // actively know it's running.
        false
    }
}

/// `kill(pid, 0)`: ESRCH = dead; EPERM or success = alive.
#[cfg(unix)]
fn pid_is_dead(pid: u32) -> bool {
    use libc::{kill, pid_t, ESRCH};
    let r = unsafe { kill(pid as pid_t, 0) };
    if r == 0 {
        return false; // alive and signalable
    }
    // errno
    let e = std::io::Error::last_os_error();
    match e.raw_os_error() {
        Some(err) if err == ESRCH => true, // dead
        _ => false,                         // EPERM (alive but ours) or other
    }
}

#[cfg(not(unix))]
fn pid_is_dead(_pid: u32) -> bool {
    // On non-Unix we don't have a cheap PID check; assume alive unless
    // age-based rule kicks in via the cross-host path.
    false
}

// Previous `write_atomic` (tempfile + rename) removed — that is NOT
// exclusive between concurrent acquirers. Use `try_create_exclusive`
// above, which uses `OpenOptions::create_new` (O_EXCL).

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acquire_and_release() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("locks");
        let (g, _broken) = acquire(&dir, "-proj").unwrap();
        assert!(g.path.exists());
        g.release().unwrap();
        assert!(!dir.join("-proj.lock").exists());
    }

    #[test]
    fn test_acquire_drops_releases() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("locks");
        let path;
        {
            let (g, _broken) = acquire(&dir, "-proj").unwrap();
            path = g.path.clone();
            assert!(path.exists());
            // Guard goes out of scope without explicit release.
        }
        assert!(!path.exists(), "Drop should release the lock");
    }

    #[test]
    fn test_double_acquire_blocked_while_live() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("locks");
        let _g = acquire(&dir, "-proj").unwrap();
        // Second acquire: our own PID is alive, same host → blocked
        // by the exclusive-create + same-host-live classification.
        let err = acquire(&dir, "-proj").err().unwrap();
        assert!(matches!(err, ProjectError::Ambiguous(_)));
    }

    #[test]
    fn test_break_stale_same_host_dead_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("locks");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("-proj.lock");
        // Write a lock with a PID that will never exist (u32::MAX).
        // Use a high PID that fits in i32 (pid_t) and is well above the
        // typical kernel pid_max (~99999 on macOS, ~4M on Linux). Not
        // bulletproof but reliable enough for tests.
        let fake = Lock {
            version: 1,
            pid: 99_999_999,
            hostname: whoami::fallible::hostname()
                .unwrap_or_else(|_| "unknown".to_string()),
            start_iso8601: "2026-01-01T00:00:00Z".to_string(),
            start_unix_secs: 1000,
            claudepot_version: "x".to_string(),
        };
        fs::write(&path, serde_json::to_string_pretty(&fake).unwrap()).unwrap();

        // is_stale should say yes.
        assert!(is_stale(&fake));
        // acquire should break and re-take the lock.
        let (g, _broken) = acquire(&dir, "-proj").unwrap();
        assert!(g.path.exists());
        g.release().unwrap();
    }

    #[test]
    fn test_break_lock_returns_prior_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("locks");
        let (g, _broken) = acquire(&dir, "-proj").unwrap();
        let path = g.path.clone();
        // Don't release; simulate user running --break-lock while a
        // holder is still in memory. `forget` keeps the file on disk.
        let _leaked = Box::leak(Box::new(g));

        let prior = break_lock(&path).unwrap();
        assert_eq!(prior.pid, std::process::id());
        assert!(!path.exists());
    }

    #[test]
    fn test_is_live_own_pid() {
        let lock = Lock {
            version: 1,
            pid: std::process::id(),
            hostname: whoami::fallible::hostname()
                .unwrap_or_else(|_| "unknown".to_string()),
            start_iso8601: "2026-01-01T00:00:00Z".to_string(),
            start_unix_secs: 1000,
            claudepot_version: "x".to_string(),
        };
        assert!(is_live(&lock));
    }

    #[test]
    fn test_cross_host_age_threshold() {
        // Simulate cross-host by using a hostname we definitely don't match.
        let lock = Lock {
            version: 1,
            pid: 1,
            hostname: "some-other-host-that-does-not-exist".to_string(),
            start_iso8601: "2026-01-01T00:00:00Z".to_string(),
            start_unix_secs: 0, // epoch → way old
            claudepot_version: "x".to_string(),
        };
        assert!(is_stale(&lock));

        let now_lock = Lock {
            start_unix_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            ..lock
        };
        assert!(!is_stale(&now_lock));
    }
}
