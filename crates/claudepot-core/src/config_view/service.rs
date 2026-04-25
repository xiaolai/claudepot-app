//! `ConfigScanService` — owns the committed `ConfigTree` snapshot for
//! the Config section and arbitrates between concurrent writers
//! (`config_scan` Tauri command, the filesystem watcher seed, and
//! the watcher's keepalive rescans).
//!
//! The previous owner of this state lived at the Tauri layer
//! (`commands_config::ConfigTreeState`); per `architecture.md` —
//! "if you're writing business logic in `claudepot-cli` or
//! `src-tauri`, stop — it belongs in `claudepot-core`" — the cell
//! and its commit policy belong here. The service preserves the
//! audit-2026-04-24-round-3 invariant that the writer with the
//! highest claimed generation always wins, by performing the
//! generation compare and the tree write under the SAME lock.
//!
//! # Lock choice
//!
//! `parking_lot::RwLock`, sync, no poisoning. Two reasons:
//!
//! 1. The read side (`with_tree`, `current_tree`) is called from
//!    sync contexts (`config_preview` builds its `FileNode` clone
//!    inside a `spawn_blocking`, the watcher reads the previous
//!    snapshot from a worker thread). Going async-aware would force
//!    every caller to be `async fn` for no payoff — the lock is held
//!    for microseconds.
//! 2. `std::sync::Mutex` poisoning was a recurring footgun for the
//!    old `ConfigTreeCell`; `parking_lot::RwLock` removes that
//!    failure mode by construction.
//!
//! # Atomic commit invariant
//!
//! `commit(handle, tree)` only writes when `handle.gen >
//! last_committed_gen`, with the comparison and the write under the
//! same write-lock. The companion test
//! `concurrent_writers_always_leave_the_newest_tree_committed` is
//! the canonical regression guard — see comment there.

use crate::config_view::model::ConfigTree;
use crate::config_view::watch::DirtyGen;
use parking_lot::RwLock;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Internal tree-cell. `last_committed_gen == 0` before any writer
/// commits; `gen_counter::fetch_add` returns the previous value, so
/// `start_scan()` hands out generations starting at 1.
#[derive(Default)]
struct ScanState {
    tree: Option<Arc<ConfigTree>>,
    last_committed_gen: u64,
}

/// Token claimed by `start_scan` and surrendered to `commit`. Carries
/// only the generation; deliberately non-`Clone` and non-`Copy` so it
/// is consumed by `commit`. Dropping a handle without committing is
/// inert — the next `start_scan` claims the next generation, and any
/// later commit with the dropped generation is rejected by the
/// monotone check inside `commit`.
#[derive(Debug)]
pub struct ScanHandle {
    gen: u64,
}

impl ScanHandle {
    /// Read-only view of the claimed generation. Tests + tracing only;
    /// production code should treat the handle as opaque.
    pub fn generation(&self) -> u64 {
        self.gen
    }
}

/// Sole owner of the latest scanned `ConfigTree` and the dirty-gen
/// counter the watcher uses to detect mid-scan invalidation. Tauri
/// holds an `Arc<ConfigScanService>` in managed state; the watcher
/// receives the same `Arc` through `start()`.
pub struct ConfigScanService {
    cell: Arc<RwLock<ScanState>>,
    gen_counter: AtomicU64,
    dirty_gen: DirtyGen,
}

