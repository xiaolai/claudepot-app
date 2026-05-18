//! Bridge between `claudepot_core::github_pr` and the Tauri runtime.
//!
//! Owns the single shared `PrCache`. The cache is consulted by
//! `project_list` to attach PR info to each `ProjectInfoDto` (a
//! cheap in-memory read), and refreshed on the
//! `usage_snapshot::run_tick` cadence so opening the Projects tab
//! always sees recent data without paying the subprocess cost
//! synchronously.
//!
//! Zero overhead when the user has no projects, has no `gh`
//! installed, or has every project on a trunk branch — `detect_pr`
//! short-circuits each case with `Ok(None)` and the negative
//! result is cached.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use claudepot_core::github_pr::{cache::PrCache, detect_pr, PrInfo};

pub struct PrOrchestrator {
    cache: Arc<PrCache>,
}

impl PrOrchestrator {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(PrCache::new()),
        }
    }

    /// Synchronous cache-only lookup. Returns the cached value
    /// if fresh, otherwise `None`. Callers that want a fresh value
    /// should pair this with `refresh_one` in a `spawn_blocking`.
    ///
    /// We look up under the empty-string branch first — that's the
    /// orchestrator's storage key, since the tick discovers the
    /// branch and doesn't expose it back to the caller. If a future
    /// caller already knows the branch, it can pass it directly.
    pub fn cached_for(&self, repo_root: &Path) -> Option<PrInfo> {
        // The tick stores under the branch it discovered, which the
        // command path doesn't know. Walk all entries for this repo
        // and return the first fresh positive — there's only one
        // current branch per repo at a time, so at most one fresh
        // entry exists.
        self.cache.get_any_for(repo_root)
    }

    /// Refresh a single project. Synchronous (shells out to
    /// `git` + `gh`). Callers must dispatch via `spawn_blocking`
    /// or run on a dedicated thread.
    pub fn refresh_one(&self, repo_root: &Path) {
        // Skip if a fresh entry already exists — we don't know the
        // branch yet, but if anything cached for this repo is still
        // within TTL, refreshing wouldn't change the answer.
        if self.cache.get_any_for(repo_root).is_some() {
            return;
        }
        let result = detect_pr(repo_root);
        // Discover the branch we just queried so we can key the
        // cache by it. If discovery failed, store under the empty
        // string — the cache is a hint, not a source of truth.
        let branch = claudepot_core::github_pr::cli::current_branch(repo_root)
            .ok()
            .flatten()
            .unwrap_or_default();
        match result {
            Ok(info) => self.cache.insert(repo_root, &branch, info),
            Err(e) => {
                if e.is_noteworthy() {
                    tracing::debug!(
                        repo = %repo_root.display(),
                        error = %e,
                        "pr_orchestrator: detect failed"
                    );
                }
                // Cache the miss so we don't retry every tick
                // for a repo whose `gh` is broken.
                self.cache.insert(repo_root, &branch, None);
            }
        }
    }

    /// Refresh PR info for every project in the listing. Runs each
    /// detection synchronously on the calling thread — must be
    /// dispatched via `spawn_blocking` by the caller.
    pub fn tick_all(&self, repo_roots: &[PathBuf]) {
        for root in repo_roots {
            // Force-expire so the tick refreshes even if the cache
            // entry is still inside TTL — we want the tick cadence
            // to win over the TTL when they conflict.
            self.cache.forget_repo(root);
            // refresh_one's early-return checks the cache; the
            // forget above guarantees we get past it.
            self.refresh_one(root);
        }
    }
}

impl Default for PrOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}
