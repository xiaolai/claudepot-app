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
//!
//! ## The active CLI account reads CC's keychain, not the private slot
//!
//! For every account EXCEPT the one CC is currently signed in as, the
//! per-account private slot is authoritative — CC never touches those
//! tokens, so refreshing a slot copy is safe. The *active* account is
//! different: CC holds its live token in the keychain and rotates it on
//! its own schedule (single-use refresh tokens). Verifying the active
//! account off the private slot would either (a) present an
//! already-rotated token and falsely report `Rejected`, or (b) rotate a
//! token CC still holds, orphaning CC's session and forcing a real
//! re-login. So for the active account we delegate to the keychain-aware
//! resolver (`account_service::resolve_cc_identity`), which reads CC's
//! keychain, re-checks on 401 before spending a refresh token, and writes
//! any rotation back to the keychain (healing CC, not orphaning it).
//!
//! ## We never spend a refresh token while `claude` is running
//!
//! Re-checking on 401 closes the window where CC rotates *first*, but not
//! the one where **we** rotate first: CC caches its refresh token in
//! memory and writes it back, so a rotation behind its back leaves it
//! holding a token the server has retired — its next refresh fails and
//! the user must re-login. So the resolver refuses to spend the token
//! whenever a live `claude` process is detected, returning
//! `RegisterError::CcLiveRefreshSkipped`, which maps to
//! `VerifyOutcome::NetworkError` here. This mirrors the gate
//! `swap::switch` already applies. Skipping is self-healing: CC refreshes
//! on its next request and the following pass verifies normally.
use crate::account::{AccountStore, VerifyOutcome};
use crate::blob::CredentialBlob;
use crate::cli_backend::swap;
use crate::cli_backend::swap::{DefaultRefresher, ProfileFetcher, TokenRefresher};
use crate::cli_backend::CliPlatform;
use crate::error::OAuthError;
use crate::services::account_service;
use chrono::Utc;
use uuid::Uuid;

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
    let platform = crate::cli_backend::create_platform();
    verify_account_identity_with_probe(
        store,
        uuid,
        platform.as_ref(),
        fetcher,
        &DefaultRefresher,
        &crate::cli_backend::swap::DefaultLiveSessionProbe,
    )
    .await
}

/// Testable variant: inject a [`TokenRefresher`] so the 401→refresh
/// branch can be exercised without real HTTP.
///
/// Uses [`NoLiveSessionProbe`], so the refresh branch stays
/// deterministically reachable regardless of what is running on the test
/// machine. **Production must call [`verify_account_identity`]**, which
/// supplies the real probe — refreshing the active account while a live
/// `claude` runs rotates a token CC still holds and forces a re-login.
///
/// [`NoLiveSessionProbe`]: crate::cli_backend::swap::NoLiveSessionProbe
pub async fn verify_account_identity_with(
    store: &AccountStore,
    uuid: Uuid,
    platform: &dyn CliPlatform,
    fetcher: &dyn ProfileFetcher,
    refresher: &dyn TokenRefresher,
) -> Result<VerifyOutcome, VerifyError> {
    verify_account_identity_with_probe(
        store,
        uuid,
        platform,
        fetcher,
        refresher,
        &crate::cli_backend::swap::NoLiveSessionProbe,
    )
    .await
}

