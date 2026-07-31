//! Change detection for readers — `PRAGMA data_version`, not a file
//! watcher.
//!
//! # Why not `notify`
//!
//! Filesystem watching is lossy and noisy against SQLite on every
//! platform Claudepot ships to: macOS coalesces events, Linux can miss
//! the distinction between the WAL sidecar and the main DB and can hit
//! inotify watch limits, and Windows produces a stream of lock and
//! rename events that do not correspond to commits. A watcher answers
//! "a file changed", which is not the question.
//!
//! `PRAGMA data_version` answers exactly the question: its value changes
//! when **another connection** commits to the database. It is portable
//! and exact.
//!
//! # What it does not give you
//!
//! A diff. `data_version` says *something* committed, never *what*. A
//! reader pairs it with a snapshot reload — poll the version, and when
//! it moves, re-read. That is why [`ChangeMonitor`] deliberately
//! exposes no row-level delta: offering one would imply a precision the
//! mechanism does not have.
//!
//! # The connection must be long-lived
//!
//! `data_version` is per-connection and only reflects writes from
//! *other* connections. A fresh connection per poll defeats it — the
//! value would be meaningless across connections. Hold one connection
//! for the monitor's lifetime.

use super::error::BoardError;
use super::store::BoardStore;

/// Tracks `data_version` on one long-lived connection.
///
/// ```no_run
/// # use claudepot_core::board::{BoardStore, ChangeMonitor, boards_db_path};
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let store = BoardStore::open(&boards_db_path())?;
/// let mut monitor = ChangeMonitor::new(&store)?;
/// // …later, on a timer:
/// if monitor.changed(&store)? {
///     // Re-read a snapshot. There is no delta to apply.
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeMonitor {
    last: i64,
}

impl ChangeMonitor {
    /// Start tracking from the store's current version.
    pub fn new(store: &BoardStore) -> Result<Self, BoardError> {
        Ok(Self {
            last: store.data_version()?,
        })
    }

    /// Whether another connection has committed since the last call.
    ///
    /// Consumes the change: two calls in a row return `true` then
    /// `false`. That makes it safe to drive a render loop directly
    /// without a separate acknowledgment step.
    pub fn changed(&mut self, store: &BoardStore) -> Result<bool, BoardError> {
        let now = store.data_version()?;
        if now == self.last {
            return Ok(false);
        }
        self.last = now;
        Ok(true)
    }

    /// The last observed version. Useful in logs; carries no ordering
    /// meaning across connections.
    pub fn version(&self) -> i64 {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::series::{Column, ColumnType};
    use crate::board::spec::BoardSpec;
    use crate::board::SeriesDef;

    fn series() -> SeriesDef {
        SeriesDef::new("runs", vec![Column::new("n", ColumnType::Integer)])
    }

    #[test]
    fn a_quiet_store_reports_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let store = BoardStore::open(&dir.path().join("boards.db")).unwrap();
        let mut m = ChangeMonitor::new(&store).unwrap();
        assert!(!m.changed(&store).unwrap());
        assert!(!m.changed(&store).unwrap());
    }

    #[test]
    fn a_write_from_another_connection_is_observed() {
        // data_version only moves for *other* connections, so the
        // writer here is a second store over the same file — which is
        // also the real topology: CLI writes, GUI watches.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.db");
        let reader = BoardStore::open(&path).unwrap();
        let writer = BoardStore::open(&path).unwrap();

        let mut m = ChangeMonitor::new(&reader).unwrap();
        assert!(!m.changed(&reader).unwrap());

        writer
            .create_board("n", &BoardSpec::empty(), &[series()])
            .unwrap();

        assert!(m.changed(&reader).unwrap());
    }

    #[test]
    fn change_is_consumed_so_a_render_loop_needs_no_ack() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.db");
        let reader = BoardStore::open(&path).unwrap();
        let writer = BoardStore::open(&path).unwrap();

        let mut m = ChangeMonitor::new(&reader).unwrap();
        writer
            .create_board("n", &BoardSpec::empty(), &[series()])
            .unwrap();

        assert!(m.changed(&reader).unwrap());
        assert!(!m.changed(&reader).unwrap());
    }
}
