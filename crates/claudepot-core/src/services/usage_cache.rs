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

/// Detailed per-account fetch result for UIs that want to explain to
/// the user *why* usage is unavailable instead of silently hiding an
/// account. Every failure mode has a distinct variant carrying just
/// enough info for a useful inline message (retry timer, error text).
#[derive(Debug, Clone)]
pub enum UsageOutcome {
    /// Data fetched within the CACHE_TTL window. `age_secs` is for UI
    /// freshness indicators (e.g. "as of 14s ago"); typically small.
    Fresh {
        response: UsageResponse,
        age_secs: u64,
    },
    /// Cached data served because the live fetch is on cooldown. The
    /// UI should render the numbers but caption them with
    /// "as of {age_secs}s ago".
    Stale {
        response: UsageResponse,
        age_secs: u64,
    },
    /// No credential blob stored — account has never been signed in
    /// via Claudepot. UI should prompt "Log in to see usage".
    NoCredentials,
    /// Local token is past expiry. UI should prompt "Token expired —
    /// log in again" linking to the per-account login flow.
    Expired,
    /// Rate-limited with no stale cache to fall back to. UI should
    /// show a countdown and retry automatically after `retry_after_secs`.
    RateLimited { retry_after_secs: u64 },
    /// Non-rate-limit failure (network, parse, 401). UI should show a
    /// Retry button plus the short error for debugging.
    Error(String),
}

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
///
/// Every control-flow path that reaches the inflight registration step
/// MUST publish one of these before dropping the tx, otherwise waiting
/// receivers get `changed()` = Err and translate that into
/// "inflight fetch was cancelled" — misleading when the initiator
/// merely returned early with no-blob / token-expired.
///
/// The identity gate runs OUTSIDE this state machine — it refuses to
/// register as initiator at all when the stored verify_status is
/// drift/rejected, so no variant is needed here for gate failures.
#[derive(Clone, Debug)]
enum FetchOutcome {
    Success(UsageResponse),
    NoBlob,
    TokenExpired,
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

        // 4. Load blob. Early returns MUST publish a FetchOutcome on
        //    `tx` before dropping it, else waiters receive a spurious
        //    "inflight fetch was cancelled" (audit M9). We also delay
        //    the inflight-cleanup to AFTER the broadcast so new callers
        //    don't slip past the dedupe and duplicate the work.
        let access_token = match self.load_access_token(uuid) {
            Ok(Some(token)) => token,
            Ok(None) => {
                let _ = tx.send(Some(FetchOutcome::NoBlob));
                guard.disarm_and_cleanup().await;
                return Ok(None);
            }
            Err(UsageFetchError::TokenExpired) => {
                let _ = tx.send(Some(FetchOutcome::TokenExpired));
                guard.disarm_and_cleanup().await;
                return Err(UsageFetchError::TokenExpired);
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = tx.send(Some(FetchOutcome::Failed(msg.clone())));
                guard.disarm_and_cleanup().await;
                return Err(UsageFetchError::FetchFailed(msg));
            }
        };

        // 5. HTTP call. If the server rejects the token (401) we surface
        //    AuthFailed → graceful path returns None. Reconciliation
        //    (`services::identity::verify_account_identity`) is the
        //    authoritative refresh path — usage_cache used to do its own
        //    refresh + retry here, but that ran without identity checks
        //    and entrenched misfiled blobs by writing fresh-but-wrong
        //    tokens to the slot. The systematic fix lives in `identity`;
        //    UI-driven reconciliation triggers it on cadence.
        //
        //    M9 fix: publish outcome BEFORE inflight cleanup so concurrent
        //    callers that arrive during cleanup don't miss the dedupe
        //    window and launch a duplicate request.
        let result = self.fetcher.fetch(&access_token).await;

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
                guard.disarm_and_cleanup().await;
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
                guard.disarm_and_cleanup().await;
                Err(UsageFetchError::RateLimited { retry_after_secs })
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = tx.send(Some(FetchOutcome::Failed(msg.clone())));
                guard.disarm_and_cleanup().await;
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