impl ConfigScanService {
    /// Construct a fresh service wrapped in `Arc` so multiple
    /// subsystems (Tauri commands, watcher) can share ownership.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cell: Arc::new(RwLock::new(ScanState::default())),
            gen_counter: AtomicU64::new(0),
            dirty_gen: DirtyGen::new(),
        })
    }

    /// Claim the next generation for a writer. The handle MUST be
    /// surrendered to `commit` for the write to take effect; dropping
    /// it without commit leaves the cell unchanged.
    pub fn start_scan(&self) -> ScanHandle {
        let g = self.gen_counter.fetch_add(1, Ordering::SeqCst) + 1;
        ScanHandle { gen: g }
    }

    /// Commit `tree` iff `handle.gen` exceeds the last committed
    /// generation. The comparison and the write happen under the
    /// same write-lock so no older writer can slip past a newer one.
    /// Returns `true` when the commit took effect, `false` when a
    /// later writer already won.
    pub fn commit(&self, handle: ScanHandle, tree: ConfigTree) -> bool {
        let mut g = self.cell.write();
        if handle.gen <= g.last_committed_gen {
            return false;
        }
        g.tree = Some(Arc::new(tree));
        g.last_committed_gen = handle.gen;
        true
    }

    /// Return an `Arc` clone of the current tree, or `None` if no
    /// writer has committed yet. The clone is cheap (a refcount
    /// bump) and the read-lock is released before the caller
    /// touches the tree, so concurrent writers don't block on
    /// downstream work.
    pub fn current_tree(&self) -> Option<Arc<ConfigTree>> {
        self.cell.read().tree.as_ref().map(Arc::clone)
    }

    /// Run `f` against the current tree under the read-lock. Used by
    /// `config_preview` to clone a single `FileNode` out without
    /// allocating a whole-tree `Arc`. Returns `None` when no tree is
    /// committed.
    pub fn with_tree<R>(&self, f: impl FnOnce(&ConfigTree) -> R) -> Option<R> {
        let g = self.cell.read();
        g.tree.as_deref().map(f)
    }

    /// Generation of the most-recently committed tree. `0` before
    /// any commit. Tests + telemetry only — production callers
    /// should drive ordering through `start_scan`/`commit`, not by
    /// inspecting this number.
    pub fn generation(&self) -> u64 {
        self.cell.read().last_committed_gen
    }

    /// Dirty-gen counter shared with the filesystem watcher. The
    /// watcher bumps it on every in-scope event; `scan_until_stable`
    /// snapshots before/after a scan to detect mid-scan invalidation.
    pub fn dirty_gen(&self) -> &DirtyGen {
        &self.dirty_gen
    }

    /// Convenience for the common case: claim a handle, run a full
    /// scan synchronously, commit, and return the committed tree.
    /// Returns the tree the caller's commit produced when it wins
    /// the race; if a newer writer beat us, returns the newer tree
    /// instead so callers always see the freshest snapshot.
    ///
    /// The scan itself runs on the calling thread — callers in
    /// async contexts should wrap this in `spawn_blocking`. (The
    /// scan walks the filesystem and can stall on slow mounts.)
    pub fn scan_and_commit(&self, anchor: Option<&Path>) -> Arc<ConfigTree> {
        let handle = self.start_scan();
        let tree = match anchor {
            Some(p) => super::scan(p),
            None => super::scan_global(),
        };
        // Two outcomes: (a) we win, the cell now holds our tree, OR
        // (b) we lose, the cell holds someone newer. In both cases
        // `current_tree()` is the right thing to return — never our
        // discarded local `tree`.
        let _ = self.commit(handle, tree);
        // Safe to unwrap: we just committed (or someone else did).
        // Falling back to `expect` here would mask a future bug
        // where `current_tree` returns `None` after a successful
        // commit; the `unwrap_or_else` keeps the type honest.
        self.current_tree().unwrap_or_else(|| {
            // Should be unreachable — `commit` always succeeds for
            // the very first writer (`gen=1 > last_committed_gen=0`).
            // If we ever land here, fall back to an empty tree at the
            // requested anchor so callers don't see a dangling Arc.
            let cwd = anchor
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")));
            Arc::new(super::empty_tree(&cwd))
        })
    }
}

#[cfg(test)]
mod tests {
    //! Race tests for `ConfigScanService`.
    //!
    //! The first three are ports of the original
    //! `commands_config::tree_state_tests` (audit 2026-04-24, round 2
    //! and round 3) — those tests were the most valuable artefact
    //! produced by the IPC-layer cell and the design doc explicitly
    //! calls them out as the keepers when the cell migrates to core.
    //!
    //! The last two are new and verify the design-doc invariants:
    //! a dropped handle is inert, and `current_tree`'s `Arc` snapshot
    //! is observed atomically across a concurrent commit.
    use super::*;
    use crate::config_view::model::ConfigTree;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn fixture_tree(tag: &str) -> ConfigTree {
        // Minimal synthetic tree keyed by `cwd` so assertions can tell
        // one writer's tree from another's.
        ConfigTree {
            scopes: Vec::new(),
            scanned_at_unix_ns: 0,
            cwd: PathBuf::from(tag),
            project_root: PathBuf::from(tag),
            memory_slug: String::new(),
            memory_slug_lossy: false,
            cc_version_hint: None,
            enterprise_mcp_lockout: false,
        }
    }

    #[test]
    fn commit_accepts_writes_in_generation_order() {
        let svc = ConfigScanService::new();
        let h1 = svc.start_scan();
        let h2 = svc.start_scan();
        assert!(svc.commit(h1, fixture_tree("a")));
        assert!(svc.commit(h2, fixture_tree("b")));
        assert_eq!(
            svc.current_tree().unwrap().cwd,
            PathBuf::from("b")
        );
    }

    #[test]
    fn commit_drops_older_writer_when_newer_already_committed() {
        // Classic stale-write race: claim h1, claim h2, commit h2,
        // commit h1. The h1 write MUST be dropped — the atomic
        // check-and-write inside `commit` guarantees this (audit
        // 2026-04-24 round 3, the original check-then-lock shape let
        // h1 slip past).
        let svc = ConfigScanService::new();
        let h1 = svc.start_scan();
        let h2 = svc.start_scan();
        assert!(svc.commit(h2, fixture_tree("new")));
        assert!(!svc.commit(h1, fixture_tree("stale")));
        assert_eq!(
            svc.current_tree().unwrap().cwd,
            PathBuf::from("new")
        );
    }

