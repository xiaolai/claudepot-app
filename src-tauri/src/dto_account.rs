//! Account / identity / status DTOs that cross the Tauri boundary.
//!
//! Sharded out of `dto.rs` along with `dto_usage.rs`; the parent
//! module re-exports both so `crate::dto::AccountSummary` keeps
//! working for existing callers.

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct AccountSummary {
    pub uuid: String,
    pub email: String,
    pub org_name: Option<String>,
    pub subscription_type: Option<String>,
    pub is_cli_active: bool,
    pub is_desktop_active: bool,
    pub has_cli_credentials: bool,
    pub has_desktop_profile: bool,
    pub last_cli_switch: Option<DateTime<Utc>>,
    pub last_desktop_switch: Option<DateTime<Utc>>,
    /// "valid", "expired", "no credentials", "missing", "corrupt blob"
    pub token_status: String,
    pub token_remaining_mins: Option<i64>,
    /// True iff the stored blob actually exists and parses. Mirrors reality,
    /// not the DB flag. Used by the UI to gate the "Use CLI" button — the
    /// DB's has_cli_credentials can lie after external state changes.
    pub credentials_healthy: bool,
    /// Last persisted verification outcome: "never" | "ok" | "drift" |
    /// "rejected" | "network_error". Drives the drift badge in the UI.
    pub verify_status: String,
    /// When verify_status != "never", the actual email `/api/oauth/profile`
    /// returned for THIS slot. Equals `email` when ok; differs on drift.
    pub verified_email: Option<String>,
    /// ISO-8601 timestamp of the last verification pass.
    pub verified_at: Option<DateTime<Utc>>,
    /// Computed: verified_email is set AND differs from `email`. Handy
    /// for the GUI to avoid comparing strings itself.
    pub drift: bool,
    /// Per-file-on-disk truth for the Desktop profile snapshot dir.
    /// Computed at list time via `paths::desktop_profile_dir(uuid).exists()`.
    /// Differs from `has_desktop_profile` only when the DB flag has
    /// drifted from disk (e.g., the user manually deleted the snapshot).
    /// UI should prefer this field when gating Desktop affordances.
    pub desktop_profile_on_disk: bool,
}

/// Keychain-free subset of [`AccountSummary`]. Returned by
/// `account_list_basic` for callers that only need to resolve an
/// account's identity (uuid → email/org/subscription) and don't
/// render token health.
///
/// Every field here comes straight from `AccountStore` (sqlite), so
/// the whole list resolves in a single-digit millisecond window even
/// with dozens of accounts. The full [`AccountSummary`], by contrast,
/// issues one macOS Keychain syscall per account (via
/// `token_health` → `swap::load_private`) plus a `reconcile_flags`
/// pass, which can stall the UI for hundreds of milliseconds when
/// the Keychain is cold. Use the basic variant unless the surface
/// actually displays token state.
#[derive(Serialize, Clone)]
pub struct AccountSummaryBasic {
    pub uuid: String,
    pub email: String,
    pub org_name: Option<String>,
    pub subscription_type: Option<String>,
    pub is_cli_active: bool,
    pub is_desktop_active: bool,
    pub has_cli_credentials: bool,
    pub has_desktop_profile: bool,
}

impl From<&claudepot_core::account::Account> for AccountSummaryBasic {
    fn from(a: &claudepot_core::account::Account) -> Self {
        Self {
            uuid: a.uuid.to_string(),
            email: a.email.clone(),
            org_name: a.org_name.clone(),
            subscription_type: a.subscription_type.clone(),
            is_cli_active: a.is_cli_active,
            is_desktop_active: a.is_desktop_active,
            has_cli_credentials: a.has_cli_credentials,
            has_desktop_profile: a.has_desktop_profile,
        }
    }
}

