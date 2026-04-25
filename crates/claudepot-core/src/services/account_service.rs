//! Account registration, removal, and inspection — core business logic.
//! CLI and Tauri both call these functions.

use crate::account::{Account, AccountStore};
use crate::blob::CredentialBlob;
use crate::cli_backend;
use crate::cli_backend::swap;
use crate::error::OAuthError;
use crate::oauth::{profile, refresh, usage};
use crate::paths;
use chrono::Utc;
use uuid::Uuid;

/// Result of registering a new account.
#[derive(Debug)]
pub struct RegisterResult {
    pub uuid: Uuid,
    pub email: String,
    pub org_name: String,
    pub subscription_type: String,
    pub rate_limit_tier: Option<String>,
}

/// Register an account by importing the current CC credentials.
pub async fn register_from_current(store: &AccountStore) -> Result<RegisterResult, RegisterError> {
    let platform = cli_backend::create_platform();
    register_from_current_with(store, platform.as_ref(), &DefaultProfileFetcher).await
}

/// Testable variant: accepts injectable platform and profile fetcher.
pub(crate) async fn register_from_current_with(
    store: &AccountStore,
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
) -> Result<RegisterResult, RegisterError> {
    tracing::info!("registering from current CC credentials");
    let blob_str = platform
        .read_default()
        .await
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?
        .ok_or(RegisterError::NoCredentials)?;

    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?;

    let prof = fetch_profile
        .fetch(&blob.claude_ai_oauth.access_token)
        .await
        .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?;

    register_account_from_profile(store, &blob_str, &prof)
}

/// Trait for fetching an OAuth profile — enables testing without network.
#[async_trait::async_trait]
pub(crate) trait ProfileFetcher: Send + Sync {
    async fn fetch(&self, access_token: &str)
        -> Result<profile::Profile, crate::error::OAuthError>;
}

/// Production implementation that calls the Anthropic API.
struct DefaultProfileFetcher;

#[async_trait::async_trait]
impl ProfileFetcher for DefaultProfileFetcher {
    async fn fetch(
        &self,
        access_token: &str,
    ) -> Result<profile::Profile, crate::error::OAuthError> {
        profile::fetch(access_token).await
    }
}

/// Shared logic: given a credential blob string and a fetched profile,
/// save credentials and insert the account.
fn register_account_from_profile(
    store: &AccountStore,
    blob_str: &str,
    prof: &profile::Profile,
) -> Result<RegisterResult, RegisterError> {
    if let Some(existing) = store
        .find_by_email(&prof.email)
        .map_err(|e| RegisterError::Store(e.to_string()))?
    {
        return Err(RegisterError::AlreadyRegistered(
            existing.email,
            existing.uuid,
        ));
    }

    let account_id = Uuid::new_v4();
    swap::save_private(account_id, blob_str)
        .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid.clone()),
        org_name: Some(prof.org_name.clone()),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
        // Profile fetch just succeeded — seed verification state so the
        // account starts its lifecycle already verified instead of "never".
        verified_email: Some(prof.email.clone()),
        verified_at: Some(Utc::now()),
        verify_status: "ok".to_string(),
    };
    if let Err(e) = store.insert(&account) {
        // Rollback: delete orphaned private blob
        let _ = swap::delete_private(account_id);
        return Err(RegisterError::Store(e.to_string()));
    }

    Ok(RegisterResult {
        uuid: account_id,
        email: prof.email.clone(),
        org_name: prof.org_name.clone(),
        subscription_type: prof.subscription_type.clone(),
        rate_limit_tier: prof.rate_limit_tier.clone(),
    })
}

/// Register an account from a refresh token (headless).
pub async fn register_from_token(
    store: &AccountStore,
    refresh_token: &str,
) -> Result<RegisterResult, RegisterError> {
    use crate::cli_backend::swap::DefaultRefresher;
    register_from_token_with(
        store,
        refresh_token,
        &DefaultRefresher,
        &DefaultProfileFetcher,
    )
    .await
}

/// Testable variant: accepts injectable refresher and profile fetcher.
pub(crate) async fn register_from_token_with(
    store: &AccountStore,
    refresh_token: &str,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
    fetch_profile: &dyn ProfileFetcher,
) -> Result<RegisterResult, RegisterError> {
    use crate::oauth::refresh;

    let token_resp = refresher
        .refresh(refresh_token)
        .await
        .map_err(|e| RegisterError::ProfileFetch(format!("token exchange failed: {e}")))?;

    let prof = fetch_profile
        .fetch(&token_resp.access_token)
        .await
        .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?;

    let blob_str = refresh::build_blob(&token_resp, None);
    register_account_from_profile(store, &blob_str, &prof)
}

/// Register an account via browser-based OAuth login.
/// Runs `claude auth login` in a temp config dir, reads credentials,
/// fetches profile, and registers the account.
///
/// Non-cancellable convenience wrapper. Callers that want to wire a
/// Cancel button should call [`register_from_browser_cancellable`]
/// with an `Arc<Notify>` shared with the UI.
pub async fn register_from_browser(store: &AccountStore) -> Result<RegisterResult, RegisterError> {
    register_from_browser_cancellable(store, None).await
}

/// Cancellable variant of [`register_from_browser`]. When `cancel`
/// fires, the in-flight `claude auth login` subprocess is killed and
/// the temp config dir is dropped, producing
/// `RegisterError::CredentialRead` with the onboarding layer's
/// "cancelled" message.
pub async fn register_from_browser_cancellable(
    store: &AccountStore,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<RegisterResult, RegisterError> {
    register_from_browser_with_progress(store, cancel, &NoopLoginSink).await
}

/// Sync Claudepot state with whatever account CC is currently signed in
/// as. Idempotent; designed to run on GUI startup so users who logged in
/// externally (or in a previous session) don't see stale "missing" badges.
///
/// Returns `Ok(Some(uuid))` if a sync happened, `Ok(None)` if there was
/// nothing to adopt (CC empty, or its blob matches no registered email).
/// Errors bubble up, but callers typically log-and-continue — except
/// [`RegisterError::AuthRejected`], which the UI surfaces as an
/// actionable "sign in again" banner.
pub async fn sync_from_current_cc(store: &AccountStore) -> Result<Option<Uuid>, RegisterError> {
    let platform = cli_backend::create_platform();
    sync_from_current_cc_with(
        store,
        platform.as_ref(),
        &DefaultProfileFetcher,
        &crate::cli_backend::swap::DefaultRefresher,
    )
    .await
}

/// Resolve CC's current identity from its keychain, healing stale
/// access tokens when a paired refresh_token still works. Returns
/// `Ok(None)` if CC has no credentials or the stored blob is
/// unparseable — both are benign "nothing to sync" outcomes.
///
/// ## Race handling
///
/// A naive implementation (snapshot blob once, 401 → refresh with that
/// snapshot's refresh_token) races against CC's own refresh loop.
/// Token refresh is single-use: whichever party calls `/token` second
/// with the same refresh_token is rejected. The loser sees the
/// rejection as `AuthRejected` and asks the user to re-login even
/// though CC itself is perfectly healthy.
///
/// To close that window:
///
/// 1. `/profile` with the access_token from the initial snapshot.
/// 2. On 401, re-read the keychain. If the blob changed, retry
///    `/profile` with the fresh access_token first — the common case
///    is CC having rotated between our read and our call. Don't
///    consume a refresh_token for this.
/// 3. If the retry also 401s (or the blob hasn't changed), refresh
///    using the LATEST refresh_token we have evidence of. Only a
///    definitive `RefreshFailed` from the token endpoint is terminal
///    (`AuthRejected`); everything else is transient (`ProfileFetch`).
///
/// The rotated blob is written back to CC's keychain so CC's next
/// keychain read sees the fresh token. The write is guarded by a
/// best-effort CAS against the keychain: if another writer landed
/// something newer between our refresh and our write, we yield
/// rather than clobber.
async fn resolve_cc_identity(
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
) -> Result<Option<(String, String)>, RegisterError> {
    let blob_str_t0 = match platform
        .read_default()
        .await
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?
    {
        Some(b) => b,
        None => return Ok(None), // CC has no credentials — nothing to sync.
    };

    let blob_t0 = match CredentialBlob::from_json(&blob_str_t0) {
        Ok(b) => b,
        Err(_) => return Ok(None), // Unparseable — leave it alone.
    };

    // Step 1 — happy path. Most calls end here.
    match fetch_profile
        .fetch(&blob_t0.claude_ai_oauth.access_token)
        .await
    {
        Ok(prof) => return Ok(Some((blob_str_t0, prof.email))),
        Err(OAuthError::AuthFailed(_)) => {}
        Err(e) => return Err(RegisterError::ProfileFetch(e.to_string())),
    }

    // Step 2 — race check. The blob we snapshot at the start may have
    // been superseded by a concurrent writer (CC's own refresh loop,
    // or another Claudepot instance) by the time our /profile call
    // returned 401. Re-read and, if it changed, retry with the fresh
    // access_token before spending a refresh_token.
    let latest_blob_str = match platform
        .read_default()
        .await
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?
    {
        Some(b) => b,
        None => return Ok(None),
    };
    let latest_blob = match CredentialBlob::from_json(&latest_blob_str) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };

    if latest_blob_str != blob_str_t0 {
        match fetch_profile
            .fetch(&latest_blob.claude_ai_oauth.access_token)
            .await
        {
            Ok(prof) => return Ok(Some((latest_blob_str, prof.email))),
            Err(OAuthError::AuthFailed(_)) => {}
            Err(e) => return Err(RegisterError::ProfileFetch(e.to_string())),
        }
    }

    // Step 3 — refresh. Use the LATEST refresh_token, not the one we
    // originally snapshot. If a concurrent writer already rotated, our
    // original refresh_token is dead server-side and calling refresh
    // with it would produce a false AuthRejected.
    let token_resp = match refresher
        .refresh(&latest_blob.claude_ai_oauth.refresh_token)
        .await
    {
        Ok(tr) => tr,
        // Refresh endpoint definitively rejected the refresh_token →
        // terminal. The user has to sign in again; no amount of
        // retrying will help.
        Err(OAuthError::RefreshFailed(_)) => {
            return Err(RegisterError::AuthRejected);
        }
        // Rate-limiting, 5xx, transport — transient. Preserve the old
        // sync behavior (caller logs + moves on) rather than locking
        // the user out of the UI.
        Err(e) => {
            return Err(RegisterError::ProfileFetch(format!(
                "token refresh: {e}"
            )));
        }
    };
    let new_blob_str = refresh::build_blob(&token_resp, Some(&latest_blob));

    // Write the rotated blob back to CC's keychain so CC itself sees
    // the fresh token on its next run. Guarded by a best-effort CAS:
    // if another writer landed a different blob while we were
    // refreshing, skip the write so we don't regress their state —
    // but in that case we MUST also abandon `new_blob_str` for the
    // returned identity. Persisting a blob CC never kept would mis-
    // file the rotated token into the wrong slot. Instead, re-read
    // the live blob, re-verify identity against it, and return the
    // live pair.
    let pre_write = platform
        .read_default()
        .await
        .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;
    if pre_write.as_deref() == Some(latest_blob_str.as_str()) {
        platform
            .write_default(&new_blob_str)
            .await
            .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;

        // Retry `/profile` with the new access token. A failure here is
        // transient (we just proved refresh works, so this isn't an auth
        // issue) — map to ProfileFetch for the best-effort log-and-
        // continue path.
        let new_email = fetch_profile
            .fetch(&token_resp.access_token)
            .await
            .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?
            .email;
        return Ok(Some((new_blob_str, new_email)));
    }

    // CAS miss: another writer landed a newer blob between our refresh
    // and our writeback. The `new_blob_str` we minted is now an
    // orphan — CC discarded it before it ever ran. Surface CC's
    // *current* state instead. Re-read, parse, and re-verify so
    // callers persist what CC actually holds, never our orphan.
    let live_blob_str = match pre_write {
        Some(b) => b,
        None => {
            // CC cleared its credentials mid-refresh. Nothing to sync.
            return Err(RegisterError::CcChangedDuringRefresh);
        }
    };
    let live_blob = CredentialBlob::from_json(&live_blob_str)
        .map_err(|_| RegisterError::CcChangedDuringRefresh)?;
    let live_email = match fetch_profile
        .fetch(&live_blob.claude_ai_oauth.access_token)
        .await
    {
        Ok(prof) => prof.email,
        // The live blob's access token is no longer valid (or some
        // other error). We can't safely identify CC's current state —
        // bail rather than persist either blob.
        Err(_) => return Err(RegisterError::CcChangedDuringRefresh),
    };
    Ok(Some((live_blob_str, live_email)))
}

