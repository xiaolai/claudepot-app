//! In-process TTL cache for PR detections.
//!
//! Keyed on `repo_root` only — only one branch can be HEAD at a
//! time, so multi-branch caching wouldn't help. `insert` always
//! overwrites, so a branch flip between ticks is absorbed without
//! the cache needing to track the branch identity.
//!
//! TTL is 60 seconds rather than the 5-minute tick so a freshly-
//! opened PR appears in the UI within ~1 minute of opening it
//! (assuming the tick fires close by), without the user having to
//! restart Claudepot.
//!
//! Negative results (no PR found) are cached too — that's the
//! steady state for most projects, and without negative caching
//! every tick would re-shell `gh` for every PR-less project.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::PrInfo;

const TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct PrCache {
    entries: Mutex<HashMap<PathBuf, Entry>>,
}

struct Entry {
    stored_at: Instant,
    pr: Option<PrInfo>,
}

impl PrCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cache-only lookup. Returns the cached `PrInfo` if fresh and
    /// positive; returns `None` for cache miss, expired entry, or
    /// cached negative ("no PR exists for this branch"). The
    /// caller can't tell positive-miss from cached-negative, which
    /// is intentional — the UI renders the same in either case.
    pub fn freshest_pr(&self, repo_root: &std::path::Path) -> Option<PrInfo> {
        let map = self.entries.lock().ok()?;
        let entry = map.get(repo_root)?;
        if Instant::now().duration_since(entry.stored_at) > TTL {
            return None;
        }
        entry.pr.clone()
    }

    /// Insert or replace the entry for this repo. Branch identity
    /// is intentionally not stored — `insert` overwrites
    /// unconditionally, so a branch flip is absorbed by the next
    /// tick without the cache needing to compare.
    pub fn insert(&self, repo_root: &std::path::Path, pr: Option<PrInfo>) {
        if let Ok(mut map) = self.entries.lock() {
            map.insert(
                repo_root.to_path_buf(),
                Entry {
                    stored_at: Instant::now(),
                    pr,
                },
            );
        }
    }

    /// Drop entries for a removed project. Bounded by the single
    /// entry that exists per repo.
    pub fn forget_repo(&self, repo_root: &std::path::Path) {
        if let Ok(mut map) = self.entries.lock() {
            map.remove(repo_root);
        }
    }

    /// `true` when this repo has *any* cached entry — fresh, stale,
    /// positive, or negative. Lets observers distinguish "negative
    /// cached" from "never queried" without leaking the entry's
    /// internals. Used by tests in this crate and the
    /// `pr_orchestrator` test suite.
    pub fn contains(&self, repo_root: &std::path::Path) -> bool {
        self.entries
            .lock()
            .map(|m| m.contains_key(repo_root))
            .unwrap_or(false)
    }

    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)] // test-only inspection helper
    pub fn len(&self) -> usize {
        self.entries.lock().map(|m| m.len()).unwrap_or(0)
    }

    #[cfg(test)]
    pub fn force_expire_all(&self) {
        if let Ok(mut map) = self.entries.lock() {
            let old = Instant::now() - TTL - Duration::from_secs(1);
            for e in map.values_mut() {
                e.stored_at = old;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_pr::PrState;
    use std::path::Path;

    fn mk_pr(n: u64) -> PrInfo {
        PrInfo {
            number: n,
            url: format!("https://github.com/x/y/pull/{n}"),
            state: PrState::Open,
            head_ref_name: "feat/x".into(),
        }
    }

    #[test]
    fn miss_then_hit_then_expire() {
        let cache = PrCache::new();
        let root = Path::new("/repo");
        assert!(cache.freshest_pr(root).is_none(), "initial miss");
        cache.insert(root, Some(mk_pr(1)));
        assert_eq!(cache.freshest_pr(root), Some(mk_pr(1)));
        cache.force_expire_all();
        assert!(cache.freshest_pr(root).is_none(), "expired entry must miss");
    }

    #[test]
    fn negative_results_are_cached_as_none() {
        let cache = PrCache::new();
        let root = Path::new("/repo");
        cache.insert(root, None);
        // Negative cached results return None — caller can't
        // distinguish from a true miss, which is the desired
        // contract (the UI renders the same either way).
        assert!(cache.freshest_pr(root).is_none());
        // But the entry IS present — proving the next refresh
        // tick can detect the branch and decide whether to
        // re-query.
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn second_insert_overwrites_prior_entry() {
        // A repo can only have one HEAD branch at a time. Inserting
        // again — whether from a branch flip or a refreshed tick —
        // replaces the prior entry. No stale state accumulates.
        let cache = PrCache::new();
        let root = Path::new("/repo");
        cache.insert(root, Some(mk_pr(1)));
        cache.insert(root, Some(mk_pr(2)));
        assert_eq!(cache.freshest_pr(root), Some(mk_pr(2)));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn forget_repo_drops_only_that_repo() {
        let cache = PrCache::new();
        cache.insert(Path::new("/a"), Some(mk_pr(1)));
        cache.insert(Path::new("/b"), Some(mk_pr(2)));
        cache.forget_repo(Path::new("/a"));
        assert!(cache.freshest_pr(Path::new("/a")).is_none());
        assert!(cache.freshest_pr(Path::new("/b")).is_some());
        assert_eq!(cache.len(), 1);
    }
}
