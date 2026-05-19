//! Retention windows for the long-lived SQLite history surfaces.
//!
//! Two tables today carry per-tick / per-event history that would
//! otherwise grow forever:
//!
//! - `activity_cards` (lives in `~/.claudepot/sessions.db`, owned by
//!   [`crate::activity`]) — one card per classified JSONL event.
//!   Pruned at startup from `src-tauri/src/lib.rs::run`.
//! - `metrics_tick` (lives in `~/.claudepot/activity_metrics.db`,
//!   owned by [`crate::session_live::metrics_store`]) — one row per
//!   live session per 500 ms heartbeat. Pruned at startup from
//!   `LiveRuntime::start`.
//!
//! Both tables share the same retention horizon so a heavy-use user
//! sees their Activities pane and Trends view consistent — surfacing
//! cards for sessions whose metrics have been pruned (or vice-versa)
//! would be confusing. Keep the value in one place.
//!
//! The number matches `memory_log::MAX_ROW_AGE_NS` (90 days) — long
//! enough to keep a quarter's worth of session history visible, short
//! enough that the per-table queries stay fast on a year-old install.

/// Retention horizon, in days, for `activity_cards` and `metrics_tick`.
/// Used by both startup-prune sites to compute the `cutoff_ms`
/// argument to `prune_before`.
pub const RETENTION_DAYS: i64 = 90;

/// Milliseconds in [`RETENTION_DAYS`]. Convenience constant so the
/// two call sites don't each repeat the `24 * 60 * 60 * 1_000`
/// expansion (and so a future change to the unit is a single-site
/// edit).
pub const RETENTION_MS: i64 = RETENTION_DAYS * 24 * 60 * 60 * 1_000;

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock down the math so a future "let's tweak the constant" diff
    /// doesn't accidentally change the unit. If RETENTION_DAYS moves,
    /// RETENTION_MS must move with it.
    #[test]
    fn retention_ms_is_days_in_milliseconds() {
        assert_eq!(RETENTION_MS, RETENTION_DAYS * 24 * 60 * 60 * 1_000);
    }

    /// Sanity: 90 days fits comfortably inside an i64 millisecond
    /// budget. (i64::MAX ≈ 9.22e18 ms ≈ 292 million years; we use
    /// ~7.78e9 ms.)
    #[test]
    fn retention_ms_fits_in_i64() {
        assert!(RETENTION_MS > 0);
        assert!(RETENTION_MS < i64::MAX / 1_000);
    }
}
