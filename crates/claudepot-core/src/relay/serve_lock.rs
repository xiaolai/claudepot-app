//! `relay-serve.lock`: one process owns a data dir's relay stores.
//!
//! A serving relay holds `relay-devices.json` in memory and writes it from
//! that copy, so the lock is owned by that copy: `runtime::prepare` moves
//! it into the `RelayState` it builds, and it is released when the last
//! reference to that state drops — not when the caller stops listening.
//! A revoke still in flight over the control socket, or a GUI call holding
//! the state, keeps an offline edit refused until it has written.
//! [`super::service::acquire_serve_lock`] takes it.

use std::fs::File;
use std::path::{Path, PathBuf};

/// Held for as long as a relay may write its stores. Dropping it releases
/// the lock.
pub struct ServeLock {
    file: File,
    dir: PathBuf,
}

impl ServeLock {
    pub(super) fn new(file: File, dir: PathBuf) -> Self {
        Self { file, dir }
    }

    /// The data dir whose stores this lock guards.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A second reference to the locked file description, as a child
    /// process spawned while the lock is held inherits one.
    #[cfg(test)]
    pub(super) fn try_clone_file(&self) -> std::io::Result<File> {
        self.file.try_clone()
    }
}

impl std::fmt::Debug for ServeLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServeLock")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl Drop for ServeLock {
    /// Unlock explicitly rather than rely on the close. A child process
    /// spawned while the lock was held shares this open file description
    /// until it execs, and `flock` lasts until the last reference closes —
    /// so a bare close could leave the lock held for a moment after Stop,
    /// and a Start in that moment refused with "a relay is serving".
    fn drop(&mut self) {
        if let Err(e) = self.file.unlock() {
            tracing::warn!(error = %e, "could not release the relay serve lock");
        }
    }
}