pub(crate) async fn sync_from_current_cc_with(
    store: &AccountStore,
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
) -> Result<Option<Uuid>, RegisterError> {
    let (effective_blob_str, email) =
        match resolve_cc_identity(platform, fetch_profile, refresher).await? {
            Some(pair) => pair,
            None => return Ok(None),
        };

    let account = match store
        .find_by_email(&email)
        .map_err(|e| RegisterError::Store(e.to_string()))?
    {
        Some(a) => a,
        None => return Ok(None), // Unknown account — user hasn't added it yet.
    };

    // Defensive: `find_by_email` returns an account whose email matches by
    // construction, but make the invariant explicit at the callsite. If
    // anyone ever changes the lookup to be a prefix or fuzzy match, this
    // assertion fires before we mis-file CC's blob into the wrong slot.
    debug_assert!(
        account.email.eq_ignore_ascii_case(&email),
        "find_by_email returned account whose email doesn't match query"
    );

    // Write if the stored blob differs from or is missing vs. CC's current.
    let needs_write = match swap::load_private(account.uuid) {
        Ok(stored) => stored != effective_blob_str,
        Err(_) => true,
    };
    if needs_write {
        swap::save_private(account.uuid, &effective_blob_str)
            .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;
        let _ = store.update_credentials_flag(account.uuid, true);
    }

    // Profile fetch just succeeded with a matching email — record the
    // verification outcome so the account starts the session verified.
    let _ = store.update_verification(
        account.uuid,
        &crate::account::VerifyOutcome::Ok {
            email: email.clone(),
        },
    );

    // Always sync the active pointer so downstream swaps see the truth.
    if let Err(e) = store.set_active_cli(account.uuid) {
        tracing::warn!(
            "sync_from_current_cc: set_active_cli({}) failed: {e}",
            account.uuid
        );
    }

    Ok(Some(account.uuid))
}

/// Launch `claude auth login` (which opens the browser), wait for the
/// user to complete OAuth, then import CC's resulting blob into the
/// EXISTING account's slot. The full "Log in" UX.
///
/// Identity check: the blob's profile email must match `account_id`'s
/// stored email. If the user authenticates as a different account,
/// the import is rejected — they'd otherwise be overwriting the wrong
/// slot (the mis-filing corruption we otherwise guard against).
pub async fn login_and_reimport(
    store: &AccountStore,
    account_id: Uuid,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), RegisterError> {
    login_and_reimport_with_progress(store, account_id, cancel, &NoopLoginSink).await
}

/// Re-import credentials into an EXISTING account's slot from the blob
/// CC is currently holding. One-click recovery when Claudepot's stored
/// blob is missing/corrupt but the user has logged back into CC as the
/// matching account.
///
/// Verifies identity first: fetches the profile for CC's current token and
/// refuses the re-import if the email doesn't match the stored account's
/// email. That prevents re-creating the mis-filed-blob corruption the
/// identity-verified swap was designed to catch.
pub async fn reimport_from_current(
    store: &AccountStore,
    account_id: Uuid,
) -> Result<(), RegisterError> {
    let platform = cli_backend::create_platform();
    reimport_from_current_with(store, account_id, platform.as_ref(), &DefaultProfileFetcher).await
}

/// Testable variant: accepts injectable platform and profile fetcher.
pub(crate) async fn reimport_from_current_with(
    store: &AccountStore,
    account_id: Uuid,
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
) -> Result<(), RegisterError> {
    let account = store
        .find_by_uuid(account_id)
        .map_err(|e| RegisterError::Store(e.to_string()))?
        .ok_or(RegisterError::NotFound)?;

    let blob_str = platform
        .read_default()
        .await
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?
        .ok_or(RegisterError::NoCredentials)?;

    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?;

    let prof = fetch_profile
        .fetch(&blob.claude_ai_oauth.access_token)
        .await
        .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?;

    if !prof.email.eq_ignore_ascii_case(&account.email) {
        return Err(RegisterError::ProfileFetch(format!(
            "CC is currently signed in as {}, not {}. Log into CC as {} first.",
            prof.email, account.email, account.email
        )));
    }

    swap::save_private(account_id, &blob_str)
        .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;

    // Sync the flag — storage is now populated.
    let _ = store.update_credentials_flag(account_id, true);

    // Profile fetch just confirmed label == blob identity. Persist that
    // so the DB row reflects reality; otherwise a prior Drift/Rejected/
    // NetworkError state would linger until the next verify pass even
    // though re-login has already fixed things.
    let _ = store.update_verification(
        account_id,
        &crate::account::VerifyOutcome::Ok {
            email: prof.email.clone(),
        },
    );

    // Align Claudepot's active_cli with CC's reality: CC is now holding
    // this account's blob (that's the premise of re-import), so this
    // account IS the active CLI. Without this sync a subsequent swap
    // would see drift on the outgoing-blob check.
    if let Err(e) = store.set_active_cli(account_id) {
        tracing::warn!("post-reimport failed to sync active_cli to {account_id}: {e}");
    }

    Ok(())
}

/// Remove an account and all its associated data.
/// Collects non-fatal warnings instead of silently swallowing errors.
pub async fn remove_account(
    store: &AccountStore,
    uuid: Uuid,
    cache: Option<&super::usage_cache::UsageCache>,
) -> Result<RemoveResult, RegisterError> {
    let account = store
        .find_by_uuid(uuid)
        .map_err(|e| RegisterError::Store(e.to_string()))?
        .ok_or(RegisterError::NotFound)?;

    let mut warnings: Vec<String> = Vec::new();

    let profile_dir = paths::desktop_profile_dir(uuid);
    let had_profile = profile_dir.exists();

    // Clear active pointers first (reversible DB operations)
    if account.is_cli_active {
        if let Err(e) = store.clear_active_cli() {
            warnings.push(format!("failed to clear active CLI pointer: {e}"));
        }
    }
    if account.is_desktop_active {
        if let Err(e) = store.clear_active_desktop() {
            warnings.push(format!("failed to clear active Desktop pointer: {e}"));
        }
    }

    // Remove from DB before irreversible file deletions
    store
        .remove(uuid)
        .map_err(|e| RegisterError::Store(e.to_string()))?;

    // Now safe to delete files — DB row is already gone
    if let Err(e) = swap::delete_private(uuid) {
        warnings.push(format!("failed to delete credential file: {e}"));
    }
    if had_profile {
        if let Err(e) = std::fs::remove_dir_all(&profile_dir) {
            warnings.push(format!("failed to delete desktop profile: {e}"));
        }
    }

    // Invalidate usage cache so stale data doesn't linger.
    if let Some(c) = cache {
        c.invalidate(uuid).await;
    }

    Ok(RemoveResult {
        email: account.email,
        was_cli_active: account.is_cli_active,
        was_desktop_active: account.is_desktop_active,
        had_desktop_profile: had_profile,
        warnings,
    })
}

#[derive(Debug)]
pub struct RemoveResult {
    pub email: String,
    pub was_cli_active: bool,
    pub was_desktop_active: bool,
    pub had_desktop_profile: bool,
    pub warnings: Vec<String>,
}

/// Token health info for an account.
#[derive(Debug)]
pub struct TokenHealth {
    pub status: String,
    pub remaining_mins: Option<i64>,
}

/// Format a duration in minutes as "Xd Xh Xm", dropping zero-leading
/// units. Examples: 88 → "1h 28m", 45 → "45m", 1500 → "1d 1h",
/// 60 → "1h", 0 → "0m".
fn format_duration_mins(mins: i64) -> String {
    if mins <= 0 {
        return "0m".to_string();
    }
    let days = mins / (24 * 60);
    let hours = (mins % (24 * 60)) / 60;
    let minutes = mins % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    // Show minutes if no larger unit, or if they're non-zero and we're
    // not showing days (avoid "1d 0h 5m" — collapse to "1d 5m" feels off,
    // so we just drop minutes when days are present).
    if minutes > 0 && days == 0 {
        parts.push(format!("{minutes}m"));
    }
    if parts.is_empty() {
        // e.g., exactly 1 day with no remainder
        parts.push(format!("{days}d"));
    }
    parts.join(" ")
}

/// Get token health for an account.
pub fn token_health(uuid: Uuid, has_credentials: bool) -> TokenHealth {
    if !has_credentials {
        return TokenHealth {
            status: "no credentials".into(),
            remaining_mins: None,
        };
    }
    match swap::load_private(uuid) {
        Ok(blob_str) => match CredentialBlob::from_json(&blob_str) {
            Ok(blob) => {
                let remaining =
                    (blob.claude_ai_oauth.expires_at - Utc::now().timestamp_millis()) / 60_000;
                if remaining > 0 {
                    TokenHealth {
                        status: format!("valid ({} remaining)", format_duration_mins(remaining)),
                        remaining_mins: Some(remaining),
                    }
                } else {
                    TokenHealth {
                        status: "expired".into(),
                        remaining_mins: Some(remaining),
                    }
                }
            }
            Err(_) => TokenHealth {
                status: "corrupt blob".into(),
                remaining_mins: None,
            },
        },
        Err(_) => TokenHealth {
            status: "missing".into(),
            remaining_mins: None,
        },
    }
}

/// Fetch live usage for an account through the cache layer.
pub async fn fetch_usage(
    cache: &super::usage_cache::UsageCache,
    uuid: Uuid,
    force: bool,
) -> Result<Option<usage::UsageResponse>, super::usage_cache::UsageFetchError> {
    cache.fetch_usage(uuid, force).await
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("no CC credentials found — log in with `claude auth login` first")]
    NoCredentials,
    #[error("failed to read credentials: {0}")]
    CredentialRead(String),
    #[error("failed to write credentials: {0}")]
    CredentialWrite(String),
    #[error("profile fetch failed: {0}")]
    ProfileFetch(String),
    #[error("already registered: {0} (uuid: {1})")]
    AlreadyRegistered(String, Uuid),
    #[error("store error: {0}")]
    Store(String),
    #[error("account not found")]
    NotFound,
    /// CC's stored access token was rejected AND the paired refresh token
    /// failed to mint a new one — the user must re-login to recover.
    /// Distinct from `ProfileFetch` so callers (UI banner, CLI exit code)
    /// can surface an actionable "sign in again" state instead of a vague
    /// transient warning. Triggered by `sync_from_current_cc` when both
    /// `/profile` returns 401 and `/v1/oauth/token` refuses the refresh.
    #[error("CC's stored login is no longer valid — sign in again to Claude Code")]
    AuthRejected,
    /// CC's keychain blob changed (or cleared) between our refresh and
    /// the writeback CAS, so the rotated blob we minted was never
    /// installed and we couldn't re-verify the live blob's identity.
    /// Callers should retry `sync_from_current_cc` rather than persist
    /// stale state.
    #[error("CC credentials changed during refresh — retry sync")]
    CcChangedDuringRefresh,
}

