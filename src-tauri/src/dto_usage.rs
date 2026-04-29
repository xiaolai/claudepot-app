//! Usage / rate-limit DTOs that cross the Tauri boundary.
//!
//! Sharded out of `dto.rs`; mirrors the windows the GUI's UsageBlock
//! renders. Currency / extras handling lives here too.

use serde::Serialize;

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
///     for has_cli_credentials=true, included for completeness)
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
    pub fn from_outcome(outcome: claudepot_core::services::usage_cache::UsageOutcome) -> Self {
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