/// [`verify_account_identity_with`] with an injectable live-session
/// probe, which gates whether the active account's single-use refresh
/// token may be spent.
pub async fn verify_account_identity_with_probe(
    store: &AccountStore,
    uuid: Uuid,
    platform: &dyn CliPlatform,
    fetcher: &dyn ProfileFetcher,
    refresher: &dyn TokenRefresher,
    probe: &dyn crate::cli_backend::swap::LiveSessionProbe,
) -> Result<VerifyOutcome, VerifyError> {
    let account = store
        .find_by_uuid(uuid)
        .map_err(|e| VerifyError::Store(e.to_string()))?
        .ok_or(VerifyError::AccountNotFound(uuid))?;

    // Active CLI account: verify against CC's LIVE keychain and heal a
    // rotated token in place, rather than rotating the stale private-slot
    // copy (which would orphan CC's session → forced re-login). Only when
    // CC's keychain holds no usable blob do we fall through to the
    // private-slot check below (there is nothing live to protect then).
    if active_cli_matches(store, uuid)? {
        if let Some(outcome) =
            verify_active_via_keychain(&account.email, platform, fetcher, refresher, probe).await
        {
            // TOCTOU guard: a concurrent swap may have changed the active
            // account during the keychain round-trip above (verify holds no
            // swap lock). If `uuid` is no longer active, the outcome we just
            // computed describes CC's keychain — which now belongs to a
            // DIFFERENT account — so it can't be persisted against `uuid`.
            // Downgrade to NetworkError ("couldn't confirm this pass"); the
            // next tick re-runs with a stable view. Any keychain heal the
            // resolver performed was CAS-guarded and remains valid regardless.
            let outcome = if active_cli_matches(store, uuid)? {
                outcome
            } else {
                tracing::info!(
                    account = %uuid,
                    "active account changed during keychain verify — recording NetworkError instead of a cross-account outcome"
                );
                VerifyOutcome::NetworkError
            };
            store
                .update_verification(uuid, &outcome)
                .map_err(|e| VerifyError::Store(e.to_string()))?;
            return Ok(outcome);
        }
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

/// True when `uuid` is the account CC is currently signed in as (the
/// active CLI slot).
///
/// Fails CLOSED, not open. Both an unreadable pointer (DB error) and a
/// present-but-unparseable pointer surface as `VerifyError::Store` rather
/// than being treated as "not active". Silently returning `false` on a
/// corrupt pointer would route the account that is *actually* active down
/// the private-slot path — reintroducing the rotation kill this module
/// exists to prevent, on the one account where it matters, precisely when
/// state is already corrupt. A no-active-account pointer (`None`) is the
/// only benign "not active" case.
fn active_cli_matches(store: &AccountStore, uuid: Uuid) -> Result<bool, VerifyError> {
    match store
        .active_cli_uuid()
        .map_err(|e| VerifyError::Store(e.to_string()))?
    {
        None => Ok(false),
        Some(raw) => {
            let active = Uuid::parse_str(&raw).map_err(|e| {
                VerifyError::Store(format!(
                    "active_cli pointer is not a valid uuid ({raw:?}): {e}"
                ))
            })?;
            Ok(active == uuid)
        }
    }
}

/// Verify the ACTIVE CLI account against CC's live keychain via the
/// hardened, single-use-rotation-safe resolver. Returns:
/// - `Some(outcome)` — a definitive verification outcome to persist.
/// - `None` — CC's keychain holds no usable blob (empty / unparseable);
///   the caller falls back to the private-slot check.
///
/// The resolver reads CC's keychain, re-checks `/profile` on a 401 before
/// spending a refresh token, and CAS-writes any rotation back to the
/// keychain — so a refresh HEALS CC's session instead of orphaning it. It
/// never touches the private slot, so the "don't entrench a misfiling in
/// the slot" invariant holds trivially for the drift case.
async fn verify_active_via_keychain(
    stored_email: &str,
    platform: &dyn CliPlatform,
    fetcher: &dyn ProfileFetcher,
    refresher: &dyn TokenRefresher,
    probe: &dyn crate::cli_backend::swap::LiveSessionProbe,
) -> Option<VerifyOutcome> {
    use account_service::RegisterError as RE;
    let adapter = KeychainFetcher(fetcher);
    match account_service::resolve_cc_identity(platform, &adapter, refresher, probe).await {
        Ok(Some((_blob, cc_email))) => Some(classify(stored_email, cc_email)),
        // CC has no credentials / unparseable blob — nothing live to
        // verify against. Defer to the private-slot path.
        Ok(None) => None,
        // The ONE terminal outcome: access token AND refresh token both
        // refused → the user must re-login.
        Err(RE::AuthRejected) => Some(VerifyOutcome::Rejected),
        // Transient — never flip to Rejected on a blip; NetworkError
        // preserves verified_email history. These are the variants
        // resolve_cc_identity actually emits on a non-terminal failure.
        Err(e @ (RE::ProfileFetch(_) | RE::CredentialRead(_) | RE::CredentialWrite(_))) => {
            tracing::debug!("active-account verify: transient resolver error: {e}");
            Some(VerifyOutcome::NetworkError)
        }
        Err(e @ RE::CcChangedDuringRefresh) => {
            tracing::debug!("active-account verify: keychain changed mid-refresh: {e}");
            Some(VerifyOutcome::NetworkError)
        }
        // A live `claude` is running, so the resolver declined to spend
        // the single-use refresh token. Transient by construction: CC
        // refreshes its own token on its next request and the following
        // pass verifies normally. Reporting anything terminal here would
        // resurrect the very re-login prompt the gate prevents;
        // NetworkError also preserves `verified_email` history so the UI
        // shows "last verified N ago" rather than a scary unknown.
        Err(e @ RE::CcLiveRefreshSkipped) => {
            tracing::debug!("active-account verify: {e}");
            Some(VerifyOutcome::NetworkError)
        }
        // resolve_cc_identity does not emit these today. They are listed
        // EXPLICITLY (no `_` wildcard) on purpose: adding a new variant to
        // RegisterError makes THIS match non-exhaustive and breaks the
        // build here, forcing a deliberate terminal-vs-transient decision
        // rather than silently defaulting a future terminal variant to
        // NetworkError. For now the conservative mapping is NetworkError
        // (never a *false* re-login), logged loudly since reaching here is
        // unexpected.
        Err(e @ (RE::NoCredentials | RE::AlreadyRegistered(..) | RE::Store(_) | RE::NotFound)) => {
            tracing::warn!(
                "active-account verify: unexpected resolver error mapped to NetworkError: {e}"
            );
            Some(VerifyOutcome::NetworkError)
        }
    }
}

/// Adapts a [`swap::ProfileFetcher`] (the `fetch_email` / `fetch_profile`
/// trait threaded through this module) to the
/// [`account_service::ProfileFetcher`] (`fetch`) that
/// `resolve_cc_identity` expects. Both return a `profile::Profile`; this
/// is a pure trait bridge so the same injected fetcher (real or mock)
/// drives both verification paths.
struct KeychainFetcher<'a>(&'a dyn ProfileFetcher);

#[async_trait::async_trait]
impl account_service::ProfileFetcher for KeychainFetcher<'_> {
    async fn fetch(
        &self,
        access_token: &str,
    ) -> Result<crate::oauth::profile::Profile, OAuthError> {
        self.0.fetch_profile(access_token).await
    }
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

    /// Mock CC keychain: `read_default` returns the configured blob;
    /// `write_default` records the write AND updates the stored blob so
    /// the resolver's CAS re-read sees exactly what it just wrote.
    struct MockKeychain {
        blob: Mutex<Option<String>>,
        writes: Mutex<Vec<String>>,
    }

    impl MockKeychain {
        fn with(blob: Option<&str>) -> Self {
            Self {
                blob: Mutex::new(blob.map(|s| s.to_string())),
                writes: Mutex::new(Vec::new()),
            }
        }
        fn empty() -> Self {
            Self::with(None)
        }
        fn writes(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl crate::cli_backend::CliPlatform for MockKeychain {
        async fn read_default(&self) -> Result<Option<String>, crate::cli_backend::SwapError> {
            Ok(self.blob.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), crate::cli_backend::SwapError> {
            self.writes.lock().unwrap().push(blob.to_string());
            *self.blob.lock().unwrap() = Some(blob.to_string());
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), crate::cli_backend::SwapError> {
            Ok(())
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
        let platform = MockKeychain::empty();
        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
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
        let platform = MockKeychain::empty();
        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
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
        let platform = MockKeychain::empty();

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
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
        let platform = MockKeychain::empty();

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
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

    // ---- Active CLI account: verify against CC's keychain, not the slot ----

    /// The active account is verified against CC's live keychain even when
    /// its private slot is EMPTY. Under the old private-slot-only path this
    /// returned `NoBlob`; now the keychain is the source of truth, so a
    /// valid live token verifies `Ok`. Proves the read source switched.
    #[tokio::test]
    async fn active_account_verifies_against_keychain_when_slot_empty() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();
        // No private slot saved on purpose — the old path would NoBlob here.

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        let fetcher = MockFetcher::ok("alice@example.com");
        let refresher = MockRefresher::ok_with("unused", "unused");

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );
        // Valid token → no refresh → keychain untouched.
        assert!(platform.writes().is_empty());
    }

    /// THE mode-B fix: when CC's live access token is expired, the refresh
    /// is written back to the KEYCHAIN (healing CC's session), NOT rotated
    /// into the private slot while CC keeps the dead token. Proves the
    /// rotation lands where CC will read it.
    #[tokio::test]
    async fn active_account_expired_token_heals_keychain_in_place() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();
        // A stale private slot must be left untouched by the active path.
        let stale_slot = crate::testing::expired_blob_json();
        swap::save_private(uuid, &stale_slot).await.unwrap();

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        // /profile 401 first (token rejected), then the post-refresh call
        // returns the matching email.
        let fetcher = MockFetcher {
            email: Mutex::new(Some("alice@example.com".into())),
            err: Mutex::new(Some(OAuthError::AuthFailed("401".into()))),
        };
        let refresher = MockRefresher::ok_with("sk-ant-oat01-healed", "sk-ant-ort01-healed");

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );

        // The rotated token was written to CC's keychain — CC stays alive.
        let writes = platform.writes();
        assert_eq!(writes.len(), 1, "exactly one keychain heal write");
        assert!(
            writes[0].contains("sk-ant-oat01-healed"),
            "keychain must hold the rotated access token"
        );
        // The private slot was NOT the thing we rotated.
        let slot_after = swap::load_private(uuid).await.unwrap();
        assert_eq!(
            slot_after, stale_slot,
            "active path must not touch the slot"
        );
        swap::delete_private(uuid).await.unwrap();
    }

    /// Drift is reported from CC's ACTUAL keychain identity. No refresh on
    /// the happy path, so the keychain is not written (nothing to entrench).
    #[tokio::test]
    async fn active_account_drift_reported_from_keychain_identity() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        let fetcher = MockFetcher::ok("bob@example.com");
        let refresher = MockRefresher::ok_with("unused", "unused");

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert!(matches!(outcome, VerifyOutcome::Drift { .. }));

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "drift");
        assert_eq!(row.verified_email.as_deref(), Some("bob@example.com"));
        assert!(platform.writes().is_empty());
    }

    /// Active account, keychain token rejected AND refresh definitively
    /// refused → `Rejected` (genuine re-login), same terminal semantics as
    /// the private-slot path.
    #[tokio::test]
    async fn active_account_rejected_when_keychain_token_and_refresh_fail() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        let fetcher = MockFetcher::rejecting();
        let refresher = MockRefresher::rejecting();

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(outcome, VerifyOutcome::Rejected);
    }

    /// Active account, keychain token rejected but refresh hits a 5xx →
    /// `NetworkError`, NOT `Rejected`. A transient blip must never push the
    /// user to re-login.
    #[tokio::test]
    async fn active_account_transient_refresh_error_is_network_not_rejected() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        let fetcher = MockFetcher::rejecting();
        let refresher = MockRefresher::server_erroring();

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(outcome, VerifyOutcome::NetworkError);
    }

    /// Probe reporting a live `claude` process.
    struct LiveProbe;

    #[async_trait::async_trait]
    impl crate::cli_backend::swap::LiveSessionProbe for LiveProbe {
        async fn is_cc_running(&self) -> bool {
            true
        }
    }

    /// Refresher that fails the test if it is ever called. Proves the
    /// live-session gate declined to spend the single-use refresh token
    /// rather than merely discarding the result afterwards.
    struct NeverRefresher;

    #[async_trait::async_trait]
    impl TokenRefresher for NeverRefresher {
        async fn refresh(
            &self,
            _refresh_token: &str,
        ) -> Result<crate::oauth::refresh::TokenResponse, OAuthError> {
            panic!("refresh must not be attempted while a live claude session is running");
        }
    }

    /// THE re-login fix. A live `claude` process is running and the active
    /// account's access token is rejected. We must NOT spend the single-use
    /// refresh token: CC keeps its copy in memory and writes it back, so
    /// rotating here retires the token CC still holds and its next refresh
    /// fails — forcing exactly the re-login this gate exists to prevent.
    ///
    /// Contract: refresher never called, keychain never written, outcome is
    /// transient, and prior `verified_email` history survives.
    #[tokio::test]
    async fn active_account_live_cc_session_does_not_spend_refresh_token() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();
        // Seed a prior good verification so we can prove history survives.
        store
            .update_verification(
                uuid,
                &VerifyOutcome::Ok {
                    email: "alice@example.com".into(),
                },
            )
            .unwrap();

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        let fetcher = MockFetcher::rejecting(); // /profile → 401

        let outcome = verify_account_identity_with_probe(
            &store,
            uuid,
            &platform,
            &fetcher,
            &NeverRefresher,
            &LiveProbe,
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            VerifyOutcome::NetworkError,
            "a live-session skip is transient, never Rejected — a Rejected \
             here is the re-login prompt we are preventing"
        );
        assert!(
            platform.writes().is_empty(),
            "keychain must not be rotated while a claude session is live"
        );

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "network_error");
        assert_eq!(
            row.verified_email.as_deref(),
            Some("alice@example.com"),
            "transient skip must preserve last-known-good identity"
        );
    }

    /// The gate is conditional, not a blanket ban: with NO live session the
    /// same 401 still refreshes and heals the keychain. Pairs with the test
    /// above so a regression that disables refreshing entirely is caught.
    #[tokio::test]
    async fn active_account_without_live_cc_session_still_heals() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        let fetcher = MockFetcher {
            email: Mutex::new(Some("alice@example.com".into())),
            err: Mutex::new(Some(OAuthError::AuthFailed("401".into()))),
        };
        let refresher = MockRefresher::ok_with("sk-ant-oat01-healed", "sk-ant-ort01-healed");

        let outcome = verify_account_identity_with_probe(
            &store,
            uuid,
            &platform,
            &fetcher,
            &refresher,
            &crate::cli_backend::swap::NoLiveSessionProbe,
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            VerifyOutcome::Ok {
                email: "alice@example.com".into()
            }
        );
        let writes = platform.writes();
        assert_eq!(writes.len(), 1, "no live session → heal proceeds");
        assert!(writes[0].contains("sk-ant-oat01-healed"));
    }

    /// Active account but CC's keychain is EMPTY — nothing live to protect,
    /// so we fall back to the private-slot check and verify that blob.
    #[tokio::test]
    async fn active_account_falls_back_to_slot_when_keychain_empty() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();
        swap::save_private(uuid, &crate::testing::fresh_blob_json())
            .await
            .unwrap();

        let platform = MockKeychain::empty(); // CC has no credentials
        let fetcher = MockFetcher::ok("alice@example.com");
        let refresher = MockRefresher::ok_with("unused", "unused");

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
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

    /// F2 regression: a corrupt `active_cli` pointer must FAIL CLOSED —
    /// verify errors rather than silently routing the (possibly
    /// truly-active) account down the unsafe private-slot path.
    #[tokio::test]
    async fn malformed_active_pointer_fails_closed() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json())
            .await
            .unwrap();
        // set_active_cli only accepts a real Uuid — inject garbage directly
        // into the state table to simulate DB corruption / hand-editing.
        store
            .db()
            .execute(
                "INSERT OR REPLACE INTO state (key, value) VALUES ('active_cli', 'not-a-uuid')",
                [],
            )
            .unwrap();

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        let fetcher = MockFetcher::ok("alice@example.com");
        let refresher = MockRefresher::ok_with("unused", "unused");

        let result =
            verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher).await;
        assert!(
            matches!(result, Err(VerifyError::Store(_))),
            "expected Store error on malformed active pointer, got {result:?}"
        );
        swap::delete_private(uuid).await.unwrap();
    }

    /// A [`ProfileFetcher`] that clears the active-CLI pointer the first
    /// time it is called — simulating a concurrent swap landing DURING the
    /// keychain verify round-trip.
    struct ActiveFlippingFetcher {
        store2: AccountStore,
        email: String,
    }

    #[async_trait::async_trait]
    impl ProfileFetcher for ActiveFlippingFetcher {
        async fn fetch_email(&self, _access_token: &str) -> Result<String, OAuthError> {
            self.store2.clear_active_cli().unwrap();
            Ok(self.email.clone())
        }
    }

    /// F1 regression: if a concurrent swap changes the active account while
    /// the keychain verify is in flight, the computed outcome (about CC's
    /// keychain, now a DIFFERENT account) must NOT be persisted against this
    /// uuid — it downgrades to NetworkError.
    #[tokio::test]
    async fn active_change_mid_verify_downgrades_to_network_error() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, dir, uuid) = setup_account("alice@example.com");
        store.set_active_cli(uuid).unwrap();
        // A second connection on the same DB, handed to the fetcher so it
        // can flip the active pointer mid-call.
        let store2 = AccountStore::open(&dir.path().join("t.db")).unwrap();

        let platform = MockKeychain::with(Some(&crate::testing::fresh_blob_json()));
        let fetcher = ActiveFlippingFetcher {
            store2,
            email: "alice@example.com".into(),
        };
        let refresher = MockRefresher::ok_with("unused", "unused");

        let outcome = verify_account_identity_with(&store, uuid, &platform, &fetcher, &refresher)
            .await
            .unwrap();
        // Keychain said "alice = Ok", but active flipped away mid-verify, so
        // we must not persist that as this account's truth — the returned
        // outcome AND the persisted row must both be NetworkError.
        assert_eq!(outcome, VerifyOutcome::NetworkError);
        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "network_error");
    }
}
