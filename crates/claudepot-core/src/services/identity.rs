//! Identity verification: reconcile a per-account blob with `/api/oauth/profile`.
//!
//! The account store's `email` column is a LABEL — immutable, set at
//! registration from the first profile fetch. Over time, credential blobs
//! can drift from this label (misfiled writes, external `claude auth login`
//! under a different account, revocation-then-refresh chains). Without
//! verification, the UI paints whatever the label says and the user can't
//! see the drift.
//!
//! This module is the single authoritative "does blob match label" primitive.
//! Every save path, switch boundary, and reconciliation pass calls it and
//! persists the outcome to `accounts.verify_status` / `verified_email` /
//! `verified_at`.
//!
//! Callers don't need to know about token refresh — when the blob's
//! access_token is rejected (401), this module attempts a refresh_token
//! exchange once, writes the refreshed blob back, and retries the profile
//! fetch. If the refresh itself fails, the outcome is `Rejected` (the user
//! must re-login). If the profile fetch comes back with a different email
//! than the label, the outcome is `Drift` — and we do NOT persist the
//! refreshed blob, so we don't entrench the misfiling.
use crate::account::{Account, AccountStore, VerifyOutcome};
use crate::blob::CredentialBlob;
use crate::cli_backend::swap;
use crate::cli_backend::swap::{DefaultRefresher, ProfileFetcher, TokenRefresher};
use crate::cli_backend::{self, CliPlatform};
use crate::error::OAuthError;
use chrono::Utc;
use uuid::Uuid;

/// "Is a Claude Code process live right now?" — injectable so tests
/// don't depend on whatever happens to be running on the host (a
/// developer machine is very often running `claude`).
///
/// Only consulted on the active account, and only once its token has
/// actually expired, so the process probe stays off the common path.
#[async_trait::async_trait]
pub trait LiveSessionProbe: Send + Sync {
    async fn cc_running(&self) -> bool;
}

/// Production probe — the same `ps`-based detector `swap::switch` gates
/// on.
pub struct ProcessProbe;

#[async_trait::async_trait]
impl LiveSessionProbe for ProcessProbe {
    async fn cc_running(&self) -> bool {
        swap::is_cc_process_running_public().await
    }
}

/// Error surfaced to callers that need the cause (mostly doctor / CLI).
/// Most GUI code converts this to a [`VerifyOutcome`] via
/// [`verify_account_identity`] and renders that instead.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("no account with uuid {0}")]
    AccountNotFound(Uuid),

    #[error("no credential blob stored for this account")]
    NoBlob,

    #[error("stored blob is not valid JSON: {0}")]
    CorruptBlob(String),

    #[error("store update failed: {0}")]
    Store(String),
}

/// Verify the blob under `uuid` against `/api/oauth/profile` and persist
/// the outcome on the account row. Returns the outcome so the caller can
/// log, exit-code, or render from it.
///
/// Flow:
/// 1. Load the blob from the per-account private slot.
/// 2. Call `/profile` with the blob's access_token.
///    - 401 → attempt refresh_token exchange once; on success, re-fetch
///      profile and (only if email matches the label) persist the rotated
///      blob. On refresh failure → outcome is `Rejected`.
/// 3. Compare the returned email against `account.email` (case-insensitive).
///    - Match → `Ok`, verified_email/verified_at/verify_status persisted.
///    - Mismatch → `Drift`, verified_email updated to the ACTUAL email,
///      status = "drift". Crucially: on a drift detected AFTER refresh,
///      we do NOT write the rotated blob back to the slot — that would
///      entrench the corruption with a fresh working-looking token.
/// 4. On network / 5xx / parse errors → `NetworkError`. `verified_email`
///    is preserved (a transient blip must not wipe history); only the
///    status is updated so the UI can show "last verified N ago · stale".
pub async fn verify_account_identity(
    store: &AccountStore,
    uuid: Uuid,
    fetcher: &dyn ProfileFetcher,
) -> Result<VerifyOutcome, VerifyError> {
    let platform = cli_backend::create_platform();
    verify_account_identity_with(
        store,
        uuid,
        fetcher,
        &DefaultRefresher,
        platform.as_ref(),
        &ProcessProbe,
    )
    .await
}