    #[test]
    fn concurrent_writers_always_leave_the_newest_tree_committed() {
        use std::thread;
        // Spawn 32 writers; each claims a generation, spins for a
        // variable interval, then commits a tree tagged with its
        // generation. Repeat to hammer the compare-and-write path.
        //
        // Strong assertion (round-3 follow-up audit, L-fix): after
        // every writer finishes, `generation()` MUST equal the
        // highest claimed generation AND the committed tree's tag
        // must match — the winner is the writer who claimed the
        // highest generation, not merely "some writer with a
        // generation <= max." The round-2 non-atomic commit would
        // frequently leave `last_committed_gen < max_gen` here and
        // fail this assertion.
        for _trial in 0..50 {
            let svc = ConfigScanService::new();
            let mut handles = Vec::new();
            for i in 0..32 {
                let s = Arc::clone(&svc);
                handles.push(thread::spawn(move || {
                    let h = s.start_scan();
                    let g = h.generation();
                    for _ in 0..(i * 7 % 13) {
                        std::hint::spin_loop();
                    }
                    let _ = s.commit(h, fixture_tree(&format!("w{g}")));
                    g
                }));
            }
            let gens: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            let max_gen = *gens.iter().max().unwrap();
            assert_eq!(
                svc.generation(),
                max_gen,
                "the writer with the highest claimed generation must win"
            );
            let tree = svc.current_tree().expect("at least one writer committed");
            assert_eq!(
                tree.cwd,
                PathBuf::from(format!("w{max_gen}")),
                "committed tree must come from the winning writer"
            );
        }
    }

    #[test]
    fn start_scan_then_drop_handle_does_not_change_state() {
        // A handle that never commits is inert. Drop it explicitly —
        // the cell must still be empty and `generation()` must be 0.
        // The next start_scan still gets the next number (handles are
        // NOT recycled), but the committed state is what we care about.
        let svc = ConfigScanService::new();
        assert!(svc.current_tree().is_none());
        assert_eq!(svc.generation(), 0);
        let h = svc.start_scan();
        assert_eq!(h.generation(), 1);
        drop(h);
        assert!(svc.current_tree().is_none());
        assert_eq!(svc.generation(), 0);
        // A subsequent commit with a fresh handle still works —
        // dropping a handle doesn't poison the counter.
        let h2 = svc.start_scan();
        assert!(svc.commit(h2, fixture_tree("after-drop")));
        assert_eq!(svc.generation(), 2);
        assert_eq!(
            svc.current_tree().unwrap().cwd,
            PathBuf::from("after-drop")
        );
    }

    #[test]
    fn current_tree_returns_arc_observed_atomically() {
        // The Arc returned by `current_tree` is a stable snapshot:
        // a concurrent commit installs a new Arc, but the snapshot
        // a reader already holds keeps pointing at the old tree.
        // This is what makes the read-side lock-free for callers
        // that hand the Arc off to a worker.
        let svc = ConfigScanService::new();
        let h1 = svc.start_scan();
        assert!(svc.commit(h1, fixture_tree("v1")));
        let snap_before = svc.current_tree().expect("v1 committed");
        assert_eq!(snap_before.cwd, PathBuf::from("v1"));

        let h2 = svc.start_scan();
        assert!(svc.commit(h2, fixture_tree("v2")));
        let snap_after = svc.current_tree().expect("v2 committed");
        assert_eq!(snap_after.cwd, PathBuf::from("v2"));

        // The pre-commit snapshot still points at v1 — observed
        // atomically, never partially mutated.
        assert_eq!(snap_before.cwd, PathBuf::from("v1"));
        // And the two Arcs are distinct allocations — the commit
        // installed a fresh Arc rather than mutating in-place.
        assert!(!Arc::ptr_eq(&snap_before, &snap_after));
    }

    #[test]
    fn scan_and_commit_returns_tree_after_first_call() {
        // Uses the real `scan_global()` — succeeds even with no `~/`
        // because `scan_global` falls back to `/`. This is more of a
        // smoke test than a unit test; the heavy lifting is exercised
        // by the four race tests above.
        let svc = ConfigScanService::new();
        let tree = svc.scan_and_commit(None);
        // Some snapshot must be installed.
        assert!(svc.current_tree().is_some());
        // And the returned Arc must be the committed one.
        assert!(Arc::ptr_eq(&tree, &svc.current_tree().unwrap()));
        assert_eq!(svc.generation(), 1);
    }
}