/// Inline-I/O fallback used by the verify commands in
/// `commands_account.rs` (`verify_account`, `verify_all_accounts`).
/// Those handlers hand back a fresh `AccountSummary` after a single
/// row mutation and don't have an `AccountSummaryView` in hand —
/// recomputing per-row token health here is the cheapest path. New
/// callers should prefer the `From<&AccountSummaryView>` impl, which
/// keeps Keychain I/O upstream in `list_summaries` where it can be
/// sequenced.
impl From<&claudepot_core::account::Account> for AccountSummary {
    fn from(a: &claudepot_core::account::Account) -> Self {
        let health =
            claudepot_core::services::account_service::token_health(a.uuid, a.has_cli_credentials);
        let credentials_healthy = health.status.starts_with("valid") || health.status == "expired";
        let desktop_profile_on_disk =
            claudepot_core::paths::desktop_profile_dir(a.uuid).exists();
        Self {
            uuid: a.uuid.to_string(),
            email: a.email.clone(),
            org_name: a.org_name.clone(),
            subscription_type: a.subscription_type.clone(),
            is_cli_active: a.is_cli_active,
            is_desktop_active: a.is_desktop_active,
            has_cli_credentials: a.has_cli_credentials,
            has_desktop_profile: a.has_desktop_profile,
            last_cli_switch: a.last_cli_switch,
            last_desktop_switch: a.last_desktop_switch,
            token_status: health.status,
            token_remaining_mins: health.remaining_mins,
            credentials_healthy,
            verify_status: a.verify_status.clone(),
            verified_email: a.verified_email.clone(),
            verified_at: a.verified_at,
            // Derive from verify_status, not `verified_email != email`:
            // update_verification() preserves verified_email across
            // rejected/network_error so a transient blip doesn't wipe
            // history; comparing emails would spuriously paint stale
            // history as drift.
            drift: a.verify_status == "drift",
            desktop_profile_on_disk,
        }
    }
}

/// Pure field copy from the listing-time aggregate. The Keychain
/// read + filesystem stat happened upstream in
/// `services::account_summary::list_summaries`; this impl is the
/// thin DTO boundary the webview wants.
impl From<&claudepot_core::services::account_summary::AccountSummaryView> for AccountSummary {
    fn from(v: &claudepot_core::services::account_summary::AccountSummaryView) -> Self {
        let a = &v.account;
        Self {
            uuid: a.uuid.to_string(),
            email: a.email.clone(),
            org_name: a.org_name.clone(),
            subscription_type: a.subscription_type.clone(),
            is_cli_active: a.is_cli_active,
            is_desktop_active: a.is_desktop_active,
            has_cli_credentials: a.has_cli_credentials,
            has_desktop_profile: a.has_desktop_profile,
            last_cli_switch: a.last_cli_switch,
            last_desktop_switch: a.last_desktop_switch,
            token_status: v.token_health.status.clone(),
            token_remaining_mins: v.token_health.remaining_mins,
            credentials_healthy: v.credentials_healthy,
            verify_status: a.verify_status.clone(),
            verified_email: a.verified_email.clone(),
            verified_at: a.verified_at,
            drift: v.drift,
            desktop_profile_on_disk: v.desktop_profile_on_disk,
        }
    }
}

#[cfg(test)]
mod account_summary_dto_tests {
    use super::*;
    use claudepot_core::account::Account;
    use claudepot_core::services::account_service::TokenHealth;
    use claudepot_core::services::account_summary::AccountSummaryView;

    fn sample_account(email: &str) -> Account {
        Account {
            uuid: uuid::Uuid::new_v4(),
            email: email.to_string(),
            org_uuid: Some("org-test".to_string()),
            org_name: Some("Test Org".to_string()),
            subscription_type: Some("pro".to_string()),
            rate_limit_tier: None,
            created_at: chrono::Utc::now(),
            last_cli_switch: None,
            last_desktop_switch: None,
            has_cli_credentials: true,
            has_desktop_profile: false,
            is_cli_active: false,
            is_desktop_active: false,
            verified_email: None,
            verified_at: None,
            verify_status: "never".to_string(),
        }
    }