/// Testable variant: inject a [`TokenRefresher`], a [`CliPlatform`] and
/// a [`LiveSessionProbe`] so the 401→refresh branch and the
/// active-account branch can be exercised without real HTTP, a real
/// keychain, or a real `claude` process.
pub async fn verify_account_identity_with(
    store: &AccountStore,
    uuid: Uuid,
    fetcher: &dyn ProfileFetcher,
    refresher: &dyn TokenRefresher,
    platform: &dyn CliPlatform,
    probe: &dyn LiveSessionProbe,
) -> Result<VerifyOutcome, VerifyError> {
    let account = store
        .find_by_uuid(uuid)
        .map_err(|e| VerifyError::Store(e.to_string()))?
        .ok_or(VerifyError::AccountNotFound(uuid))?;

    // The ACTIVE CLI account is the one case where two live copies of
    // the same OAuth token family exist: CC's own slot and Claudepot's
    // private slot. Anthropic rotates (and invalidates) the
    // refresh_token on every exchange, so refreshing the private copy
    // here would strand CC holding a dead token — the user gets
    // "/login" the moment their access token expires. Route the active
    // account through the CC slot instead: it reads CC's live blob,
    // CAS-writes any rotation back into it, and mirrors the result
    // into the private slot so the two never split into separate
    // token families.
    if account.is_cli_active {
        if let Some(outcome) =
            verify_active_from_cc_slot(&account, uuid, platform, probe, fetcher, refresher).await?
        {
            store
                .update_verification(uuid, &outcome)
                .map_err(|e| VerifyError::Store(e.to_string()))?;
            return Ok(outcome);
        }
        // CC's slot is empty or unparseable — there is no competing
        // copy to strand, so the private-slot path below is safe.
    }

    let blob_str = match swap::load_private(uuid).await {
        Ok(s) => s,
        Err(_) => return Err(VerifyError::NoBlob),
    };
    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| VerifyError::CorruptBlob(e.to_string()))?;

    let outcome = match run_profile_check(&blob, fetcher).await {
        ProfileCheck::Ok(actual) => classify(&account.email, actual),
        ProfileCheck::Rejected => {
            // 401 — try one refresh, then re-check. If refresh fails, it's
            // Rejected. If refresh succeeds but the new profile email
            // doesn't match the label, Drift (and we DON'T save).
            match try_refresh(uuid, &blob, fetcher, refresher).await {
                Ok(Some((new_blob_json, actual))) => {
                    let drift = !actual.eq_ignore_ascii_case(&account.email);
                    if !drift {
                        // Safe to persist — label and server agree. The
                        // write is the load-bearing step: if it fails the
                        // in-memory success is meaningless (next fetch
                        // will hit the stale blob again), so surface it
                        // instead of marking the row Ok.
                        //
                        // TOCTOU guard: only overwrite the slot if it
                        // still holds the exact bytes we loaded earlier.
                        // A concurrent reimport/sync/login that raced us
                        // has already written the newer blob; our stale
                        // rotation would clobber it. Skip the write in
                        // that case and treat the fetched identity as
                        // authoritative for this pass.
                        match swap::load_private(uuid).await {
                            Ok(current) if current == blob_str => {
                                if let Err(e) = swap::save_private(uuid, &new_blob_json).await {
                                    tracing::error!(
                                        account = %uuid,
                                        "persisting refreshed blob failed: {e}"
                                    );
                                    return Err(VerifyError::Store(format!(
                                        "save_private failed: {e}"
                                    )));
                                }
                                // Write stuck — label and server agree,
                                // classify as Ok (falls through below).
                                classify(&account.email, actual)
                            }
                            Ok(_) | Err(_) => {
                                // Slot changed mid-flight (concurrent
                                // reimport/sync/login) — our rotated
                                // blob is stale. We didn't write, so
                                // don't persist an "ok" claim about a
                                // slot we can't vouch for. Return
                                // NetworkError so the UI shows "could
                                // not confirm last pass" rather than
                                // falsely green, and verified_email
                                // history is preserved by
                                // update_verification's blip semantics.
                                tracing::info!(
                                    account = %uuid,
                                    "slot changed during refresh — classifying as NetworkError to avoid a stale Ok persistence"
                                );
                                VerifyOutcome::NetworkError
                            }
                        }
                    } else {
                        tracing::warn!(
                            account = %uuid,
                            expected = %account.email,
                            actual = %actual,
                            "drift detected after refresh — NOT persisting rotated blob"
                        );
                        classify(&account.email, actual)
                    }
                }
                Ok(None) => VerifyOutcome::Rejected,
                // RateLimited + ServerError + transport errors are all
                // transient. Only definitive RefreshFailed / AuthFailed
                // (mapped to Ok(None) above) should ever produce Rejected.
                Err(_) => VerifyOutcome::NetworkError,
            }
        }
        ProfileCheck::NetworkError => VerifyOutcome::NetworkError,
    };

    store
        .update_verification(uuid, &outcome)
        .map_err(|e| VerifyError::Store(e.to_string()))?;
    Ok(outcome)
}

