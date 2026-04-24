//! Frontend DTOs — what crosses the Tauri command boundary.
//!
//! We deliberately do NOT expose credential blobs, access tokens, or refresh
//! tokens to the webview. Only non-sensitive metadata leaves Rust.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

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

impl From<&claudepot_core::account::Account> for AccountSummary {
    fn from(a: &claudepot_core::account::Account) -> Self {
        let health =
            claudepot_core::services::account_service::token_health(a.uuid, a.has_cli_credentials);
        // A stored blob is "healthy" if it exists and parses. Any other
        // status ("missing", "corrupt blob", "no credentials") means the
        // swap can't succeed — the UI should gate on this, not the DB flag.
        let credentials_healthy = health.status.starts_with("valid") || health.status == "expired";
        // Cheap on-disk check per plan v2 §D18: just exists(), no
        // recursive walk. Size + enumeration moves to
        // desktop_profile_info(uuid) in a later phase.
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
            // Derive from verify_status, not `verified_email != email`.
            // update_verification() intentionally preserves
            // verified_email across rejected/network_error so history
            // isn't wiped by a blip — meaning a stored row where
            // verified_email still points at the old drift target but
            // verify_status has since moved to "network_error" would
            // spuriously paint as drift if we compared emails.
            drift: a.verify_status == "drift",
            desktop_profile_on_disk,
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

/// A single usage window (utilization + reset time).
#[derive(Serialize, Clone)]
pub struct UsageWindowDto {
    // resets_at is optional: the server returns null for windows with
    // no activity yet. The frontend renders "\u2014" when missing.
    pub utilization: f64,
    pub resets_at: Option<String>, // RFC3339; null when the window has no reset yet
}

/// Extra-usage (monthly overage billing) info.
#[derive(Serialize, Clone)]
pub struct ExtraUsageDto {
    pub is_enabled: bool,
    /// Monthly cap and spend are returned in MINOR currency units
    /// (e.g. pence for GBP, cents for USD). Frontend divides by 100
    /// before rendering.
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    /// Server-computed utilization percent (0–100). Preferred over a
    /// client-side `used / limit` recompute since the server knows
    /// about rollover, prorated credits, and grace adjustments the
    /// bare ratio misses.
    pub utilization: Option<f64>,
    /// ISO 4217 currency code ("USD", "GBP", …). None on older
    /// responses; frontend falls back to USD.
    pub currency: Option<String>,
}

/// Per-account usage data. `None` fields mean the window is not active
/// for this subscription type, or no data is available.
#[derive(Serialize, Clone)]
pub struct AccountUsageDto {
    pub five_hour: Option<UsageWindowDto>,
    pub seven_day: Option<UsageWindowDto>,
    pub seven_day_opus: Option<UsageWindowDto>,
    pub seven_day_sonnet: Option<UsageWindowDto>,
    /// Usage attributed to third-party OAuth apps authorized against
    /// this account (IDEs, tools, etc). Null on plans that don't
    /// split this out; GUI renders render-if-nonzero.
    pub seven_day_oauth_apps: Option<UsageWindowDto>,
    /// Usage attributed to cowork / shared-seat pool. Null on
    /// personal plans; GUI renders render-if-nonzero.
    pub seven_day_cowork: Option<UsageWindowDto>,
    pub extra_usage: Option<ExtraUsageDto>,
}

impl AccountUsageDto {
    pub fn from_response(r: &claudepot_core::oauth::usage::UsageResponse) -> Self {
        let map_window = |w: &Option<claudepot_core::oauth::usage::UsageWindow>| {
            w.as_ref().map(|w| UsageWindowDto {
                utilization: w.utilization,
                resets_at: w.resets_at.as_ref().map(|t| t.to_rfc3339()),
            })
        };
        Self {
            five_hour: map_window(&r.five_hour),
            seven_day: map_window(&r.seven_day),
            seven_day_opus: map_window(&r.seven_day_opus),
            seven_day_sonnet: map_window(&r.seven_day_sonnet),
            seven_day_oauth_apps: map_window(&r.seven_day_oauth_apps),
            seven_day_cowork: map_window(&r.seven_day_cowork),
            extra_usage: r.extra_usage.as_ref().map(|e| ExtraUsageDto {
                is_enabled: e.is_enabled,
                monthly_limit: e.monthly_limit,
                used_credits: e.used_credits,
                utilization: e.utilization,
                currency: e.currency.clone(),
            }),
        }
    }
}

/// Per-account usage entry for the GUI. Carries enough state so the
/// UI can render an inline explanation when usage is unavailable
/// instead of silently dropping the account.
///
/// Status values (string-typed so the TS side can narrow on them):
///   - "ok"            — fresh data
///   - "stale"         — cached data, see `age_secs`
///   - "no_credentials"— account has no stored blob (shouldn't happen
///                       for has_cli_credentials=true, included for
///                       completeness)
///   - "expired"       — token past local expiry
///   - "rate_limited"  — on cooldown, no cache fallback
///   - "error"         — see `error_detail`
#[derive(Serialize, Clone)]
pub struct UsageEntryDto {
    pub status: String,
    /// Populated for "ok" and "stale".
    pub usage: Option<AccountUsageDto>,
    /// Seconds since the cache entry was written. Populated for "stale"
    /// and (approximately 0) for "ok".
    pub age_secs: Option<u64>,
    /// For "rate_limited": seconds until the cooldown clears.
    pub retry_after_secs: Option<u64>,
    /// For "error": a short technical string (e.g. "http 502",
    /// "timeout", "invalid json"). **This IS rendered verbatim** in
    /// the detail pane's UsageUnavailable block — keep the source
    /// (`UsageFetchError::FetchFailed`) free of tokens, URLs, or
    /// anything else that would be unsafe to show the user. The
    /// source is already scrubbed: it mirrors the error chain from
    /// the HTTP fetcher without request/response bodies.
    pub error_detail: Option<String>,
}

impl UsageEntryDto {
    pub fn from_outcome(
        outcome: claudepot_core::services::usage_cache::UsageOutcome,
    ) -> Self {
        use claudepot_core::services::usage_cache::UsageOutcome;
        match outcome {
            UsageOutcome::Fresh { response, age_secs } => Self {
                status: "ok".to_string(),
                usage: Some(AccountUsageDto::from_response(&response)),
                age_secs: Some(age_secs),
                retry_after_secs: None,
                error_detail: None,
            },
            UsageOutcome::Stale { response, age_secs } => Self {
                status: "stale".to_string(),
                usage: Some(AccountUsageDto::from_response(&response)),
                age_secs: Some(age_secs),
                retry_after_secs: None,
                error_detail: None,
            },
            UsageOutcome::NoCredentials => Self {
                status: "no_credentials".to_string(),
                usage: None,
                age_secs: None,
                retry_after_secs: None,
                error_detail: None,
            },
            UsageOutcome::Expired => Self {
                status: "expired".to_string(),
                usage: None,
                age_secs: None,
                retry_after_secs: None,
                error_detail: None,
            },
            UsageOutcome::RateLimited { retry_after_secs } => Self {
                status: "rate_limited".to_string(),
                usage: None,
                age_secs: None,
                retry_after_secs: Some(retry_after_secs),
                error_detail: None,
            },
            UsageOutcome::Error(msg) => Self {
                status: "error".to_string(),
                usage: None,
                age_secs: None,
                retry_after_secs: None,
                error_detail: Some(msg),
            },
        }
    }
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

// ---------------------------------------------------------------------------
// Shared helpers used by the split-out DTO modules.
// ---------------------------------------------------------------------------

/// Millisecond epoch helper. `SystemTime` isn't directly serde-friendly
/// for the JS heap; the webview wants a number it can pass to
/// `new Date()`. Visible to sibling `dto_*` modules that need to
/// translate `Option<SystemTime>` fields at the boundary.
pub(crate) fn system_time_to_ms(t: Option<SystemTime>) -> Option<i64> {
    t.and_then(|st| st.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

// ---------------------------------------------------------------------------
// Topic-split DTO re-exports.
//
// The webview-facing DTOs are grouped by section into sibling files
// (`dto_session.rs`, `dto_project.rs`, etc.) to keep each file under
// the LOC ceiling. Consumers `use crate::dto::Foo` as before — the
// re-exports below preserve that surface so `commands.rs` doesn't
// need to track the split. A handful of them are referenced only via
// their sibling module (`crate::dto_session::Foo`) today; they're
// retained here as the canonical facade so new callers have one
// obvious place to look.
// ---------------------------------------------------------------------------

#[allow(unused_imports)]
pub use crate::{
    dto_activity::{
        ActivityTrendsDto, LiveDeltaDto, LiveDeltaKindDto, LiveSessionSummaryDto,
    },
    dto_desktop::{
        DesktopAdoptOutcome, DesktopClearOutcome, DesktopIdentity, DesktopProbeMethod,
        DesktopSyncOutcome,
    },
    dto_keys::{ApiKeySummaryDto, OauthTokenSummaryDto},
    dto_project::{
        CleanPreviewDto, DryRunPlanDto, MoveArgsDto, PendingJournalsSummaryDto,
        ProjectDetailDto, ProjectInfoDto, SessionInfoDto,
    },
    dto_project_repair::{JournalEntryDto, JournalFlagsDto},
    dto_session::{
        ProtectedPathDto, SessionDetailDto, SessionEventDto, SessionRowDto, TokenUsageDto,
    },
    dto_session_debug::{
        ChunkHeaderDto, ChunkMetricsDto, ContextCategoryDto, ContextInjectionDto,
        ContextPhaseDto, ContextStatsDto, LinkedToolDto, RepositoryGroupDto, SearchHitDto,
        SessionChunkDto, TokensByCategoryDto,
    },
    dto_session_move::{
        AdoptFailureDto, AdoptReportDto, DiscardReportDto, MoveSessionReportDto,
        OrphanedProjectDto,
    },
    dto_session_prune::{
        BulkSlimEntryDto, BulkSlimPlanDto, PruneEntryDto, PruneFilterDto, PrunePlanDto,
        SlimOptsDto, SlimPlanDto, TrashEntryDto, TrashListingDto,
    },
};
