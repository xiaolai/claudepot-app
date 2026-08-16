//! IPC commands for Settings → Retention.
//!
//! Reads / writes CC's user-level `cleanupPeriodDays` via
//! `claudepot_core::cc_retention` — the one CC setting that destroys
//! user data, and the one nothing in CC's own UI ever mentions.
//!
//! Per `rules/architecture.md`, NO business logic here: the state/risk
//! composition lives in `cc_retention::report`, and this layer only
//! samples the clock and maps errors. The returned types are plain
//! counts and enums with no path or secret fields, so they cross to JS
//! directly rather than through a hand-mirrored DTO — same call as
//! `auto_dream`.
//!
//! The write surface is deliberately narrow: `retention_set` refuses
//! anything below CC's floor of `1`, and there is **no** command that
//! writes `0`. There used to be one (`retention_disable_persistence`);
//! it was removed when CC 2.1.233 started rejecting `0`, because every
//! possible implementation of that verb now writes a value CC refuses
//! and reports success. See `claudepot_core::cc_retention` for the
//! evidence and `RetentionMode::LegacyZero` for what a `0` written by
//! an older Claudepot does today.

use crate::dto_error::ErrorDto;
use claudepot_core::cc_retention::{
    clear_retention, report, set_retention_days, RetentionReport, DEFAULT_RISK_HORIZON_DAYS,
};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `retention_report` — current `cleanupPeriodDays` plus what it costs
/// on this machine (how many transcripts CC will delete, and when).
///
/// Infallible today; the `ErrorDto` shape is here so a future read
/// failure reaches `RetentionPane`'s load-error branch as a code. That
/// branch matters more here than elsewhere: a pane stuck on "Loading…"
/// reads as "still working", which in this pane reads as "still safe".
#[tauri::command]
pub async fn retention_report(horizon_days: Option<i64>) -> Result<RetentionReport, ErrorDto> {
    let horizon = horizon_days.unwrap_or(DEFAULT_RISK_HORIZON_DAYS).max(1);
    Ok(report(now_ms(), horizon))
}

/// `retention_set` — write an explicit retention window. Rejects
/// anything below CC's floor of `1`; there is no command that writes a
/// lower value.
#[tauri::command]
pub async fn retention_set(days: i64) -> Result<RetentionReport, ErrorDto> {
    // The `write setting: ` prefix moves to the UI. What must not move
    // is the distinction core already draws: `0` is rejected here as
    // `cc_retention.non_positive`, never as an ordinary out-of-range
    // number — CC rejects it too, and a `0` that lands in the file
    // suppresses cleanup rather than doing nothing.
    set_retention_days(days).map_err(ErrorDto::from)?;
    Ok(report(now_ms(), DEFAULT_RISK_HORIZON_DAYS))
}

/// `retention_clear` — remove the key so CC's 30-day default applies.
///
/// This re-arms deletion, so the UI confirms before calling it. It is
/// also the repair for a legacy `0`: removing the key is the only way
/// to clear the validation error that is currently suppressing cleanup.
#[tauri::command]
pub async fn retention_clear() -> Result<RetentionReport, ErrorDto> {
    clear_retention().map_err(ErrorDto::from)?;
    Ok(report(now_ms(), DEFAULT_RISK_HORIZON_DAYS))
}
