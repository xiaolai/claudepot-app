//! Per-`scope_root` advisory file lock used by lifecycle ops to
//! serialize disable / enable / trash within a single Claudepot
//! process AND across cooperating processes. The lock guards the
//! "validate destination, then rename" critical section so a
//! concurrent op can't race in and create the destination between
//! our check and our rename.
//!
//! The lock file lives at `<scope_root>/.disabled/.lock` and is
//! created on first use. We use `fs2::FileExt::lock_exclusive`
//! (advisory `flock` on Unix, `LockFileEx` on Windows). The lock
//! releases automatically when the returned `ScopeLock` is dropped.
//!
//! This is application-level cooperation only — a non-Claudepot
//! process editing the same artifact tree won't see the lock. CC
//! itself doesn't write to artifact files at runtime, so the
//! cooperative model covers our actual collision surface.

use crate::artifact_lifecycle::error::{LifecycleError, Result};
use crate::artifact_lifecycle::paths::DISABLED_DIR;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;

pub struct ScopeLock {
    file: File,
}

impl Drop for ScopeLock {
    fn drop(&mut self) {
        // Best-effort unlock; if the OS already released (process
        // shutdown, FD reuse), there's nothing meaningful to do.
        let _ = self.file.unlock();
    }
}

/// Acquire an exclusive lock on `<scope_root>/.disabled/.lock`,
/// creating the directory and file on demand. The returned guard
/// holds the lock until dropped.
///
/// Lifecycle ops should keep the guard alive for the entire
/// validate-and-mutate window, then drop it before announcing
/// success to higher layers.
pub fn acquire(scope_root: &Path) -> Result<ScopeLock> {
    let lock_dir = scope_root.join(DISABLED_DIR);
    std::fs::create_dir_all(&lock_dir).map_err(LifecycleError::io("create .disabled"))?;
    let lock_path = lock_dir.join(".lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(LifecycleError::io("open scope lock"))?;
    file.lock_exclusive()
        .map_err(LifecycleError::io("lock scope"))?;
    Ok(ScopeLock { file })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn second_acquire_blocks_until_first_drops() {
        let tmp = tempfile::tempdir().unwrap();
        let scope = tmp.path().join(".claude");
        std::fs::create_dir_all(&scope).unwrap();

        let lock1 = acquire(&scope).unwrap();
        let scope2 = scope.clone();
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _l = acquire(&scope2).unwrap();
            tx.send(()).unwrap();
        });

        // Other thread must block; nothing arrives within a short
        // window.
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());

        drop(lock1);

        // After we drop, the other thread acquires and signals.
        rx.recv_timeout(Duration::from_secs(2))
            .expect("second acquire must succeed once first releases");
        handle.join().unwrap();
    }

    #[test]
    fn acquire_creates_disabled_dir_on_demand() {
        let tmp = tempfile::tempdir().unwrap();
        let scope = tmp.path().join(".claude");
        std::fs::create_dir_all(&scope).unwrap();
        assert!(!scope.join(DISABLED_DIR).exists());
        let _l = acquire(&scope).unwrap();
        assert!(scope.join(DISABLED_DIR).exists());
        assert!(scope.join(DISABLED_DIR).join(".lock").exists());
    }
}
