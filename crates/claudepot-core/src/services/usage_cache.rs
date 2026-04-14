//! Usage cache with result caching, in-flight dedup, and 429 cooldown.
//!
//! All usage API calls go through [`UsageCache`] so callers get rate-limit
//! protection without any extra work.

use crate::blob::CredentialBlob;
use crate::cli_backend::swap;
use crate::error::OAuthError;
use crate::oauth::usage::UsageResponse;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::{watch, Mutex};
use uuid::Uuid;

const CACHE_TTL: Duration = Duration::from_secs(60);
const BATCH_STAGGER: Duration = Duration::from_millis(200);

// ---------------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum UsageFetchError {
    #[error("rate limited — suppressed for {remaining_secs}s")]
    Cooldown { remaining_secs: u64 },

    #[error("rate limited by server — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("access token expired")]
    TokenExpired,

    #[error("fetch failed: {0}")]
    FetchFailed(String),
}

// ---------------------------------------------------------------------------
// Fetcher trait (for test injection)
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait UsageFetcher: Send + Sync {
    async fn fetch(&self, access_token: &str) -> Result<UsageResponse, OAuthError>;
}

/// Production fetcher — calls the real HTTP endpoint.
pub struct DefaultFetcher;

#[async_trait::async_trait]
impl UsageFetcher for DefaultFetcher {
    async fn fetch(&self, access_token: &str) -> Result<UsageResponse, OAuthError> {
        crate::oauth::usage::fetch(access_token).await
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

struct CachedUsage {
    response: UsageResponse,
    fetched_at: Instant,
}

/// Outcome shared with waiting receivers via watch channel.
#[derive(Clone, Debug)]
enum FetchOutcome {
    Success(UsageResponse),
    RateLimited { retry_after_secs: u64 },
    Failed(String),
}

// ---------------------------------------------------------------------------
// UsageCache
// ---------------------------------------------------------------------------

pub struct UsageCache {
    results: Mutex<HashMap<Uuid, CachedUsage>>,
    inflight: Mutex<HashMap<Uuid, watch::Receiver<Option<FetchOutcome>>>>,
    cooldowns: Mutex<HashMap<Uuid, Instant>>,
    fetcher: Box<dyn UsageFetcher>,
}

impl UsageCache {
    pub fn new() -> Self {
        Self {
            results: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            cooldowns: Mutex::new(HashMap::new()),
            fetcher: Box::new(DefaultFetcher),
        }
    }

    /// Test-only: inject a mock fetcher.
    #[cfg(test)]
    fn with_fetcher(fetcher: Box<dyn UsageFetcher>) -> Self {
        Self {
            results: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            cooldowns: Mutex::new(HashMap::new()),
            fetcher,
        }
    }

    /// Fetch usage for a single account.
    ///
    /// - `Ok(Some(r))` — data (fresh or cached)
    /// - `Ok(None)` — no credentials stored
    /// - `Err(Cooldown)` — suppressed after a previous 429
    /// - `Err(RateLimited)` — 429 just received from server
    /// - `Err(TokenExpired)` — blob exists but access token past expiry
    /// - `Err(FetchFailed)` — network/parse error
    pub async fn fetch_usage(
        &self,
        uuid: Uuid,
        force: bool,
    ) -> Result<Option<UsageResponse>, UsageFetchError> {
        // 1. Check cooldown (force does NOT bypass cooldown — never hammer a 429)
        {
            let now = Instant::now();
            let cooldowns = self.cooldowns.lock().await;
            if let Some(&suppress_until) = cooldowns.get(&uuid) {
                if let Some(remaining) = suppress_until.checked_duration_since(now) {
                    return Err(UsageFetchError::Cooldown {
                        remaining_secs: remaining.as_secs(),
                    });
                }
            }
        }

        // 2. Check result cache (skip if force=true)
        if !force {
            let results = self.results.lock().await;
            if let Some(cached) = results.get(&uuid) {
                if cached.fetched_at.elapsed() < CACHE_TTL {
                    return Ok(Some(cached.response.clone()));
                }
            }
        }

        // 3. Check inflight AND register as initiator atomically (single lock scope)
        //    This prevents two tasks from both deciding they are the initiator.
        let tx = {
            let mut inflight = self.inflight.lock().await;
            if let Some(rx) = inflight.get(&uuid) {
                let mut rx = rx.clone();
                drop(inflight); // release lock before await
                // Wait for the initiator to finish. If sender was dropped (panic),
                // changed() returns Err — we handle that explicitly.
                if rx.changed().await.is_err() {
                    return Err(UsageFetchError::FetchFailed(
                        "inflight fetch was cancelled".into(),
                    ));
                }
                let outcome = rx.borrow().clone();
                return Self::outcome_to_result(outcome);
            }
            // We are the initiator — register watch channel under the same lock
            let (tx, rx) = watch::channel(None);
            inflight.insert(uuid, rx);
            tx
        };

        // Drop guard: if anything below panics, ensure the inflight entry is
        // cleaned up so the UUID isn't permanently broken.
        let guard = InflightGuard {
            uuid,
            inflight: &self.inflight,
            armed: true,
        };

        // 4. Load blob
        let access_token = match self.load_access_token(uuid).await {
            Ok(Some(token)) => token,
            Ok(None) => {
                guard.disarm_and_cleanup().await;
                return Ok(None);
            }
            Err(e) => {
                guard.disarm_and_cleanup().await;
                return Err(e);
            }
        };

        // 5. HTTP call. If the server rejects the token (401) we try one
        //    forced refresh and retry — "not yet expired locally but already
        //    revoked upstream" happens whenever the user logs in elsewhere.
        let mut result = self.fetcher.fetch(&access_token).await;
        if matches!(&result, Err(OAuthError::AuthFailed(_))) {
            tracing::info!(account = %uuid, "usage fetch got 401 — forcing refresh and retrying once");
            match self.force_refresh(uuid).await {
                Ok(Some(new_token)) => {
                    result = self.fetcher.fetch(&new_token).await;
                }
                Ok(None) => tracing::warn!(account = %uuid, "force refresh returned None — giving up"),
                Err(e) => tracing::warn!(account = %uuid, "force refresh errored: {e:?}"),
            }
        }
        guard.disarm_and_cleanup().await;

        match result {
            Ok(response) => {
                {
                    let mut results = self.results.lock().await;
                    results.insert(
                        uuid,
                        CachedUsage {
                            response: response.clone(),
                            fetched_at: Instant::now(),
                        },
                    );
                }
                let _ = tx.send(Some(FetchOutcome::Success(response.clone())));
                Ok(Some(response))
            }
            Err(OAuthError::RateLimited { retry_after_secs }) => {
                {
                    let mut cooldowns = self.cooldowns.lock().await;
                    cooldowns.insert(
                        uuid,
                        Instant::now() + Duration::from_secs(retry_after_secs),
                    );
                }
                let _ = tx.send(Some(FetchOutcome::RateLimited { retry_after_secs }));
                Err(UsageFetchError::RateLimited { retry_after_secs })
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = tx.send(Some(FetchOutcome::Failed(msg.clone())));
                Err(UsageFetchError::FetchFailed(msg))
            }
        }
    }

    /// Fetch usage for multiple accounts with stagger between requests.
    pub async fn fetch_batch(
        &self,
        uuids: &[Uuid],
    ) -> HashMap<Uuid, Result<Option<UsageResponse>, UsageFetchError>> {
        let mut out = HashMap::new();
        let mut first = true;
        for &uuid in uuids {
            if !first {
                tokio::time::sleep(BATCH_STAGGER).await;
            }
            first = false;
            out.insert(uuid, self.fetch_usage(uuid, false).await);
        }
        out
    }

    /// Fetch usage gracefully: never returns rate-limit errors.
    ///
    /// On cooldown or rate-limit: returns the last cached value (even if
    /// stale), or `None` if nothing was ever cached. The caller never
    /// sees rate-limit state — designed for user-facing UIs.
    pub async fn fetch_usage_graceful(&self, uuid: Uuid) -> Option<UsageResponse> {
        match self.fetch_usage(uuid, false).await {
            Ok(data) => data,
            Err(UsageFetchError::Cooldown { .. }) | Err(UsageFetchError::RateLimited { .. }) => {
                // Serve stale cache if available, otherwise None.
                let results = self.results.lock().await;
                results.get(&uuid).map(|c| c.response.clone())
            }
            Err(_) => None,
        }
    }

    /// Batch-fetch for the GUI: never exposes rate-limit errors.
    pub async fn fetch_batch_graceful(
        &self,
        uuids: &[Uuid],
    ) -> HashMap<Uuid, Option<UsageResponse>> {
        let mut out = HashMap::new();
        let mut first = true;
        for &uuid in uuids {
            if !first {
                tokio::time::sleep(BATCH_STAGGER).await;
            }
            first = false;
            out.insert(uuid, self.fetch_usage_graceful(uuid).await);
        }
        out
    }

    /// Evict cached result, cooldown, and inflight entry for a UUID.
    ///
    /// Call after credential changes (remove, reimport, login).
    pub async fn invalidate(&self, uuid: Uuid) {
        {
            let mut results = self.results.lock().await;
            results.remove(&uuid);
        }
        {
            let mut cooldowns = self.cooldowns.lock().await;
            cooldowns.remove(&uuid);
        }
        {
            let mut inflight = self.inflight.lock().await;
            inflight.remove(&uuid);
        }
    }

    // -- private helpers --

    /// Force a token refresh for `uuid` regardless of local expiry.
    ///
    /// Used when the server rejects a locally-"valid" access_token (401).
    /// Returns `Ok(Some(new_token))` on success, `Ok(None)` if there is
    /// nothing to refresh (no blob on disk) or if the refresh itself
    /// failed silently.
    async fn force_refresh(&self, uuid: Uuid) -> Result<Option<String>, UsageFetchError> {
        use crate::oauth::refresh;

        let blob_str = match swap::load_private(uuid) {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let mut blob = CredentialBlob::from_json(&blob_str)
            .map_err(|e| UsageFetchError::FetchFailed(format!("corrupt blob: {e}")))?;
        let rt = &blob.claude_ai_oauth.refresh_token;
        let refreshed = match refresh::refresh(rt).await {
            Ok(r) => r,
            Err(OAuthError::RateLimited { retry_after_secs }) => {
                return Err(UsageFetchError::RateLimited { retry_after_secs });
            }
            Err(e) => {
                tracing::warn!(account = %uuid, "force refresh failed: {e}");
                return Ok(None);
            }
        };
        blob.claude_ai_oauth.access_token = refreshed.access_token.clone();
        blob.claude_ai_oauth.refresh_token = refreshed.refresh_token;
        let now_ms = chrono::Utc::now().timestamp_millis();
        blob.claude_ai_oauth.expires_at = now_ms + (refreshed.expires_in as i64) * 1000;
        if let Ok(j) = blob.to_json() {
            let _ = swap::save_private(uuid, &j);
        }
        Ok(Some(refreshed.access_token))
    }

    /// Load a usable access token for `uuid`.
    ///
    /// Inactive accounts snapshot a blob at login/switch time, but CC never
    /// refreshes their token afterwards — so without an in-app refresh, the
    /// stored access_token expires in ~8h and `fetch_usage` would give up.
    /// When the stored blob is expired we exchange its refresh_token for a
    /// new access_token via `oauth::refresh`, persist the rotated blob back
    /// to the account's private slot, and return the fresh access_token.
    ///
    /// Failure modes:
    /// - no blob on disk → `Ok(None)` (account has no CLI credentials)
    /// - corrupt blob → `Err(FetchFailed)` (surfaced as fetch failure)
    /// - refresh fails (revoked, network, etc.) → `Ok(None)` in the graceful
    ///   path (usage appears blank rather than painting an error)
    async fn load_access_token(
        &self,
        uuid: Uuid,
    ) -> Result<Option<String>, UsageFetchError> {
        use crate::oauth::refresh;

        let blob_str = match swap::load_private(uuid) {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let mut blob = CredentialBlob::from_json(&blob_str)
            .map_err(|e| UsageFetchError::FetchFailed(format!("corrupt blob: {e}")))?;

        if !blob.is_expired(0) {
            return Ok(Some(blob.claude_ai_oauth.access_token.clone()));
        }

        // Expired — attempt refresh.
        tracing::info!(account = %uuid, "access token expired, refreshing via refresh_token");
        let rt = &blob.claude_ai_oauth.refresh_token;
        let refreshed = match refresh::refresh(rt).await {
            Ok(r) => r,
            Err(OAuthError::RateLimited { retry_after_secs }) => {
                return Err(UsageFetchError::RateLimited { retry_after_secs });
            }
            Err(e) => {
                tracing::warn!(account = %uuid, "refresh failed: {e}");
                return Ok(None);
            }
        };

        // Rotate blob and persist.
        blob.claude_ai_oauth.access_token = refreshed.access_token.clone();
        blob.claude_ai_oauth.refresh_token = refreshed.refresh_token;
        // expires_at is ms since epoch (i64). expires_in is seconds.
        let now_ms = chrono::Utc::now().timestamp_millis();
        blob.claude_ai_oauth.expires_at = now_ms + (refreshed.expires_in as i64) * 1000;
        let new_json = blob
            .to_json()
            .map_err(|e| UsageFetchError::FetchFailed(format!("serialize blob: {e}")))?;
        if let Err(e) = swap::save_private(uuid, &new_json) {
            tracing::warn!(account = %uuid, "persisting refreshed blob failed: {e}");
            // Still return the fresh token for this fetch — next call will
            // re-refresh, which is merely suboptimal, not broken.
        }

        Ok(Some(refreshed.access_token))
    }

    fn outcome_to_result(
        outcome: Option<FetchOutcome>,
    ) -> Result<Option<UsageResponse>, UsageFetchError> {
        match outcome {
            Some(FetchOutcome::Success(r)) => Ok(Some(r)),
            Some(FetchOutcome::RateLimited { retry_after_secs }) => {
                Err(UsageFetchError::RateLimited { retry_after_secs })
            }
            Some(FetchOutcome::Failed(msg)) => Err(UsageFetchError::FetchFailed(msg)),
            None => Err(UsageFetchError::FetchFailed(
                "inflight fetch did not produce a result".into(),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// InflightGuard — panic-safe cleanup for inflight map entries
// ---------------------------------------------------------------------------

/// RAII guard that removes the inflight entry on drop (including panics).
/// Call `disarm_and_cleanup()` on the normal path; if the code panics before
/// that call, the `Drop` impl handles cleanup synchronously via `try_lock`.
struct InflightGuard<'a> {
    uuid: Uuid,
    inflight: &'a Mutex<HashMap<Uuid, watch::Receiver<Option<FetchOutcome>>>>,
    armed: bool,
}

impl<'a> InflightGuard<'a> {
    async fn disarm_and_cleanup(mut self) {
        self.armed = false;
        let mut inflight = self.inflight.lock().await;
        inflight.remove(&self.uuid);
    }
}

impl<'a> Drop for InflightGuard<'a> {
    fn drop(&mut self) {
        if self.armed {
            // Panic path: can't .await here, so use try_lock.
            // If the lock is held (unlikely during unwind), the entry
            // will be orphaned but the watch sender is also dropped,
            // so waiters will get RecvError immediately.
            if let Ok(mut inflight) = self.inflight.try_lock() {
                inflight.remove(&self.uuid);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::usage::UsageWindow;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn make_response(utilization: f64) -> UsageResponse {
        UsageResponse {
            five_hour: Some(UsageWindow {
                utilization,
                resets_at: chrono::DateTime::parse_from_rfc3339("2026-04-13T10:00:00+00:00")
                    .unwrap(),
            }),
            seven_day: None,
            seven_day_oauth_apps: None,
            seven_day_opus: None,
            seven_day_sonnet: None,
            seven_day_cowork: None,
            iguana_necktie: None,
            extra_usage: None,
            unknown: HashMap::new(),
        }
    }

    struct MockFetcher {
        call_count: Arc<AtomicU32>,
        response: UsageResponse,
    }

    impl MockFetcher {
        fn new(utilization: f64) -> (Self, Arc<AtomicU32>) {
            let count = Arc::new(AtomicU32::new(0));
            (
                Self {
                    call_count: count.clone(),
                    response: make_response(utilization),
                },
                count,
            )
        }
    }

    #[async_trait::async_trait]
    impl UsageFetcher for MockFetcher {
        async fn fetch(&self, _access_token: &str) -> Result<UsageResponse, OAuthError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    struct RateLimitFetcher {
        retry_after: u64,
        call_count: Arc<AtomicU32>,
    }

    impl RateLimitFetcher {
        fn new(retry_after: u64) -> (Self, Arc<AtomicU32>) {
            let count = Arc::new(AtomicU32::new(0));
            (
                Self {
                    retry_after,
                    call_count: count.clone(),
                },
                count,
            )
        }
    }

    #[async_trait::async_trait]
    impl UsageFetcher for RateLimitFetcher {
        async fn fetch(&self, _access_token: &str) -> Result<UsageResponse, OAuthError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Err(OAuthError::RateLimited {
                retry_after_secs: self.retry_after,
            })
        }
    }

    // These tests exercise the cache logic in isolation. They bypass
    // load_access_token by directly manipulating the cache state, because
    // swap::load_private requires on-disk blobs we don't want in unit tests.

    #[tokio::test]
    async fn test_cache_hit_returns_cached_no_second_fetch() {
        let (fetcher, count) = MockFetcher::new(42.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        let uuid = Uuid::new_v4();

        // Pre-populate the cache as if a prior fetch succeeded
        {
            let mut results = cache.results.lock().await;
            results.insert(
                uuid,
                CachedUsage {
                    response: make_response(42.0),
                    fetched_at: Instant::now(),
                },
            );
        }

        let result = cache.fetch_usage(uuid, false).await;
        assert!(result.is_ok());
        let resp = result.unwrap().unwrap();
        assert_eq!(resp.five_hour.unwrap().utilization, 42.0);
        assert_eq!(count.load(Ordering::SeqCst), 0, "fetcher should not be called");
    }

    #[tokio::test]
    async fn test_cache_miss_after_ttl() {
        let (fetcher, count) = MockFetcher::new(55.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        let uuid = Uuid::new_v4();

        // Insert an expired cache entry
        {
            let mut results = cache.results.lock().await;
            results.insert(
                uuid,
                CachedUsage {
                    response: make_response(42.0),
                    fetched_at: Instant::now() - CACHE_TTL - Duration::from_secs(1),
                },
            );
        }

        // fetch_usage will see stale cache and try to call the fetcher.
        // Since there's no real blob on disk, it will return Ok(None) before
        // reaching the fetcher. This tests the cache staleness check.
        let result = cache.fetch_usage(uuid, false).await;
        // Ok(None) because load_access_token can't find a blob — that's expected.
        // The important thing: it did NOT return the stale cached value.
        assert!(
            result.is_ok(),
            "should not error on missing blob, got: {:?}",
            result
        );
        assert!(result.unwrap().is_none(), "should return None, not stale cache");
    }

    #[tokio::test]
    async fn test_cooldown_blocks_fetch() {
        let (fetcher, count) = RateLimitFetcher::new(30);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        let uuid = Uuid::new_v4();

        // Simulate a prior 429 by inserting a cooldown
        {
            let mut cooldowns = cache.cooldowns.lock().await;
            cooldowns.insert(uuid, Instant::now() + Duration::from_secs(30));
        }

        let result = cache.fetch_usage(uuid, false).await;
        assert!(matches!(result, Err(UsageFetchError::Cooldown { .. })));
        assert_eq!(count.load(Ordering::SeqCst), 0, "fetcher should not be called during cooldown");
    }

    #[tokio::test]
    async fn test_cooldown_expiry_allows_fetch() {
        let (fetcher, count) = MockFetcher::new(10.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        let uuid = Uuid::new_v4();

        // Cooldown already expired
        {
            let mut cooldowns = cache.cooldowns.lock().await;
            cooldowns.insert(uuid, Instant::now() - Duration::from_secs(1));
        }

        // Should pass cooldown check and proceed to load_access_token
        let result = cache.fetch_usage(uuid, false).await;
        // Ok(None) because no blob on disk — but cooldown didn't block it
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_force_bypasses_cache_but_not_cooldown() {
        let (fetcher, _count) = MockFetcher::new(99.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        let uuid = Uuid::new_v4();

        // Pre-populate cache
        {
            let mut results = cache.results.lock().await;
            results.insert(
                uuid,
                CachedUsage {
                    response: make_response(42.0),
                    fetched_at: Instant::now(),
                },
            );
        }

        // force=true should bypass the cache
        let result = cache.fetch_usage(uuid, true).await;
        // Returns Ok(None) because load_access_token finds no blob,
        // but it did NOT return the cached 42.0 value — force worked.
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());

        // Now add a cooldown — force should NOT bypass it
        {
            let mut cooldowns = cache.cooldowns.lock().await;
            cooldowns.insert(uuid, Instant::now() + Duration::from_secs(30));
        }
        let result = cache.fetch_usage(uuid, true).await;
        assert!(matches!(result, Err(UsageFetchError::Cooldown { .. })));
    }

    #[tokio::test]
    async fn test_invalidate_clears_cache_and_cooldown() {
        let (fetcher, _count) = MockFetcher::new(42.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        let uuid = Uuid::new_v4();

        // Populate both cache and cooldown
        {
            let mut results = cache.results.lock().await;
            results.insert(
                uuid,
                CachedUsage {
                    response: make_response(42.0),
                    fetched_at: Instant::now(),
                },
            );
        }
        {
            let mut cooldowns = cache.cooldowns.lock().await;
            cooldowns.insert(uuid, Instant::now() + Duration::from_secs(60));
        }

        cache.invalidate(uuid).await;

        // Cache should be empty
        {
            let results = cache.results.lock().await;
            assert!(!results.contains_key(&uuid));
        }
        // Cooldown should be cleared
        {
            let cooldowns = cache.cooldowns.lock().await;
            assert!(!cooldowns.contains_key(&uuid));
        }
    }

    #[tokio::test]
    async fn test_outcome_mapping() {
        // Success
        let r = UsageCache::outcome_to_result(Some(FetchOutcome::Success(make_response(50.0))));
        assert!(r.is_ok());
        assert_eq!(r.unwrap().unwrap().five_hour.unwrap().utilization, 50.0);

        // RateLimited
        let r = UsageCache::outcome_to_result(Some(FetchOutcome::RateLimited {
            retry_after_secs: 30,
        }));
        assert!(matches!(r, Err(UsageFetchError::RateLimited { retry_after_secs: 30 })));

        // Failed
        let r = UsageCache::outcome_to_result(Some(FetchOutcome::Failed("boom".into())));
        assert!(matches!(r, Err(UsageFetchError::FetchFailed(_))));

        // None (channel dropped)
        let r = UsageCache::outcome_to_result(None);
        assert!(matches!(r, Err(UsageFetchError::FetchFailed(_))));
    }
}