/// Verify the ACTIVE CLI account against CC's own credential slot.
///
/// Returns `Ok(None)` when CC's slot holds nothing we can work with
/// (empty, or not parseable as a credential blob) — the caller falls
/// back to the private-slot path, which is safe precisely because no
/// competing copy exists in that case.
/// The refresh, when one is needed, is spent on CC's OWN refresh_token
/// and the rotated blob is CAS-written straight back into CC's slot by
/// [`account_service::resolve_cc_identity`] — so CC never ends up
/// holding a token the server has already invalidated. Whatever CC ends
/// up with is then mirrored into the private slot, which also self-heals
/// the mirror-image case where CC rotated on its own and Claudepot's
/// copy went stale.
///
/// A `Drift` outcome deliberately leaves the private slot alone: CC is
/// signed in as somebody else, and copying that blob into this
/// account's slot is exactly the misfiling the verification pass exists
/// to catch.
///
/// When CC's token has already expired AND a `claude` process is live,
/// the refresh is deferred entirely — see [`LiveSessionProbe`].
async fn verify_active_from_cc_slot(
    account: &Account,
    uuid: Uuid,
    platform: &dyn CliPlatform,
    probe: &dyn LiveSessionProbe,
    fetcher: &dyn ProfileFetcher,
    refresher: &dyn TokenRefresher,
) -> Result<Option<VerifyOutcome>, VerifyError> {
    use crate::services::account_service::{self, RegisterError};

    // A rotation is only in play once CC's own token has lapsed. If a
    // `claude` process is live at that moment, refreshing would rotate
    // behind its back — it holds its credentials in memory and writes
    // them back, so it would keep using the refresh_token the server
    // just invalidated. `swap::switch` refuses to touch credentials
    // under a live session for the same reason; defer here too and let
    // CC refresh itself. Costs at most a stale verification row for the
    // few minutes until CC's next request.
    if let Ok(Some(cc_blob)) = platform.read_default().await {
        let expiring = CredentialBlob::from_json(&cc_blob)
            .map(|b| b.is_expired(300))
            .unwrap_or(false);
        if expiring && probe.cc_running().await {
            tracing::info!(
                account = %uuid,
                "CC's token needs a refresh but a claude process is live — deferring to it"
            );
            return Ok(Some(VerifyOutcome::NetworkError));
        }
    }

    let adapter = ProfileFetcherAdapter(fetcher);
    match account_service::resolve_cc_identity(platform, &adapter, refresher).await {
        Ok(Some((blob_str, actual))) => {
            let outcome = classify(&account.email, actual);
            if matches!(outcome, VerifyOutcome::Ok { .. }) {
                let needs_write = match swap::load_private(uuid).await {
                    Ok(current) => current != blob_str,
                    Err(_) => true,
                };
                if needs_write {
                    swap::save_private(uuid, &blob_str).await.map_err(|e| {
                        VerifyError::Store(format!("mirroring CC's blob to the private slot: {e}"))
                    })?;
                }
            } else {
                tracing::warn!(
                    account = %uuid,
                    expected = %account.email,
                    "CC's slot holds another account — NOT mirroring it into this slot"
                );
            }
            Ok(Some(outcome))
        }
        // CC's slot is empty or unparseable — nothing to reconcile
        // against; the caller falls back to the private slot.
        Ok(None) => Ok(None),
        // Both the access token and the refresh token were refused:
        // the user really does have to sign in again.
        Err(RegisterError::AuthRejected) => Ok(Some(VerifyOutcome::Rejected)),
        // Everything else (5xx, transport, a concurrent writer landing
        // a blob mid-flight) is transient — preserve verification
        // history rather than flipping the row to a terminal state.
        Err(e) => {
            tracing::debug!(account = %uuid, "active-account verify could not confirm: {e}");
            Ok(Some(VerifyOutcome::NetworkError))
        }
    }
}

/// Bridges this module's [`ProfileFetcher`] (email-first, what the
/// verification path injects) onto the profile-returning trait
/// `account_service::resolve_cc_identity` expects. Only `.email` is
/// read out of the returned profile, so the default `fetch_profile`
/// degradation in [`ProfileFetcher`] is sufficient here.
struct ProfileFetcherAdapter<'a>(&'a dyn ProfileFetcher);

#[async_trait::async_trait]
impl crate::services::account_service::ProfileFetcher for ProfileFetcherAdapter<'_> {
    async fn fetch(
        &self,
        access_token: &str,
    ) -> Result<crate::oauth::profile::Profile, OAuthError> {
        self.0.fetch_profile(access_token).await
    }
}

