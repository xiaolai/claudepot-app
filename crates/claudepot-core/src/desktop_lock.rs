//! Cross-process + cross-command operation lock for Claude Desktop
//! mutators (adopt, clear, switch, reconcile-write, sync).
//!
//! Per Codex plan review D2-1: the Mutex<Connection> inside
//! AccountStore serializes SQLite writes but nothing guards against
//! two Tauri commands, CLI + GUI, or tray + CLI all racing on the
//! on-disk snapshot directory and Desktop's live data_dir.
//!
//! This module ships the core primitive — an advisory flock on
//! `~/.claudepot/desktop.lock`. The Tauri state layer layers an
//! in-process async Mutex on top (see src-tauri/src/state.rs) so
//! two commands inside the GUI also serialize without spinning on
//! flock.

use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// RAII guard — holds the advisory lock until drop. Create via
/// [`acquire`] or [`try_acquire`].
#[derive(Debug)]
pub struct DesktopLockGuard {
    file: File,
    // Path stored for diagnostics only.
    #[allow(dead_code)]
    path: PathBuf,
}

impl Drop for DesktopLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DesktopLockError {
    #[error("desktop lock file open failed: {0}")]
    Open(String),
    #[error("desktop operation already in progress — retry in a moment")]
    Held,
    #[error("desktop lock wait timed out after {0:?}")]
    Timeout(Duration),
}

fn lock_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join("desktop.lock")
}

fn open_lockfile() -> Result<File, DesktopLockError> {
    let path = lock_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| DesktopLockError::Open(e.to_string()))
}

/// Non-blocking acquire. Returns [`DesktopLockError::Held`] immediately
/// if another process holds the lock. Use this for commands that should
/// fail-fast rather than queue — the tray, for instance.
pub fn try_acquire() -> Result<DesktopLockGuard, DesktopLockError> {
    let file = open_lockfile()?;
    file.try_lock_exclusive()
        .map_err(|_| DesktopLockError::Held)?;
    Ok(DesktopLockGuard {
        file,
        path: lock_path(),
    })
}

/// Blocking acquire with a bounded wait. Polls every 200 ms up to
/// `timeout`. Used by interactive commands (CLI + GUI confirmed
/// operations) where queuing is preferable to failing.
pub fn acquire(timeout: Duration) -> Result<DesktopLockGuard, DesktopLockError> {
    let deadline = Instant::now() + timeout;
    loop {
        match try_acquire() {
            Ok(g) => return Ok(g),
            Err(DesktopLockError::Held) => {
                if Instant::now() >= deadline {
                    return Err(DesktopLockError::Timeout(timeout));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acquire_and_release() {
        let _env_lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let g = try_acquire().unwrap();
        drop(g);
        // Second acquire after drop should succeed.
        let _g2 = try_acquire().unwrap();
    }

    #[test]
    fn test_try_acquire_returns_held_when_locked() {
        let _env_lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let _g = try_acquire().unwrap();
        // Second try from the SAME process still needs to respect
        // the exclusive flock — fs2 enforces this correctly on
        // macOS + Linux via flock(2).
        let err = try_acquire().unwrap_err();
        assert!(matches!(err, DesktopLockError::Held));
    }

    #[test]
    fn test_acquire_times_out() {
        let _env_lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let _g = try_acquire().unwrap();
        let t0 = Instant::now();
        let err = acquire(Duration::from_millis(300)).unwrap_err();
        // Timeout path must surface Timeout, not Held.
        assert!(matches!(err, DesktopLockError::Timeout(_)));
        assert!(t0.elapsed() >= Duration::from_millis(300));
    }
}