    /// Check the store's recorded identity-verification status before
    /// serving a usage token. Refuses when the slot is known-misfiled
    /// ("drift") or the token was server-rejected ("rejected") — in
    /// both cases the access token in the slot DOES NOT belong to
    /// the labelled account, so serving /usage against it would
    /// attribute another person's numbers to this UUID (audit H4,
    /// privacy bug).
    ///
    /// "never" (first-time, no verify yet) and "network_error" are
    /// allowed — the periodic reconciliation pass (`verify_all_accounts`
    /// in the GUI, `account verify` in the CLI) will update the
    /// status shortly. Blocking those would stall the UI on every
    /// new account.
    fn identity_gate(
        store: &crate::account::AccountStore,
        uuid: Uuid,
    ) -> Result<(), UsageFetchError> {
        match store.find_by_uuid(uuid) {
            Ok(Some(acct)) => match acct.verify_status.as_str() {
                "drift" | "rejected" => Err(UsageFetchError::FetchFailed(format!(
                    "identity gate: verify_status={}; run verify to reconcile",
                    acct.verify_status
                ))),
                _ => Ok(()),
            },
            Ok(None) => Err(UsageFetchError::FetchFailed(
                "identity gate: account not in store".to_string(),
            )),
            Err(e) => Err(UsageFetchError::FetchFailed(format!(
                "identity gate: store lookup failed: {e}"
            ))),
        }
    }