// ---------------------------------------------------------------------------
// Login progress — phase events for the multi-step browser-OAuth flow.
//
// `register_from_browser` and `login_and_reimport` move through six discrete
// stages, the slowest of which (`WaitingForBrowser`) can take minutes. The
// Tauri layer wants to surface that progress on `op-progress::<op_id>`
// channels; the CLI wants the same primitives for telemetry. The progress
// sink trait is the seam — implementations live in their respective surfaces.
// ---------------------------------------------------------------------------

/// Discrete steps in the browser-OAuth login pipeline. Surfaces in the GUI
/// as a phase-by-phase progress modal; the CLI logs each phase entry so a
/// hung subprocess shows up in the journal.
///
/// `Spawning` and `WaitingForBrowser` straddle the `claude auth login`
/// subprocess: `Spawning` fires before the child boots, `WaitingForBrowser`
/// fires once the temp config dir has been seeded and we are blocked on
/// the user finishing OAuth in the browser. The remaining four phases run
/// after the subprocess exits — fast, but worth surfacing so the modal does
/// not appear to freeze on `WaitingForBrowser` while the post-processing
/// runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginPhase {
    /// Pre-flight: validating account, preparing to spawn `claude auth login`.
    Spawning,
    /// Subprocess running, waiting for the user to complete OAuth.
    WaitingForBrowser,
    /// Reading the credential blob from the temp config dir / keychain.
    ReadingBlob,
    /// Calling `/api/oauth/profile` with the fresh access token.
    FetchingProfile,
    /// Comparing returned profile email against the stored account label.
    VerifyingIdentity,
    /// Writing the blob to the per-account slot + updating DB rows.
    Persisting,
}

/// Progress sink for the login pipeline. Implementations forward
/// `LoginPhase` events to whichever surface needs them — Tauri emits
/// `op-progress::<op_id>` events; tests record into a vector; CLI logs.
pub trait LoginProgressSink: Send + Sync {
    /// Called when a phase transitions to running. Idempotent from the
    /// caller's perspective — implementations should treat repeats as
    /// "still running" rather than asserting on order.
    fn phase(&self, phase: LoginPhase);
    /// Called when a phase transitions to error. Carries the human
    /// message so the UI can render it inline. After `error`, the
    /// pipeline returns; no further phases will fire.
    fn error(&self, phase: LoginPhase, msg: &str);
}

/// No-op sink — used by the legacy non-progress-aware shims so the CLI
/// doesn't pay for events it ignores.
pub struct NoopLoginSink;

impl LoginProgressSink for NoopLoginSink {
    fn phase(&self, _phase: LoginPhase) {}
    fn error(&self, _phase: LoginPhase, _msg: &str) {}
}

/// Progress-emitting variant of [`login_and_reimport`].
///
/// Phases (in order):
/// 1. `Spawning` — emitted before validating the account / launching the
///    subprocess.
/// 2. `WaitingForBrowser` — emitted as soon as the subprocess is alive.
/// 3. `ReadingBlob` — emitted after the subprocess returns, before we
///    pull credentials from the now-shared CC keychain.
/// 4. `FetchingProfile` — emitted before `/profile`.
/// 5. `VerifyingIdentity` — emitted before the email-vs-label check.
/// 6. `Persisting` — emitted before `swap::save_private` + DB update.
///
/// Errors fire the matching `error(phase, msg)` and stop the pipeline.
pub async fn login_and_reimport_with_progress(
    store: &AccountStore,
    account_id: Uuid,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
    progress: &dyn LoginProgressSink,
) -> Result<(), RegisterError> {
    use crate::onboard;

    progress.phase(LoginPhase::Spawning);

    // Validate the account exists before spending minutes in the browser.
    let account = match store.find_by_uuid(account_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            progress.error(LoginPhase::Spawning, "account not found");
            return Err(RegisterError::NotFound);
        }
        Err(e) => {
            let msg = e.to_string();
            progress.error(LoginPhase::Spawning, &msg);
            return Err(RegisterError::Store(msg));
        }
    };

    tracing::info!(
        email = %account.email,
        "launching `claude auth login` for re-authentication"
    );

    progress.phase(LoginPhase::WaitingForBrowser);
    if let Err(e) = onboard::run_auth_login_in_place_cancellable(cancel).await {
        let msg = e.to_string();
        progress.error(LoginPhase::WaitingForBrowser, &msg);
        return Err(RegisterError::CredentialRead(msg));
    }
    finish_login_after_subprocess(store, account_id, &account.email, progress).await
}

/// Test-only seam: same as [`login_and_reimport_with_progress`] but
/// accepts an explicit `claude` binary path so the integration test
/// can point at a controllable stub. Not re-exported publicly.
#[cfg(test)]
pub(crate) async fn login_and_reimport_with_progress_test_binary(
    store: &AccountStore,
    account_id: Uuid,
    claude_binary: &std::path::Path,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
    progress: &dyn LoginProgressSink,
) -> Result<(), RegisterError> {
    use crate::onboard;

    progress.phase(LoginPhase::Spawning);

    let account = match store.find_by_uuid(account_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            progress.error(LoginPhase::Spawning, "account not found");
            return Err(RegisterError::NotFound);
        }
        Err(e) => {
            let msg = e.to_string();
            progress.error(LoginPhase::Spawning, &msg);
            return Err(RegisterError::Store(msg));
        }
    };

    progress.phase(LoginPhase::WaitingForBrowser);
    if let Err(e) =
        onboard::run_auth_login_in_place_cancellable_with_binary(claude_binary, cancel).await
    {
        let msg = e.to_string();
        progress.error(LoginPhase::WaitingForBrowser, &msg);
        return Err(RegisterError::CredentialRead(msg));
    }
    finish_login_after_subprocess(store, account_id, &account.email, progress).await
}

/// Shared post-subprocess pipeline: read blob, fetch profile, verify
/// identity, persist. Extracted so the test seam above and the
/// production entry point share one body.
async fn finish_login_after_subprocess(
    store: &AccountStore,
    account_id: Uuid,
    expected_email: &str,
    progress: &dyn LoginProgressSink,
) -> Result<(), RegisterError> {

    // After success, CC holds fresh credentials. Mirror the post-spawn
    // pipeline of `reimport_from_current_with` but emit per-step progress
    // so the GUI can show progress through the (fast) tail.
    let platform = cli_backend::create_platform();
    let fetch_profile = DefaultProfileFetcher;

    progress.phase(LoginPhase::ReadingBlob);
    let blob_str = match platform.read_default().await {
        Ok(Some(b)) => b,
        Ok(None) => {
            let msg = "no CC credentials found after login";
            progress.error(LoginPhase::ReadingBlob, msg);
            return Err(RegisterError::NoCredentials);
        }
        Err(e) => {
            let msg = e.to_string();
            progress.error(LoginPhase::ReadingBlob, &msg);
            return Err(RegisterError::CredentialRead(msg));
        }
    };

    let blob = match CredentialBlob::from_json(&blob_str) {
        Ok(b) => b,
        Err(e) => {
            let msg = e.to_string();
            progress.error(LoginPhase::ReadingBlob, &msg);
            return Err(RegisterError::CredentialRead(msg));
        }
    };

    progress.phase(LoginPhase::FetchingProfile);
    let prof = match fetch_profile.fetch(&blob.claude_ai_oauth.access_token).await {
        Ok(p) => p,
        Err(e) => {
            let msg = e.to_string();
            progress.error(LoginPhase::FetchingProfile, &msg);
            return Err(RegisterError::ProfileFetch(msg));
        }
    };

    progress.phase(LoginPhase::VerifyingIdentity);
    if !prof.email.eq_ignore_ascii_case(expected_email) {
        let msg = format!(
            "CC is currently signed in as {}, not {}. Log into CC as {} first.",
            prof.email, expected_email, expected_email
        );
        progress.error(LoginPhase::VerifyingIdentity, &msg);
        return Err(RegisterError::ProfileFetch(msg));
    }

    progress.phase(LoginPhase::Persisting);
    if let Err(e) = swap::save_private(account_id, &blob_str) {
        let msg = e.to_string();
        progress.error(LoginPhase::Persisting, &msg);
        return Err(RegisterError::CredentialWrite(msg));
    }

    // Sync the flag — storage is now populated.
    let _ = store.update_credentials_flag(account_id, true);
    let _ = store.update_verification(
        account_id,
        &crate::account::VerifyOutcome::Ok {
            email: prof.email.clone(),
        },
    );

    // Align Claudepot's active_cli with CC's reality.
    if let Err(e) = store.set_active_cli(account_id) {
        tracing::warn!("post-login failed to sync active_cli to {account_id}: {e}");
    }

    Ok(())
}

/// Progress-emitting variant of [`register_from_browser_cancellable`].
/// Phases mirror [`login_and_reimport_with_progress`]; the difference is
/// that this path creates a fresh account rather than re-importing into
/// an existing slot.
pub async fn register_from_browser_with_progress(
    store: &AccountStore,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
    progress: &dyn LoginProgressSink,
) -> Result<RegisterResult, RegisterError> {
    use crate::onboard;

    progress.phase(LoginPhase::Spawning);

    progress.phase(LoginPhase::WaitingForBrowser);
    let config_dir = match onboard::run_auth_login_cancellable(cancel).await {
        Ok(d) => d,
        Err(e) => {
            let msg = e.to_string();
            progress.error(LoginPhase::WaitingForBrowser, &msg);
            return Err(RegisterError::CredentialRead(msg));
        }
    };

    progress.phase(LoginPhase::ReadingBlob);
    let blob_str = match onboard::read_credentials_from_dir(&config_dir).await {
        Ok(b) => b,
        Err(e) => {
            let msg = e.to_string();
            onboard::cleanup(&config_dir).await;
            progress.error(LoginPhase::ReadingBlob, &msg);
            return Err(RegisterError::CredentialRead(msg));
        }
    };

    let blob = match CredentialBlob::from_json(&blob_str) {
        Ok(b) => b,
        Err(e) => {
            let msg = e.to_string();
            // Fire-and-forget cleanup — don't propagate cleanup errors
            let cd = config_dir.clone();
            tokio::spawn(async move { onboard::cleanup(&cd).await });
            progress.error(LoginPhase::ReadingBlob, &msg);
            return Err(RegisterError::CredentialRead(msg));
        }
    };

    progress.phase(LoginPhase::FetchingProfile);
    let prof = match profile::fetch(&blob.claude_ai_oauth.access_token).await {
        Ok(p) => p,
        Err(e) => {
            let msg = e.to_string();
            onboard::cleanup(&config_dir).await;
            progress.error(LoginPhase::FetchingProfile, &msg);
            return Err(RegisterError::ProfileFetch(msg));
        }
    };

    progress.phase(LoginPhase::VerifyingIdentity);
    match store.find_by_email(&prof.email) {
        Ok(Some(existing)) => {
            onboard::cleanup(&config_dir).await;
            let msg = format!("already registered: {} (uuid: {})", existing.email, existing.uuid);
            progress.error(LoginPhase::VerifyingIdentity, &msg);
            return Err(RegisterError::AlreadyRegistered(existing.email, existing.uuid));
        }
        Ok(None) => {}
        Err(e) => {
            let msg = e.to_string();
            onboard::cleanup(&config_dir).await;
            progress.error(LoginPhase::VerifyingIdentity, &msg);
            return Err(RegisterError::Store(msg));
        }
    }

    progress.phase(LoginPhase::Persisting);
    let account_id = Uuid::new_v4();
    if let Err(e) = swap::save_private(account_id, &blob_str) {
        let msg = e.to_string();
        // Cleanup on credential write failure
        let cd = config_dir.clone();
        tokio::spawn(async move { onboard::cleanup(&cd).await });
        progress.error(LoginPhase::Persisting, &msg);
        return Err(RegisterError::CredentialWrite(msg));
    }

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid.clone()),
        org_name: Some(prof.org_name.clone()),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
        verified_email: Some(prof.email.clone()),
        verified_at: Some(Utc::now()),
        verify_status: "ok".to_string(),
    };
    if let Err(e) = store.insert(&account) {
        let msg = e.to_string();
        // Rollback: delete orphaned private blob + cleanup temp dir
        let _ = swap::delete_private(account_id);
        onboard::cleanup(&config_dir).await;
        progress.error(LoginPhase::Persisting, &msg);
        return Err(RegisterError::Store(msg));
    }
    onboard::cleanup(&config_dir).await;

    Ok(RegisterResult {
        uuid: account_id,
        email: prof.email,
        org_name: prof.org_name,
        subscription_type: prof.subscription_type,
        rate_limit_tier: prof.rate_limit_tier,
    })
}

