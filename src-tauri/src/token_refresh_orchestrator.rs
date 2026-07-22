//! Proactive token-refresh orchestrator — bridges
//! `claudepot_core::token_refresh` to the Tauri runtime.
//!
//! Called from `usage_snapshot::run_tick`, ahead of the usage fetch, so
//! an account healed this tick reports live usage in the same tick
//! instead of waiting five more minutes.
//!
//! Holds one piece of managed state: an in-memory map of the last
//! refresh attempt per account, which drives the round-robin in
//! [`claudepot_core::token_refresh::select_next`]. Deliberately not
//! persisted — losing it on restart costs at most one extra refresh per
//! account, and a file would need the same lost-update care as
//! `permission-grants.json` for no real benefit.
//!
//! Zero overhead when nothing is expired: one DB list plus one keychain
//! read per account, no network.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use claudepot_core::account::{AccountStore, VerifyOutcome};
use claudepot_core::blob::CredentialBlob;
use claudepot_core::cli_backend::swap;
use claudepot_core::services::identity;
use claudepot_core::services::usage_cache::UsageCache;
use claudepot_core::token_refresh::{is_eligible, select_next, Candidate, Facts};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

/// Don't re-attempt the same account inside this window. The tick is
/// 5 min, so this lets a failing account retry roughly every other tick
/// while its peers keep their turns.
const MIN_RETRY_GAP_MINS: i64 = 11;

#[derive(Default)]
pub struct TokenRefreshOrchestrator {
    last_attempt: Mutex<HashMap<Uuid, DateTime<Utc>>>,
}

impl TokenRefreshOrchestrator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Refresh at most ONE expired, inactive account.
    ///
    /// See the module docs in `claudepot_core::token_refresh` for why
    /// it is one-per-tick and why the refresh itself is delegated to
    /// `identity::verify_account_identity` rather than reimplemented.
    pub async fn tick(&self, app: &AppHandle) {
        let store = match crate::commands::open_store() {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(error = %e, "token_refresh: open_store failed");
                return;
            }
        };

        let Some(uuid) = self.pick(&store).await else {
            return;
        };

        self.last_attempt.lock_or_recover().insert(uuid, Utc::now());

        let fetcher = swap::DefaultProfileFetcher;
        match identity::verify_account_identity(&store, uuid, &fetcher).await {
            Ok(VerifyOutcome::Ok { .. }) => {
                tracing::info!(account = %uuid, "token_refresh: healed an expired slot");
                // The usage cache keys on account, not on token, and a
                // fetch that failed with an expired token is not cached
                // — but evicting is cheap and lets the dashboard repaint
                // on this tick rather than after the 60s TTL.
                app.state::<UsageCache>().invalidate(uuid).await;
            }
            Ok(VerifyOutcome::Rejected) => {
                // Terminal: the refresh token is dead. `verify_status`
                // is now "rejected", which `pick` filters out, so this
                // account stops consuming ticks until the user
                // re-authenticates. The UI already surfaces the state.
                tracing::info!(account = %uuid, "token_refresh: refresh token rejected — re-login required");
            }
            Ok(other) => {
                // Drift is filtered by `pick`, so this is almost always
                // NetworkError — including the live-session skip, which
                // is exactly the "leave it alone" answer we want.
                tracing::debug!(account = %uuid, status = other.as_str(), "token_refresh: not healed this pass");
            }
            Err(e) => {
                tracing::debug!(account = %uuid, error = %e, "token_refresh: verify failed");
            }
        }
    }

    /// Build the eligible set and let the pure selector choose.
    ///
    /// Eligible = has CLI credentials, is NOT the active CLI account,
    /// is not already known-bad (`drift` / `rejected`), and its stored
    /// blob has actually expired.
    ///
    /// The active account is excluded on purpose: its live token belongs
    /// to Claude Code, which rotates it on its own schedule. Refreshing
    /// it from here is the sign-out bug fixed in 0.2.10 — and
    /// `verify_account_identity` would route it through the keychain
    /// resolver anyway, which declines while a session is live.
    async fn pick(&self, store: &AccountStore) -> Option<Uuid> {
        let accounts = store.list().ok()?;
        let active = store
            .active_cli_uuid()
            .ok()
            .flatten()
            .and_then(|raw| Uuid::parse_str(&raw).ok());

        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let last = self.last_attempt.lock_or_recover().clone();

        let mut candidates: Vec<Candidate> = Vec::new();
        for a in &accounts {
            // Cheap checks first — skip the keychain read for accounts
            // that can't qualify regardless of what the slot holds.
            if !a.has_cli_credentials
                || Some(a.uuid) == active
                || matches!(a.verify_status.as_str(), "drift" | "rejected")
            {
                continue;
            }
            let Ok(blob_str) = swap::load_private(a.uuid).await else {
                continue;
            };
            let Ok(blob) = CredentialBlob::from_json(&blob_str) else {
                continue;
            };
            let expires_at_ms = blob.claude_ai_oauth.expires_at;
            let facts = Facts {
                has_cli_credentials: a.has_cli_credentials,
                is_active_cli: Some(a.uuid) == active,
                verify_status: &a.verify_status,
                expires_at_ms,
            };
            if !is_eligible(&facts, now_ms) {
                continue;
            }
            candidates.push(Candidate {
                uuid: a.uuid,
                expires_at_ms,
                last_attempt: last.get(&a.uuid).copied(),
            });
        }

        select_next(&candidates, now, Duration::minutes(MIN_RETRY_GAP_MINS))
    }
}

/// Lock helper that survives a poisoned mutex. A panic in one tick must
/// not disable proactive refresh for the app's lifetime; the map is a
/// scheduling hint, so a partially-updated one is harmless.
trait LockOrRecover<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockOrRecover<T> for Mutex<T> {
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|p| p.into_inner())
    }
}
