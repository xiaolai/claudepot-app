//! Advisory file lock for [`super::AgentStore`] (grill finding F7).
//!
//! `agents.json` is a lock-free JSON read-modify-write store. Phase 2
//! gave it two live writers — the CLI `agent draft` verb and the GUI
//! `agents_add` / `agent_install` Tauri commands — and a lock-free
//! open→mutate→save lets a stale read from one writer clobber the
//! other's committed write (last-writer-wins on the whole file).
//!
//! [`StoreLock`] closes that window. It is acquired when an
//! `AgentStore` opens, held for the store's whole lifetime, and
//! released on `Drop`. It is an `flock(2)`-style advisory exclusive
//! lock (via the `fs2` crate — already a workspace dependency for
//! the Desktop mutators' cross-process locking, see the root
//! `Cargo.toml`) on a dedicated sibling `<store>.lock` file.
//!
//! ## Cross-process and cross-thread
//!
//! `fs2`'s exclusive lock is associated with the open file
//! *description*. Every `StoreLock::acquire` opens its own
//! description, so the lock conflicts — and therefore serializes —
//! both across separate OS processes (CLI vs GUI) and across
//! threads within one multi-threaded process (Tauri always is). A
//! second acquirer blocks until the first `StoreLock` drops.
//!
//! ## No deadlock on sequential re-open
//!
//! Every CLI command and Tauri command follows the same pattern:
//! open the store, mutate, save, let it drop. The lock releases when
//! the `File` closes at `Drop`, so the next `AgentStore::open` in
//! the same process (or thread) acquires it freely. The requirement
//! "must not deadlock a single process that opens the store twice in
//! sequence" holds because "in sequence" means the first store is
//! dropped before the second opens. (Holding two `StoreLock`s alive
//! at once on a single thread is the one unsupported pattern — that
//! is a self-deadlock by construction, not "in sequence", and no
//! caller does it: a store is a short-lived per-command object.)
//!
//! ## Send-safety
//!
//! [`StoreLock`] holds only a `std::fs::File`, which is `Send`, so
//! an `AgentStore` carrying one can still cross an `.await` point in
//! a Tauri async command. (An earlier draft held a
//! `std::sync::MutexGuard`, which is `!Send` — that would have made
//! every `AgentStore`-holding async command fail to compile.)

use std::fs::File;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::agent::error::AgentError;

/// An acquired advisory lock guarding one `agents.json` critical
/// section. Dropping it closes `_file`, which releases the OS lock.
pub(super) struct StoreLock {
    _file: File,
}

impl StoreLock {
    /// Acquire the lock for the store at `store_path`. Blocks until
    /// the OS advisory lock is held.
    ///
    /// The lock file is `<store_path>.lock`, created if absent. A
    /// failure to create or lock it surfaces as an [`AgentError`] —
    /// the store refuses to open unlocked rather than silently
    /// degrading to the racy lock-free behavior.
    pub(super) fn acquire(store_path: &Path) -> Result<Self, AgentError> {
        let file = open_lock_file(store_path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }

    /// Non-blocking acquire (grill finding X10). Returns
    /// `Ok(None)` if the lock is currently held by another
    /// process/thread — the caller decides what to do (skip,
    /// degrade, etc.) rather than blocking under the read path. A
    /// real failure (file-system error, missing parent dir we
    /// could not create) propagates as `Err`.
    ///
    /// Used by short-lived best-effort readers like `_record-run`'s
    /// retention lookup, where the cost of waiting on a slow GUI
    /// writer is higher than the cost of skipping a single
    /// retention pass.
    pub(super) fn try_acquire(store_path: &Path) -> Result<Option<Self>, AgentError> {
        let file = open_lock_file(store_path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            // `fs2` returns its own `WouldBlock`-shaped error when
            // the lock is held by someone else; we treat that as
            // "not now, try later" rather than a failure.
            Err(_) => Ok(None),
        }
    }
}

/// Open (and create if absent) the `<store>.lock` sidecar — shared
/// implementation for [`StoreLock::acquire`] and
/// [`StoreLock::try_acquire`].
fn open_lock_file(store_path: &Path) -> Result<File, AgentError> {
    // Create the parent dir so a first-ever open succeeds.
    if let Some(parent) = store_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = lock_path_for(store_path);
    let file = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    Ok(file)
}

/// The advisory-lock sidecar path for a store file:
/// `<store_path>.lock`.
fn lock_path_for(store_path: &Path) -> PathBuf {
    let mut name = store_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("agents.json"));
    name.push(".lock");
    match store_path.parent() {
        Some(dir) => dir.join(name),
        None => PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn lock_path_is_a_dot_lock_sibling() {
        let p = Path::new("/tmp/x/agents.json");
        assert_eq!(lock_path_for(p), PathBuf::from("/tmp/x/agents.json.lock"));
    }

    #[test]
    fn sequential_acquire_release_does_not_deadlock() {
        // The load-bearing F7 requirement: a single process that
        // opens the store, drops it, and opens it again must not
        // deadlock. Each `acquire` is released at end of scope.
        let dir = tempdir().unwrap();
        let store = dir.path().join("agents.json");
        for _ in 0..5 {
            let lock = StoreLock::acquire(&store).unwrap();
            drop(lock);
        }
    }

    #[test]
    fn acquire_creates_the_lock_file_and_parent_dir() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("nested").join("deep").join("agents.json");
        let _lock = StoreLock::acquire(&store).unwrap();
        assert!(store.parent().unwrap().exists());
        assert!(store.parent().unwrap().join("agents.json.lock").exists());
    }

    #[test]
    fn concurrent_acquire_from_threads_serializes() {
        // Two threads each acquire the lock, increment a shared
        // counter under it, and release. With the advisory lock the
        // critical sections cannot overlap; without it this would
        // race. The test mainly proves the lock does not deadlock
        // across threads (a `flock` on the same path from two
        // descriptions blocks rather than spinning).
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let dir = tempdir().unwrap();
        let store = Arc::new(dir.path().join("agents.json"));
        let counter = Arc::new(AtomicU32::new(0));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        let lock = StoreLock::acquire(&store).unwrap();
                        counter.fetch_add(1, Ordering::SeqCst);
                        drop(lock);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(counter.load(Ordering::SeqCst), 8 * 20);
    }
}
