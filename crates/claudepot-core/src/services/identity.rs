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
use crate::account::{AccountStore, VerifyOutcome};
use crate::blob::CredentialBlob;
use crate::cli_backend::swap;
use crate::cli_backend::swap::ProfileFetcher;
use crate::error::OAuthError;
use crate::oauth::refresh;
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
    let account = store
        .find_by_uuid(uuid)
        .map_err(|e| VerifyError::Store(e.to_string()))?
        .ok_or(VerifyError::AccountNotFound(uuid))?;

    let blob_str = match swap::load_private(uuid) {
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
            match try_refresh(uuid, &blob, fetcher).await {
                Ok(Some((new_blob_json, actual))) => {
                    let drift = !actual.eq_ignore_ascii_case(&account.email);
                    if !drift {
                        // Safe to persist — label and server agree.
                        let _ = swap::save_private(uuid, &new_blob_json);
                    } else {
                        tracing::warn!(
                            account = %uuid,
                            expected = %account.email,
                            actual = %actual,
                            "drift detected after refresh — NOT persisting rotated blob"
                        );
                    }
                    classify(&account.email, actual)
                }
                Ok(None) => VerifyOutcome::Rejected,
                Err(OAuthError::RateLimited { .. }) => VerifyOutcome::NetworkError,
                Err(_) => VerifyOutcome::Rejected,
            }
        }
        ProfileCheck::NetworkError => VerifyOutcome::NetworkError,
    };

    store
        .update_verification(uuid, &outcome)
        .map_err(|e| VerifyError::Store(e.to_string()))?;
    Ok(outcome)
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
    let outcome = verify_account_identity(store, uuid, fetcher).await?;
    let token = if matches!(outcome, VerifyOutcome::Ok { .. }) {
        // Re-read the slot — it may have been rotated by a refresh inside
        // `verify_account_identity`.
        match swap::load_private(uuid) {
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
        Err(OAuthError::AuthFailed(_)) => ProfileCheck::Rejected,
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
) -> Result<Option<(String, String)>, OAuthError> {
    tracing::info!(account = %uuid, "profile returned 401 — attempting refresh_token exchange");
    let refreshed = match refresh::refresh(&blob.claude_ai_oauth.refresh_token).await {
        Ok(r) => r,
        Err(OAuthError::RefreshFailed(_)) | Err(OAuthError::AuthFailed(_)) => return Ok(None),
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

#[cfg(test)]
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
        swap::save_private(uuid, &crate::testing::fresh_blob_json()).unwrap();

        let fetcher = MockFetcher::ok("alice@example.com");
        let outcome = verify_account_identity(&store, uuid, &fetcher).await.unwrap();
        assert_eq!(outcome, VerifyOutcome::Ok {
            email: "alice@example.com".into()
        });

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "ok");
        assert_eq!(row.verified_email.as_deref(), Some("alice@example.com"));
        assert!(row.verified_at.is_some());
        swap::delete_private(uuid).unwrap();
    }

    #[tokio::test]
    async fn test_verify_drift_surfaces_actual_email_and_status() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json()).unwrap();

        let fetcher = MockFetcher::ok("bob@example.com");
        let outcome = verify_account_identity(&store, uuid, &fetcher).await.unwrap();
        assert!(matches!(outcome, VerifyOutcome::Drift { .. }));

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "drift");
        assert_eq!(row.verified_email.as_deref(), Some("bob@example.com"));
        swap::delete_private(uuid).unwrap();
    }

    // Hits the real Anthropic refresh endpoint via `oauth::refresh::refresh`
    // on the 401 → try-refresh branch. Injecting a mock refresher into
    // this module is a follow-up refactor (parallel to ProfileFetcher);
    // the three non-ignored tests already cover every branch that doesn't
    // require network mocking.
    #[tokio::test]
    #[ignore]
    async fn test_verify_rejected_when_token_refused_and_refresh_fails() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json()).unwrap();

        // First call 401s; refresh also fails (network path isn't mockable
        // here, so we rely on the default fetcher returning 401 again).
        let fetcher = MockFetcher::rejecting();
        let outcome = verify_account_identity(&store, uuid, &fetcher).await.unwrap();
        // Refresh will hit the real endpoint with the test refresh_token and
        // either fail (likely) or succeed. We accept either Rejected or
        // NetworkError — both preserve verified_email (doesn't matter here
        // since we never had one), and both mark status non-"ok".
        assert!(
            matches!(outcome, VerifyOutcome::Rejected | VerifyOutcome::NetworkError),
            "expected Rejected or NetworkError, got {outcome:?}"
        );

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_ne!(row.verify_status, "ok");
        swap::delete_private(uuid).unwrap();
    }

    #[tokio::test]
    async fn test_verify_network_error_preserves_prior_verified_email() {
        let _lock = crate::testing::lock_data_dir();
        let _env = crate::testing::setup_test_data_dir();
        let (store, _dir, uuid) = setup_account("alice@example.com");
        swap::save_private(uuid, &crate::testing::fresh_blob_json()).unwrap();

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
        let outcome = verify_account_identity(&store, uuid, &fetcher).await.unwrap();
        assert_eq!(outcome, VerifyOutcome::NetworkError);

        let row = store.find_by_uuid(uuid).unwrap().unwrap();
        assert_eq!(row.verify_status, "network_error");
        // Prior verified_email must survive the blip.
        assert_eq!(row.verified_email.as_deref(), Some("alice@example.com"));
        swap::delete_private(uuid).unwrap();
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
}
