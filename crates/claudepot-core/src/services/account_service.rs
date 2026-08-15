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

    // A cleared sentinel is indistinguishable from "not signed in" for
    // import purposes, and `NoCredentials` already says exactly that
    // with the right next step. Without this the empty bearer 401s and
    // the user gets "Couldn't confirm which account this login belongs
    // to: …" — a diagnostic about a login that isn't there.
    if blob.is_signed_out() {
        return Err(RegisterError::NoCredentials);
    }

    let prof = fetch_profile
        .fetch(&blob.claude_ai_oauth.access_token)
        .await
        .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?;

    register_account_from_profile(store, &blob_str, &prof).await
}

/// Trait for fetching an OAuth profile — enables testing without network.
#[async_trait::async_trait]
pub(crate) trait ProfileFetcher: Send + Sync {
    async fn fetch(&self, access_token: &str)
        -> Result<profile::Profile, crate::error::OAuthError>;
}

/// Production implementation that calls the Anthropic API.
/// `pub(crate)` so `launcher` can drive `resolve_cc_identity` with the
/// real profile fetcher when healing the active account's keychain token.
pub(crate) struct DefaultProfileFetcher;

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
async fn register_account_from_profile(
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
        .await
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
        let _ = swap::delete_private(account_id).await;
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
    register_account_from_profile(store, &blob_str, &prof).await
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
    sync_from_current_cc_with_probe(
        store,
        platform.as_ref(),
        &DefaultProfileFetcher,
        &crate::cli_backend::swap::DefaultRefresher,
        &crate::cli_backend::swap::DefaultLiveSessionProbe,
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
///
/// `pub(crate)` so `services::identity` can reuse it as the single
/// keychain-aware resolver for the *active* CLI account, instead of
/// rotating that account's stale private-slot copy (which orphans CC's
/// live session → forced re-login). See `verify_active_via_keychain`.
/// `probe` decides whether we may spend the account's single-use refresh
/// token — see the gate in [`resolve_cc_identity_locked`].
pub(crate) async fn resolve_cc_identity(
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
    probe: &dyn crate::cli_backend::swap::LiveSessionProbe,
) -> Result<Option<(String, String)>, RegisterError> {
    let _lock = swap::acquire_swap_lock().map_err(|e| {
        RegisterError::CredentialRead(format!("failed to acquire credential lock: {e}"))
    })?;
    resolve_cc_identity_locked(platform, fetch_profile, refresher, false, None, None, probe).await
}

/// Force-refresh the resolver for callers that already hold `swap`'s
/// cross-process credential lock.
pub(crate) async fn resolve_cc_identity_force_refresh_locked(
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
    expected_email: &str,
    expected_refresh_token: &str,
) -> Result<Option<(String, String)>, RegisterError> {
    resolve_cc_identity_locked(
        platform,
        fetch_profile,
        refresher,
        true,
        Some(expected_email),
        Some(expected_refresh_token),
        // Never consulted: the live-session gate is skipped entirely on
        // the forced path (see the gate in `resolve_cc_identity_locked`).
        &crate::cli_backend::swap::NoLiveSessionProbe,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn resolve_cc_identity_locked(
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
    force_refresh: bool,
    expected_email: Option<&str>,
    expected_refresh_token: Option<&str>,
    probe: &dyn crate::cli_backend::swap::LiveSessionProbe,
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
        Err(e) if force_refresh => {
            return Err(RegisterError::CredentialRead(format!(
                "CC credentials became unparseable during token heal: {e}"
            )))
        }
        Err(_) => return Ok(None), // Unparseable — leave it alone.
    };

    // CC's cleared-credentials sentinel. It parses, so without this it
    // flows into the profile fetch below with an empty bearer, 401s, and
    // ends up classified by whatever the 401 path decides — which, with
    // a live `claude` running, is the transient live-session skip. That
    // reported a state only a re-login can fix as a network blip the
    // user should wait out (issue #74).
    //
    // Checked before the first network call because an empty bearer
    // cannot succeed: there is no information a round-trip could add.
    if blob_t0.is_signed_out() {
        tracing::info!(
            "CC's stored credentials are empty (cleared-credentials sentinel) — signed out"
        );
        return Err(RegisterError::CcSignedOut);
    }

    if let Some(expected_refresh_token) = expected_refresh_token {
        if blob_t0.claude_ai_oauth.refresh_token != expected_refresh_token {
            return Err(RegisterError::CcChangedDuringRefresh);
        }
    }

    // Step 1 — happy path. Most calls end here. The forced path still
    // verifies the current identity, but deliberately continues to the
    // refresh even when /profile accepts the near-expiry access token.
    let initial_profile = fetch_profile
        .fetch(&blob_t0.claude_ai_oauth.access_token)
        .await;
    match initial_profile {
        Ok(prof) if force_refresh => {
            if let Some(expected_email) = expected_email {
                if !prof.email.eq_ignore_ascii_case(expected_email) {
                    return Err(RegisterError::CcChangedDuringRefresh);
                }
            }
        }
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
        Err(e) if force_refresh => {
            return Err(RegisterError::CredentialRead(format!(
                "CC credentials became unparseable during token heal: {e}"
            )))
        }
        Err(_) => return Ok(None),
    };

    // CC cleared itself between our snapshot and now. The same terminal
    // answer as the entry check — re-tested here because this is a
    // genuinely fresh read, and the reported failures landed within
    // minutes of an expiry, exactly when this window is widest.
    if latest_blob.is_signed_out() {
        tracing::info!("CC cleared its stored credentials during this pass — signed out");
        return Err(RegisterError::CcSignedOut);
    }

    let expected_token_changed = match expected_refresh_token {
        Some(expected_refresh_token) => {
            latest_blob.claude_ai_oauth.refresh_token != expected_refresh_token
        }
        None => false,
    };

    if let Some(expected_email) = expected_email {
        if expected_token_changed {
            // Another serialized healer may already have installed a fresh
            // token. Adopt it after verifying its identity instead of
            // spending a second single-use refresh token. A different
            // account, or an unverifiable live blob, fails closed.
            match fetch_profile
                .fetch(&latest_blob.claude_ai_oauth.access_token)
                .await
            {
                Ok(prof) if prof.email.eq_ignore_ascii_case(expected_email) => {
                    if !latest_blob.is_expired(300) {
                        return Ok(Some((latest_blob_str, prof.email)));
                    }
                }
                _ => return Err(RegisterError::CcChangedDuringRefresh),
            }
        }
    }

    if latest_blob_str != blob_str_t0 && !expected_token_changed {
        match fetch_profile
            .fetch(&latest_blob.claude_ai_oauth.access_token)
            .await
        {
            Ok(prof) if !force_refresh => return Ok(Some((latest_blob_str, prof.email))),
            Ok(prof) => {
                if let Some(expected_email) = expected_email {
                    if !prof.email.eq_ignore_ascii_case(expected_email) {
                        return Err(RegisterError::CcChangedDuringRefresh);
                    }
                }
                if !latest_blob.is_expired(300) {
                    return Ok(Some((latest_blob_str, prof.email)));
                }
            }
            Err(OAuthError::AuthFailed(_)) => {}
            Err(e) => return Err(RegisterError::ProfileFetch(e.to_string())),
        }
    }

    // Step 3 — refresh.
    //
    // Live-session gate. Refresh tokens are single-use, and CC holds its
    // copy in memory and writes it back to the keychain on its own
    // schedule. If we rotate while a `claude` process is alive, CC keeps
    // a token the server has already retired: its next refresh fails and
    // the user is forced to re-login. That is precisely the "accounts
    // need re-login too often" bug, reached from the two observational
    // callers — `verify_active_via_keychain` (Verify all) and
    // `sync_from_current_cc` (GUI window focus, `account list`,
    // `cli status`). `swap::switch` already gates on this same condition
    // for the same reason; this closes the matching hole on the refresh
    // paths.
    //
    // Skipping is self-healing: CC refreshes its own token on its next
    // request, so the following pass verifies normally. Callers map this
    // to a transient "couldn't confirm", never to a re-login prompt.
    //
    // The forced path (`launcher`, Mode D) is deliberately exempt — it is
    // a user-initiated launch that needs a fresh token in hand, and it
    // CAS-guards on `expected_refresh_token`.
    //
    // Probed lazily here rather than at entry so the happy path (token
    // still valid → Step 1 returns early) never pays the subprocess scan.
    //
    // Ordering is load-bearing: the empty-refresh-token check runs
    // BEFORE the gate. The gate's entire rationale is "don't retire a
    // single-use token CC still holds in memory" — with no token there
    // is nothing to retire and nothing to protect, so consulting it
    // first converts a terminal state into a transient one and hands
    // the user a "wait for it to clear" that never clears. That
    // inversion is the reported bug: the state was reported correctly
    // only when nobody was using Claude Code.
    // Reaching here means the access token was non-empty and the server
    // refused it — `is_signed_out()` already returned above for the
    // cleared sentinel. So this is `AuthRejected`, not `CcSignedOut`:
    // a request WAS made and answered. `CcSignedOut` asserts CC cleared
    // its own credentials and the account is untouched, and neither is
    // established here. The check still runs BEFORE the live-session
    // gate — that ordering is the fix; only the label differs.
    if !latest_blob.can_refresh() {
        tracing::info!(
            "CC's stored refresh token is empty — nothing to exchange, re-login required"
        );
        return Err(RegisterError::AuthRejected);
    }

    if !force_refresh && probe.is_cc_running().await {
        return Err(RegisterError::CcLiveRefreshSkipped);
    }

    // Use the LATEST refresh_token, not the one we originally snapshot.
    // If a concurrent writer already rotated, our original refresh_token
    // is dead server-side and calling refresh with it would produce a
    // false AuthRejected.
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
            return Err(RegisterError::ProfileFetch(format!("token refresh: {e}")));
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

    if let (Some(expected_email), Some(expected_refresh_token)) =
        (expected_email, expected_refresh_token)
    {
        let live_blob_str = pre_write
            .as_deref()
            .ok_or(RegisterError::CcChangedDuringRefresh)?;
        let live_blob = CredentialBlob::from_json(live_blob_str)
            .map_err(|_| RegisterError::CcChangedDuringRefresh)?;
        // CC signed itself out while we were refreshing. Terminal, and
        // it must be said so here: without this the sentinel's empty
        // bearer goes to `/profile`, 401s, and falls into the `_` arm
        // below as `CcChangedDuringRefresh` — a "retry, nothing was
        // lost" answer for a state no retry can fix.
        if live_blob.is_signed_out() {
            return Err(RegisterError::CcSignedOut);
        }
        if live_blob.claude_ai_oauth.refresh_token != expected_refresh_token {
            match fetch_profile
                .fetch(&live_blob.claude_ai_oauth.access_token)
                .await
            {
                Ok(prof) if prof.email.eq_ignore_ascii_case(expected_email) => {
                    return Ok(Some((live_blob_str.to_string(), prof.email)));
                }
                _ => return Err(RegisterError::CcChangedDuringRefresh),
            }
        }
    }

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
    // Same sentinel check as the forced path above, for the same
    // reason. This is the branch the original fix missed: a CAS miss
    // whose winning writer was CC clearing itself reported
    // `CcChangedDuringRefresh` → `NetworkError` → "couldn't confirm,
    // wait" — the identical misreport the whole change removes, one
    // branch over.
    if live_blob.is_signed_out() {
        return Err(RegisterError::CcSignedOut);
    }
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

/// Testable seam. Uses [`NoLiveSessionProbe`], so the refresh branch
/// stays deterministically reachable regardless of what is running on
/// the test machine.
///
/// **Production must call [`sync_from_current_cc`]**, which supplies the
/// real probe. Reaching the refresh with a live `claude` running rotates
/// a token CC still holds and forces a re-login — hence `#[cfg(test)]`,
/// so this probe-less shape can never be wired into production.
///
/// [`NoLiveSessionProbe`]: crate::cli_backend::swap::NoLiveSessionProbe
#[cfg(test)]
pub(crate) async fn sync_from_current_cc_with(
    store: &AccountStore,
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
) -> Result<Option<Uuid>, RegisterError> {
    sync_from_current_cc_with_probe(
        store,
        platform,
        fetch_profile,
        refresher,
        &crate::cli_backend::swap::NoLiveSessionProbe,
    )
    .await
}

/// [`sync_from_current_cc_with`] with an injectable live-session probe.
pub(crate) async fn sync_from_current_cc_with_probe(
    store: &AccountStore,
    platform: &dyn cli_backend::CliPlatform,
    fetch_profile: &dyn ProfileFetcher,
    refresher: &dyn crate::cli_backend::swap::TokenRefresher,
    probe: &dyn crate::cli_backend::swap::LiveSessionProbe,
) -> Result<Option<Uuid>, RegisterError> {
    let (effective_blob_str, email) =
        match resolve_cc_identity(platform, fetch_profile, refresher, probe).await? {
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
    let needs_write = match swap::load_private(account.uuid).await {
        Ok(stored) => stored != effective_blob_str,
        Err(_) => true,
    };
    if needs_write {
        swap::save_private(account.uuid, &effective_blob_str)
            .await
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
    if let Err(e) = swap::delete_private(uuid).await {
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
///
/// Async because `swap::load_private` is async — every credential
/// read goes through the macOS keychain subprocess (or file storage
/// on other platforms), and both branches need to be reachable
/// without blocking the tokio worker that called us.
pub async fn token_health(uuid: Uuid, has_credentials: bool) -> TokenHealth {
    if !has_credentials {
        return TokenHealth {
            status: "no credentials".into(),
            remaining_mins: None,
        };
    }
    match swap::load_private(uuid).await {
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
    /// A live `claude` process is running, so we declined to spend the
    /// account's single-use refresh token. CC keeps its refresh token in
    /// memory and writes it back to the keychain; rotating it here would
    /// leave CC holding a retired token and force a re-login on its next
    /// refresh. Strictly **transient** — CC refreshes on its own next
    /// request, so the following pass succeeds. Callers must map this to
    /// a "couldn't confirm this pass" state, never to a re-login prompt.
    #[error(
        "a live Claude Code session is running — skipped token refresh to avoid signing it out"
    )]
    CcLiveRefreshSkipped,
    /// CC's keychain holds its **cleared-credentials sentinel**: the blob
    /// parses, but both tokens are empty (see
    /// [`CredentialBlob::is_signed_out`]). Claude Code signed *itself*
    /// out without deleting the item.
    ///
    /// Terminal — only a re-login recovers it — and deliberately not
    /// [`AuthRejected`]. Nothing was rejected: with an empty bearer no
    /// request was ever sent, so a sentence about the server refusing
    /// the login would describe an event that did not happen and send
    /// the user to check an account that is fine.
    ///
    /// [`CredentialBlob::is_signed_out`]: crate::blob::CredentialBlob::is_signed_out
    /// [`AuthRejected`]: RegisterError::AuthRejected
    #[error("Claude Code signed itself out — its stored credentials are empty, so log in again")]
    CcSignedOut,
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// The three variants below the fold are the reason this enum could not
/// keep riding one generic `accounts.register_failed` code carrying only
/// `{{detail}}` — each needs a different sentence and a different button:
///
/// - **`AuthRejected` is the only one that means "sign in again".** Both
///   the access token and its paired refresh token were refused, so no
///   retry recovers it.
/// - **`CcChangedDuringRefresh` means "retry", not "re-login".** CC's
///   keychain blob moved under our CAS write; nothing was lost.
/// - **`CcLiveRefreshSkipped` is a deliberate refusal, not a failure.**
///   Spending the single-use refresh token while `claude` is live would
///   sign that session out. It is strictly transient, and a UI that
///   renders it as a re-login prompt would be actively harmful.
/// - **`CcSignedOut` is terminal but blameless.** CC cleared its own
///   credentials; nothing was refused and nothing is wrong with the
///   account. It says "log in again" without the "your login was
///   rejected" story that `AuthRejected` carries.
///
/// `AlreadyRegistered` crosses the account's email and uuid rather than
/// one pre-composed English clause: the GUI names the existing account
/// and offers to open it, which the CLI's `(uuid: …)` suffix cannot do.
/// Emails are user-facing identity here, never a secret — the same
/// judgement `cli_backend::SwapError::IdentityMismatch` already makes.
impl crate::error_code::ErrorCode for RegisterError {
    fn code(&self) -> &'static str {
        match self {
            RegisterError::NoCredentials => "account_register.no_credentials",
            RegisterError::CredentialRead(_) => "account_register.credential_read",
            RegisterError::CredentialWrite(_) => "account_register.credential_write",
            RegisterError::ProfileFetch(_) => "account_register.profile_fetch",
            RegisterError::AlreadyRegistered(_, _) => "account_register.already_registered",
            RegisterError::Store(_) => "account_register.store",
            RegisterError::NotFound => "account_register.not_found",
            RegisterError::AuthRejected => "account_register.auth_rejected",
            RegisterError::CcChangedDuringRefresh => "account_register.cc_changed_during_refresh",
            RegisterError::CcLiveRefreshSkipped => "account_register.cc_live_refresh_skipped",
            RegisterError::CcSignedOut => "account_register.cc_signed_out",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            RegisterError::NoCredentials
            | RegisterError::NotFound
            | RegisterError::AuthRejected
            | RegisterError::CcChangedDuringRefresh
            | RegisterError::CcLiveRefreshSkipped
            | RegisterError::CcSignedOut => serde_json::json!({}),
            RegisterError::CredentialRead(detail)
            | RegisterError::CredentialWrite(detail)
            | RegisterError::ProfileFetch(detail)
            | RegisterError::Store(detail) => serde_json::json!({ "detail": detail }),
            RegisterError::AlreadyRegistered(email, uuid) => {
                serde_json::json!({ "email": email, "uuid": uuid.to_string() })
            }
        }
    }
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
    // After success, CC holds fresh credentials: read blob → fetch
    // profile → verify identity → persist, emitting per-step progress
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
    let prof = match fetch_profile
        .fetch(&blob.claude_ai_oauth.access_token)
        .await
    {
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
    if let Err(e) = swap::save_private(account_id, &blob_str).await {
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
            let msg = format!(
                "already registered: {} (uuid: {})",
                existing.email, existing.uuid
            );
            progress.error(LoginPhase::VerifyingIdentity, &msg);
            return Err(RegisterError::AlreadyRegistered(
                existing.email,
                existing.uuid,
            ));
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
    if let Err(e) = swap::save_private(account_id, &blob_str).await {
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
        let _ = swap::delete_private(account_id).await;
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
    SignedOut,
    NetworkError,
}

impl VerifyOutcomeKind {
    /// The wire string for this kind — the same token
    /// `VerifyOutcome::as_str` produces and the TypeScript
    /// `VerifyOutcomeKind` union spells.
    ///
    /// Exists so no surface has to re-spell them. `src-tauri`'s
    /// progress sink kept its own `match` producing string literals,
    /// which meant every new variant needed a hand edit in a second
    /// place to avoid emitting a status the frontend cannot parse.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Drift => "drift",
            Self::Rejected => "rejected",
            Self::SignedOut => "signed_out",
            Self::NetworkError => "network_error",
        }
    }
}

impl From<&crate::account::VerifyOutcome> for VerifyOutcomeKind {
    fn from(v: &crate::account::VerifyOutcome) -> Self {
        match v {
            crate::account::VerifyOutcome::Ok { .. } => Self::Ok,
            crate::account::VerifyOutcome::Drift { .. } => Self::Drift,
            crate::account::VerifyOutcome::Rejected => Self::Rejected,
            crate::account::VerifyOutcome::SignedOut => Self::SignedOut,
            crate::account::VerifyOutcome::NetworkError => Self::NetworkError,
        }
    }
}

#[cfg(test)]
mod verify_outcome_kind_tests {
    use super::VerifyOutcomeKind;
    use crate::account::VerifyOutcome;

    /// The lifted kind and the full outcome must agree on every wire
    /// string. They are separate types with separate `as_str`s — the
    /// kind is what crosses to the webview — so this is the only thing
    /// stopping the two from drifting into statuses the frontend
    /// silently fails to match.
    #[test]
    fn kind_and_outcome_agree_on_every_wire_string() {
        for outcome in [
            VerifyOutcome::Ok {
                email: "a@b.c".into(),
            },
            VerifyOutcome::Drift {
                stored_email: "a@b.c".into(),
                actual_email: "d@e.f".into(),
            },
            VerifyOutcome::Rejected,
            VerifyOutcome::SignedOut,
            VerifyOutcome::NetworkError,
        ] {
            assert_eq!(VerifyOutcomeKind::from(&outcome).as_str(), outcome.as_str());
        }
    }

    /// `serde` renames to snake_case, and the frontend parses the
    /// serialized form — so `as_str` has to match what serde emits, not
    /// merely look like it.
    #[test]
    fn as_str_matches_the_serialized_form() {
        for kind in [
            VerifyOutcomeKind::Ok,
            VerifyOutcomeKind::Drift,
            VerifyOutcomeKind::Rejected,
            VerifyOutcomeKind::SignedOut,
            VerifyOutcomeKind::NetworkError,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
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

        let (outcome_kind, detail) =
            match crate::services::identity::verify_account_identity(store, uuid, fetcher).await {
                Ok(outcome) => {
                    let kind = VerifyOutcomeKind::from(&outcome);
                    let detail = match &outcome {
                        crate::account::VerifyOutcome::Drift { actual_email, .. } => {
                            Some(format!("actual: {actual_email}"))
                        }
                        crate::account::VerifyOutcome::Rejected => {
                            Some("re-login required".to_string())
                        }
                        crate::account::VerifyOutcome::SignedOut => {
                            Some("Claude Code signed itself out".to_string())
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
pub async fn reconcile_cli_flags(store: &AccountStore) -> Result<Vec<CliFlagFlip>, ReconcileError> {
    let accounts = store
        .list()
        .map_err(|e| ReconcileError::Store(e.to_string()))?;
    let mut flips = Vec::new();
    for a in &accounts {
        // Probe the keychain directly. A present, parseable blob is the
        // truth the DB flag should mirror — anything else (missing,
        // unreadable, malformed JSON) means the swap can't succeed and
        // the flag should be `false`.
        let truth = match swap::load_private(a.uuid).await {
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
pub async fn reconcile_all(store: &AccountStore) -> Result<ReconcileReport, ReconcileError> {
    let cli_flips = reconcile_cli_flags(store).await?;
    let desktop = super::desktop_service::reconcile_flags(store)
        .map_err(|e| ReconcileError::Store(e.to_string()))?;
    Ok(ReconcileReport { cli_flips, desktop })
}

#[cfg(test)]
#[path = "account_service_tests.rs"]
mod tests;
