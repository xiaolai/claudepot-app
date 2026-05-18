//! Bridge between `claudepot_core::github_pr` and the Tauri runtime.
//!
//! Owns the single shared `PrCache`. The cache is consulted by
//! `project_list` to attach PR info to each `ProjectInfoDto` (a
//! cheap in-memory read), and refreshed on the
//! `usage_snapshot::run_tick` cadence so opening the Projects tab
//! always sees recent data without paying the subprocess cost
//! synchronously.
//!
//! Three optimizations carry the steady-state cost to near-zero:
//!   * Negative results are cached — a project without an open PR
//!     pays one subprocess per TTL window, not one per tick.
//!   * Detections fan out with bounded concurrency, so a 30-project
//!     tick completes in ~(30 / `MAX_PARALLEL`) × per-call latency
//!     instead of 30 × per-call.
//!   * `gh`-absent is observed once and short-circuits every
//!     subsequent refresh for the lifetime of the orchestrator.
//!     Restart Claudepot after installing `gh` to flip it back.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use claudepot_core::github_pr::{cache::PrCache, detect_pr, DetectOutcome, GhError, PrInfo};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Cap on concurrent `gh` invocations. GitHub's REST API
/// rate-limits per-user; `gh` shares that budget across calls, so
/// unbounded fan-out is hostile. Four balances throughput with
/// politeness — completes a 30-project tick in ~8 batches.
const MAX_PARALLEL: usize = 4;

/// Trait isolating the subprocess work so unit tests can swap in a
/// deterministic fake. Production wires this to `github_pr::detect_pr`.
#[async_trait]
pub trait PrDetector: Send + Sync + 'static {
    async fn detect(&self, repo_root: &Path) -> Result<DetectOutcome, GhError>;
}

/// Production detector — calls into `github_pr::detect_pr`.
pub struct RealDetector;

#[async_trait]
impl PrDetector for RealDetector {
    async fn detect(&self, repo_root: &Path) -> Result<DetectOutcome, GhError> {
        detect_pr(repo_root).await
    }
}

pub struct PrOrchestrator {
    cache: Arc<PrCache>,
    detector: Arc<dyn PrDetector>,
    /// Latches `true` the first time we observe `gh` is not on PATH.
    /// Subsequent refreshes return immediately without touching
    /// `git` or `gh`. Reset only by process restart.
    gh_absent: Arc<AtomicBool>,
}

impl PrOrchestrator {
    pub fn new() -> Self {
        Self::with_detector(Arc::new(RealDetector))
    }

