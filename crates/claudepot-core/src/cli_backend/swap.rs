//! Mode A atomic swap primitive for CLI credentials.
//! See reference.md §I.7.

use super::claude_json;
use super::storage;
use super::CliPlatform;
use crate::account::AccountStore;
use crate::error::{OAuthError, SwapError};
use crate::oauth::refresh::TokenResponse;
use std::fs;
use uuid::Uuid;

/// Abstraction over token refresh — enables testing without network calls.
#[async_trait::async_trait]
pub trait TokenRefresher: Send + Sync {
    async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, OAuthError>;
}

/// Production refresher that calls the Anthropic token endpoint.
pub struct DefaultRefresher;

#[async_trait::async_trait]
impl TokenRefresher for DefaultRefresher {
    async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, OAuthError> {
        crate::oauth::refresh::refresh(refresh_token).await
    }
}

/// Abstraction over OAuth profile lookup — enables testing without network calls.
/// The returned string is the email the server associates with the access token.
#[async_trait::async_trait]
pub trait ProfileFetcher: Send + Sync {
    async fn fetch_email(&self, access_token: &str) -> Result<String, OAuthError>;

    /// Full profile, for callers that need more than the email (e.g.
    /// rewriting the `oauthAccount` block in `~/.claude.json`).
    /// Default implementation degrades to a minimal Profile with only
    /// the email populated — test mocks that only implement
    /// `fetch_email` still work, but the oauthAccount rewrite gets
    /// empty strings for org_name, uuids, etc.
    async fn fetch_profile(
        &self,
        access_token: &str,
    ) -> Result<crate::oauth::profile::Profile, OAuthError> {
        let email = self.fetch_email(access_token).await?;
        Ok(crate::oauth::profile::Profile {
            email,
            org_uuid: String::new(),
            org_name: String::new(),
            subscription_type: String::new(),
            rate_limit_tier: None,
            account_uuid: String::new(),
            display_name: None,
        })
    }
}

/// Production profile fetcher that calls `/api/oauth/profile`.
pub struct DefaultProfileFetcher;

#[async_trait::async_trait]
impl ProfileFetcher for DefaultProfileFetcher {
    async fn fetch_email(&self, access_token: &str) -> Result<String, OAuthError> {
        self.fetch_profile(access_token).await.map(|p| p.email)
    }
    async fn fetch_profile(
        &self,
        access_token: &str,
    ) -> Result<crate::oauth::profile::Profile, OAuthError> {
        crate::oauth::profile::fetch(access_token).await
    }
}

/// Verify that a credential blob actually represents `expected_email`.
/// Parses the blob, fetches the profile via its access token, compares emails.
///
/// Returns:
/// - `Ok(())` if the blob's access token resolves to `expected_email`.
/// - `Ok(())` if the blob isn't a recognisable credentials JSON (degrades
///   open — CC's real format is always JSON in production; this branch exists
///   so storage-mechanics tests using opaque placeholder blobs don't need to
///   fabricate full credentials).
/// - `Err(IdentityMismatch)` on a verified mismatch (the corruption case).
/// - `Err(IdentityVerificationFailed)` on network / server errors.
///
/// NOTE on deliberate asymmetry with `services::identity::
/// verify_account_identity`: this function **does NOT refresh** the
/// access_token on a 401 response. Refreshing inside `swap` would mean
/// the act of switching accounts could silently rotate a token as a
/// side effect — undesirable both for observability (the user didn't
/// ask for a refresh) and for correctness (swap already holds the swap
/// lock; a refresh inside that lock widens the critical section and
/// creates TOCTOU surface vs. whoever holds the rotated blob's
/// refresh_token). The refresh-aware path lives in
/// `services::identity::verify_account_identity_with`, invoked from
/// reconciliation passes, which is the intended recovery mechanism for
/// an expired-but-refreshable blob that swap rejects.
async fn verify_blob_identity(
    blob_str: &str,
    expected_email: &str,
    fetcher: &dyn ProfileFetcher,
) -> Result<(), SwapError> {
    let blob = crate::blob::CredentialBlob::from_json(blob_str).map_err(|e| {
        // An unparseable blob cannot be verified; the conservative
        // answer is to REJECT, not to skip silently. Previously this
        // returned Ok(()), which allowed a corrupted/non-JSON blob to
        // pass both the pre-write and post-write verification gates
        // and then be marked active in the DB — defeating the core
        // invariant that `switch` only succeeds after identity
        // verification.
        tracing::warn!("rejecting swap: blob is not recognisable credential JSON ({e})");
        SwapError::IdentityVerificationFailed(format!("blob parse failed: {e}"))
    })?;
    let actual = fetcher
        .fetch_email(&blob.claude_ai_oauth.access_token)
        .await
        .map_err(|e| SwapError::IdentityVerificationFailed(e.to_string()))?;
    if actual.eq_ignore_ascii_case(expected_email) {
        Ok(())
    } else {
        Err(SwapError::IdentityMismatch {
            stored_email: expected_email.to_string(),
            actual_email: actual,
        })
    }
}

/// Detect whether a Claude Code CLI process is currently running. If
/// one is, its in-memory refresh token will eventually overwrite
/// whatever we put in the keychain — making the swap silently revert.
///
/// Detection: `pgrep -x claude` on Unix (exact binary match — avoids
/// false positives on "claude-desktop", grep pipelines, etc.). On
/// non-Unix platforms: best-effort skip (returns false) since the
/// primary risk is macOS/Linux where pgrep exists.
/// Public wrapper around the live-session detector so the CLI's
/// post-swap warning path can re-check without duplicating the
/// `ps`/`pgrep` logic. Same cost as the gate check (one subprocess).
pub async fn is_cc_process_running_public() -> bool {
    is_cc_process_running().await
}