/// Convenience wrapper for callers that need both an outcome AND the
/// usable access_token (e.g. `usage_cache` immediately calls /usage with
/// it). Returns the access_token from the slot only when verification
/// succeeded — on Drift / Rejected / NetworkError the caller gets `None`,
/// preventing them from making an API call with a token whose identity
/// just failed to confirm.
pub async fn verify_and_get_access_token(
    store: &AccountStore,
    uuid: Uuid,
    fetcher: &dyn ProfileFetcher,
) -> Result<(VerifyOutcome, Option<String>), VerifyError> {
    let platform = cli_backend::create_platform();
    let outcome = verify_account_identity_with(
        store,
        uuid,
        fetcher,
        &DefaultRefresher,
        platform.as_ref(),
        &ProcessProbe,
    )
    .await?;
    let token = if matches!(outcome, VerifyOutcome::Ok { .. }) {
        // Re-read the slot — it may have been rotated by a refresh inside
        // `verify_account_identity`.
        match swap::load_private(uuid).await {
            Ok(s) => CredentialBlob::from_json(&s)
                .ok()
                .map(|b| b.claude_ai_oauth.access_token),
            Err(_) => None,
        }
    } else {
        None
    };
    Ok((outcome, token))
}

fn classify(stored: &str, actual: String) -> VerifyOutcome {
    if actual.eq_ignore_ascii_case(stored) {
        VerifyOutcome::Ok { email: actual }
    } else {
        VerifyOutcome::Drift {
            stored_email: stored.to_string(),
            actual_email: actual,
        }
    }
}

enum ProfileCheck {
    Ok(String),
    Rejected,
    NetworkError,
}

async fn run_profile_check(blob: &CredentialBlob, fetcher: &dyn ProfileFetcher) -> ProfileCheck {
    match fetcher
        .fetch_email(&blob.claude_ai_oauth.access_token)
        .await
    {
        Ok(email) => ProfileCheck::Ok(email),
        // 401 only — genuine token rejection. Trigger refresh path.
        Err(OAuthError::AuthFailed(_)) => ProfileCheck::Rejected,
        // Everything else (5xx via ServerError, reqwest transport,
        // rate-limit, malformed response) is transient — do NOT refresh,
        // do NOT mark as rejected. NetworkError preserves history.
        Err(_) => ProfileCheck::NetworkError,
    }
}

/// Attempt one `refresh_token` exchange, then re-verify identity on the
/// rotated access_token. Returns `Ok(Some((new_blob_json, actual_email)))`
/// when the refresh itself succeeded and the new profile was fetched — the
/// caller decides whether to persist. `Ok(None)` means the refresh RPC
/// returned a definitive "no" (invalid refresh_token).
async fn try_refresh(
    uuid: Uuid,
    blob: &CredentialBlob,
    fetcher: &dyn ProfileFetcher,
    refresher: &dyn TokenRefresher,
) -> Result<Option<(String, String)>, OAuthError> {
    tracing::info!(account = %uuid, "profile returned 401 — attempting refresh_token exchange");
    let refreshed = match refresher.refresh(&blob.claude_ai_oauth.refresh_token).await {
        Ok(r) => r,
        // Only RefreshFailed (400/401 from token endpoint) is a definitive
        // "no". ServerError + reqwest errors propagate so the caller can
        // map them to NetworkError instead of flipping status to Rejected.
        Err(OAuthError::RefreshFailed(_)) => return Ok(None),
        Err(e) => return Err(e),
    };

    let mut new_blob = blob.clone();
    new_blob.claude_ai_oauth.access_token = refreshed.access_token.clone();
    new_blob.claude_ai_oauth.refresh_token = refreshed.refresh_token;
    new_blob.claude_ai_oauth.expires_at =
        Utc::now().timestamp_millis() + (refreshed.expires_in as i64) * 1000;

    let new_json = new_blob
        .to_json()
        .map_err(|e| OAuthError::RefreshFailed(format!("serialize blob: {e}")))?;

    let actual = match fetcher.fetch_email(&refreshed.access_token).await {
        Ok(e) => e,
        Err(OAuthError::AuthFailed(_)) => return Ok(None),
        Err(e) => return Err(e),
    };

    Ok(Some((new_json, actual)))
}