// ---------------------------------------------------------------------------
// Verify-all progress — typed events for the per-account reconcile loop.
//
// `verify_all_accounts` iterates every account with credentials, sleeping
// 200ms between calls. Without progress events the GUI can't paint per-row
// badges; with them, the row flips from "Verifying…" to its outcome the
// instant the per-account event fires.
// ---------------------------------------------------------------------------

/// Lifted classification of a per-account verify outcome. Mirrors
/// [`crate::account::VerifyOutcome`] but flattened (no payload) so a
/// progress event can carry it as a tagged enum without re-emitting
/// the whole `VerifyOutcome` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyOutcomeKind {
    Ok,
    Drift,
    Rejected,
    NetworkError,
}

impl From<&crate::account::VerifyOutcome> for VerifyOutcomeKind {
    fn from(v: &crate::account::VerifyOutcome) -> Self {
        match v {
            crate::account::VerifyOutcome::Ok { .. } => Self::Ok,
            crate::account::VerifyOutcome::Drift { .. } => Self::Drift,
            crate::account::VerifyOutcome::Rejected => Self::Rejected,
            crate::account::VerifyOutcome::NetworkError => Self::NetworkError,
        }
    }
}

/// Discrete events emitted by [`verify_all_with_progress`]. Each event is
/// an independent payload — the progress channel can be dropped at any
/// point and replayed via the running-ops backstop without losing
/// completed accounts.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VerifyEvent {
    /// Fired once before the per-account loop. Carries the eligible
    /// total so the UI can render `1/N` immediately.
    Started { total: usize },
    /// Fired once per account, after its `/profile` round-trip resolves.
    /// `idx` is 1-based; `idx == total` is the last call.
    Account {
        uuid: Uuid,
        email: String,
        idx: usize,
        total: usize,
        outcome: VerifyOutcomeKind,
        /// Optional human-readable detail (drift email, network error
        /// message, etc.) — surfaced inline in row badges.
        detail: Option<String>,
    },
    /// Fired exactly once after the final `Account` event, signalling
    /// the loop has fully drained.
    Done,
}

/// Progress sink for [`verify_all_with_progress`]. Same shape as
/// [`LoginProgressSink`] but with a single typed event channel.
pub trait VerifyProgressSink: Send + Sync {
    fn event(&self, ev: VerifyEvent);
}

/// No-op sink for legacy callers that do not consume events.
pub struct NoopVerifySink;

impl VerifyProgressSink for NoopVerifySink {
    fn event(&self, _ev: VerifyEvent) {}
}

/// Reconcile every account's blob identity against `/profile`, emitting
/// per-account progress as the loop advances.
///
/// Behavior parity with the legacy `verify_all_accounts` IPC:
/// - Skips accounts without `has_cli_credentials`.
/// - Sleeps 200ms BETWEEN calls (not before the first or after the last)
///   so the request rate caps at ~5/s.
///
/// This function is infallible at the top level — per-account errors
/// flow through the sink as `Account { outcome: NetworkError, … }` so
/// one rate-limited account doesn't stop the others.
pub async fn verify_all_with_progress(
    store: &AccountStore,
    fetcher: &dyn crate::cli_backend::swap::ProfileFetcher,
    progress: &dyn VerifyProgressSink,
) {
    use std::time::Duration;

    let accounts = match store.list() {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("verify_all_with_progress: list failed: {e}");
            progress.event(VerifyEvent::Started { total: 0 });
            progress.event(VerifyEvent::Done);
            return;
        }
    };

    // Eligible set: accounts with credentials. Mirror the legacy filter
    // exactly — any other account is invisible to the verify pass.
    let eligible: Vec<(Uuid, String)> = accounts
        .iter()
        .filter(|a| a.has_cli_credentials)
        .map(|a| (a.uuid, a.email.clone()))
        .collect();
    let total = eligible.len();

    progress.event(VerifyEvent::Started { total });

    let mut first = true;
    for (idx, (uuid, email)) in eligible.into_iter().enumerate() {
        if !first {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        first = false;

        let (outcome_kind, detail) = match crate::services::identity::verify_account_identity(
            store, uuid, fetcher,
        )
        .await
        {
            Ok(outcome) => {
                let kind = VerifyOutcomeKind::from(&outcome);
                let detail = match &outcome {
                    crate::account::VerifyOutcome::Drift { actual_email, .. } => {
                        Some(format!("actual: {actual_email}"))
                    }
                    crate::account::VerifyOutcome::Rejected => {
                        Some("re-login required".to_string())
                    }
                    crate::account::VerifyOutcome::NetworkError => {
                        Some("/profile unreachable".to_string())
                    }
                    crate::account::VerifyOutcome::Ok { .. } => None,
                };
                tracing::info!(
                    account = %uuid,
                    status = outcome.as_str(),
                    "verify_all_with_progress: result"
                );
                (kind, detail)
            }
            Err(e) => {
                tracing::warn!(
                    account = %uuid,
                    "verify_all_with_progress: error {e}"
                );
                (VerifyOutcomeKind::NetworkError, Some(e.to_string()))
            }
        };

        progress.event(VerifyEvent::Account {
            uuid,
            email: email.clone(),
            idx: idx + 1,
            total,
            outcome: outcome_kind,
            detail,
        });
    }

    progress.event(VerifyEvent::Done);
}

// ---------------------------------------------------------------------------
// Reconcile — DB ↔ truth-from-disk/keychain alignment.
//
// `account_list` used to opportunistically rewrite `accounts.has_cli_credentials`
// and call `desktop_service::reconcile_flags` on every poll. That coupled a
// "list" command to mutation and let two GUI sections race each other on the
// same row. The functions below lift the two reconcile passes out of the read
// path so callers can run them once at startup and on user request.
// ---------------------------------------------------------------------------

/// One account whose `accounts.has_cli_credentials` flag was flipped to
/// match keychain truth. Returned by [`reconcile_cli_flags`].
#[derive(Debug, Clone)]
pub struct CliFlagFlip {
    pub uuid: Uuid,
    pub email: String,
    /// The new (post-flip) value of `has_cli_credentials`.
    pub new_value: bool,
}

/// Bundled output of [`reconcile_all`]. Combines the CLI-flag flips with
/// `desktop_service::reconcile_flags`'s outcome so a caller can render one
/// counts-only summary without two round-trips.
#[derive(Debug, Clone, Default)]
pub struct ReconcileReport {
    pub cli_flips: Vec<CliFlagFlip>,
    pub desktop: super::desktop_service::ReconcileOutcome,
}

#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    #[error("store: {0}")]
    Store(String),
}

/// Bring `accounts.has_cli_credentials` into alignment with keychain truth.
///
/// For every account, probes the keychain directly via [`swap::load_private`]:
/// a present, parseable blob means `truth=true`; missing or corrupt means
/// `truth=false`. When the DB flag disagrees, the row is updated and a
/// [`CliFlagFlip`] is recorded.
///
/// Note: this does NOT delegate to [`token_health`] because that helper
/// short-circuits on `has_credentials=false` (returning "no credentials"
/// without touching the keychain), which would hide the
/// flag-says-false-but-blob-exists drift direction. Reconcile must inspect
/// the keychain regardless of the cached flag.
///
/// Idempotent: a second pass on a converged store returns an empty `Vec`.
/// Errors short-circuit on the first sqlite failure (read or write).
pub fn reconcile_cli_flags(store: &AccountStore) -> Result<Vec<CliFlagFlip>, ReconcileError> {
    let accounts = store.list().map_err(|e| ReconcileError::Store(e.to_string()))?;
    let mut flips = Vec::new();
    for a in &accounts {
        // Probe the keychain directly. A present, parseable blob is the
        // truth the DB flag should mirror — anything else (missing,
        // unreadable, malformed JSON) means the swap can't succeed and
        // the flag should be `false`.
        let truth = match swap::load_private(a.uuid) {
            Ok(blob_str) => CredentialBlob::from_json(&blob_str).is_ok(),
            Err(_) => false,
        };
        if a.has_cli_credentials != truth {
            store
                .update_credentials_flag(a.uuid, truth)
                .map_err(|e| ReconcileError::Store(e.to_string()))?;
            flips.push(CliFlagFlip {
                uuid: a.uuid,
                email: a.email.clone(),
                new_value: truth,
            });
        }
    }
    Ok(flips)
}