    pub fn with_detector(detector: Arc<dyn PrDetector>) -> Self {
        Self {
            cache: Arc::new(PrCache::new()),
            detector,
            gh_absent: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Synchronous cache-only lookup. Returns the cached value if
    /// fresh, otherwise `None`. Called by `project_list` on the
    /// hot path; must not block on subprocesses.
    pub fn cached_for(&self, repo_root: &Path) -> Option<PrInfo> {
        self.cache.freshest_pr(repo_root)
    }

    /// Test-only introspection: `true` once the orchestrator has
    /// latched onto an absent `gh` CLI. Promoted to non-test if a
    /// future surface (e.g. a Settings status banner) wants to
    /// disclose this to the user.
    #[cfg(test)]
    pub fn gh_absent(&self) -> bool {
        self.gh_absent.load(Ordering::Relaxed)
    }

    /// Test-only introspection: `true` when the orchestrator has
    /// any cache entry for this repo (fresh or stale). Distinguishes
    /// "negative cached" from "never queried" — the public
    /// `cached_for` collapses both to `None`.
    #[cfg(test)]
    pub fn contains_entry_for(&self, repo_root: &Path) -> bool {
        self.cache.contains(repo_root)
    }

    /// Refresh PR info for every project in the listing. Fans out
    /// to at most `MAX_PARALLEL` simultaneous subprocesses. Returns
    /// when every refresh has completed (success or failure).
    ///
    /// If the gh-absent latch is set, returns immediately without
    /// spawning anything — the user lacks `gh`, so every detection
    /// would resolve to the same "no PR" answer.
    ///
    /// Once a task that's already in flight observes
    /// `MissingCli("gh")`, the latch flips mid-tick — every later
    /// task re-reads it after acquiring its semaphore permit and
    /// skips the detection, so the rest of the tick degenerates to
    /// permit-handoff instead of subprocess calls.
    pub async fn tick_all(self: Arc<Self>, repo_roots: Vec<PathBuf>) {
        if self.gh_absent.load(Ordering::Relaxed) {
            return;
        }
        let sem = Arc::new(Semaphore::new(MAX_PARALLEL));
        let mut set = JoinSet::new();
        for root in repo_roots {
            let me = Arc::clone(&self);
            let sem = Arc::clone(&sem);
            set.spawn(async move {
                // Permit dropped after the blocking task finishes
                // (the closure owns it). spawn_blocking is the
                // right boundary because detect_pr issues
                // subprocess calls — pure-tokio would block the
                // reactor.
                let _permit = sem.acquire_owned().await.ok()?;
                // Mid-tick re-check: a sibling task may have
                // observed gh missing while we waited on the
                // permit. Skip the detect rather than burn another
                // gh-not-found cycle.
                if me.gh_absent.load(Ordering::Relaxed) {
                    return Some(());
                }
                me.refresh_one(&root).await;
                Some(())
            });
        }
        while set.join_next().await.is_some() {}
    }

    /// Async refresh for a single project. Always overwrites the
    /// cache entry — TTL is the cache's concern, not ours. Updates
    /// the gh-absent latch on `MissingCli("gh")`.
    async fn refresh_one(&self, repo_root: &Path) {
        match self.detector.detect(repo_root).await {
            Ok(DetectOutcome { branch: _, pr }) => {
                // Cache regardless of whether `branch` is empty
                // (detached HEAD / non-repo) — the negative entry
                // suppresses retries until the next tick.
                self.cache.insert(repo_root, pr);
            }
            Err(e) => {
                if matches!(&e, GhError::MissingCli("gh")) {
                    self.gh_absent.store(true, Ordering::Relaxed);
                } else if e.is_noteworthy() {
                    tracing::debug!(
                        repo = %repo_root.display(),
                        error = %e,
                        "pr_orchestrator: detect failed"
                    );
                }
                // Cache a miss so the next tick doesn't
                // immediately retry the same broken repo.
                self.cache.insert(repo_root, None);
            }
        }
    }
}

impl Default for PrOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::github_pr::PrState;
    use std::sync::Mutex;

    /// Records every detect call + lets the test script the result.
    struct FakeDetector {
        responses: Mutex<Vec<Result<DetectOutcome, GhError>>>,
        calls: Mutex<Vec<PathBuf>>,
    }

    impl FakeDetector {
        fn new(responses: Vec<Result<DetectOutcome, GhError>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl PrDetector for FakeDetector {
        async fn detect(&self, repo_root: &Path) -> Result<DetectOutcome, GhError> {
            self.calls.lock().unwrap().push(repo_root.to_path_buf());
            let mut q = self.responses.lock().unwrap();
            if q.is_empty() {
                Ok(DetectOutcome {
                    branch: "main".into(),
                    pr: None,
                })
            } else {
                q.remove(0)
            }
        }
    }

    fn mk_pr() -> PrInfo {
        PrInfo {
            number: 7,
            url: "https://github.com/x/y/pull/7".into(),
            state: PrState::Open,
            head_ref_name: "feat/x".into(),
        }
    }

    #[tokio::test]
    async fn cached_pr_round_trips_through_orchestrator() {
        let fake = Arc::new(FakeDetector::new(vec![Ok(DetectOutcome {
            branch: "feat/x".into(),
            pr: Some(mk_pr()),
        })]));
        let orch = Arc::new(PrOrchestrator::with_detector(fake.clone()));
        orch.clone().tick_all(vec![PathBuf::from("/repo")]).await;
        assert_eq!(orch.cached_for(Path::new("/repo")), Some(mk_pr()));
        assert_eq!(fake.call_count(), 1);
    }

    #[tokio::test]
    async fn negative_result_is_cached_as_none_but_entry_exists() {
        let fake = Arc::new(FakeDetector::new(vec![Ok(DetectOutcome {
            branch: "feat/x".into(),
            pr: None,
        })]));
        let orch = Arc::new(PrOrchestrator::with_detector(fake.clone()));
        orch.clone().tick_all(vec![PathBuf::from("/repo")]).await;
        assert!(orch.cached_for(Path::new("/repo")).is_none());
        // Entry present — distinguishes negative-cached from
        // never-queried, which is the whole point of caching the
        // miss.
        assert!(orch.contains_entry_for(Path::new("/repo")));
    }

    #[tokio::test]
    async fn missing_gh_latches_and_short_circuits_subsequent_ticks() {
        let fake = Arc::new(FakeDetector::new(vec![Err(GhError::MissingCli("gh"))]));
        let orch = Arc::new(PrOrchestrator::with_detector(fake.clone()));
        orch.clone().tick_all(vec![PathBuf::from("/repo")]).await;
        assert!(orch.gh_absent(), "first MissingCli must latch");
        // Now a second tick should not call the detector again.
        let before = fake.call_count();
        orch.clone()
            .tick_all(vec![PathBuf::from("/repo"), PathBuf::from("/repo2")])
            .await;
        assert_eq!(
            fake.call_count(),
            before,
            "post-latch tick must not invoke the detector"
        );
    }

    #[tokio::test]
    async fn timeout_error_is_cached_as_miss() {
        // Timeout is non-fatal: detector returns Timeout, cache
        // records the miss (so next tick can retry without burning
        // an extra slot), and the orchestrator does NOT latch
        // gh-absent (the binary exists, it just hung).
        let fake = Arc::new(FakeDetector::new(vec![Err(GhError::Timeout("gh"))]));
        let orch = Arc::new(PrOrchestrator::with_detector(fake.clone()));
        orch.clone().tick_all(vec![PathBuf::from("/repo")]).await;
        assert!(orch.cached_for(Path::new("/repo")).is_none());
        assert!(orch.contains_entry_for(Path::new("/repo")));
        assert!(!orch.gh_absent(), "timeout must not latch gh-absent");
    }

    #[tokio::test]
    async fn parallel_tick_visits_every_root_once() {
        // 12 roots vs MAX_PARALLEL=4 — exercises the bounded
        // concurrency path (multiple semaphore acquires per root).
        let responses = (0..12)
            .map(|_| {
                Ok(DetectOutcome {
                    branch: "feat/x".into(),
                    pr: Some(mk_pr()),
                })
            })
            .collect();
        let fake = Arc::new(FakeDetector::new(responses));
        let orch = Arc::new(PrOrchestrator::with_detector(fake.clone()));
        let roots: Vec<PathBuf> = (0..12).map(|i| PathBuf::from(format!("/r{i}"))).collect();
        orch.clone().tick_all(roots).await;
        assert_eq!(fake.call_count(), 12);
    }
}
