//! Config tree watcher — deterministic state machine + a pluggable
//! filesystem event source. The real-FS backend uses `notify` (wired
//! via a thin adapter in the Tauri layer so the core stays dep-light);
//! the `FakeEventSource` below drives the state machine in tests.
//!
//! Plan §11 rules:
//! - Single debounce window (250 ms default).
//! - `dirty_generation` bumped on every in-scope event.
//! - `scan_until_stable` retries scans if events arrived mid-scan, up
//!   to MAX_CONVERGE_ATTEMPTS (5).
//! - When we bail out dirty, the emitted patch carries
//!   `dirty_during_emit = true`.

use crate::config_view::diff::{diff, ConfigTreePatch};
use crate::config_view::model::ConfigTree;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub const MAX_CONVERGE_ATTEMPTS: u32 = 5;

/// A dirty-generation counter. Every in-scope filesystem event bumps
/// this via `bump()`; scanners snapshot it before/after work to detect
/// mid-scan invalidation.
#[derive(Default)]
pub struct DirtyGen {
    inner: AtomicU64,
}

impl DirtyGen {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get(&self) -> u64 {
        self.inner.load(Ordering::Acquire)
    }
    pub fn bump(&self) {
        self.inner.fetch_add(1, Ordering::AcqRel);
    }
}

/// Run a scan loop that retries until the dirty-gen stops changing
/// mid-scan, bounded by MAX_CONVERGE_ATTEMPTS.
///
/// `do_scan` produces the latest tree. The caller is responsible for
/// its thread-safety; our state machine is pure.
pub fn scan_until_stable<F>(gen: &DirtyGen, mut do_scan: F) -> (ConfigTree, bool)
where
    F: FnMut() -> ConfigTree,
{
    let mut attempts = 0u32;
    loop {
        let gen_before = gen.get();
        let tree = do_scan();
        let gen_after = gen.get();
        if gen_after == gen_before {
            return (tree, false);
        }
        attempts += 1;
        if attempts >= MAX_CONVERGE_ATTEMPTS {
            return (tree, true); // dirty — bail
        }
    }
}

/// Compute the patch between `prev` and `next`, tagging it with the
/// `dirty_during_emit` flag from `scan_until_stable`.
pub fn make_patch(prev: &ConfigTree, next: &ConfigTree, dirty_during_emit: bool) -> ConfigTreePatch {
    let mut p = diff(prev, next);
    p.dirty_during_emit = dirty_during_emit;
    p
}

// ---------- Event-source trait (real + fake) --------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FsEventKind {
    Created,
    Modified,
    Removed,
    Renamed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FsEvent {
    pub kind: FsEventKind,
    pub path: std::path::PathBuf,
    /// For Renamed, the "to" path. None otherwise.
    pub rename_to: Option<std::path::PathBuf>,
}

/// Deny-list check — delegates to `discover::is_denied` so watcher
/// and discovery share ONE deny-list (plan §6.3). Divergence here
/// would mean the watcher wakes up for files discovery would ignore,
/// causing unnecessary rescans.
pub fn is_in_scope(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    !crate::config_view::discover::is_denied(name)
}

/// Apply an event to the dirty-gen via the in-scope filter. Renames
/// use both from and to paths per plan §11.3.
pub fn ingest_event(gen: &DirtyGen, ev: &FsEvent) -> bool {
    let from_in = is_in_scope(&ev.path);
    let to_in = ev
        .rename_to
        .as_ref()
        .map(|p| is_in_scope(p))
        .unwrap_or(false);
    if from_in || to_in {
        gen.bump();
        return true;
    }
    false
}

// ---------- Fake event source for tests -------------------------------

#[derive(Default)]
pub struct FakeEventSource {
    events: Mutex<Vec<FsEvent>>,
}