// See launcher.rs for the rationale: tests hold `lock_data_dir()`
// across `.await` deliberately to serialize the shared data-dir
// env-var across the test binary.
#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use crate::error::OAuthError;
    use std::sync::Mutex;

    /// Mock fetcher: returns a configured email (or error) for any token.
    struct MockFetcher {
        email: Mutex<Option<String>>,
        err: Mutex<Option<OAuthError>>,
    }

    impl MockFetcher {
        fn ok(email: &str) -> Self {
            Self {
                email: Mutex::new(Some(email.to_string())),
                err: Mutex::new(None),
            }
        }
        fn rejecting() -> Self {
            Self {
                email: Mutex::new(None),
                err: Mutex::new(Some(OAuthError::AuthFailed("401".into()))),
            }
        }
        fn network_failing() -> Self {
            Self {
                email: Mutex::new(None),
                err: Mutex::new(Some(OAuthError::RefreshFailed("dns".into()))),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProfileFetcher for MockFetcher {
        async fn fetch_email(&self, _access_token: &str) -> Result<String, OAuthError> {
            if let Some(e) = self.err.lock().unwrap().take() {
                return Err(e);
            }
            Ok(self.email.lock().unwrap().clone().unwrap())
        }
    }

    /// Mock refresher: injectable TokenRefresher for the 401→refresh path.
    /// Returns either a configured `TokenResponse` or an `OAuthError`.
    struct MockRefresher {
        result: Mutex<Option<Result<crate::oauth::refresh::TokenResponse, OAuthError>>>,
    }

    impl MockRefresher {
        fn ok_with(access_token: &str, refresh_token: &str) -> Self {
            Self {
                result: Mutex::new(Some(Ok(crate::oauth::refresh::TokenResponse {
                    access_token: access_token.to_string(),
                    refresh_token: refresh_token.to_string(),
                    expires_in: 3600,
                    scope: None,
                    token_type: None,
                }))),
            }
        }
        fn rejecting() -> Self {
            // RefreshFailed = definitive "no" (token endpoint 400/401).
            Self {
                result: Mutex::new(Some(Err(OAuthError::RefreshFailed("revoked".into())))),
            }
        }
        fn server_erroring() -> Self {
            // ServerError = 5xx on the token endpoint — transient.
            Self {
                result: Mutex::new(Some(Err(OAuthError::ServerError("503".into())))),
            }
        }
    }

    #[async_trait::async_trait]
    impl TokenRefresher for MockRefresher {
        async fn refresh(
            &self,
            _refresh_token: &str,
        ) -> Result<crate::oauth::refresh::TokenResponse, OAuthError> {
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("MockRefresher: second call without reconfigure")
        }
    }

    /// Stand-in for CC's credential slot. Records every write so tests
    /// can assert a rotation actually landed back in CC's slot (the
    /// whole point of the active-account path).
    struct MockPlatform {
        blob: Mutex<Option<String>>,
        writes: Mutex<Vec<String>>,
    }

    impl MockPlatform {
        fn holding(blob: &str) -> Self {
            Self {
                blob: Mutex::new(Some(blob.to_string())),
                writes: Mutex::new(Vec::new()),
            }
        }
        fn empty() -> Self {
            Self {
                blob: Mutex::new(None),
                writes: Mutex::new(Vec::new()),
            }
        }
        fn writes(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, crate::error::SwapError> {
            Ok(self.blob.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), crate::error::SwapError> {
            *self.blob.lock().unwrap() = Some(blob.to_string());
            self.writes.lock().unwrap().push(blob.to_string());
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), crate::error::SwapError> {
            Ok(())
        }
    }

    /// Refresher that records how many exchanges were spent. The
    /// self-heal test asserts this stays at zero: adopting CC's newer
    /// blob must not burn a refresh_token.
    struct CountingRefresher {
        calls: Mutex<u32>,
        access_token: String,
        refresh_token: String,
    }

    impl CountingRefresher {
        fn new(access_token: &str, refresh_token: &str) -> Self {
            Self {
                calls: Mutex::new(0),
                access_token: access_token.to_string(),
                refresh_token: refresh_token.to_string(),
            }
        }
        fn calls(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl TokenRefresher for CountingRefresher {
        async fn refresh(
            &self,
            _refresh_token: &str,
        ) -> Result<crate::oauth::refresh::TokenResponse, OAuthError> {
            *self.calls.lock().unwrap() += 1;
            Ok(crate::oauth::refresh::TokenResponse {
                access_token: self.access_token.clone(),
                refresh_token: self.refresh_token.clone(),
                expires_in: 3600,
                scope: None,
                token_type: None,
            })
        }
    }

    /// No `claude` process running — the default for tests, so results
    /// don't depend on what the developer happens to have open.
    struct IdleProbe;

    #[async_trait::async_trait]
    impl LiveSessionProbe for IdleProbe {
        async fn cc_running(&self) -> bool {
            false
        }
    }

    /// A `claude` process is live.
    struct BusyProbe;

    #[async_trait::async_trait]
    impl LiveSessionProbe for BusyProbe {
        async fn cc_running(&self) -> bool {
            true
        }
    }

    /// Caller MUST already hold the data-dir lock — std::sync::Mutex isn't
    /// reentrant, so taking it a second time on the same thread deadlocks.
    fn setup_account(email: &str) -> (AccountStore, tempfile::TempDir, Uuid) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("t.db");
        let store = AccountStore::open(&db_path).unwrap();
        let acct = crate::testing::make_account(email);
        let uuid = acct.uuid;
        store.insert(&acct).unwrap();
        (store, dir, uuid)
    }

    #[tokio::test]
    async fn test_verify_ok_matches_stored_email() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json())
            .await
            .unwrap();

        let fetcher = MockFetcher::ok("alice@example.com");
        let outcome = verify_account_identity(&store, uuid, &fetcher)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "ok");
        assert_eq!(row.verified_email.as_deref(), Some("alice@example.com"));
        assert!(row.verified_at.is_some());
        swap::delete_private(uuid).await.unwrap();
    }

    #[tokio::test]
    async fn test_verify_drift_surfaces_actual_email_and_status() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json())
            .await
            .unwrap();

        let fetcher = MockFetcher::ok("bob@example.com");
        let outcome = verify_account_identity(&store, uuid, &fetcher)
            .await
            .unwrap();
        assert!(matches!(outcome, VerifyOutcome::Drift { .. }));

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "drift");
        assert_eq!(row.verified_email.as_deref(), Some("bob@example.com"));
        swap::delete_private(uuid).await.unwrap();
    }

    /// 401 on /profile + definitive RefreshFailed on refresh → Rejected.
    #[tokio::test]
    async fn test_verify_rejected_when_token_refused_and_refresh_definitively_fails() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json())
            .await
            .unwrap();

        let fetcher = MockFetcher::rejecting();
        let refresher = MockRefresher::rejecting();
        let outcome = verify_account_identity_with(
            &store,
            uuid,
            &fetcher,
            &refresher,
            &MockPlatform::empty(),
            &IdleProbe,
        )
        .await
        .unwrap();
        assert_eq!(outcome, VerifyOutcome::Rejected);

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "rejected");
        swap::delete_private(uuid).await.unwrap();
    }

    /// 401 on /profile + ServerError on refresh (5xx) → NetworkError, NOT
    /// Rejected. Preserves history so a blip doesn't forget verified_email.
    #[tokio::test]
    async fn test_verify_refresh_server_error_is_transient_not_rejected() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json())
            .await
            .unwrap();
        store
            .update_verification(
                uuid,
                &VerifyOutcome::Ok {
                    email: "alice@example.com".into(),
                },
            )
            .unwrap();

        let fetcher = MockFetcher::rejecting();
        let refresher = MockRefresher::server_erroring();
        let outcome = verify_account_identity_with(
            &store,
            uuid,
            &fetcher,
            &refresher,
            &MockPlatform::empty(),
            &IdleProbe,
        )
        .await
        .unwrap();
        assert_eq!(outcome, VerifyOutcome::NetworkError);

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "network_error");
        // Previous verified_email must survive the transient error.
        assert_eq!(row.verified_email.as_deref(), Some("alice@example.com"));
        swap::delete_private(uuid).await.unwrap();
    }

    /// 401 → refresh succeeds → new profile matches stored email → the
    /// rotated blob MUST be written to the slot and the row marked Ok.
    #[tokio::test]
    async fn test_verify_refresh_success_persists_rotated_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        let original_blob = crate::testing::fresh_blob_json();
        swap::save_private(uuid, &original_blob).await.unwrap();

        // /profile rejects first, then the post-refresh call returns the
        // expected email.
        let fetcher = MockFetcher {
            email: Mutex::new(Some("alice@example.com".into())),
            err: Mutex::new(Some(OAuthError::AuthFailed("401".into()))),
        };
        let refresher = MockRefresher::ok_with("sk-ant-oat01-new", "sk-ant-ort01-new");

        let outcome = verify_account_identity_with(
            &store,
            uuid,
            &fetcher,
            &refresher,
            &MockPlatform::empty(),
            &IdleProbe,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );

        // Slot must hold the rotated blob now — NOT the original.
        let stored = swap::load_private(uuid).await.unwrap();
        assert_ne!(stored, original_blob, "slot must hold the refreshed blob");
        assert!(stored.contains("sk-ant-oat01-new"));
        swap::delete_private(uuid).await.unwrap();
    }

    /// 401 → refresh succeeds → new profile returns DIFFERENT email →
    /// Drift. The rotated blob must NOT be written (that would entrench
    /// the misfiling with a fresh-but-wrong token — exactly the bug
    /// that motivated this module).
    #[tokio::test]
    async fn test_verify_drift_after_refresh_does_not_persist() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        let original_blob = crate::testing::fresh_blob_json();
        swap::save_private(uuid, &original_blob).await.unwrap();

        let fetcher = MockFetcher {
            // 1st (pre-refresh) call: 401. 2nd (post-refresh) call: bob.
            email: Mutex::new(Some("bob@example.com".into())),
            err: Mutex::new(Some(OAuthError::AuthFailed("401".into()))),
        };
        let refresher = MockRefresher::ok_with("new-access", "new-refresh");

        let outcome = verify_account_identity_with(
            &store,
            uuid,
            &fetcher,
            &refresher,
            &MockPlatform::empty(),
            &IdleProbe,
        )
        .await
        .unwrap();
        assert!(matches!(outcome, VerifyOutcome::Drift { .. }));

        // Critical: the rotated blob must NOT have been written.
        let stored = swap::load_private(uuid).await.unwrap();
        assert_eq!(
            stored, original_blob,
            "drift must not persist the rotated blob"
        );
        swap::delete_private(uuid).await.unwrap();
    }

    #[tokio::test]
    async fn test_verify_network_error_preserves_prior_verified_email() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json())
            .await
            .unwrap();

        // Seed a prior successful verification.
        store
            .update_verification(
                uuid,
                &VerifyOutcome::Ok {
                    email: "alice@example.com".into(),
                },
            )
            .unwrap();

        let fetcher = MockFetcher::network_failing();
        let outcome = verify_account_identity(&store, uuid, &fetcher)
            .await
            .unwrap();
        assert_eq!(outcome, VerifyOutcome::NetworkError);

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "network_error");
        // Prior verified_email must survive the blip.
        assert_eq!(row.verified_email.as_deref(), Some("alice@example.com"));
        swap::delete_private(uuid).await.unwrap();
    }

    #[tokio::test]
    async fn test_verify_missing_blob_returns_error() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");

        let fetcher = MockFetcher::ok("alice@example.com");
        let result = verify_account_identity(&store, uuid, &fetcher).await;
        assert!(matches!(result, Err(VerifyError::NoBlob)));
    }

    // -- Active-account path: CC's slot owns the live token family --

    /// The regression this whole path exists for. The active account's
    /// access token has expired, so /profile 401s and a refresh is
    /// spent. The rotated blob MUST land back in CC's slot — refreshing
    /// only Claudepot's private copy leaves CC holding a refresh_token
    /// the server just invalidated, which surfaces to the user as a
    /// forced `/login` every time the 8-hour access token lapses.
    #[tokio::test]
    async fn active_account_refresh_writes_rotated_blob_back_to_cc_slot() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");

        // Both copies start identical and expired — the state right
        // after a swap, eight hours on.
        let expired = crate::testing::expired_blob_json();
        swap::save_private(uuid, &expired).await.unwrap();
        store.set_active_cli(uuid).unwrap();
        let platform = MockPlatform::holding(&expired);

        // /profile rejects the stale access token, then accepts the
        // rotated one.
        let fetcher = MockFetcher {
            email: Mutex::new(Some("alice@example.com".into())),
            err: Mutex::new(Some(OAuthError::AuthFailed("401".into()))),
        };
        let refresher = CountingRefresher::new("sk-ant-oat01-rotated", "sk-ant-ort01-rotated");

        let outcome =
            verify_account_identity_with(&store, uuid, &fetcher, &refresher, &platform, &IdleProbe)
                .await
                .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );
        assert_eq!(refresher.calls(), 1, "exactly one refresh should be spent");

        let writes = platform.writes();
        assert_eq!(writes.len(), 1, "CC's slot must receive the rotated blob");
        assert!(
            writes[0].contains("sk-ant-ort01-rotated"),
            "CC's slot must hold the rotated refresh_token, not the dead one"
        );

        // And Claudepot's private copy must track it, so the two stay
        // one token family.
        let private = swap::load_private(uuid).await.unwrap();
        assert!(
            private.contains("sk-ant-ort01-rotated"),
            "private slot must mirror the rotated blob"
        );
        swap::delete_private(uuid).await.unwrap();
    }

    /// The mirror-image failure: CC refreshed on its own and Claudepot's
    /// private copy went stale. Verification must adopt CC's newer blob
    /// rather than spend the dead refresh_token it still holds — which
    /// would otherwise mark a perfectly healthy account `rejected`.
    #[tokio::test]
    async fn active_account_adopts_cc_rotation_without_spending_refresh_token() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");

        let stale = crate::testing::expired_blob_json();
        swap::save_private(uuid, &stale).await.unwrap();
        store.set_active_cli(uuid).unwrap();

        // CC holds a fresh blob it minted itself.
        let cc_blob = crate::testing::fresh_blob_json().replace("oat01-test", "oat01-cc-rotated");
        let platform = MockPlatform::holding(&cc_blob);

        let fetcher = MockFetcher::ok("alice@example.com");
        let refresher = CountingRefresher::new("unused", "unused");

        let outcome =
            verify_account_identity_with(&store, uuid, &fetcher, &refresher, &platform, &IdleProbe)
                .await
                .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );
        assert_eq!(
            refresher.calls(),
            0,
            "CC's blob is live — no refresh_token should be spent"
        );
        assert!(
            platform.writes().is_empty(),
            "nothing to write back when CC's blob already verifies"
        );

        let private = swap::load_private(uuid).await.unwrap();
        assert_eq!(private, cc_blob, "private slot must adopt CC's blob");
        swap::delete_private(uuid).await.unwrap();
    }

    /// Non-active accounts have no competing copy in CC's slot, so they
    /// keep the private-slot path — and must never write to CC's slot.
    #[tokio::test]
    async fn inactive_account_refresh_never_touches_cc_slot() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::expired_blob_json())
            .await
            .unwrap();
        // Deliberately NOT set_active_cli.

        let other = crate::testing::fresh_blob_json();
        let platform = MockPlatform::holding(&other);
        let fetcher = MockFetcher {
            email: Mutex::new(Some("alice@example.com".into())),
            err: Mutex::new(Some(OAuthError::AuthFailed("401".into()))),
        };
        let refresher = CountingRefresher::new("sk-ant-oat01-new", "sk-ant-ort01-new");

        let outcome =
            verify_account_identity_with(&store, uuid, &fetcher, &refresher, &platform, &IdleProbe)
                .await
                .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );
        assert!(
            platform.writes().is_empty(),
            "an inactive account must never write CC's slot"
        );
        let private = swap::load_private(uuid).await.unwrap();
        assert!(private.contains("sk-ant-ort01-new"));
        swap::delete_private(uuid).await.unwrap();
    }

    /// A live `claude` process holds its credentials in memory and
    /// writes them back, so rotating CC's expired token behind its back
    /// hands it a refresh_token the server has already invalidated —
    /// the same hazard `swap::switch` refuses to take. Defer instead
    /// and let CC refresh itself.
    #[tokio::test]
    async fn active_account_defers_refresh_while_a_claude_process_is_live() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");

        let expired = crate::testing::expired_blob_json();
        swap::save_private(uuid, &expired).await.unwrap();
        store.set_active_cli(uuid).unwrap();
        // Seed a prior success so we can prove history survives.
        store
            .update_verification(
                uuid,
                &VerifyOutcome::Ok {
                    email: "alice@example.com".into(),
                },
            )
            .unwrap();

        let platform = MockPlatform::holding(&expired);
        let fetcher = MockFetcher::rejecting();
        let refresher = CountingRefresher::new("sk-ant-oat01-rotated", "sk-ant-ort01-rotated");

        let outcome =
            verify_account_identity_with(&store, uuid, &fetcher, &refresher, &platform, &BusyProbe)
                .await
                .unwrap();
        assert_eq!(outcome, VerifyOutcome::NetworkError);
        assert_eq!(
            refresher.calls(),
            0,
            "must not rotate behind a live claude process"
        );
        assert!(platform.writes().is_empty(), "CC's slot must be left alone");

        // Transient outcome — prior verification history survives.
        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verified_email.as_deref(), Some("alice@example.com"));
        swap::delete_private(uuid).await.unwrap();
    }

    /// The gate is scoped to the expired window: a live `claude` process
    /// must not block plain verification of a token that is still good.
    #[tokio::test]
    async fn live_claude_process_does_not_block_verifying_a_live_token() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");

        let fresh = crate::testing::fresh_blob_json();
        swap::save_private(uuid, &fresh).await.unwrap();
        store.set_active_cli(uuid).unwrap();

        let platform = MockPlatform::holding(&fresh);
        let fetcher = MockFetcher::ok("alice@example.com");
        let refresher = CountingRefresher::new("unused", "unused");

        let outcome =
            verify_account_identity_with(&store, uuid, &fetcher, &refresher, &platform, &BusyProbe)
                .await
                .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );
        swap::delete_private(uuid).await.unwrap();
    }

    /// Active account, but CC's slot is empty — no competing copy, so
    /// the private-slot path is safe and must still run.
    #[tokio::test]
    async fn active_account_with_empty_cc_slot_falls_back_to_private_slot() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json())
            .await
            .unwrap();
        store.set_active_cli(uuid).unwrap();

        let platform = MockPlatform::empty();
        let fetcher = MockFetcher::ok("alice@example.com");
        let refresher = CountingRefresher::new("unused", "unused");

        let outcome =
            verify_account_identity_with(&store, uuid, &fetcher, &refresher, &platform, &IdleProbe)
                .await
                .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );
        swap::delete_private(uuid).await.unwrap();
    }
}
