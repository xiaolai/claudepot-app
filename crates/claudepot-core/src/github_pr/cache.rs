//! In-process TTL cache for PR detections.
//!
//! The orchestrator polls every project on each
//! `usage_snapshot::run_tick` (5-minute cadence). Without a cache,
//! that's ~one `gh pr view` subprocess per project per tick — for a
//! user with 30 projects, 30 subprocess spawns every 5 minutes. The
//! cache also stores *negative* results (no PR found), which is the
//! common case for the average project, so the cache keeps even
//! "nothing here" projects from hitting `gh` every tick.
//!
//! TTL is 60 seconds rather than the 5-minute tick so a freshly-
//! opened PR appears in the UI within ~1 minute of opening it
//! (assuming the tick fires close by), without the user having to
//! restart Claudepot.
//!
//! Cache key is `(repo_root, branch)` — a project switching branches
//! triggers a fresh detection without polluting the previous
//! branch's cache slot.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::PrInfo;

const TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct PrCache {
    entries: Mutex<HashMap<(PathBuf, String), Entry>>,
}

struct Entry {
    stored_at: Instant,
    value: Option<PrInfo>,
}

impl PrCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a cached entry. Returns `Some(Some(info))` for a
    /// cached positive hit, `Some(None)` for a cached negative
    /// (no PR exists for this branch), and `None` for a cache miss
    /// or expired entry — caller should compute and `insert`.
    pub fn get(&self, repo_root: &std::path::Path, branch: &str) -> Option<Option<PrInfo>> {
        let now = Instant::now();
        let map = self.entries.lock().ok()?;
        let key = (repo_root.to_path_buf(), branch.to_string());
        let e = map.get(&key)?;
        if now.duration_since(e.stored_at) > TTL {
            return None;
        }
        Some(e.value.clone())
    }

    /// Branch-agnostic lookup. Returns the first fresh positive PR
    /// info for any branch under `repo_root`. Used by the
    /// orchestrator command path when it doesn't yet know which
    /// branch is current — there's only one current branch at a
    /// time, so at most one fresh positive ever exists.
    pub fn get_any_for(&self, repo_root: &std::path::Path) -> Option<PrInfo> {
        let now = Instant::now();
        let map = self.entries.lock().ok()?;
        for ((k_root, _branch), entry) in map.iter() {
            if k_root != repo_root {
                continue;
            }
            if now.duration_since(entry.stored_at) > TTL {
                continue;
            }
            if let Some(ref info) = entry.value {
                return Some(info.clone());
            }
        }
        None
    }

    pub fn insert(&self, repo_root: &std::path::Path, branch: &str, value: Option<PrInfo>) {
        if let Ok(mut map) = self.entries.lock() {
            map.insert(
                (repo_root.to_path_buf(), branch.to_string()),
                Entry {
                    stored_at: Instant::now(),
                    value,
                },
            );
        }
    }

    /// Drop entries for a given repo (e.g. when the project is
    /// removed from Claudepot). Bounded by the number of branches
    /// we've seen for that repo, which in practice is < 5.
    pub fn forget_repo(&self, repo_root: &std::path::Path) {
        if let Ok(mut map) = self.entries.lock() {
            map.retain(|(k_root, _), _| k_root != repo_root);
        }
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
        assert!(cache.get(root, "feat/x").is_none(), "initial miss");
        cache.insert(root, "feat/x", Some(mk_pr(1)));
        assert_eq!(cache.get(root, "feat/x"), Some(Some(mk_pr(1))));
        cache.force_expire_all();
        assert!(
            cache.get(root, "feat/x").is_none(),
            "expired entry must miss"
        );
    }

    #[test]
    fn negative_results_are_cached() {
        let cache = PrCache::new();
        let root = Path::new("/repo");
        cache.insert(root, "feat/x", None);
        assert_eq!(
            cache.get(root, "feat/x"),
            Some(None),
            "Some(None) distinguishes cached-negative from cache-miss",
        );
    }

    #[test]
    fn different_branches_share_a_repo() {
        let cache = PrCache::new();
        let root = Path::new("/repo");
        cache.insert(root, "feat/x", Some(mk_pr(1)));
        cache.insert(root, "feat/y", Some(mk_pr(2)));
        assert_eq!(cache.get(root, "feat/x"), Some(Some(mk_pr(1))));
        assert_eq!(cache.get(root, "feat/y"), Some(Some(mk_pr(2))));
    }

    #[test]
    fn forget_repo_drops_only_that_repo() {
        let cache = PrCache::new();
        cache.insert(Path::new("/a"), "main", Some(mk_pr(1)));
        cache.insert(Path::new("/b"), "main", Some(mk_pr(2)));
        cache.forget_repo(Path::new("/a"));
        assert!(cache.get(Path::new("/a"), "main").is_none());
        assert!(cache.get(Path::new("/b"), "main").is_some());
        assert_eq!(cache.len(), 1);
    }
}