impl FakeEventSource {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&self, ev: FsEvent) {
        self.events.lock().unwrap().push(ev);
    }
    pub fn drain(&self) -> Vec<FsEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_view::model::{Scope, ScopeNode};
    use std::path::PathBuf;

    fn empty_tree(stamp: i64) -> ConfigTree {
        ConfigTree {
            scopes: vec![ScopeNode {
                id: "s".to_string(),
                scope: Scope::User,
                label: "s".to_string(),
                recursive_count: 0,
                children: vec![],
            }],
            scanned_at_unix_ns: stamp,
            cwd: PathBuf::from("/"),
            project_root: PathBuf::from("/"),
            memory_slug: String::new(),
            memory_slug_lossy: false,
            cc_version_hint: None,
            enterprise_mcp_lockout: false,
        }
    }

    #[test]
    fn scan_until_stable_converges_on_first_try() {
        let gen = DirtyGen::new();
        let (_tree, dirty) = scan_until_stable(&gen, || empty_tree(0));
        assert!(!dirty);
    }

    #[test]
    fn scan_until_stable_retries_on_dirty_bump_mid_scan() {
        let gen = DirtyGen::new();
        let mut calls = 0u32;
        let (_tree, dirty) = scan_until_stable(&gen, || {
            calls += 1;
            if calls < 3 {
                gen.bump(); // dirty mid-scan
            }
            empty_tree(0)
        });
        assert_eq!(calls, 3);
        assert!(!dirty);
    }

    #[test]
    fn scan_until_stable_bails_after_max_attempts_dirty() {
        let gen = DirtyGen::new();
        let (_tree, dirty) = scan_until_stable(&gen, || {
            gen.bump();
            empty_tree(0)
        });
        assert!(dirty);
    }

    #[test]
    fn ingest_event_bumps_gen_for_in_scope_path() {
        let gen = DirtyGen::new();
        let before = gen.get();
        let ev = FsEvent {
            kind: FsEventKind::Modified,
            path: PathBuf::from("/home/u/.claude/settings.json"),
            rename_to: None,
        };
        assert!(ingest_event(&gen, &ev));
        assert_eq!(gen.get(), before + 1);
    }

    #[test]
    fn ingest_event_skips_deny_list_paths() {
        let gen = DirtyGen::new();
        let ev = FsEvent {
            kind: FsEventKind::Modified,
            path: PathBuf::from("/home/u/.claude/paste-cache"),
            rename_to: None,
        };
        assert!(!ingest_event(&gen, &ev));
        assert_eq!(gen.get(), 0);
    }

    #[test]
    fn rename_from_in_scope_to_deny_treated_as_modification() {
        let gen = DirtyGen::new();
        let ev = FsEvent {
            kind: FsEventKind::Renamed,
            path: PathBuf::from("/home/u/.claude/settings.json"),
            rename_to: Some(PathBuf::from("/home/u/.claude/paste-cache/foo")),
        };
        assert!(ingest_event(&gen, &ev));
    }

    #[test]
    fn rename_from_deny_to_in_scope_counts() {
        let gen = DirtyGen::new();
        let ev = FsEvent {
            kind: FsEventKind::Renamed,
            path: PathBuf::from("/home/u/.claude/paste-cache/foo"),
            rename_to: Some(PathBuf::from("/home/u/.claude/settings.json")),
        };
        assert!(ingest_event(&gen, &ev));
    }

    #[test]
    fn fake_event_source_drain_resets_buffer() {
        let fake = FakeEventSource::new();
        fake.push(FsEvent {
            kind: FsEventKind::Modified,
            path: PathBuf::from("/a"),
            rename_to: None,
        });
        let drained = fake.drain();
        assert_eq!(drained.len(), 1);
        assert!(fake.drain().is_empty());
    }

    #[test]
    fn make_patch_sets_dirty_flag() {
        let a = empty_tree(1);
        let b = empty_tree(2);
        let patch = make_patch(&a, &b, true);
        assert!(patch.dirty_during_emit);
    }
}