    /// The new `From<&AccountSummaryView>` impl must be a pure field
    /// copy — no Keychain reads, no filesystem stats. This test
    /// confirms every field on `AccountSummary` traces back to a
    /// plain field on either `view.account` or `view` itself.
    #[test]
    fn test_account_summary_dto_is_pure_field_copy() {
        let mut account = sample_account("copy@example.com");
        account.is_cli_active = true;
        account.has_desktop_profile = true;
        account.verify_status = "drift".to_string();
        account.verified_email = Some("other@example.com".to_string());

        let view = AccountSummaryView {
            account: account.clone(),
            token_health: TokenHealth {
                status: "valid (1h)".to_string(),
                remaining_mins: Some(60),
            },
            credentials_healthy: true,
            desktop_profile_on_disk: true,
            drift: true,
        };

        let dto = AccountSummary::from(&view);

        assert_eq!(dto.uuid, account.uuid.to_string());
        assert_eq!(dto.email, account.email);
        assert_eq!(dto.org_name, account.org_name);
        assert_eq!(dto.subscription_type, account.subscription_type);
        assert_eq!(dto.is_cli_active, account.is_cli_active);
        assert_eq!(dto.is_desktop_active, account.is_desktop_active);
        assert_eq!(dto.has_cli_credentials, account.has_cli_credentials);
        assert_eq!(dto.has_desktop_profile, account.has_desktop_profile);
        assert_eq!(dto.last_cli_switch, account.last_cli_switch);
        assert_eq!(dto.last_desktop_switch, account.last_desktop_switch);
        assert_eq!(dto.token_status, "valid (1h)");
        assert_eq!(dto.token_remaining_mins, Some(60));
        assert!(dto.credentials_healthy);
        assert_eq!(dto.verify_status, account.verify_status);
        assert_eq!(dto.verified_email, account.verified_email);
        assert_eq!(dto.verified_at, account.verified_at);
        assert!(dto.drift);
        assert!(dto.desktop_profile_on_disk);
    }
}

/// Counts-only summary of the reconcile pass crossed back to the
/// webview. Per-row detail (which uuid flipped, which email)
/// intentionally stays in the Rust [`ReconcileReport`]; the JS side
/// renders a one-line "synced N flags" toast and refetches
/// `account_list`, so the bytes-on-the-wire payload is just three
/// counters.
#[derive(Serialize, Clone)]
pub struct ReconcileReportDto {
    pub cli_flipped: usize,
    pub desktop_flipped: usize,
    pub orphan_pointer_cleared: bool,
}

impl From<&claudepot_core::services::account_service::ReconcileReport> for ReconcileReportDto {
    fn from(r: &claudepot_core::services::account_service::ReconcileReport) -> Self {
        Self {
            cli_flipped: r.cli_flips.len(),
            desktop_flipped: r.desktop.flag_flips.len(),
            orphan_pointer_cleared: r.desktop.orphan_pointer_cleared,
        }
    }
}
#[derive(Serialize)]
pub struct AppStatus {
    pub platform: String,
    pub arch: String,
    pub cli_active_email: Option<String>,
    pub desktop_active_email: Option<String>,
    pub desktop_installed: bool,
    pub data_dir: String,
    /// Absolute path of CC's config dir (`~/.claude`). The webview uses
    /// this to construct paths it hands straight back to
    /// `reveal_in_finder` — for example the session transcript at
    /// `<cc_config_dir>/projects/<slug>/<session_id>.jsonl`. Read-only
    /// metadata; shares code with `paths::claude_config_dir()` so the
    /// JS side never has to guess the home directory.
    pub cc_config_dir: String,
    pub account_count: usize,
}

#[derive(Serialize)]
pub struct RegisterOutcome {
    pub email: String,
    pub org_name: String,
    pub subscription_type: String,
}

#[derive(Serialize)]
pub struct RemoveOutcome {
    pub email: String,
    pub was_cli_active: bool,
    pub was_desktop_active: bool,
    pub had_desktop_profile: bool,
    pub warnings: Vec<String>,
}
/// Ground-truth "what is CC actually authenticated as right now".
///
/// Produced by the `current_cc_identity` Tauri command: reads CC's
/// shared credential slot, calls `/api/oauth/profile`, returns the
/// email the server confirms. The GUI's top-of-window truth strip
/// renders this directly — it's what `claude auth status` would print.
#[derive(Serialize)]
pub struct CcIdentity {
    /// The email `/api/oauth/profile` returned. `None` if CC has no
    /// stored blob or the blob is not parseable JSON.
    pub email: Option<String>,
    /// RFC3339 timestamp of when we ran the profile check. Lets the UI
    /// show "verified Ns ago" staleness.
    pub verified_at: chrono::DateTime<chrono::Utc>,
    /// Populated when CC has a blob but `/profile` failed — separate
    /// from `email=None` so the UI can distinguish "no CC credentials"
    /// from "couldn't reach the server" from "token revoked".
    pub error: Option<String>,
}
