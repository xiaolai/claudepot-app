//! In-memory cache of the expensive half of a project listing.
//!
//! `project_list` needs, per project, a recursive size and a recursive
//! max-mtime. On the reference machine that walk is 29,810 `stat` calls
//! over 11 GB — measured at 1.1–1.3 s in a release build, paid again on
//! every mount of the Projects tab and every ⌘R. 90% of those calls are
//! below the top level: the per-session folders CC writes beside each
//! transcript (`subagents/`, `workflows/`, …), which hold 10,977 of the
//! 13,562 transcripts here.
//!
//! Only that nested share is cached. The top level — session counts,
//! memory-file counts, transcript bytes, and every flag that gates
//! behaviour, `is_empty` included — is measured fresh on every listing,
//! so nothing a user can act on is served from here. What lags is the
//! nested contribution to one size column and to a sort key, by at most
//! one listing.
//!
//! Deliberately **in memory and not persisted**. It is a pure function
//! of the filesystem, so it is safe to lose and safe to recompute; a
//! file in `~/.claudepot/` would be a new thing to document, migrate,
//! and invalidate for a number that is rebuilt in under a second.
//!
//! Why not simply parallelise the walk: rayon was measured at 1.25 s →
//! 0.72 s here. The work is stat-bound and one slug dominates it, so
//! parallelism alone does not get the listing under a tenth of a
//! second. The listing is parallel *as well* — see
//! `project::list_projects_cached`.

use claudepot_core::project::helpers::NestedScan;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Managed Tauri state. `Arc` so the background refresh can own a
/// handle without borrowing from the command's `State`.
#[derive(Clone, Default)]
pub struct ProjectSizeCache {
    inner: Arc<Mutex<HashMap<String, NestedScan>>>,
    /// Set while a background refresh is walking. `project_list`
    /// dispatches one on every call, so without this a user holding ⌘R
    /// queues a redundant full walk per press onto the blocking pool —
    /// each one recomputing what the one before it just computed, while
    /// occupying a worker other commands need.
    refreshing: Arc<AtomicBool>,
}

