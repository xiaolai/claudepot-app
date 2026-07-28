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

use claudepot_core::cc_retention::{
    clear_retention, disable_persistence, report, set_retention_days, RetentionReport,
    DEFAULT_RISK_HORIZON_DAYS,
};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `retention_report` — current `cleanupPeriodDays` plus what it costs
/// on this machine (how many transcripts CC will delete, and when).
#[tauri::command]
pub async fn retention_report(horizon_days: Option<i64>) -> Result<RetentionReport, String> {
    let horizon = horizon_days.unwrap_or(DEFAULT_RISK_HORIZON_DAYS).max(1);
    Ok(report(now_ms(), horizon))
}

/// `retention_set` — write an explicit positive retention window.
/// Rejects `0` and negatives; zero is only reachable through
/// [`retention_disable_persistence`].
#[tauri::command]
pub async fn retention_set(days: i64) -> Result<RetentionReport, String> {
    set_retention_days(days).map_err(|e| format!("write setting: {e}"))?;
    Ok(report(now_ms(), DEFAULT_RISK_HORIZON_DAYS))
}

/// `retention_clear` — remove the key so CC's 30-day default applies.
/// This re-arms deletion, so the UI confirms before calling it.
#[tauri::command]
pub async fn retention_clear() -> Result<RetentionReport, String> {
    clear_retention().map_err(|e| format!("write setting: {e}"))?;
    Ok(report(now_ms(), DEFAULT_RISK_HORIZON_DAYS))
}

/// `retention_disable_persistence` — write `cleanupPeriodDays: 0`.
/// CC then writes no transcripts at all and deletes the existing ones
/// at next startup. Named for what it does; the UI puts it behind a
/// `ConfirmDialog` and never on the duration scale.
#[tauri::command]
pub async fn retention_disable_persistence() -> Result<RetentionReport, String> {
    disable_persistence().map_err(|e| format!("write setting: {e}"))?;
    Ok(report(now_ms(), DEFAULT_RISK_HORIZON_DAYS))
}
