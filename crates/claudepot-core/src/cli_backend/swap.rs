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
pub(crate) async fn verify_blob_identity(
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
        // Use a real cross-process exclusive lock. Plain
        // `OpenOptions::write` does NOT exclude other processes — a
        // concurrent process can still open + write the file. Use
        // LockFileEx via the `fs2` shim, which is already used for the
        // Desktop swap lock in `desktop_lock.rs`.
        use fs2::FileExt;
        file.lock_exclusive().map_err(SwapError::FileError)?;
    }

    Ok(file)
}

/// Result of a `maybe_refresh_blob` call. Carries the (possibly
/// refreshed) blob string and a flag the caller uses to decide
/// whether to persist after identity verification has passed.
///
/// Audit fix for swap.rs:229: the previous shape persisted the
/// refreshed blob inside `maybe_refresh_blob`, before the caller
/// had a chance to verify the new blob's identity matches the slot.
/// A misfiled or attacker-controlled refresh token would then write
/// the wrong account's credentials into a slot before anyone
/// noticed. Now we hand the refreshed bytes back without touching
/// disk; the caller persists if and only if `verify_blob_identity`
/// approves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MaybeRefreshed {
    /// Token was not expired (no refresh needed); the original blob
    /// stays in the slot, no save is required.
    Unchanged,
    /// Token was refreshed in memory. Caller must verify identity
    /// against `blob` and then persist via `storage::save` to commit.
    Refreshed { blob: String },
}

impl MaybeRefreshed {
    /// Borrow the live blob — refreshed if so, original otherwise.
    pub fn blob<'a>(&'a self, original: &'a str) -> &'a str {
        match self {
            Self::Unchanged => original,
            Self::Refreshed { blob } => blob.as_str(),
        }
    }
}

/// Conditionally refresh the credential blob if it is expired or expiring
/// within a 5-minute margin. Returns a `MaybeRefreshed` describing
/// the new bytes (if any). The caller is responsible for persisting
/// after verifying identity.
pub(crate) async fn maybe_refresh_blob(
    blob_str: &str,
    refresher: &dyn TokenRefresher,
) -> Result<MaybeRefreshed, SwapError> {
    let blob = crate::blob::CredentialBlob::from_json(blob_str)
        .map_err(|e| SwapError::CorruptBlob(e.to_string()))?;

    if !blob.is_expired(300) {
        return Ok(MaybeRefreshed::Unchanged);
    }

    tracing::info!("token expired or expiring soon, refreshing...");
    let token_resp = refresher
        .refresh(&blob.claude_ai_oauth.refresh_token)
        .await
        .map_err(|e| SwapError::RefreshFailed(e.to_string()))?;

    let new_blob = crate::oauth::refresh::build_blob(&token_resp, Some(&blob));
    Ok(MaybeRefreshed::Refreshed { blob: new_blob })
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
        store,
        current_id,
        target_id,
        platform,
        auto_refresh,
        false,
        claude_json::default_path().as_deref(),
        refresher,
        fetcher,
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
        store,
        current_id,
        target_id,
        platform,
        auto_refresh,
        true,
        claude_json::default_path().as_deref(),
        refresher,
        fetcher,
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
    let target_blob_original = storage::load(target_id)?;

    // Conditionally refresh in MEMORY if expired/expiring. The
    // refreshed bytes don't touch disk yet — see MaybeRefreshed
    // docstring. Persist below, after identity verification passes.
    let refresh_outcome = if auto_refresh {
        maybe_refresh_blob(&target_blob_original, refresher).await?
    } else {
        MaybeRefreshed::Unchanged
    };
    let target_blob = refresh_outcome.blob(&target_blob_original).to_string();

    // Verify target blob actually belongs to target_id's stored email. If it
    // doesn't, the slot is mis-filed — abort before writing to CC.
    let target_email = store
        .find_by_uuid(target_id)
        .map_err(|e| SwapError::WriteFailed(format!("db lookup failed: {e}")))?
        .ok_or_else(|| SwapError::WriteFailed(format!("target {target_id} not in DB")))?
        .email;
    tracing::debug!(target = %target_id, "verifying target blob identity");
    verify_blob_identity(&target_blob, &target_email, fetcher).await?;

    // Identity verified — NOW persist if a refresh actually happened.
    // A verify failure leaves the original (un-refreshed) blob on
    // disk so a stale-but-correct slot is preferable to a fresh-but-
    // misattributed one.
    if let MaybeRefreshed::Refreshed { blob } = &refresh_outcome {
        storage::save(target_id, blob)?;
    }

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
            let target_after = platform.read_default().await?.ok_or_else(|| {
                SwapError::IdentityVerificationFailed(
                    "post-write slot empty during oauthAccount rewrite".into(),
                )
            })?;
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
        // Audit fix for swap.rs:563 — restore CC's PRE-WRITE state,
        // not whatever `storage::load(cur)` yields. The captured
        // `pre_write_default` is the exact bytes (or absence) that
        // CC held before we wrote `target_blob` over it; falling
        // back to `storage::load(cur)` could substitute a different
        // (older / refreshed) blob that wasn't what CC was actually
        // showing the user. The `None` arm — CC had no creds before
        // the swap — must clear the keychain rather than leave the
        // target blob in place.
        match pre_write_default.as_deref() {
            Some(prev) => {
                let _ = platform.write_default(prev).await;
            }
            None => {
                let _ = platform.clear_default().await;
            }
        }
        if let Some(cj_path) = claude_json_path {
            let _ = claude_json::restore_oauth_account(cj_path, prior_oauth_account.as_ref());
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
        store,
        current_id,
        target_id,
        platform,
        auto_refresh,
        true,
        None, // don't touch ~/.claude.json in tests
        refresher,
        fetcher,
    )
    .await
}

// Re-export storage functions for external callers (account_service, etc.)
#[cfg(test)]
pub(crate) use storage::private_path;
pub use storage::{delete as delete_private, load as load_private, save as save_private};

#[cfg(test)]
#[path = "swap_tests.rs"]
mod tests;
