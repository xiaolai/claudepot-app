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
//! Note the deliberate asymmetry in the write surface: `retention_set`
//! refuses `0`, and reaching zero requires the separately-named
//! `retention_disable_persistence`. A renderer bug that passes a stray
//! `0` through the ordinary setter must not be able to delete the
//! user's history.

use crate::dto_error::ErrorDto;
use claudepot_core::cc_retention::{
    clear_retention, disable_persistence, report, set_retention_days, RetentionReport,
    DEFAULT_RISK_HORIZON_DAYS,
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

/// `retention_set` — write an explicit positive retention window.
/// Rejects `0` and negatives; zero is only reachable through
/// [`retention_disable_persistence`].
#[tauri::command]
pub async fn retention_set(days: i64) -> Result<RetentionReport, ErrorDto> {
    // The `write setting: ` prefix moves to the UI. What must not move
    // is the distinction core already draws: `0` is rejected here as
    // `cc_retention.non_positive` — the most destructive value on this
    // key, never an ordinary out-of-range number.
    set_retention_days(days).map_err(ErrorDto::from)?;
    Ok(report(now_ms(), DEFAULT_RISK_HORIZON_DAYS))
}

/// `retention_clear` — remove the key so CC's 30-day default applies.
/// This re-arms deletion, so the UI confirms before calling it.
#[tauri::command]
pub async fn retention_clear() -> Result<RetentionReport, ErrorDto> {
    clear_retention().map_err(ErrorDto::from)?;
    Ok(report(now_ms(), DEFAULT_RISK_HORIZON_DAYS))
}

/// `retention_disable_persistence` — write `cleanupPeriodDays: 0`.
/// CC then writes no transcripts at all and deletes the existing ones
/// at next startup. Named for what it does; the UI puts it behind a
/// `ConfirmDialog` and never on the duration scale.
#[tauri::command]
pub async fn retention_disable_persistence() -> Result<RetentionReport, ErrorDto> {
    disable_persistence().map_err(ErrorDto::from)?;
    Ok(report(now_ms(), DEFAULT_RISK_HORIZON_DAYS))
}