    /// Identity-gated variant of `fetch_usage`. Refuses to serve when
    /// the stored slot's `verify_status` is drift/rejected, preventing
    /// /usage from being called with a misfiled token (H4). The
    /// authoritative reconciliation path is
    /// `services::identity::verify_account_identity`; callers who see
    /// this error should run it and retry.
    pub async fn fetch_usage_verified(
        &self,
        store: &crate::account::AccountStore,
        uuid: Uuid,
        force: bool,
    ) -> Result<Option<UsageResponse>, UsageFetchError> {
        Self::identity_gate(store, uuid)?;
        self.fetch_usage(uuid, force).await
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

    /// Identity-gated batch detailed fetch. Every input uuid goes
    /// through `identity_gate` before its fetch; gate failures produce
    /// `UsageOutcome::Error` entries so the UI can render "drift —
    /// reconcile first" instead of silently attributing data to the
    /// wrong slot (H4).
    pub async fn fetch_batch_detailed_verified(
        &self,
        store: &crate::account::AccountStore,
        uuids: &[Uuid],
    ) -> HashMap<Uuid, UsageOutcome> {
        let mut out = HashMap::new();
        let mut first = true;
        for &uuid in uuids {
            if !first {
                tokio::time::sleep(BATCH_STAGGER).await;
            }
            first = false;
            let outcome = match Self::identity_gate(store, uuid) {
                Ok(()) => self.fetch_usage_detailed(uuid).await,
                Err(UsageFetchError::FetchFailed(msg)) => UsageOutcome::Error(msg),
                Err(e) => UsageOutcome::Error(e.to_string()),
            };
            out.insert(uuid, outcome);
        }
        out
    }

    /// Detailed single-account fetch for UIs that want to SHOW the reason
    /// usage is unavailable instead of silently hiding it. Never throws
    /// — every failure mode is encoded in the returned variant.
    pub async fn fetch_usage_detailed(&self, uuid: Uuid) -> UsageOutcome {
        match self.fetch_usage(uuid, false).await {
            Ok(Some(response)) => {
                // Discriminate fresh-vs-stale: if the cache says the
                // record is older than CACHE_TTL we are serving a
                // served-from-inflight-fallback or raced write; treat
                // as fresh by default, stale only on explicit cooldown
                // path below.
                let age_secs = self.cached_age_secs(uuid).await;
                UsageOutcome::Fresh { response, age_secs }
            }
            Ok(None) => UsageOutcome::NoCredentials,
            Err(UsageFetchError::Cooldown { .. })
            | Err(UsageFetchError::RateLimited { .. }) => {
                // Serve stale cache if we have one; otherwise signal
                // rate-limited-without-fallback so the UI can render
                // "retry in Ns".
                let results = self.results.lock().await;
                if let Some(cached) = results.get(&uuid) {
                    let response = cached.response.clone();
                    let age_secs = cached.fetched_at.elapsed().as_secs();
                    return UsageOutcome::Stale { response, age_secs };
                }
                drop(results);
                let retry_after_secs = match self.fetch_usage(uuid, false).await {
                    Err(UsageFetchError::Cooldown { remaining_secs }) => remaining_secs,
                    Err(UsageFetchError::RateLimited { retry_after_secs }) => retry_after_secs,
                    _ => 60, // shouldn't happen; benign default
                };
                UsageOutcome::RateLimited { retry_after_secs }
            }
            Err(UsageFetchError::TokenExpired) => UsageOutcome::Expired,
            Err(UsageFetchError::FetchFailed(msg)) => UsageOutcome::Error(msg),
        }
    }

    /// Batch variant of `fetch_usage_detailed`. Every input uuid appears
    /// in the output map — status is carried per entry so the UI can
    /// render the exact reason each account is unavailable.
    pub async fn fetch_batch_detailed(
        &self,
        uuids: &[Uuid],
    ) -> HashMap<Uuid, UsageOutcome> {
        let mut out = HashMap::new();
        let mut first = true;
        for &uuid in uuids {
            if !first {
                tokio::time::sleep(BATCH_STAGGER).await;
            }
            first = false;
            out.insert(uuid, self.fetch_usage_detailed(uuid).await);
        }
        out
    }

    /// Peek cached age in seconds, or None if nothing cached.
    async fn cached_age_secs(&self, uuid: Uuid) -> u64 {
        let results = self.results.lock().await;
        results
            .get(&uuid)
            .map(|c| c.fetched_at.elapsed().as_secs())
            .unwrap_or(0)
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

    /// Load the access_token for `uuid` from its private slot.
    ///
    /// This is the fast read path: parse the blob, check local expiry,
    /// return the stored token. If the token is past its local expiry
    /// we return `TokenExpired` (graceful path → blank); the systematic
    /// refresh + identity check belongs to
    /// `services::identity::verify_account_identity`, which is invoked
    /// by reconciliation passes (CLI / GUI) — not here. Doing both
    /// refresh and usage in one place was how a misfiled blob got
    /// re-saved with a fresh-but-wrong token.
    ///
    /// Failure modes:
    /// - no blob → `Ok(None)` (account has no CLI credentials)
    /// - corrupt blob → `Err(FetchFailed)`
    /// - blob expired → `Err(TokenExpired)` — caller's graceful path
    ///   returns None so the UI shows blank until reconciliation rotates
    ///   the slot
    fn load_access_token(&self, uuid: Uuid) -> Result<Option<String>, UsageFetchError> {
        let blob_str = match swap::load_private(uuid) {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        let blob = CredentialBlob::from_json(&blob_str)
            .map_err(|e| UsageFetchError::FetchFailed(format!("corrupt blob: {e}")))?;

        if blob.is_expired(0) {
            return Err(UsageFetchError::TokenExpired);
        }
        Ok(Some(blob.claude_ai_oauth.access_token.clone()))
    }

    fn outcome_to_result(
        outcome: Option<FetchOutcome>,
    ) -> Result<Option<UsageResponse>, UsageFetchError> {
        match outcome {
            Some(FetchOutcome::Success(r)) => Ok(Some(r)),
            Some(FetchOutcome::NoBlob) => Ok(None),
            Some(FetchOutcome::TokenExpired) => Err(UsageFetchError::TokenExpired),
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
                resets_at: Some(
                    chrono::DateTime::parse_from_rfc3339("2026-04-13T10:00:00+00:00").unwrap(),
                ),
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

    // -- Disk-backed tests for load_access_token, covering the branches
    //    that got simplified when ad-hoc refresh was retired. We want
    //    explicit coverage for "fresh → returns token", "expired →
    //    TokenExpired", "missing → Ok(None)", "corrupt → FetchFailed".

    #[test]
    fn test_load_access_token_fresh_blob_returns_token() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let uuid = uuid::Uuid::new_v4();
        crate::cli_backend::swap::save_private(uuid, &crate::testing::fresh_blob_json()).unwrap();

        let (fetcher, _) = MockFetcher::new(0.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        let token = cache.load_access_token(uuid).unwrap();
        assert_eq!(token.as_deref(), Some("sk-ant-oat01-test"));
        crate::cli_backend::swap::delete_private(uuid).unwrap();
    }

    #[test]
    fn test_load_access_token_expired_blob_returns_token_expired() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let uuid = uuid::Uuid::new_v4();
        crate::cli_backend::swap::save_private(uuid, &crate::testing::expired_blob_json()).unwrap();

        let (fetcher, _) = MockFetcher::new(0.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        assert!(matches!(
            cache.load_access_token(uuid),
            Err(UsageFetchError::TokenExpired)
        ));
        crate::cli_backend::swap::delete_private(uuid).unwrap();
    }

    #[test]
    fn test_load_access_token_missing_blob_returns_ok_none() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let uuid = uuid::Uuid::new_v4();
        // No save_private — slot is empty.

        let (fetcher, _) = MockFetcher::new(0.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        assert!(matches!(cache.load_access_token(uuid), Ok(None)));
    }

    #[test]
    fn test_load_access_token_corrupt_blob_returns_fetch_failed() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let uuid = uuid::Uuid::new_v4();
        crate::cli_backend::swap::save_private(uuid, "not-json-at-all").unwrap();

        let (fetcher, _) = MockFetcher::new(0.0);
        let cache = UsageCache::with_fetcher(Box::new(fetcher));
        assert!(matches!(
            cache.load_access_token(uuid),
            Err(UsageFetchError::FetchFailed(_))
        ));
        crate::cli_backend::swap::delete_private(uuid).unwrap();
    }
}