pub(crate) async fn is_cc_process_running() -> bool {
    #[cfg(unix)]
    {
        // macOS `pgrep` has trouble matching Mach-O binaries installed
        // via symlink (e.g. ~/.local/bin/claude → versioned binary).
        // `ps -axco comm` reliably lists the short process name and
        // pipe-to-grep is simple enough for a pre-swap gate check.
        let output = tokio::process::Command::new("ps")
            .args(["-axco", "comm"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await;
        match output {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                text.lines().any(|l| l.trim() == "claude")
            }
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Acquire an exclusive file lock for swap operations.
/// Returns the locked file handle — lock is released when dropped.
fn acquire_swap_lock() -> Result<fs::File, SwapError> {
    let lock_path = crate::paths::claudepot_data_dir().join(".swap.lock");
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        // Blocking exclusive lock — waits if another swap is in progress.
        let ret = unsafe { libc::flock(fd, libc::LOCK_EX) };
        if ret != 0 {
            return Err(SwapError::FileError(std::io::Error::last_os_error()));
        }
    }
    #[cfg(windows)]
    {
        // On Windows, opening with write + no sharing provides exclusion.
        // The OpenOptions above already provide this behavior.
    }

    Ok(file)
}

/// Conditionally refresh the credential blob if it is expired or expiring
/// within a 5-minute margin. Returns the (possibly refreshed) blob JSON string.
/// On refresh, the new blob is persisted to private storage.
pub(crate) async fn maybe_refresh_blob(
    blob_str: &str,
    account_id: Uuid,
    refresher: &dyn TokenRefresher,
) -> Result<String, SwapError> {
    let blob = crate::blob::CredentialBlob::from_json(blob_str)
        .map_err(|e| SwapError::CorruptBlob(e.to_string()))?;

    if !blob.is_expired(300) {
        return Ok(blob_str.to_string());
    }

    tracing::info!("token expired or expiring soon, refreshing...");
    let token_resp = refresher
        .refresh(&blob.claude_ai_oauth.refresh_token)
        .await
        .map_err(|e| SwapError::RefreshFailed(e.to_string()))?;

    let new_blob = crate::oauth::refresh::build_blob(&token_resp, Some(&blob));
    storage::save(account_id, &new_blob)?;
    Ok(new_blob)
}

/// Swap the active CLI account from `current_id` to `target_id`.
///
/// Acquires an exclusive file lock to prevent concurrent swaps.
///
/// Verifies identity at both boundaries via `fetcher` before any storage
/// mutation, so a divergence between Claudepot's view and CC's actual
/// credentials is caught before it corrupts a blob slot.
///
/// 1. Load target blob + (optionally) refresh.
/// 2. Verify target blob's email matches the stored account's email.
/// 3. Read CC's current blob. Verify it matches `current_id`'s stored email.
/// 4. Save outgoing to private storage.
/// 5. Write target to CC storage (with rollback on failure).
/// 6. Update active pointer in the DB.
pub async fn switch(
    store: &AccountStore,
    current_id: Option<Uuid>,
    target_id: Uuid,
    platform: &dyn CliPlatform,
    auto_refresh: bool,
    refresher: &dyn TokenRefresher,
    fetcher: &dyn ProfileFetcher,
) -> Result<(), SwapError> {
    switch_inner(
        store, current_id, target_id, platform, auto_refresh, false,
        claude_json::default_path().as_deref(),
        refresher, fetcher,
    )
    .await
}

/// Like [`switch`] but bypasses the live-session gate. The `--force`
/// flag in the CLI and the "Force switch" button in the GUI use this.
pub async fn switch_force(
    store: &AccountStore,
    current_id: Option<Uuid>,
    target_id: Uuid,
    platform: &dyn CliPlatform,
    auto_refresh: bool,
    refresher: &dyn TokenRefresher,
    fetcher: &dyn ProfileFetcher,
) -> Result<(), SwapError> {
    switch_inner(
        store, current_id, target_id, platform, auto_refresh, true,
        claude_json::default_path().as_deref(),
        refresher, fetcher,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn switch_inner(
    store: &AccountStore,
    current_id: Option<Uuid>,
    target_id: Uuid,
    platform: &dyn CliPlatform,
    auto_refresh: bool,
    force: bool,
    claude_json_path: Option<&std::path::Path>,
    refresher: &dyn TokenRefresher,
    fetcher: &dyn ProfileFetcher,
) -> Result<(), SwapError> {
    // Gate: refuse if a CC process is running — its in-memory refresh
    // token will overwrite the keychain on the next token refresh,
    // silently reverting the swap.
    //
    // Cheap pre-check BEFORE acquiring the swap lock so obvious
    // conflicts fail fast without contending on the lock.
    if !force && is_cc_process_running().await {
        return Err(SwapError::LiveSessionConflict);
    }

    // Acquire exclusive lock — prevents concurrent swaps.
    tracing::debug!("acquiring swap lock...");
    let _lock = acquire_swap_lock()?;
    tracing::debug!("swap lock acquired");

    // Re-check AFTER acquiring the lock. Another swap could have been
    // holding the lock while a CC process launched; the pre-lock check
    // above doesn't cover the window between "pre-check passed" and
    // "we own the lock". Without this second check, `force=false` can
    // still mutate credentials while CC is live, losing the guarantee.
    if !force && is_cc_process_running().await {
        return Err(SwapError::LiveSessionConflict);
    }

    // Load target blob from Claudepot private storage first.
    // If it doesn't exist, fail before touching anything.
    tracing::debug!(target = %target_id, "loading target credentials");
    let target_blob = storage::load(target_id)?;

    // Conditionally refresh if expired/expiring and auto_refresh is enabled.
    let target_blob = if auto_refresh {
        maybe_refresh_blob(&target_blob, target_id, refresher).await?
    } else {
        target_blob
    };

    // Verify target blob actually belongs to target_id's stored email. If it
    // doesn't, the slot is mis-filed — abort before writing to CC.
    let target_email = store
        .find_by_uuid(target_id)
        .map_err(|e| SwapError::WriteFailed(format!("db lookup failed: {e}")))?
        .ok_or_else(|| SwapError::WriteFailed(format!("target {target_id} not in DB")))?
        .email;
    tracing::debug!(target = %target_id, "verifying target blob identity");
    verify_blob_identity(&target_blob, &target_email, fetcher).await?;

    // Save outgoing (current CC blob may have been refreshed by the CLI).
    //
    // If the outgoing check detects drift — CC holds a blob for a DIFFERENT
    // account than DB's active_cli — we can't safely cache it under `cur`'s
    // slot (that's the mis-filing corruption). Instead of aborting the
    // swap, we log + skip the backup save. The target-blob check still
    // runs unconditionally and will abort on a real target mismatch. Net
    // effect: drift is self-healing, never silently corrupting.
    // Capture CC's shared slot state ONCE up front. Used twice below:
    // (a) outgoing-backup identity check + conditional backup save; and
    // (b) post-switch rollback so we can restore the exact pre-write
    // contents even when `current_id` is None or its private slot was
    // empty. Without this capture, a post-switch mismatch had no way
    // to put CC back the way it was.
    let pre_write_default = platform.read_default().await?;

    if let Some(cur) = current_id {
        if let Some(current_blob) = pre_write_default.as_deref() {
            let cur_email = store
                .find_by_uuid(cur)
                .map_err(|e| SwapError::WriteFailed(format!("db lookup failed: {e}")))?
                .ok_or_else(|| SwapError::WriteFailed(format!("current {cur} not in DB")))?
                .email;
            tracing::debug!(current = %cur, "verifying outgoing blob identity");

            let skip_backup = match verify_blob_identity(current_blob, &cur_email, fetcher).await {
                Ok(()) => false,
                Err(SwapError::IdentityMismatch { actual_email, .. }) => {
                    tracing::warn!(
                        "CC is currently signed in as {actual_email}, not {cur_email} \
                         (Claudepot's last-known active CLI). Skipping the outgoing backup \
                         to avoid mis-filing; proceeding with the target swap."
                    );
                    true
                }
                Err(SwapError::IdentityVerificationFailed(reason)) => {
                    // Unparseable or otherwise-unverifiable outgoing blob.
                    // Same treatment as IdentityMismatch: we cannot safely
                    // attribute this blob to `cur`'s slot, so skip the
                    // backup step rather than corrupt private storage.
                    // The target-blob verify already ran and succeeded,
                    // so the target swap itself is still safe to proceed.
                    tracing::warn!(
                        "outgoing blob for {cur_email} is unverifiable ({reason}); \
                         skipping backup to avoid mis-filing"
                    );
                    true
                }
                Err(other) => return Err(other),
            };

            let previous_private = storage::load_opt(cur);
            if !skip_backup {
                storage::save(cur, current_blob)?;
            }

            // Write target to CC storage.
            if let Err(e) = platform.write_default(&target_blob).await {
                // Rollback: restore previous Claudepot blob for outgoing account.
                if !skip_backup {
                    match previous_private {
                        Some(prev) => {
                            let _ = storage::save(cur, &prev);
                        }
                        None => {
                            let _ = storage::delete(cur);
                        }
                    }
                }
                return Err(e);
            }
        } else {
            // No current blob in CC — just write target directly.
            platform.write_default(&target_blob).await?;
        }
    } else {
        platform.write_default(&target_blob).await?;
    }

    // Bump mtime for cross-process invalidation (best-effort).
    let _ = platform.touch_credfile().await;

    // Post-switch verification: read CC's shared slot back, call /profile,
    // confirm the identity matches the target's label. If we can't
    // verify (mismatch OR unreadable OR empty after write) we roll back
    // to `pre_write_default` and return an error — reporting "switched"
    // only after a confirmed verification.
    //
    // Rollback helper: restore pre_write_default if we captured one;
    // otherwise best-effort writes an empty blob. CliPlatform has no
    // `clear_default` so fully rolling back to "no credentials at all"
    // isn't possible from here — log and leave CC in whatever state
    // write_default("") produces.
    let rollback = || async {
        match pre_write_default.as_deref() {
            Some(prev) => {
                if let Err(e) = platform.write_default(prev).await {
                    tracing::error!(
                        target = %target_id,
                        "post-switch rollback write_default failed: {e}"
                    );
                }
            }
            None => {
                // Pre-write state was empty — restore that cleanly via
                // clear_default. Without this, our write_default left
                // the target blob in a slot that should be empty.
                if let Err(e) = platform.clear_default().await {
                    tracing::error!(
                        target = %target_id,
                        "post-switch rollback clear_default failed: {e}"
                    );
                }
            }
        }
    };

    let target_email = store
        .find_by_uuid(target_id)
        .map_err(|e| SwapError::WriteFailed(format!("db lookup failed: {e}")))?
        .ok_or_else(|| SwapError::WriteFailed(format!("target {target_id} not in DB")))?
        .email;

    match platform.read_default().await {
        Ok(Some(after_blob)) => {
            if let Err(e) = verify_blob_identity(&after_blob, &target_email, fetcher).await {
                tracing::error!(
                    target = %target_id,
                    "post-switch identity check failed: {e}"
                );
                rollback().await;
                return Err(e);
            }
        }
        Ok(None) => {
            tracing::error!(
                target = %target_id,
                "post-switch read-back returned None — CC's slot is empty after write_default claimed success"
            );
            rollback().await;
            return Err(SwapError::IdentityVerificationFailed(
                "post-switch read-back returned empty slot".into(),
            ));
        }
        Err(e) => {
            tracing::error!(
                target = %target_id,
                "post-switch read-back errored: {e}"
            );
            rollback().await;
            return Err(SwapError::IdentityVerificationFailed(format!(
                "post-switch read-back failed: {e}"
            )));
        }
    }

    // Rewrite ~/.claude.json's oauthAccount block to match the new
    // target. CC reads this file for its user-visible identity
    // (`claude auth status`, in-app displays). Without this step, the
    // keychain holds the new token but CC still reports the old
    // account — a silent half-swap that looks broken from the user's
    // side.
    //
    // Best-effort: if we can't fetch the full profile (network) or
    // can't find ~/.claude.json (fresh install), we log + continue.
    // The keychain swap has already succeeded; blocking on this
    // cosmetic fix would be more disruptive than the stale display.
    //
    // Tests pass `claude_json_path = None` so they don't scribble on
    // the real user file via dirs::home_dir().
    let prior_oauth_account =
        claude_json_path.and_then(|p| claude_json::read_oauth_account(p).ok().flatten());

    if let Some(cj_path) = claude_json_path {
        // Fetch the target's full profile via the already-verified
        // access token. We re-read the blob rather than plumb the
        // target_blob through — keeps the post-verify code unchanged.
        let cj_update_result: Result<(), SwapError> = async {
            let target_after = platform
                .read_default()
                .await?
                .ok_or_else(|| SwapError::IdentityVerificationFailed(
                    "post-write slot empty during oauthAccount rewrite".into(),
                ))?;
            let blob = crate::blob::CredentialBlob::from_json(&target_after)
                .map_err(|e| SwapError::CorruptBlob(e.to_string()))?;
            let profile = fetcher
                .fetch_profile(&blob.claude_ai_oauth.access_token)
                .await
                .map_err(|e| SwapError::IdentityVerificationFailed(e.to_string()))?;
            claude_json::update_oauth_account(cj_path, &profile)?;
            Ok(())
        }
        .await;

        if let Err(e) = cj_update_result {
            // Don't roll back the successful keychain swap — the user
            // would see the new token stop working AND the old
            // displayed identity. Better to log and leave
            // oauthAccount stale; `sync_from_current_cc` on the next
            // run will still correct claudepot's DB pointer.
            tracing::warn!(
                "oauthAccount rewrite failed ({e}); keychain swap kept. \
                 `claude auth status` may display the old account until \
                 ~/.claude.json is updated manually or by the next swap."
            );
        }
    }

    // Update active pointer in account store.
    tracing::debug!(target = %target_id, "updating active CLI pointer");
    if let Err(e) = store.set_active_cli(target_id) {
        // Best-effort rollback: restore previous CC credentials AND
        // the prior oauthAccount block so the two stay consistent.
        if let Some(cur) = current_id {
            if let Ok(prev_blob) = storage::load(cur) {
                let _ = platform.write_default(&prev_blob).await;
            }
        }
        if let Some(cj_path) = claude_json_path {
            let _ = claude_json::restore_oauth_account(
                cj_path,
                prior_oauth_account.as_ref(),
            );
        }
        return Err(SwapError::WriteFailed(format!("db update failed: {e}")));
    }

    tracing::info!(target = %target_id, "swap complete");
    // _lock dropped here — releases the file lock.
    Ok(())
}

/// Test-only swap entry point that skips the oauthAccount rewrite so
/// tests don't scribble on the real `~/.claude.json`. Production code
/// MUST use [`switch`] / [`switch_force`] — they resolve the real path.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn switch_force_for_tests(
    store: &AccountStore,
    current_id: Option<Uuid>,
    target_id: Uuid,
    platform: &dyn CliPlatform,
    auto_refresh: bool,
    refresher: &dyn TokenRefresher,
    fetcher: &dyn ProfileFetcher,
) -> Result<(), SwapError> {
    switch_inner(
        store, current_id, target_id, platform, auto_refresh, true,
        None, // don't touch ~/.claude.json in tests
        refresher, fetcher,
    )
    .await
}

// Re-export storage functions for external callers (account_service, etc.)
#[cfg(test)]
pub(crate) use storage::private_path;
pub use storage::{delete as delete_private, load as load_private, save as save_private};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock CliPlatform for testing swap logic.
    struct MockPlatform {
        storage: Mutex<Option<String>>,
        fail_write: bool,
    }

    impl MockPlatform {
        fn new(initial: Option<&str>) -> Self {
            Self {
                storage: Mutex::new(initial.map(String::from)),
                fail_write: false,
            }
        }
        fn failing() -> Self {
            Self {
                storage: Mutex::new(None),
                fail_write: true,
            }
        }
        fn get(&self) -> Option<String> {
            self.storage.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            Ok(self.storage.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
            if self.fail_write {
                return Err(SwapError::WriteFailed("mock write failure".into()));
            }
            *self.storage.lock().unwrap() = Some(blob.to_string());
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    /// Mock TokenRefresher for testing refresh logic.
    struct MockRefresher {
        response: Result<TokenResponse, OAuthError>,
    }

    impl MockRefresher {
        fn success() -> Self {
            Self {
                response: Ok(TokenResponse {
                    access_token: "sk-ant-oat01-refreshed".into(),
                    refresh_token: "sk-ant-ort01-refreshed".into(),
                    expires_in: 3600,
                    scope: Some("user:inference user:profile".into()),
                    token_type: Some("Bearer".into()),
                }),
            }
        }
        fn failing(msg: &str) -> Self {
            Self {
                response: Err(OAuthError::RefreshFailed(msg.to_string())),
            }
        }
    }

    #[async_trait::async_trait]
    impl TokenRefresher for MockRefresher {
        async fn refresh(&self, _refresh_token: &str) -> Result<TokenResponse, OAuthError> {
            match &self.response {
                Ok(r) => Ok(TokenResponse {
                    access_token: r.access_token.clone(),
                    refresh_token: r.refresh_token.clone(),
                    expires_in: r.expires_in,
                    scope: r.scope.clone(),
                    token_type: r.token_type.clone(),
                }),
                Err(OAuthError::RefreshFailed(msg)) => Err(OAuthError::RefreshFailed(msg.clone())),
                Err(OAuthError::RateLimited { retry_after_secs }) => Err(OAuthError::RateLimited {
                    retry_after_secs: *retry_after_secs,
                }),
                _ => Err(OAuthError::RefreshFailed("unexpected error variant".into())),
            }
        }
    }

    use crate::testing::{make_account, setup_test_data_dir};

    /// Insert a placeholder account for `uuid` so set_active_cli's strict
    /// zero-row check doesn't fail in tests that only care about swap mechanics.
    fn seed_account(store: &super::AccountStore, uuid: Uuid) {
        let mut a = make_account(&format!("seed-{uuid}@example.com"));
        a.uuid = uuid;
        store.insert(&a).unwrap();
    }

    /// Mock ProfileFetcher — lets tests assert identity-verification behavior
    /// without any network calls. Most tests pass a placeholder via
    /// `noop_fetcher()`; tests that exercise the verification path configure
    /// a specific email to return.
    struct MockProfileFetcher {
        email: String,
    }

    impl MockProfileFetcher {
        fn returning(email: &str) -> Self {
            Self {
                email: email.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl super::ProfileFetcher for MockProfileFetcher {
        async fn fetch_email(&self, _access_token: &str) -> Result<String, OAuthError> {
            Ok(self.email.clone())
        }
    }

    /// Placeholder fetcher used by tests that want verify_blob_identity
    /// to always succeed against the seeded account. Returns the email
    /// that `seed_account` writes for the given uuid, so the identity
    /// check matches the store row. Previously `verify_blob_identity`
    /// skipped on unparseable blobs and the name "noop_fetcher" fit —
    /// after that security-relevant bypass was removed (audit H2),
    /// swap tests need a real fetcher paired with valid blob JSON.
    fn noop_fetcher() -> MockProfileFetcher {
        // Tests that still pass this accept any email — they assert on
        // swap mechanics, not identity. The generic returning() value
        // is intentionally a string the seeded accounts won't use, so
        // tests that rely on verify-succeeds must switch to
        // `matching_fetcher(uuid)` explicitly.
        MockProfileFetcher::returning("never-called@example.com")
    }

    /// Fetcher matched to `seed_account(uuid)` — returns the exact
    /// email the store row carries so `verify_blob_identity` passes.
    fn matching_fetcher(uuid: Uuid) -> MockProfileFetcher {
        MockProfileFetcher::returning(&format!("seed-{uuid}@example.com"))
    }

    /// Minimal valid CredentialBlob JSON for tests that don't care
    /// about token values. expires_at = year 2100 so blob never looks
    /// expired. Paired with `matching_fetcher(uuid)` so the identity
    /// check passes against the stored account email.
    fn test_blob_json() -> String {
        crate::testing::sample_blob_json(4_102_444_800_000)
    }

    fn test_store() -> (AccountStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = AccountStore::open(&db).unwrap();
        (store, dir)
    }

    #[test]
    fn test_private_storage_roundtrip() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = r#"{"test":"data"}"#;

        // Save
        save_private(id, blob).unwrap();

        // Load
        let loaded = load_private(id).unwrap();
        assert_eq!(loaded, blob);

        // Verify 0600 permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(private_path(id)).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }

        // Delete
        delete_private(id).unwrap();
        assert!(load_private(id).is_err());
    }

    #[test]
    fn test_private_storage_missing() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        assert!(matches!(
            load_private(id),
            Err(SwapError::NoStoredCredentials(_))
        ));
    }

    #[test]
    fn test_private_storage_overwrite() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        save_private(id, "first").unwrap();
        save_private(id, "second").unwrap();
        assert_eq!(load_private(id).unwrap(), "second");
        delete_private(id).unwrap();
    }

    // -- Group 7: permission auto-repair on load_private --
    #[cfg(unix)]
    #[test]
    fn test_load_private_permission_repair_succeeds() {
        // Save a private blob (0600), widen to 0644, then load.
        // Expected: load succeeds, perms auto-repaired back to 0600.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        save_private(id, "secret-blob").unwrap();

        use std::os::unix::fs::PermissionsExt;
        let path = private_path(id);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644,
            "setup: widened to 0644"
        );

        let loaded = load_private(id).unwrap();
        assert_eq!(loaded, "secret-blob");

        let after = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(after, 0o600, "load_private must auto-repair perms to 0600");

        delete_private(id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_success() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id);

        // Valid blob JSON — post-H2 verify_blob_identity REJECTS
        // unparseable blobs instead of silently accepting them.
        let target_blob = test_blob_json();
        save_private(target_id, &target_blob).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;
        switch_force_for_tests(
            &store,
            None,
            target_id,
            &platform,
            false,
            &refresher,
            &matching_fetcher(target_id),
        )
        .await
        .unwrap();

        assert_eq!(platform.get(), Some(target_blob));
        assert_eq!(
            store.active_cli_uuid().unwrap(),
            Some(target_id.to_string())
        );

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_saves_outgoing() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        seed_account(&store, current_id);
        seed_account(&store, target_id);

        // Both blobs must be valid JSON — post-H2, verify_blob_identity
        // rejects unparseable blobs for both target (pre-write) and
        // outgoing backup (skip_backup path). Making the outgoing blob
        // parseable is what exercises the save-outgoing branch.
        let target_blob = test_blob_json();
        let refreshed_current = test_blob_json();
        save_private(target_id, &target_blob).unwrap();

        let platform = MockPlatform::new(Some(&refreshed_current));
        let refresher = DefaultRefresher;

        // Both current and target blobs authenticate as their seeded
        // emails; since target_id != current_id, we need a fetcher that
        // honors whichever token is queried. For this test the simplest
        // is an email-dispatching stub: but since both blobs carry the
        // same fixture token (sample_blob_json's constant), we instead
        // use the matching fetcher for `current_id` so the outgoing
        // backup path verifies and runs. The target verify uses a
        // separate matching_fetcher in the pre-write call — we need
        // one fetcher that returns the right email for each. Since
        // test_blob_json shares the access_token across calls, split
        // the responsibility by using two fetchers is not possible;
        // use `EchoingFetcher` that returns different emails by uuid
        // resolution via the store. Simpler: since `matching_fetcher`
        // returns a single email and the two calls are sequential,
        // run with a fetcher that returns current_email for the first
        // call and target_email for the second. `MockProfileFetcher`
        // doesn't support that, so use a round-robin fetcher helper.
        // Three verify calls in order: (1) pre-write target, (2)
        // outgoing backup against CC's current blob, (3) post-write
        // read-back of target. The target_id blob is shared as the
        // test_blob_json fixture but the fetcher is consulted by
        // sequence, so we return the right email at each step.
        let fetcher = RoundRobinFetcher::new(vec![
            format!("seed-{target_id}@example.com"),  // (1) pre-write
            format!("seed-{current_id}@example.com"), // (2) outgoing
            format!("seed-{target_id}@example.com"),  // (3) post-write
        ]);

        switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            false,
            &refresher,
            &fetcher,
        )
        .await
        .unwrap();

        // Current's credentials should be saved to private storage
        assert_eq!(load_private(current_id).unwrap(), refreshed_current);
        // Target should be in platform
        assert_eq!(platform.get(), Some(target_blob));

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    /// Test fetcher that cycles through a fixed list of emails so
    /// sequential verify calls can return different identities. Used
    /// by tests that exercise both the outgoing-backup identity check
    /// and the post-write target identity check in one swap.
    struct RoundRobinFetcher {
        emails: Vec<String>,
        idx: std::sync::atomic::AtomicUsize,
    }

    impl RoundRobinFetcher {
        fn new(emails: Vec<String>) -> Self {
            Self {
                emails,
                idx: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl super::ProfileFetcher for RoundRobinFetcher {
        async fn fetch_email(&self, _access_token: &str) -> Result<String, OAuthError> {
            let i = self
                .idx
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.emails[i % self.emails.len()].clone())
        }
    }

    #[tokio::test]
    async fn test_swap_rollback_on_write_failure() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        // Pre-store both
        save_private(current_id, "original_current").unwrap();
        save_private(target_id, "target_blob").unwrap();

        // Platform will fail on write
        let platform = MockPlatform::failing();
        // Set initial storage to simulate current CC credentials
        *platform.storage.lock().unwrap() = Some("cc_current".to_string());
        let refresher = DefaultRefresher;

        let result = switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            false,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(result.is_err());

        // Current's private storage should be rolled back to original
        assert_eq!(load_private(current_id).unwrap(), "original_current");

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_no_target_credentials() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        // Don't pre-store — target has no credentials
        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;

        let result = switch_force_for_tests(
            &store,
            None,
            target_id,
            &platform,
            false,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(matches!(result, Err(SwapError::NoStoredCredentials(_))));
    }

    #[tokio::test]
    async fn test_swap_db_pointer_matches_after_success() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id);
        save_private(target_id, &test_blob_json()).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;
        switch_force_for_tests(
            &store,
            None,
            target_id,
            &platform,
            false,
            &refresher,
            &matching_fetcher(target_id),
        )
        .await
        .unwrap();

        // DB active pointer must match target
        assert_eq!(
            store.active_cli_uuid().unwrap(),
            Some(target_id.to_string())
        );
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_db_not_updated_on_write_failure() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        seed_account(&store, current_id);
        seed_account(&store, target_id);
        save_private(target_id, "target").unwrap();

        // Set initial active to current
        store.set_active_cli(current_id).unwrap();

        let platform = MockPlatform::failing();
        *platform.storage.lock().unwrap() = Some("cc".to_string());
        let refresher = DefaultRefresher;

        let result = switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            false,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(result.is_err());

        // DB should still point to current, NOT target
        assert_eq!(
            store.active_cli_uuid().unwrap(),
            Some(current_id.to_string())
        );

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_rollback_deletes_when_no_previous() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        // current has NO prior private storage
        save_private(target_id, "target").unwrap();

        let platform = MockPlatform::failing();
        *platform.storage.lock().unwrap() = Some("cc_blob".to_string());
        let refresher = DefaultRefresher;

        let result = switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            false,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(result.is_err());

        // Rollback should have deleted the private storage that was created during swap
        // (since there was no previous blob to restore to)
        assert!(load_private(current_id).is_err());

        delete_private(target_id).unwrap();
    }

    #[test]
    fn test_swap_error_corrupt_blob_display() {
        let err = SwapError::CorruptBlob("missing field accessToken".into());
        assert_eq!(
            err.to_string(),
            "corrupt credential blob: missing field accessToken"
        );
    }

    #[test]
    fn test_swap_error_refresh_failed_display() {
        let err = SwapError::RefreshFailed("token endpoint returned 401".into());
        assert_eq!(
            err.to_string(),
            "token refresh failed: token endpoint returned 401"
        );
    }

    #[tokio::test]
    async fn test_swap_target_load_fails_before_any_mutation() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        // Target has no stored credentials — should fail immediately

        // Platform tracks whether read_default was ever called
        let platform = MockPlatform::new(Some("should-not-be-read"));
        let refresher = DefaultRefresher;

        let result = switch_force_for_tests(
            &store,
            None,
            target_id,
            &platform,
            false,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(result.is_err());

        // Platform storage should be untouched (read_default never called for write path)
        assert_eq!(platform.get(), Some("should-not-be-read".to_string()));
    }

    #[tokio::test]
    async fn test_swap_current_none_writes_directly() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id);
        let direct_blob = test_blob_json();
        save_private(target_id, &direct_blob).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;
        switch_force_for_tests(
            &store,
            None,
            target_id,
            &platform,
            false,
            &refresher,
            &matching_fetcher(target_id),
        )
        .await
        .unwrap();

        // Target written directly, no outgoing save
        assert_eq!(platform.get(), Some(direct_blob));
        delete_private(target_id).unwrap();
    }

    // --- switch_force_for_tests() auto_refresh tests ---

    #[tokio::test]
    async fn test_swap_auto_refresh_writes_fresh_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id);

        // Store an expired blob for the target
        save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = MockRefresher::success();
        let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

        switch_force_for_tests(
            &store, None, target_id, &platform, true, &refresher, &fetcher,
        )
        .await
        .unwrap();

        // The platform should have the refreshed blob, not the expired one
        let written = platform.get().unwrap();
        let parsed = crate::blob::CredentialBlob::from_json(&written).unwrap();
        assert_eq!(
            parsed.claude_ai_oauth.access_token,
            "sk-ant-oat01-refreshed"
        );

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_noop_for_fresh_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id);

        let fresh = crate::testing::fresh_blob_json();
        save_private(target_id, &fresh).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = MockRefresher::success();
        let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

        switch_force_for_tests(
            &store, None, target_id, &platform, true, &refresher, &fetcher,
        )
        .await
        .unwrap();

        // Should use the original fresh blob (not refreshed)
        let written = platform.get().unwrap();
        let parsed = crate::blob::CredentialBlob::from_json(&written).unwrap();
        assert_eq!(parsed.claude_ai_oauth.access_token, "sk-ant-oat01-test");

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_false_skips_refresh() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id);

        let expired = crate::testing::expired_blob_json();
        save_private(target_id, &expired).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = MockRefresher::success();
        let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

        // auto_refresh = false — should NOT call refresher
        switch_force_for_tests(
            &store, None, target_id, &platform, false, &refresher, &fetcher,
        )
        .await
        .unwrap();

        // Should use the expired blob as-is
        let written = platform.get().unwrap();
        assert_eq!(written, expired);

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_failure_aborts_before_mutation() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();

        save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

        let platform = MockPlatform::new(Some("original-cc-blob"));
        let refresher = MockRefresher::failing("network error");

        let result = switch_force_for_tests(
            &store,
            None,
            target_id,
            &platform,
            true,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(matches!(result, Err(SwapError::RefreshFailed(_))));

        // Platform should be untouched
        assert_eq!(platform.get(), Some("original-cc-blob".to_string()));

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_rollback_works_after_refresh() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        save_private(current_id, "original_current").unwrap();
        save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

        // Platform will fail on write
        let platform = MockPlatform::failing();
        *platform.storage.lock().unwrap() = Some("cc_current".to_string());
        let refresher = MockRefresher::success();

        let result = switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            true,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(result.is_err());

        // Current's private storage should be rolled back to original
        assert_eq!(load_private(current_id).unwrap(), "original_current");

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    // --- maybe_refresh_blob tests ---

    #[tokio::test]
    async fn test_swap_maybe_refresh_not_expired_returns_unchanged() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::fresh_blob_json();
        let refresher = MockRefresher::success();

        let result = maybe_refresh_blob(&blob, id, &refresher).await.unwrap();
        // Fresh blob should be returned unchanged (same string)
        assert_eq!(result, blob);
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_within_margin_triggers_refresh() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::expiring_soon_blob_json();
        let refresher = MockRefresher::success();

        let result = maybe_refresh_blob(&blob, id, &refresher).await.unwrap();
        // Should have refreshed — result should contain the new token
        let parsed = crate::blob::CredentialBlob::from_json(&result).unwrap();
        assert_eq!(
            parsed.claude_ai_oauth.access_token,
            "sk-ant-oat01-refreshed"
        );
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_corrupt_input_errors() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let refresher = MockRefresher::success();

        let result = maybe_refresh_blob("not valid json", id, &refresher).await;
        assert!(matches!(result, Err(SwapError::CorruptBlob(_))));
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_expired_refreshes() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::expired_blob_json();
        let refresher = MockRefresher::success();

        let result = maybe_refresh_blob(&blob, id, &refresher).await.unwrap();
        let parsed = crate::blob::CredentialBlob::from_json(&result).unwrap();
        assert_eq!(
            parsed.claude_ai_oauth.access_token,
            "sk-ant-oat01-refreshed"
        );
        assert_eq!(
            parsed.claude_ai_oauth.refresh_token,
            "sk-ant-ort01-refreshed"
        );
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_refresh_failure_errors() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::expired_blob_json();
        let refresher = MockRefresher::failing("network timeout");

        let result = maybe_refresh_blob(&blob, id, &refresher).await;
        assert!(matches!(result, Err(SwapError::RefreshFailed(_))));
    }

    #[tokio::test]
    async fn test_swap_maybe_refresh_saves_refreshed_blob() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = crate::testing::expired_blob_json();
        let refresher = MockRefresher::success();

        maybe_refresh_blob(&blob, id, &refresher).await.unwrap();

        // The refreshed blob should be persisted in private storage
        let saved = load_private(id).unwrap();
        let parsed = crate::blob::CredentialBlob::from_json(&saved).unwrap();
        assert_eq!(
            parsed.claude_ai_oauth.access_token,
            "sk-ant-oat01-refreshed"
        );

        delete_private(id).unwrap();
    }

    // -- Group 11: Unix-only code gaps --

    #[test]
    fn test_save_private_no_permission_check_on_non_unix() {
        // save_private + load_private roundtrip works on every platform.
        // On Unix, perms are verified (0o600). On non-Unix, the whole
        // #[cfg(unix)] block is skipped and data integrity is all that matters.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let id = Uuid::new_v4();
        let blob = r#"{"platform":"agnostic"}"#;

        save_private(id, blob).unwrap();
        let loaded = load_private(id).unwrap();
        assert_eq!(loaded, blob);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(private_path(id))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "unix: enforced 0o600");
        }

        delete_private(id).unwrap();
    }

    #[test]
    fn test_swap_lock_works_on_all_platforms() {
        // acquire_swap_lock() uses flock on Unix, OpenOptions exclusion on
        // Windows. The contract: returns Ok, creates the lock file, and
        // releases on drop. Verify the lock file exists after acquire.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();

        let lock_path = crate::paths::claudepot_data_dir().join(".swap.lock");
        let _guard = acquire_swap_lock().expect("acquire_swap_lock must work on all platforms");
        assert!(lock_path.exists(), "lock file created");
    }

    // -- Group 4: CLI swap DB rollback (3 tests) --

    #[tokio::test]
    async fn test_swap_db_failure_restores_cc_credentials() {
        // Drop the state table so set_active_cli() fails AFTER write_default
        // succeeded. The rollback reads load_private(current) — which, at that
        // point, contains the CC blob that was saved during phase 2 (what CC
        // had before the swap). That's the correct "outgoing" blob to restore.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        // Any older private content for current — not what rollback restores.
        save_private(current_id, "older_private_from_prior_swap").unwrap();
        save_private(target_id, "target_blob").unwrap();

        let platform = MockPlatform::new(Some("outgoing_cc_blob"));
        let refresher = DefaultRefresher;

        // Make set_active_cli fail while write_default remains working.
        store.corrupt_state_table_for_test();

        let result = switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            false,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(
            matches!(result, Err(SwapError::WriteFailed(_))),
            "expected WriteFailed from DB update, got {:?}",
            result
        );
        // Rollback restored the CC blob that was outgoing at swap start.
        assert_eq!(
            platform.get(),
            Some("outgoing_cc_blob".to_string()),
            "platform must be restored to outgoing CC credentials"
        );

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    // -- Identity verification (prevents mis-filed-blob corruption) --

    #[tokio::test]
    async fn test_swap_aborts_when_target_blob_belongs_to_different_account() {
        // Regression guard for the "wrong blob under wrong UUID" corruption
        // that motivated the verification. The target's stored blob reports
        // a different email than the target's DB record — switch_force_for_tests() must
        // abort with IdentityMismatch before writing CC.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id); // stored email: seed-{uuid}@example.com
        save_private(target_id, &crate::testing::fresh_blob_json()).unwrap();

        let platform = MockPlatform::new(None);
        let refresher = MockRefresher::success();
        // Fetcher claims the blob is for someone else — the divergence case.
        let fetcher = MockProfileFetcher::returning("intruder@example.com");

        let result = switch_force_for_tests(
            &store, None, target_id, &platform, false, &refresher, &fetcher,
        )
        .await;

        assert!(
            matches!(
                result,
                Err(SwapError::IdentityMismatch {
                    ref actual_email,
                    ..
                }) if actual_email == "intruder@example.com"
            ),
            "expected IdentityMismatch, got {:?}",
            result
        );
        // Platform untouched — abort happened before write_default.
        assert_eq!(platform.get(), None);

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_drift_on_outgoing_blob_skips_backup_but_completes() {
        // When CC's current blob represents a different account than the DB
        // thinks is active (drift from an external `claude auth login`),
        // the swap must NOT abort — that would leave the user stuck. Instead,
        // skip the outgoing backup (so we don't mis-file), but complete the
        // target swap. The target-blob verification still runs and aborts
        // on a real target mismatch.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        seed_account(&store, current_id);
        seed_account(&store, target_id);

        // Capture once: fresh_blob_json() uses Utc::now() internally, so
        // back-to-back calls return different millisecond timestamps and
        // the assert_eq below would flake ~30% of the time. Reuse the
        // captured JSON for both the target private slot and the CC
        // platform's current blob.
        let target_blob = crate::testing::fresh_blob_json();
        save_private(target_id, &target_blob).unwrap();
        // Pre-existing blob for current (a valid earlier backup). We'll
        // verify it stays UNCHANGED by this swap (we skipped the backup save).
        save_private(current_id, "original-current-backup").unwrap();

        let cc_blob = target_blob.clone();
        let platform = MockPlatform::new(Some(&cc_blob));
        let refresher = MockRefresher::success();

        // Single-valued fetcher: both verifications see target's seeded email.
        // → target check passes; outgoing check sees drift (cur_email differs).
        let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

        let result = switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            false,
            &refresher,
            &fetcher,
        )
        .await;

        assert!(
            result.is_ok(),
            "drift case must NOT abort; got {:?}",
            result
        );

        // Platform received the target blob.
        assert_eq!(platform.get(), Some(cc_blob.clone()));
        // Current's private storage was NOT overwritten with the drifted CC
        // blob — still the original backup.
        assert_eq!(
            load_private(current_id).unwrap(),
            "original-current-backup",
            "outgoing backup must be skipped when CC drift is detected"
        );

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_db_failure_with_no_outgoing() {
        // Full DB corruption (state table dropped) breaks the early
        // find_by_uuid lookup used by identity verification. The swap aborts
        // before touching the platform — safer than the old "write, then
        // fail at set_active_cli" pattern: if we can't verify the DB's
        // expectations, we don't mutate CC's state.
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id);
        save_private(target_id, "target_only_blob").unwrap();

        let platform = MockPlatform::new(None);
        let refresher = DefaultRefresher;

        store.corrupt_state_table_for_test();

        let result = switch_force_for_tests(
            &store,
            None,
            target_id,
            &platform,
            false,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(
            matches!(result, Err(SwapError::WriteFailed(_))),
            "DB failure must surface as WriteFailed, got {:?}",
            result
        );
        // Platform never written to — aborted before the mutation.
        assert_eq!(platform.get(), None);

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_auto_refresh_then_db_failure() {
        // auto_refresh=true, target is expired → refresh runs and is
        // persisted to private storage. Then write_default succeeds.
        // Then set_active_cli fails. Verify:
        //   - rollback restores previous CC credentials for current_id
        //   - refreshed blob stays in target's private storage (was persisted
        //     by maybe_refresh_blob BEFORE the swap mutations)
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        save_private(current_id, "older_private_from_prior_swap").unwrap();
        save_private(target_id, &crate::testing::expired_blob_json()).unwrap();

        let platform = MockPlatform::new(Some("outgoing_cc_blob"));

        // Use a mock refresher that returns fresh tokens.
        let refresher = MockRefresher::success();

        store.corrupt_state_table_for_test();

        let result = switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            true,
            &refresher,
            &noop_fetcher(),
        )
        .await;
        assert!(
            matches!(result, Err(SwapError::WriteFailed(_))),
            "DB update failure must surface, got {:?}",
            result
        );
        // Rollback restored the outgoing CC blob that was in the platform
        // before the swap (which got saved into private storage for `current`).
        assert_eq!(platform.get(), Some("outgoing_cc_blob".to_string()));
        // Target's refreshed blob persisted before swap mutations.
        let target_priv = load_private(target_id).unwrap();
        assert!(
            target_priv.contains("sk-ant-oat01-refreshed"),
            "refreshed token must remain in target's private storage after rollback"
        );

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    // ------- Post-switch verification tests (added by audit round 2) -------

    /// Post-switch read-back returns None after a write that claimed
    /// success — swap must roll back and return IdentityVerificationFailed,
    /// not declare the switch complete.
    struct MockPlatformDropsOnRead {
        storage: std::sync::Mutex<Option<String>>,
    }
    impl MockPlatformDropsOnRead {
        fn new(initial: Option<&str>) -> Self {
            Self {
                storage: std::sync::Mutex::new(initial.map(String::from)),
            }
        }
        fn get(&self) -> Option<String> {
            self.storage.lock().unwrap().clone()
        }
    }
    #[async_trait::async_trait]
    impl CliPlatform for MockPlatformDropsOnRead {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            // First read (pre-write outgoing-check) returns whatever is
            // in storage. After a write happens, subsequent reads return
            // None as if CC's slot went empty under us.
            Ok(self.storage.lock().unwrap().clone())
        }
        async fn write_default(&self, _blob: &str) -> Result<(), SwapError> {
            *self.storage.lock().unwrap() = None;
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
        async fn clear_default(&self) -> Result<(), SwapError> {
            *self.storage.lock().unwrap() = None;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_switch_aborts_on_post_switch_read_returns_none() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        seed_account(&store, target_id);
        save_private(target_id, &crate::testing::fresh_blob_json()).unwrap();

        let platform = MockPlatformDropsOnRead::new(None);
        let refresher = MockRefresher::success();
        let fetcher = MockProfileFetcher::returning(&format!("seed-{target_id}@example.com"));

        let result = switch_force_for_tests(
            &store,
            None,
            target_id,
            &platform,
            false,
            &refresher,
            &fetcher,
        )
        .await;

        assert!(
            matches!(result, Err(SwapError::IdentityVerificationFailed(_))),
            "expected IdentityVerificationFailed when post-switch read returns None, got {result:?}"
        );
        // active_cli must NOT have been set on failure.
        assert!(store.active_cli_uuid().unwrap().is_none());
        // Rollback path had no prior blob — platform stays empty.
        assert!(platform.get().is_none());
        delete_private(target_id).unwrap();
    }

    /// Post-switch read-back returns a DIFFERENT blob than the one we
    /// wrote (the fetcher says it authenticates as someone else). Swap
    /// must return IdentityMismatch and restore the previous CC blob.
    struct MockPlatformReadReplacesBlob {
        storage: std::sync::Mutex<Option<String>>,
        replace_on_read_after_write: std::sync::Mutex<Option<String>>,
        wrote: std::sync::Mutex<bool>,
    }
    impl MockPlatformReadReplacesBlob {
        fn new(initial: &str, replace_with: &str) -> Self {
            Self {
                storage: std::sync::Mutex::new(Some(initial.to_string())),
                replace_on_read_after_write: std::sync::Mutex::new(Some(
                    replace_with.to_string(),
                )),
                wrote: std::sync::Mutex::new(false),
            }
        }
        fn get(&self) -> Option<String> {
            self.storage.lock().unwrap().clone()
        }
    }
    #[async_trait::async_trait]
    impl CliPlatform for MockPlatformReadReplacesBlob {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            if *self.wrote.lock().unwrap() {
                if let Some(r) = self.replace_on_read_after_write.lock().unwrap().take() {
                    *self.storage.lock().unwrap() = Some(r);
                }
            }
            Ok(self.storage.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
            *self.storage.lock().unwrap() = Some(blob.to_string());
            *self.wrote.lock().unwrap() = true;
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_switch_aborts_and_rolls_back_on_post_switch_identity_mismatch() {
        let _lock = crate::testing::lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        seed_account(&store, current_id);
        seed_account(&store, target_id);

        let initial_cc_blob = crate::testing::fresh_blob_json();
        let replacement = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-intruder","refreshToken":"sk-ant-ort01-intruder","expiresAt":9999999999999,"scopes":["user:inference"],"subscriptionType":"pro","rateLimitTier":"default_claude_pro"}}"#;

        save_private(current_id, &initial_cc_blob).unwrap();
        save_private(target_id, &crate::testing::fresh_blob_json()).unwrap();
        store.set_active_cli(current_id).unwrap();

        let platform = MockPlatformReadReplacesBlob::new(&initial_cc_blob, replacement);
        let refresher = MockRefresher::success();
        // First call (pre-write outgoing check) matches current_id;
        // Second call (pre-write target check) matches target_id;
        // Third call (post-write check) returns intruder email so it mismatches target.
        struct PostSwitchMismatchFetcher {
            calls: std::sync::Mutex<usize>,
            current_email: String,
            target_email: String,
        }
        #[async_trait::async_trait]
        impl super::ProfileFetcher for PostSwitchMismatchFetcher {
            async fn fetch_email(&self, _t: &str) -> Result<String, OAuthError> {
                let mut c = self.calls.lock().unwrap();
                *c += 1;
                Ok(match *c {
                    1 => self.current_email.clone(),
                    2 => self.target_email.clone(),
                    _ => "intruder@example.com".into(),
                })
            }
        }
        let fetcher = PostSwitchMismatchFetcher {
            calls: std::sync::Mutex::new(0),
            current_email: format!("seed-{current_id}@example.com"),
            target_email: format!("seed-{target_id}@example.com"),
        };

        let result = switch_force_for_tests(
            &store,
            Some(current_id),
            target_id,
            &platform,
            false,
            &refresher,
            &fetcher,
        )
        .await;

        assert!(
            matches!(result, Err(SwapError::IdentityMismatch { .. })),
            "expected IdentityMismatch on post-switch verification, got {result:?}"
        );
        // active_cli must still be current — not flipped to target.
        assert_eq!(
            store.active_cli_uuid().unwrap(),
            Some(current_id.to_string()),
            "active_cli must not flip on post-switch failure"
        );
        // Rollback restored the original CC blob (not the replacement).
        assert_eq!(platform.get().as_deref(), Some(initial_cc_blob.as_str()));

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }
}