/// Run both reconcile passes (CLI flags + Desktop flags / orphan pointer)
/// and bundle the outcomes. Failures from either pass surface as
/// [`ReconcileError::Store`].
pub fn reconcile_all(store: &AccountStore) -> Result<ReconcileReport, ReconcileError> {
    let cli_flips = reconcile_cli_flags(store)?;
    let desktop = super::desktop_service::reconcile_flags(store)
        .map_err(|e| ReconcileError::Store(e.to_string()))?;
    Ok(ReconcileReport { cli_flips, desktop })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{OAuthError, SwapError};
    use crate::oauth::refresh::TokenResponse;
    use crate::testing::{
        fresh_blob_json, make_account, setup_test_data_dir, test_store, DATA_DIR_LOCK,
    };

    fn insert_account(store: &AccountStore, email: &str) -> Account {
        let account = make_account(email);
        store.insert(&account).unwrap();
        account
    }

    // -- Mock infrastructure --

    struct MockPlatform {
        /// Scripted queue of `read_default` responses. Calls pop from
        /// the front while more than one response remains; once the
        /// queue holds its last entry, further calls clone it
        /// indefinitely. This preserves the original single-blob
        /// behaviour (`MockPlatform::new`) while letting race-sensitive
        /// tests (`MockPlatform::with_read_sequence`) script a keychain
        /// that appears to change between reads — modelling a
        /// concurrent writer.
        reads: std::sync::Mutex<std::collections::VecDeque<Option<String>>>,
        /// Full history of `write_default` payloads so tests can assert
        /// both whether a write happened and in what order.
        writes: std::sync::Mutex<Vec<String>>,
    }

    impl MockPlatform {
        fn new(blob: Option<String>) -> Self {
            let mut q = std::collections::VecDeque::new();
            q.push_back(blob);
            Self {
                reads: std::sync::Mutex::new(q),
                writes: std::sync::Mutex::new(Vec::new()),
            }
        }
        /// Build a platform whose `read_default` returns each scripted
        /// value in order, then repeats the last one. Used by the
        /// race regression tests to inject a "CC rotated the blob
        /// while we were mid-flight" transition.
        fn with_read_sequence(reads: Vec<Option<String>>) -> Self {
            assert!(
                !reads.is_empty(),
                "with_read_sequence needs at least one scripted response"
            );
            Self {
                reads: std::sync::Mutex::new(reads.into_iter().collect()),
                writes: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn last_written(&self) -> Option<String> {
            self.writes.lock().unwrap().last().cloned()
        }
        fn write_count(&self) -> usize {
            self.writes.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl cli_backend::CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            let mut q = self.reads.lock().unwrap();
            if q.len() > 1 {
                Ok(q.pop_front().unwrap())
            } else {
                Ok(q.front().cloned().unwrap_or(None))
            }
        }
        async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
            self.writes.lock().unwrap().push(blob.to_string());
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    /// Profile fetcher with an optional response queue. `ok`/`failing`
    /// preserve the original single-response behaviour (every `fetch`
    /// returns the same result). `sequence` pops one result per call —
    /// used by auto-refresh tests where the first `/profile` call 401s
    /// on a stale access_token and the second call succeeds with the
    /// fresh one.
    struct MockProfileFetcher {
        profile: Result<profile::Profile, OAuthError>,
        queue: std::sync::Mutex<std::collections::VecDeque<Result<profile::Profile, OAuthError>>>,
        /// Records every access_token passed to `fetch` so tests can
        /// assert the retry used the new token (not the stale one).
        seen_tokens: std::sync::Mutex<Vec<String>>,
    }

    fn sample_profile(email: &str) -> profile::Profile {
        profile::Profile {
            email: email.to_string(),
            org_uuid: "org-uuid-1".to_string(),
            org_name: "Test Org".to_string(),
            subscription_type: "pro".to_string(),
            rate_limit_tier: Some("default_claude_pro".to_string()),
            account_uuid: "acc-uuid-1".to_string(),
            display_name: Some("Test User".to_string()),
        }
    }

    fn clone_oauth_error(e: &OAuthError) -> OAuthError {
        match e {
            OAuthError::AuthFailed(m) => OAuthError::AuthFailed(m.clone()),
            OAuthError::RefreshFailed(m) => OAuthError::RefreshFailed(m.clone()),
            OAuthError::ServerError(m) => OAuthError::ServerError(m.clone()),
            OAuthError::RateLimited { retry_after_secs } => OAuthError::RateLimited {
                retry_after_secs: *retry_after_secs,
            },
            // HttpError isn't constructible in tests; any remaining
            // variant collapses to AuthFailed so the fall-through
            // behaves like the original mock.
            _ => OAuthError::AuthFailed("mock error".into()),
        }
    }

    impl MockProfileFetcher {
        fn ok(email: &str) -> Self {
            Self {
                profile: Ok(sample_profile(email)),
                queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
                seen_tokens: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn failing(msg: &str) -> Self {
            Self {
                profile: Err(OAuthError::AuthFailed(msg.to_string())),
                queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
                seen_tokens: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn failing_with(err: OAuthError) -> Self {
            Self {
                profile: Err(err),
                queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
                seen_tokens: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn sequence(responses: Vec<Result<profile::Profile, OAuthError>>) -> Self {
            Self {
                // `profile` is used as the fall-through once the queue
                // drains — set to a hard AuthFailed so an unexpected
                // extra call during tests fails loudly instead of
                // silently succeeding.
                profile: Err(OAuthError::AuthFailed(
                    "MockProfileFetcher sequence exhausted".into(),
                )),
                queue: std::sync::Mutex::new(responses.into_iter().collect()),
                seen_tokens: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn tokens_seen(&self) -> Vec<String> {
            self.seen_tokens.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl ProfileFetcher for MockProfileFetcher {
        async fn fetch(&self, access_token: &str) -> Result<profile::Profile, OAuthError> {
            self.seen_tokens
                .lock()
                .unwrap()
                .push(access_token.to_string());
            if let Some(next) = self.queue.lock().unwrap().pop_front() {
                return match next {
                    Ok(p) => Ok(p),
                    Err(e) => Err(clone_oauth_error(&e)),
                };
            }
            match &self.profile {
                Ok(p) => Ok(p.clone()),
                Err(e) => Err(clone_oauth_error(e)),
            }
        }
    }

    struct MockRefresher {
        response: Result<TokenResponse, OAuthError>,
        /// Records every refresh_token passed to `refresh` so tests can
        /// assert the race-aware path used the LATEST refresh_token,
        /// not the stale snapshot captured at the top of sync.
        seen_tokens: std::sync::Mutex<Vec<String>>,
    }

    impl MockRefresher {
        fn success() -> Self {
            Self {
                response: Ok(TokenResponse {
                    access_token: "sk-ant-oat01-new".into(),
                    refresh_token: "sk-ant-ort01-new".into(),
                    expires_in: 3600,
                    scope: Some("user:inference".into()),
                    token_type: Some("Bearer".into()),
                }),
                seen_tokens: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn failing(msg: &str) -> Self {
            Self {
                response: Err(OAuthError::RefreshFailed(msg.to_string())),
                seen_tokens: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn tokens_seen(&self) -> Vec<String> {
            self.seen_tokens.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl crate::cli_backend::swap::TokenRefresher for MockRefresher {
        async fn refresh(&self, rt: &str) -> Result<TokenResponse, OAuthError> {
            self.seen_tokens.lock().unwrap().push(rt.to_string());
            match &self.response {
                Ok(r) => Ok(TokenResponse {
                    access_token: r.access_token.clone(),
                    refresh_token: r.refresh_token.clone(),
                    expires_in: r.expires_in,
                    scope: r.scope.clone(),
                    token_type: r.token_type.clone(),
                }),
                Err(OAuthError::RefreshFailed(msg)) => Err(OAuthError::RefreshFailed(msg.clone())),
                _ => Err(OAuthError::RefreshFailed("mock".into())),
            }
        }
    }

    // -- register_from_current_with tests --

    #[tokio::test]
    async fn test_register_from_current_success() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let platform = MockPlatform::new(Some(fresh_blob_json()));
        let fetcher = MockProfileFetcher::ok("alice@example.com");

        let result = register_from_current_with(&store, &platform, &fetcher)
            .await
            .unwrap();
        assert_eq!(result.email, "alice@example.com");
        assert_eq!(result.org_name, "Test Org");
        assert_eq!(result.subscription_type, "pro");

        // Account inserted into store
        let found = store.find_by_email("alice@example.com").unwrap().unwrap();
        assert_eq!(found.email, "alice@example.com");
        assert!(found.has_cli_credentials);

        swap::delete_private(result.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_register_from_current_no_credentials() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let platform = MockPlatform::new(None);
        let fetcher = MockProfileFetcher::ok("alice@example.com");

        let result = register_from_current_with(&store, &platform, &fetcher).await;
        assert!(matches!(result, Err(RegisterError::NoCredentials)));
    }

    #[tokio::test]
    async fn test_register_from_current_profile_fetch_fails() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let platform = MockPlatform::new(Some(fresh_blob_json()));
        let fetcher = MockProfileFetcher::failing("401 Unauthorized");

        let result = register_from_current_with(&store, &platform, &fetcher).await;
        assert!(matches!(result, Err(RegisterError::ProfileFetch(_))));
    }

    #[tokio::test]
    async fn test_register_from_current_duplicate_account() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        // Pre-register
        insert_account(&store, "dup@example.com");

        let platform = MockPlatform::new(Some(fresh_blob_json()));
        let fetcher = MockProfileFetcher::ok("dup@example.com");

        let result = register_from_current_with(&store, &platform, &fetcher).await;
        assert!(matches!(
            result,
            Err(RegisterError::AlreadyRegistered(_, _))
        ));
    }

    #[tokio::test]
    async fn test_register_from_current_corrupt_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let platform = MockPlatform::new(Some("not json".to_string()));
        let fetcher = MockProfileFetcher::ok("alice@example.com");

        let result = register_from_current_with(&store, &platform, &fetcher).await;
        assert!(matches!(result, Err(RegisterError::CredentialRead(_))));
    }

    // -- register_from_token_with tests --

    #[tokio::test]
    async fn test_register_from_token_success() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let refresher = MockRefresher::success();
        let fetcher = MockProfileFetcher::ok("bob@example.com");

        let result = register_from_token_with(&store, "rt-test", &refresher, &fetcher)
            .await
            .unwrap();

        assert_eq!(result.email, "bob@example.com");
        assert!(store.find_by_email("bob@example.com").unwrap().is_some());

        swap::delete_private(result.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_register_from_token_refresh_fails() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let refresher = MockRefresher::failing("invalid token");
        let fetcher = MockProfileFetcher::ok("bob@example.com");

        let result = register_from_token_with(&store, "rt-bad", &refresher, &fetcher).await;

        assert!(matches!(result, Err(RegisterError::ProfileFetch(_))));
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("token exchange failed"));
    }

    #[tokio::test]
    async fn test_register_from_token_duplicate() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();
        insert_account(&store, "dup@example.com");

        let refresher = MockRefresher::success();
        let fetcher = MockProfileFetcher::ok("dup@example.com");

        let result = register_from_token_with(&store, "rt-test", &refresher, &fetcher).await;

        assert!(matches!(
            result,
            Err(RegisterError::AlreadyRegistered(_, _))
        ));
    }

    // -- format_duration_mins tests --

    #[test]
    fn test_format_duration_mins() {
        assert_eq!(format_duration_mins(0), "0m");
        assert_eq!(format_duration_mins(-5), "0m");
        assert_eq!(format_duration_mins(1), "1m");
        assert_eq!(format_duration_mins(45), "45m");
        assert_eq!(format_duration_mins(60), "1h");
        assert_eq!(format_duration_mins(88), "1h 28m");
        assert_eq!(format_duration_mins(120), "2h");
        assert_eq!(format_duration_mins(125), "2h 5m");
        assert_eq!(format_duration_mins(1440), "1d");
        assert_eq!(format_duration_mins(1500), "1d 1h");
        assert_eq!(format_duration_mins(2880), "2d");
        assert_eq!(format_duration_mins(2945), "2d 1h"); // minutes dropped when days present
    }

    // -- token_health tests --

    #[test]
    fn test_token_health_no_credentials() {
        let health = token_health(Uuid::new_v4(), false);
        assert_eq!(health.status, "no credentials");
        assert!(health.remaining_mins.is_none());
    }

    #[test]
    fn test_token_health_missing_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let health = token_health(Uuid::new_v4(), true);
        assert_eq!(health.status, "missing");
    }

    #[test]
    fn test_token_health_valid_token() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &fresh_blob_json()).unwrap();

        let health = token_health(id, true);
        assert!(health.status.contains("valid"));
        assert!(health.remaining_mins.unwrap() > 0);

        swap::delete_private(id).unwrap();
    }

    #[test]
    fn test_token_health_expired_token() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, &crate::testing::expired_blob_json()).unwrap();

        let health = token_health(id, true);
        assert_eq!(health.status, "expired");
        assert!(health.remaining_mins.unwrap() < 0);

        swap::delete_private(id).unwrap();
    }

    #[test]
    fn test_token_health_corrupt_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        swap::save_private(id, "not json").unwrap();

        let health = token_health(id, true);
        assert_eq!(health.status, "corrupt blob");

        swap::delete_private(id).unwrap();
    }

    #[tokio::test]
    async fn test_remove_deletes_credential_file() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "cred@example.com");

        // Save a credential file
        swap::save_private(account.uuid, r#"{"test":"blob"}"#).unwrap();
        assert!(swap::load_private(account.uuid).is_ok());

        remove_account(&store, account.uuid, None).await.unwrap();

        // Credential file should be gone
        assert!(swap::load_private(account.uuid).is_err());
    }

    #[tokio::test]
    async fn test_remove_deletes_desktop_profile() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "desk@example.com");

        // Create desktop profile dir
        let profile_dir = paths::desktop_profile_dir(account.uuid);
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(profile_dir.join("config.json"), "cfg").unwrap();

        let result = remove_account(&store, account.uuid, None).await.unwrap();
        assert!(result.had_desktop_profile);
        assert!(!profile_dir.exists());
    }

    #[tokio::test]
    async fn test_remove_removes_from_db() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "db@example.com");

        remove_account(&store, account.uuid, None).await.unwrap();
        assert!(store.find_by_uuid(account.uuid).unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove_clears_active_cli() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "cli@example.com");
        store.set_active_cli(account.uuid).unwrap();

        let result = remove_account(&store, account.uuid, None).await.unwrap();
        assert!(result.was_cli_active);
        assert!(store.active_cli_uuid().unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove_clears_active_desktop() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "desk2@example.com");
        store.set_active_desktop(account.uuid).unwrap();

        let result = remove_account(&store, account.uuid, None).await.unwrap();
        assert!(result.was_desktop_active);
        assert!(store.active_desktop_uuid().unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove_nonexistent_returns_not_found() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();

        let result = remove_account(&store, Uuid::new_v4(), None).await;
        assert!(matches!(result, Err(RegisterError::NotFound)));
    }

    #[tokio::test]
    async fn test_remove_missing_credential_succeeds_silently() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "warn@example.com");
        // Do NOT save_private — credential file doesn't exist

        let result = remove_account(&store, account.uuid, None).await.unwrap();
        // delete_private returns Ok when file doesn't exist,
        // so no warning is produced — this is correct behavior
        assert!(result.warnings.is_empty());
        // Account still removed from DB
        assert!(store.find_by_uuid(account.uuid).unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove_returns_correct_metadata() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db_dir) = test_store();
        let account = insert_account(&store, "meta@example.com");

        let result = remove_account(&store, account.uuid, None).await.unwrap();
        assert_eq!(result.email, "meta@example.com");
        assert!(!result.was_cli_active);
        assert!(!result.was_desktop_active);
        assert!(!result.had_desktop_profile);
    }

    // -- sync_from_current_cc --

    #[tokio::test]
    async fn test_sync_adopts_cc_blob_when_email_matches_registered_account() {
        // Startup scenario: CC is signed in as an account the user has
        // already registered in Claudepot, but Claudepot's stored blob
        // slot for that account is empty (e.g. after a reinstall).
        // sync_from_current_cc should write the blob + flip the flag +
        // set active_cli — no user action needed.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let account = insert_account(&store, "alice@example.com");
        // DB flag was flipped off (e.g. reinstall wiped storage).
        let _ = store.update_credentials_flag(account.uuid, false);
        // Capture the blob once — fresh_blob_json() uses Utc::now() so
        // calling it twice returns JSON strings whose expiresAt differs
        // by ~1ms, which makes the post-sync comparison flaky.
        let cc_blob = fresh_blob_json();
        let platform = MockPlatform::new(Some(cc_blob.clone()));
        let fetcher = MockProfileFetcher::ok("alice@example.com");
        let refresher = MockRefresher::success();

        let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
            .await
            .unwrap();

        assert_eq!(synced, Some(account.uuid), "should report the synced uuid");
        // Blob now in Claudepot's storage.
        assert_eq!(swap::load_private(account.uuid).unwrap(), cc_blob);
        // active_cli aligned with CC's current reality.
        assert_eq!(
            store.active_cli_uuid().unwrap(),
            Some(account.uuid.to_string())
        );
        // Happy path never touched the refresher — no blob rotation
        // should land in CC's keychain.
        assert!(
            platform.last_written().is_none(),
            "platform.write_default must not fire when /profile succeeds"
        );

        swap::delete_private(account.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_sync_is_noop_when_cc_email_is_not_registered() {
        // CC holds a blob for an account Claudepot doesn't know about.
        // We should NOT auto-register; just leave the state alone.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let platform = MockPlatform::new(Some(fresh_blob_json()));
        let fetcher = MockProfileFetcher::ok("stranger@example.com");
        let refresher = MockRefresher::success();

        let result = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
            .await
            .unwrap();

        assert_eq!(result, None);
        // No accounts were registered.
        assert_eq!(store.list().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_sync_is_noop_when_cc_has_no_credentials() {
        // CC empty (logged out) — sync should return Ok(None).
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        insert_account(&store, "alice@example.com");
        let platform = MockPlatform::new(None);
        let fetcher = MockProfileFetcher::ok("alice@example.com");
        let refresher = MockRefresher::success();

        let result = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
            .await
            .unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_sync_refreshes_stale_access_token_and_retries_profile() {
        // The xaiolai scenario: CC's access_token expired in the
        // background. /profile returns 401, but the paired
        // refresh_token is still valid. Expected behavior: sync
        // silently rotates the tokens, writes the fresh blob back to
        // CC's keychain, then retries /profile and completes the
        // adopt flow. No user-facing error.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let account = insert_account(&store, "alice@example.com");
        let _ = store.update_credentials_flag(account.uuid, false);
        let cc_blob = fresh_blob_json();
        let platform = MockPlatform::new(Some(cc_blob.clone()));
        // First call: 401. Second call (after refresh): success.
        let fetcher = MockProfileFetcher::sequence(vec![
            Err(OAuthError::AuthFailed("401 Unauthorized".into())),
            Ok(sample_profile("alice@example.com")),
        ]);
        let refresher = MockRefresher::success();

        let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
            .await
            .unwrap();

        assert_eq!(synced, Some(account.uuid));

        // CC's keychain got the rotated blob. The new blob must carry
        // the fresh access_token ("sk-ant-oat01-new" from
        // MockRefresher::success) — confirming we wrote the rotated
        // tokens back to CC and not the stale ones.
        let written = platform
            .last_written()
            .expect("write_default must fire after successful refresh");
        let written_blob = crate::blob::CredentialBlob::from_json(&written).unwrap();
        assert_eq!(
            written_blob.claude_ai_oauth.access_token, "sk-ant-oat01-new",
            "CC keychain must hold the freshly-rotated access token"
        );

        // The retry used the NEW access token, not the stale one.
        let tokens_seen = fetcher.tokens_seen();
        assert_eq!(tokens_seen.len(), 2, "profile fetch should run twice");
        assert_eq!(
            tokens_seen[0], "sk-ant-oat01-test",
            "first call must use the stale token from CC's blob"
        );
        assert_eq!(
            tokens_seen[1], "sk-ant-oat01-new",
            "retry must use the freshly-rotated access token"
        );

        // Claudepot's private slot also gets the fresh blob, and the
        // account's active_cli pointer is set.
        let stored = swap::load_private(account.uuid).unwrap();
        assert_eq!(
            stored, written,
            "private slot must match what we wrote to CC's keychain"
        );
        assert_eq!(
            store.active_cli_uuid().unwrap(),
            Some(account.uuid.to_string())
        );

        swap::delete_private(account.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_sync_returns_auth_rejected_when_refresh_token_is_dead() {
        // Terminal case: access_token rejected AND refresh_token
        // refused. The user revoked access elsewhere, or the grant
        // aged out. Expected: AuthRejected — a first-class error the
        // UI can surface as "Sign in again" instead of the generic
        // ProfileFetch warning that's currently dropped on the floor.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        insert_account(&store, "alice@example.com");
        let platform = MockPlatform::new(Some(fresh_blob_json()));
        let fetcher = MockProfileFetcher::failing("401 Unauthorized");
        let refresher = MockRefresher::failing("refresh_token revoked");

        let result = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher).await;

        assert!(
            matches!(result, Err(RegisterError::AuthRejected)),
            "expected AuthRejected, got {:?}",
            result
        );
        // We never wrote anything to CC's keychain — refresh failed
        // before we had a fresh blob to write.
        assert!(platform.last_written().is_none());
    }

    /// Build a credential blob with caller-specified access_token /
    /// refresh_token values. Used by the race regression tests to
    /// stand up distinguishable "before" and "after" snapshots so
    /// assertions can prove WHICH blob the sync path acted on.
    fn blob_json_with(access: &str, refresh: &str) -> String {
        let expires = chrono::Utc::now().timestamp_millis() + 3_600_000;
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"{access}","refreshToken":"{refresh}","expiresAt":{expires},"scopes":["user:inference","user:profile"],"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}}}"#
        )
    }

    #[tokio::test]
    async fn test_sync_race_adopts_fresh_keychain_blob_without_burning_refresh_token() {
        // The user-reported scenario: CC auto-refreshed between our
        // initial keychain read and our /profile call. Our snapshot's
        // access_token is now stale, but CC wrote a fresh blob whose
        // access_token works. Before this fix, sync would refresh
        // using the stale refresh_token — which CC just consumed —
        // and report AuthRejected. After the fix, the re-read picks
        // up CC's fresh blob and /profile succeeds without touching
        // the refresh endpoint.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();
        let account = insert_account(&store, "alice@example.com");
        let _ = store.update_credentials_flag(account.uuid, false);

        let old_blob = blob_json_with("sk-ant-oat01-stale", "sk-ant-ort01-stale");
        let fresh_blob = blob_json_with("sk-ant-oat01-fresh", "sk-ant-ort01-fresh");
        let platform = MockPlatform::with_read_sequence(vec![
            Some(old_blob.clone()),
            Some(fresh_blob.clone()),
        ]);
        // First /profile (stale access) → 401, second (fresh access) → Ok.
        let fetcher = MockProfileFetcher::sequence(vec![
            Err(OAuthError::AuthFailed("401 Unauthorized".into())),
            Ok(sample_profile("alice@example.com")),
        ]);
        // Refresher configured to fail loudly — if the race-aware path
        // ever dispatches it when the fresh access_token already works,
        // this test catches the regression.
        let refresher = MockRefresher::failing("refresher must not be called");

        let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
            .await
            .unwrap();

        assert_eq!(synced, Some(account.uuid));

        // /profile was called twice: once with the stale access_token,
        // then with the fresh one from CC's rotated blob.
        let tokens = fetcher.tokens_seen();
        assert_eq!(tokens.len(), 2, "expected exactly two /profile calls");
        assert_eq!(tokens[0], "sk-ant-oat01-stale");
        assert_eq!(tokens[1], "sk-ant-oat01-fresh");

        // Refresh endpoint must NOT have been hit — the race was
        // resolved by reading the fresh blob, not by burning a
        // refresh_token that CC had already consumed.
        assert!(
            refresher.tokens_seen().is_empty(),
            "refresh must not run when a fresh keychain read resolves /profile"
        );

        // CC's keychain wasn't overwritten — we didn't produce a new
        // blob to write.
        assert_eq!(platform.write_count(), 0);

        // Claudepot's private slot mirrors the FRESH blob, not the
        // stale snapshot. If we stored the stale one, the next swap
        // would feed CC dead tokens.
        let stored = swap::load_private(account.uuid).unwrap();
        assert_eq!(stored, fresh_blob);

        swap::delete_private(account.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_sync_race_refresh_uses_latest_refresh_token_not_stale_snapshot() {
        // Defence in depth: even when the fresh access_token from a
        // re-read also 401s (e.g. clock skew, server-side lag on
        // newly-rotated tokens), the refresh MUST be attempted with
        // the LATEST refresh_token from the keychain — not the stale
        // one captured at the top of sync. Calling /token with a
        // refresh_token that's already been consumed is what produced
        // the false AuthRejected in the first place.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();
        let account = insert_account(&store, "alice@example.com");
        let _ = store.update_credentials_flag(account.uuid, false);

        let stale_blob = blob_json_with("sk-ant-oat01-stale", "sk-ant-ort01-stale");
        let fresh_blob = blob_json_with("sk-ant-oat01-fresh", "sk-ant-ort01-fresh");
        let platform = MockPlatform::with_read_sequence(vec![
            Some(stale_blob.clone()),
            Some(fresh_blob.clone()),
            // Pre-write CAS read sees the same fresh blob — no further
            // race after refresh, so the CAS write proceeds.
            Some(fresh_blob.clone()),
        ]);
        // 1st /profile (stale access) → 401
        // 2nd /profile (fresh access) → 401 too (still in limbo)
        // 3rd /profile (post-refresh access) → Ok
        let fetcher = MockProfileFetcher::sequence(vec![
            Err(OAuthError::AuthFailed("401 stale".into())),
            Err(OAuthError::AuthFailed("401 fresh".into())),
            Ok(sample_profile("alice@example.com")),
        ]);
        let refresher = MockRefresher::success();

        let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
            .await
            .unwrap();

        assert_eq!(synced, Some(account.uuid));

        // Critical assertion: refresh used the FRESH refresh_token
        // (from the re-read), not the stale one from our initial
        // snapshot. Reversing this is exactly what produces the false
        // AuthRejected banner.
        let rt_seen = refresher.tokens_seen();
        assert_eq!(rt_seen.len(), 1, "refresh must run exactly once");
        assert_eq!(
            rt_seen[0], "sk-ant-ort01-fresh",
            "refresh must use the LATEST refresh_token, not the stale snapshot"
        );

        // CAS allowed the write because the keychain still matched
        // `fresh_blob` when we checked pre-write. The written blob
        // carries the post-refresh access_token.
        assert_eq!(platform.write_count(), 1);
        let written = platform.last_written().unwrap();
        let written_blob = CredentialBlob::from_json(&written).unwrap();
        assert_eq!(written_blob.claude_ai_oauth.access_token, "sk-ant-oat01-new");

        swap::delete_private(account.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_sync_race_cas_skips_keychain_writeback_when_concurrent_writer_landed() {
        // Belt-and-braces: after our refresh succeeds, another writer
        // (another Claudepot instance, or CC racing the same window)
        // may have landed a different blob in the keychain. Writing
        // our rotated blob would clobber theirs. The CAS guard reads
        // the keychain right before the write and skips if it no
        // longer matches what we refreshed from. Because the rotated
        // blob CC never installed must NOT be persisted to our slot,
        // the function then re-reads the live blob and re-verifies
        // identity against it, persisting the intruder's blob (CC's
        // truth) into the matching account slot.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();
        let account = insert_account(&store, "alice@example.com");
        let _ = store.update_credentials_flag(account.uuid, false);

        let our_blob = blob_json_with("sk-ant-oat01-stale", "sk-ant-ort01-stale");
        let intruder_blob =
            blob_json_with("sk-ant-oat01-intruder", "sk-ant-ort01-intruder");
        let platform = MockPlatform::with_read_sequence(vec![
            // #1 initial snapshot.
            Some(our_blob.clone()),
            // #2 race-check re-read — still our blob, no race yet.
            Some(our_blob.clone()),
            // #3 pre-write CAS — surprise: someone wrote between
            // refresh and write-back.
            Some(intruder_blob.clone()),
        ]);
        let fetcher = MockProfileFetcher::sequence(vec![
            // Step 1: profile call on `our_blob` access_token → 401.
            Err(OAuthError::AuthFailed("401 Unauthorized".into())),
            // Step 4: re-verify the intruder's live blob succeeds
            // (still alice — same account, just rotated by the race).
            Ok(sample_profile("alice@example.com")),
        ]);
        let refresher = MockRefresher::success();

        let synced = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
            .await
            .unwrap();

        assert_eq!(synced, Some(account.uuid));

        // CAS must have suppressed the write. Leaving the intruder's
        // newer blob in place is the correct trade-off: we don't know
        // what state they're in, but we know our rotated blob is no
        // fresher than theirs.
        assert_eq!(
            platform.write_count(),
            0,
            "CAS must skip write-back when the keychain changed during refresh"
        );

        // Persisted blob must be CC's live blob (intruder's), NOT the
        // rotated `new_blob_str` we minted but never installed. That
        // would mis-file our orphan token into the account's slot.
        let stored = swap::load_private(account.uuid).unwrap();
        assert_eq!(
            stored, intruder_blob,
            "must persist CC's live blob, never the orphan rotated blob"
        );

        swap::delete_private(account.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_sync_race_cas_miss_aborts_when_live_blob_unverifiable() {
        // CAS miss + live blob can't be verified (token rejected, blob
        // unparseable, etc.) → must NOT persist either the rotated
        // blob or the live blob. Surface a typed retry-able error.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();
        let account = insert_account(&store, "alice@example.com");
        let _ = store.update_credentials_flag(account.uuid, false);

        let our_blob = blob_json_with("sk-ant-oat01-stale", "sk-ant-ort01-stale");
        let intruder_blob =
            blob_json_with("sk-ant-oat01-intruder", "sk-ant-ort01-intruder");
        let platform = MockPlatform::with_read_sequence(vec![
            Some(our_blob.clone()),
            Some(our_blob.clone()),
            // CAS check — intruder's blob landed.
            Some(intruder_blob.clone()),
        ]);
        let fetcher = MockProfileFetcher::sequence(vec![
            // Step 1: 401 on our_blob access token.
            Err(OAuthError::AuthFailed("401 Unauthorized".into())),
            // Step 4: re-verify on intruder's blob also 401 (token
            // already rotated again, or simply unverifiable).
            Err(OAuthError::AuthFailed("401 Unauthorized".into())),
        ]);
        let refresher = MockRefresher::success();

        let err = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher)
            .await
            .expect_err("must abort when live blob can't be verified");
        assert!(
            matches!(err, RegisterError::CcChangedDuringRefresh),
            "expected CcChangedDuringRefresh, got {err:?}"
        );

        // Nothing was persisted to the account's private slot.
        assert!(
            swap::load_private(account.uuid).is_err(),
            "must not persist anything when live blob unverifiable"
        );
    }

    #[tokio::test]
    async fn test_sync_treats_non_auth_profile_errors_as_transient() {
        // Guardrail: refresh should only kick in for 401 (AuthFailed).
        // Server-side errors, rate limits, and transport failures must
        // fall through to ProfileFetch so verified_email history
        // survives transient outages. Refresher must NOT be called.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        insert_account(&store, "alice@example.com");
        let platform = MockPlatform::new(Some(fresh_blob_json()));
        let fetcher = MockProfileFetcher::failing_with(OAuthError::ServerError(
            "502 Bad Gateway".into(),
        ));
        // Configure refresher to fail loudly — if sync ever dispatches
        // it for a non-auth error, this test will catch the regression.
        let refresher = MockRefresher::failing("refresher must not be called");

        let result = sync_from_current_cc_with(&store, &platform, &fetcher, &refresher).await;

        assert!(
            matches!(result, Err(RegisterError::ProfileFetch(_))),
            "expected ProfileFetch (transient), got {:?}",
            result
        );
        assert!(
            platform.last_written().is_none(),
            "server errors must not trigger a keychain write"
        );
    }

    // -- Group 5: account service rollbacks --

    #[tokio::test]
    async fn test_register_from_current_duplicate_cleans_no_blob() {
        // When the fetched profile matches an existing account's email,
        // registration fails with AlreadyRegistered BEFORE any blob is saved.
        // Verify: no credential file for the attempted UUID exists after.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        insert_account(&store, "dup@example.com");
        let before_privates = count_private_files();

        let platform = MockPlatform::new(Some(fresh_blob_json()));
        let fetcher = MockProfileFetcher::ok("dup@example.com");
        let result = register_from_current_with(&store, &platform, &fetcher).await;
        assert!(matches!(
            result,
            Err(RegisterError::AlreadyRegistered(_, _))
        ));

        let after_privates = count_private_files();
        assert_eq!(
            before_privates, after_privates,
            "duplicate rejection must not leave orphan blob on disk"
        );
    }

    #[tokio::test]
    async fn test_remove_account_preserves_files_on_db_failure() {
        // If store.remove() fails, credential file and profile dir must still
        // exist (irreversible file deletions gated behind successful DB remove).
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let account = insert_account(&store, "dbfail@example.com");
        swap::save_private(account.uuid, "credential-content").unwrap();
        let profile_dir = paths::desktop_profile_dir(account.uuid);
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(profile_dir.join("config.json"), "{}").unwrap();

        // Make store.remove() fail by dropping the accounts table.
        store.corrupt_for_test();

        let result = remove_account(&store, account.uuid, None).await;
        assert!(matches!(result, Err(RegisterError::Store(_))));

        // Credential + profile files still on disk since DB remove failed first.
        assert!(
            swap::load_private(account.uuid).is_ok(),
            "credential blob preserved after DB failure"
        );
        assert!(
            profile_dir.exists() && profile_dir.join("config.json").exists(),
            "desktop profile preserved after DB failure"
        );

        // Cleanup — tear down manually since store is now corrupt.
        let _ = swap::delete_private(account.uuid);
        let _ = std::fs::remove_dir_all(&profile_dir);
    }

    #[tokio::test]
    async fn test_remove_account_clears_pointers_before_db_remove() {
        // The ordering fix: pointers are cleared before store.remove(). Even
        // though that's partly structural, the observable effect is: after a
        // successful remove_account, active_cli_uuid() and active_desktop_uuid()
        // return None for the removed account.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let account = insert_account(&store, "ordering@example.com");
        store.set_active_cli(account.uuid).unwrap();
        store.set_active_desktop(account.uuid).unwrap();

        let result = remove_account(&store, account.uuid, None).await.unwrap();
        assert!(result.was_cli_active);
        assert!(result.was_desktop_active);

        assert!(store.active_cli_uuid().unwrap().is_none());
        assert!(store.active_desktop_uuid().unwrap().is_none());
        assert!(store.find_by_uuid(account.uuid).unwrap().is_none());
    }

    fn count_private_files() -> usize {
        let dir = crate::paths::claudepot_data_dir().join("credentials");
        std::fs::read_dir(&dir)
            .map(|rd| rd.filter_map(|e| e.ok()).count())
            .unwrap_or(0)
    }

    // ---------------------------------------------------------------------
    // Reconcile tests (B-2)
    // ---------------------------------------------------------------------

    #[test]
    fn test_reconcile_cli_flags_flips_stale_true_to_false() {
        // DB says the account has CLI credentials but the keychain is
        // empty (the user removed the blob out-of-band, or a swap
        // failed mid-write). reconcile_cli_flags must flip the flag to
        // false and report the flip.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let mut acct = make_account("stale-true@example.com");
        acct.has_cli_credentials = true;
        store.insert(&acct).unwrap();
        // No swap::save_private — keychain is empty for this uuid.

        let flips = reconcile_cli_flags(&store).unwrap();
        assert_eq!(flips.len(), 1);
        assert_eq!(flips[0].uuid, acct.uuid);
        assert_eq!(flips[0].email, acct.email);
        assert!(!flips[0].new_value);

        let after = store.find_by_uuid(acct.uuid).unwrap().unwrap();
        assert!(!after.has_cli_credentials);
    }

    #[test]
    fn test_reconcile_cli_flags_flips_stale_false_to_true() {
        // DB says no CLI credentials but a parseable blob is on the
        // keychain. The flag must be lifted to true and the flip
        // reported.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let mut acct = make_account("stale-false@example.com");
        acct.has_cli_credentials = false;
        store.insert(&acct).unwrap();
        swap::save_private(acct.uuid, &crate::testing::fresh_blob_json()).unwrap();

        let flips = reconcile_cli_flags(&store).unwrap();
        assert_eq!(flips.len(), 1);
        assert_eq!(flips[0].uuid, acct.uuid);
        assert!(flips[0].new_value);

        let after = store.find_by_uuid(acct.uuid).unwrap().unwrap();
        assert!(after.has_cli_credentials);

        swap::delete_private(acct.uuid).unwrap();
    }

    #[test]
    fn test_reconcile_cli_flags_idempotent() {
        // After a converged pass, a second run must report zero flips
        // and leave the DB untouched.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        // One account in alignment (false / no blob), one in
        // alignment (true / present blob).
        let mut a_no = make_account("none@example.com");
        a_no.has_cli_credentials = false;
        store.insert(&a_no).unwrap();

        let mut a_yes = make_account("yes@example.com");
        a_yes.has_cli_credentials = false; // start drifted
        store.insert(&a_yes).unwrap();
        swap::save_private(a_yes.uuid, &crate::testing::fresh_blob_json()).unwrap();

        // First pass converges the drifted row.
        let first = reconcile_cli_flags(&store).unwrap();
        assert_eq!(first.len(), 1);
        // Second pass on a converged store: empty Vec.
        let second = reconcile_cli_flags(&store).unwrap();
        assert!(
            second.is_empty(),
            "expected idempotent second pass, got {} flips",
            second.len()
        );

        swap::delete_private(a_yes.uuid).unwrap();
    }

    #[test]
    fn test_reconcile_all_combines_cli_and_desktop() {
        // Drift one CLI flag (DB says true, keychain empty) and one
        // desktop flag (DB says true, snapshot dir absent). reconcile_all
        // must report both passes via its bundled outcome.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let mut cli_drift = make_account("cli@example.com");
        cli_drift.has_cli_credentials = true;
        store.insert(&cli_drift).unwrap();

        let mut desk_drift = make_account("desk@example.com");
        // Make CLI side aligned (false + no blob) so only desktop drifts.
        desk_drift.has_cli_credentials = false;
        desk_drift.has_desktop_profile = true;
        store.insert(&desk_drift).unwrap();
        // Snapshot dir intentionally missing on disk.

        let report = reconcile_all(&store).unwrap();
        assert_eq!(report.cli_flips.len(), 1);
        assert_eq!(report.cli_flips[0].uuid, cli_drift.uuid);
        assert!(!report.cli_flips[0].new_value);
        assert_eq!(report.desktop.flag_flips.len(), 1);
        assert_eq!(report.desktop.flag_flips[0].uuid, desk_drift.uuid);
        assert!(!report.desktop.flag_flips[0].new_value);
    }

    // -- Login progress tests (C-1) -------------------------------------

    /// Recording sink — captures every `phase` / `error` call so tests
    /// can assert ordering and content. Thread-safe via `Mutex`.
    struct RecordingLoginSink {
        events: std::sync::Mutex<Vec<RecordedLoginEvent>>,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum RecordedLoginEvent {
        Phase(LoginPhase),
        Error(LoginPhase, String),
    }

    impl RecordingLoginSink {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn events(&self) -> Vec<RecordedLoginEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl LoginProgressSink for RecordingLoginSink {
        fn phase(&self, phase: LoginPhase) {
            self.events
                .lock()
                .unwrap()
                .push(RecordedLoginEvent::Phase(phase));
        }
        fn error(&self, phase: LoginPhase, msg: &str) {
            self.events
                .lock()
                .unwrap()
                .push(RecordedLoginEvent::Error(phase, msg.to_string()));
        }
    }

    /// Cancel before the subprocess finishes — assert sink saw
    /// `Spawning` then `WaitingForBrowser` then an `error` whose detail
    /// mentions cancellation.
    #[tokio::test]
    #[cfg(unix)]
    async fn test_login_cancel_emits_error_phase_with_cancelled_msg() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::sync::Notify;

        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let acct = insert_account(&store, "alice@example.com");

        // Stub binary that blocks for 30s — long enough for the test
        // to fire its Notify before exit.
        let stub_dir = tempfile::tempdir().expect("mk stub tempdir");
        let stub = stub_dir.path().join("claude-stub.sh");
        std::fs::write(&stub, "#!/bin/sh\nexec sleep 30\n").expect("write stub");
        let mut perms = std::fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).unwrap();

        let sink = Arc::new(RecordingLoginSink::new());
        let notify = Arc::new(Notify::new());
        let notify_clone = notify.clone();
        let sink_clone = Arc::clone(&sink);
        let stub_path = stub.clone();
        let store_arc = Arc::new(store);
        let store_handle = Arc::clone(&store_arc);
        let uuid = acct.uuid;

        let task = tokio::spawn(async move {
            login_and_reimport_with_progress_test_binary(
                &store_handle,
                uuid,
                &stub_path,
                Some(notify_clone),
                sink_clone.as_ref(),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(150)).await;
        notify.notify_one();

        let outcome = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancel should complete within 5s")
            .expect("join handle should not panic");
        assert!(outcome.is_err(), "expected RegisterError, got Ok");

        let events = sink.events();
        // Must have at least Spawning + WaitingForBrowser + error.
        assert_eq!(events[0], RecordedLoginEvent::Phase(LoginPhase::Spawning));
        assert_eq!(
            events[1],
            RecordedLoginEvent::Phase(LoginPhase::WaitingForBrowser)
        );
        match events.last().unwrap() {
            RecordedLoginEvent::Error(LoginPhase::WaitingForBrowser, msg) => {
                assert!(
                    msg.to_lowercase().contains("cancel"),
                    "error detail should mention cancellation; got: {msg}"
                );
            }
            other => panic!("expected error on WaitingForBrowser, got {other:?}"),
        }
    }

    /// `Spawning` fires before account validation; if the account is
    /// missing the sink must see `Spawning` then `error(Spawning)`.
    #[tokio::test]
    async fn test_login_progress_emits_spawning_then_error_for_unknown_account() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();

        let sink = RecordingLoginSink::new();
        let result =
            login_and_reimport_with_progress(&store, Uuid::new_v4(), None, &sink).await;
        assert!(matches!(result, Err(RegisterError::NotFound)));

        let events = sink.events();
        assert_eq!(events[0], RecordedLoginEvent::Phase(LoginPhase::Spawning));
        match events.last().unwrap() {
            RecordedLoginEvent::Error(LoginPhase::Spawning, _) => {}
            other => panic!("expected error on Spawning, got {other:?}"),
        }
    }

    // -- Verify-all progress tests (C-2) --------------------------------

    /// Recording sink for VerifyEvent — captures every event in order.
    struct RecordingVerifySink {
        events: std::sync::Mutex<Vec<VerifyEvent>>,
    }

    impl RecordingVerifySink {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn events(&self) -> Vec<VerifyEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl VerifyProgressSink for RecordingVerifySink {
        fn event(&self, ev: VerifyEvent) {
            self.events.lock().unwrap().push(ev);
        }
    }

    /// `swap::ProfileFetcher` mock — different from the inner
    /// `ProfileFetcher` mock used elsewhere in this module. Returns
    /// `email_for(token)` so verify can drive different outcomes per
    /// account.
    struct VerifyFetcher {
        emails: std::sync::Mutex<std::collections::HashMap<String, String>>,
    }

    impl VerifyFetcher {
        fn new() -> Self {
            Self {
                emails: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
        fn returns(self, _token_prefix: &str, email: &str) -> Self {
            // Mock all calls to return this email regardless of token.
            self.emails
                .lock()
                .unwrap()
                .insert("any".into(), email.into());
            self
        }
    }

    #[async_trait::async_trait]
    impl crate::cli_backend::swap::ProfileFetcher for VerifyFetcher {
        async fn fetch_email(&self, _access_token: &str) -> Result<String, OAuthError> {
            self.emails
                .lock()
                .unwrap()
                .get("any")
                .cloned()
                .ok_or_else(|| OAuthError::AuthFailed("no scripted email".into()))
        }
    }

    #[tokio::test]
    async fn test_verify_all_with_progress_emits_started_then_per_account_then_done() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();

        // Two accounts both with credentials and a fresh blob.
        let a1 = insert_account(&store, "alice@example.com");
        swap::save_private(a1.uuid, &fresh_blob_json()).unwrap();
        let a2 = insert_account(&store, "bob@example.com");
        swap::save_private(a2.uuid, &fresh_blob_json()).unwrap();

        // Simple fetcher that returns "alice@example.com" for every call —
        // a1 is Ok, a2 is Drift (label "bob" vs server "alice").
        let fetcher = VerifyFetcher::new().returns("any", "alice@example.com");

        let sink = RecordingVerifySink::new();
        verify_all_with_progress(&store, &fetcher, &sink).await;

        let events = sink.events();
        // First event is Started { total: 2 }.
        match &events[0] {
            VerifyEvent::Started { total } => assert_eq!(*total, 2),
            other => panic!("expected Started, got {other:?}"),
        }
        // Last event is Done.
        assert!(matches!(events.last(), Some(VerifyEvent::Done)));

        // Two Account events in between, indices 1 and 2.
        let account_events: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                VerifyEvent::Account { idx, total, outcome, .. } => Some((*idx, *total, *outcome)),
                _ => None,
            })
            .collect();
        assert_eq!(account_events.len(), 2);
        assert_eq!(account_events[0].0, 1);
        assert_eq!(account_events[0].1, 2);
        assert_eq!(account_events[1].0, 2);
        assert_eq!(account_events[1].1, 2);

        // Cleanup — drop the per-account credential files so the data
        // dir lock isn't polluted for siblings.
        let _ = swap::delete_private(a1.uuid);
        let _ = swap::delete_private(a2.uuid);
    }

    #[tokio::test]
    async fn test_verify_all_with_progress_skips_accounts_without_credentials() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();

        // alice has credentials.
        let a1 = insert_account(&store, "alice@example.com");
        swap::save_private(a1.uuid, &fresh_blob_json()).unwrap();
        // bob does NOT — flip the flag explicitly. No swap::save_private
        // here so the blob is genuinely absent on disk too.
        let mut acc = make_account("nocreds@example.com");
        acc.has_cli_credentials = false;
        store.insert(&acc).unwrap();

        let fetcher = VerifyFetcher::new().returns("any", "alice@example.com");
        let sink = RecordingVerifySink::new();
        verify_all_with_progress(&store, &fetcher, &sink).await;

        let events = sink.events();
        match &events[0] {
            // Only `alice` is eligible — `nocreds` was filtered out.
            VerifyEvent::Started { total } => assert_eq!(*total, 1),
            other => panic!("expected Started {{ total: 1 }}, got {other:?}"),
        }
        let account_count = events
            .iter()
            .filter(|e| matches!(e, VerifyEvent::Account { .. }))
            .count();
        assert_eq!(account_count, 1);

        // Cleanup
        for a in store.list().unwrap() {
            let _ = swap::delete_private(a.uuid);
        }
    }

    /// Stagger only fires BETWEEN calls — not before the first or after
    /// the last. With N=3 accounts the total elapsed time should be
    /// >= 2 * 200ms but the first event fires immediately.
    #[tokio::test]
    async fn test_verify_all_with_progress_uses_200ms_stagger_only_between_calls() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();

        let a1 = insert_account(&store, "a1@example.com");
        swap::save_private(a1.uuid, &fresh_blob_json()).unwrap();
        let a2 = insert_account(&store, "a2@example.com");
        swap::save_private(a2.uuid, &fresh_blob_json()).unwrap();
        let a3 = insert_account(&store, "a3@example.com");
        swap::save_private(a3.uuid, &fresh_blob_json()).unwrap();

        let fetcher = VerifyFetcher::new().returns("any", "a1@example.com");
        let sink = RecordingVerifySink::new();

        let start = std::time::Instant::now();
        verify_all_with_progress(&store, &fetcher, &sink).await;
        let elapsed = start.elapsed();

        // Two stagger gaps for 3 accounts → >= 400ms minimum.
        assert!(
            elapsed >= std::time::Duration::from_millis(380),
            "stagger should add ~400ms; elapsed={elapsed:?}"
        );
        // Sanity upper bound — we shouldn't sleep before the first or
        // after the last (would push elapsed above 600ms+jitter).
        assert!(
            elapsed < std::time::Duration::from_millis(900),
            "no extra stagger before first / after last; elapsed={elapsed:?}"
        );

        // Cleanup
        for a in store.list().unwrap() {
            let _ = swap::delete_private(a.uuid);
        }
    }
}