impl ProjectSizeCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of what is known. Empty on the first call of a process,
    /// which is the caller's cue to measure synchronously.
    ///
    /// Recovers from poisoning rather than panicking: the value is a
    /// cache of file sizes, so a poisoned lock has nothing to roll back
    /// (project rules forbid `expect` here, and a panic in a listing
    /// would be a worse outcome than a stale byte count).
    pub fn snapshot(&self) -> HashMap<String, NestedScan> {
        match self.inner.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn store(&self, next: HashMap<String, NestedScan>) {
        match self.inner.lock() {
            Ok(mut g) => *g = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
    }

    /// Claim the right to refresh. `true` means the caller acquired
    /// it and MUST call `end_refresh`; `false` means one is already
    /// running and the caller should do nothing.
    ///
    /// Split from `refresh_if_idle` so the guard is testable on its own
    /// — a concurrency flag exercised only through a real walk is a
    /// flag whose behaviour is asserted by timing.
    pub(crate) fn begin_refresh(&self) -> bool {
        !self.refreshing.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn end_refresh(&self) {
        self.refreshing.store(false, Ordering::Release);
    }

    /// `refresh_blocking`, but only if no other refresh is in flight.
    /// Returns whether it ran.
    pub fn refresh_if_idle(&self, config_dir: &Path) -> bool {
        if !self.begin_refresh() {
            return false;
        }
        self.refresh_blocking(config_dir);
        self.end_refresh();
        true
    }

    /// Measure every slug's nested share and replace the cache.
    ///
    /// Whole-map replacement, not a merge: a slug that no longer exists
    /// must not keep a stale entry, and a merge would retain one
    /// forever. A slug that appears between refreshes is simply absent
    /// from the map, and `list_projects_cached` measures an absent slug
    /// directly — so a brand-new project is always exact.
    pub fn refresh_blocking(&self, config_dir: &Path) {
        match claudepot_core::project::scan_nested_by_slug(config_dir) {
            Ok(map) => self.store(map),
            // A failed refresh leaves the previous snapshot in place.
            // The listing that follows is one generation staler than it
            // would have been; it is not wrong about anything the user
            // can act on.
            Err(e) => {
                tracing::warn!(error = %e, "project size cache: refresh failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_project(cfg: &Path, slug: &str, nested_bytes: &[u8]) {
        let dir = cfg.join("projects").join(slug);
        std::fs::create_dir_all(dir.join("abc").join("subagents")).unwrap();
        // A top-level transcript, which must NOT count toward the
        // nested share, plus a nested one, which must.
        std::fs::write(dir.join("s1.jsonl"), b"top-level").unwrap();
        std::fs::write(
            dir.join("abc").join("subagents").join("n.jsonl"),
            nested_bytes,
        )
        .unwrap();
    }

    #[test]
    fn starts_empty_so_the_first_listing_knows_to_measure() {
        assert!(ProjectSizeCache::new().snapshot().is_empty());
    }

    #[test]
    fn refresh_records_only_the_nested_share_per_slug() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), "-repo-one", b"nested!");
        let cache = ProjectSizeCache::new();
        cache.refresh_blocking(tmp.path());

        let snap = cache.snapshot();
        assert_eq!(snap.len(), 1);
        let n = snap.get("-repo-one").expect("slug present");
        assert_eq!(
            n.size_bytes,
            b"nested!".len() as u64,
            "the top-level transcript belongs to the shallow pass, not here"
        );
        assert!(n.last_modified.is_some());
    }

    #[test]
    fn refresh_replaces_rather_than_merges_so_a_deleted_slug_drops_out() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), "-repo-one", b"a");
        write_project(tmp.path(), "-repo-two", b"bb");
        let cache = ProjectSizeCache::new();
        cache.refresh_blocking(tmp.path());
        assert_eq!(cache.snapshot().len(), 2);

        std::fs::remove_dir_all(tmp.path().join("projects").join("-repo-two")).unwrap();
        cache.refresh_blocking(tmp.path());
        let snap = cache.snapshot();
        assert_eq!(snap.len(), 1, "a merge would have kept the stale slug");
        assert!(!snap.contains_key("-repo-two"));
    }

    #[test]
    fn the_in_flight_guard_admits_one_refresher_at_a_time() {
        let cache = ProjectSizeCache::new();
        assert!(cache.begin_refresh(), "first caller acquires");
        assert!(!cache.begin_refresh(), "second caller is turned away");
        cache.end_refresh();
        assert!(cache.begin_refresh(), "released again after end_refresh");
        cache.end_refresh();
    }

    #[test]
    fn refresh_if_idle_reports_whether_it_ran() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), "-repo-one", b"a");
        let cache = ProjectSizeCache::new();
        assert!(cache.refresh_if_idle(tmp.path()), "nothing in flight");
        assert_eq!(cache.snapshot().len(), 1);

        // Simulate a walk already running.
        assert!(cache.begin_refresh());
        assert!(
            !cache.refresh_if_idle(tmp.path()),
            "must decline while another walk holds the guard"
        );
        cache.end_refresh();
    }

    /// A refresh that ERRORS must leave the previous snapshot in place:
    /// one generation stale beats empty, because an empty snapshot
    /// makes `project_list` fall back to measuring synchronously.
    ///
    /// Getting an error out of `scan_nested_by_slug` takes some care,
    /// and the care is the point. A *missing* config dir is not an
    /// error — it yields an empty map, mirroring `list_projects` — so
    /// the first version of this test pointed at a nonexistent path,
    /// watched the cache legitimately clear, and failed. A `projects`
    /// entry that exists but is not a directory is the real error path:
    /// `exists()` passes, `read_dir` does not.
    #[test]
    fn an_erroring_refresh_keeps_the_previous_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), "-repo-one", b"a");
        let cache = ProjectSizeCache::new();
        cache.refresh_blocking(tmp.path());
        assert_eq!(cache.snapshot().len(), 1);

        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join("projects"), b"not a directory").unwrap();
        cache.refresh_blocking(broken.path());
        assert_eq!(
            cache.snapshot().len(),
            1,
            "an unreadable root must not clear what we already knew"
        );
    }

    /// The other half: an empty install is a legitimate answer, not a
    /// failure, and must replace the snapshot. Without this the test
    /// above would be satisfied by a cache that never updates at all.
    #[test]
    fn an_empty_install_clears_the_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        write_project(tmp.path(), "-repo-one", b"a");
        let cache = ProjectSizeCache::new();
        cache.refresh_blocking(tmp.path());
        assert_eq!(cache.snapshot().len(), 1);

        let empty = tempfile::tempdir().unwrap();
        cache.refresh_blocking(empty.path());
        assert!(
            cache.snapshot().is_empty(),
            "an install with no projects is empty, not an error"
        );
    }
}
