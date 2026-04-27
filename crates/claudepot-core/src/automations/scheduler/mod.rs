//! Cross-platform scheduler abstraction.
//!
//! One trait, three adapters (one per OS). The trait separates
//! "what's the same on every OS" (registering, unregistering,
//! kicking, listing, capabilities) from "what's per-OS" (the
//! artifact format and the CLI tool that loads it).
//!
//! v1 ships with launchd (macOS) as the only fully-implemented
//! adapter; Windows (`schtasks.rs`) and Linux (`systemd.rs`) land
//! in subsequent commits. A `NoopScheduler` is provided so the
//! rest of the codebase can compile + test without an active
//! scheduler — useful in CI and on unsupported targets.
//!
//! Capabilities are surfaced honestly: a toggle the OS can't
//! support is reported as `false`, the UI greys it out, and the
//! adapter does not silently lie about the schedule.

use chrono::{DateTime, Utc};

use super::error::AutomationError;
use super::types::{Automation, AutomationId, Trigger};

pub mod noop;

/// Capabilities the active platform supports. Drives UI greying
/// in the Add/Edit modal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SchedulerCapabilities {
    /// `<WakeToRun>` (Windows) or `WakeSystem=true` (systemd) —
    /// system wakes from sleep to run the job.
    pub wake_to_run: bool,
    /// `<StartWhenAvailable>` (Windows) or `Persistent=true`
    /// (systemd) — fire a missed run once when the system becomes
    /// available again.
    pub catch_up_if_missed: bool,
    /// Job runs even when no user is logged in. Windows: requires
    /// stored credentials. Linux: requires `loginctl enable-linger`.
    /// macOS LaunchAgents: never (LaunchDaemons are out of scope).
    pub run_when_logged_out: bool,
    /// Display name for the underlying scheduler (shown in the UI
    /// help text).
    pub native_label: &'static str,
    /// `shell::open`-able URI pointing at where the artifacts live.
    pub artifact_dir: Option<String>,
}

/// Summary of one registration the active scheduler reports.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegisteredEntry {
    pub identifier: String,
    pub claudepot_managed: bool,
}

/// Scheduler operations. Implementations are stateless: every
/// method takes the inputs it needs.
pub trait Scheduler {
    /// Render the OS artifact and register it. Idempotent —
    /// replacing a registration always goes through unregister
    /// + register internally.
    fn register(&self, automation: &Automation) -> Result<(), AutomationError>;

    /// Remove the registration and delete the artifact. Idempotent
    /// — "not found" is success.
    fn unregister(&self, id: &AutomationId) -> Result<(), AutomationError>;

    /// Trigger an out-of-schedule run via the OS scheduler.
    /// Distinct from the in-process Run-Now path, which spawns
    /// the helper shim directly under tokio.
    fn kickstart(&self, id: &AutomationId) -> Result<(), AutomationError>;

    /// Enumerate all `claudepot_managed` registrations the OS
    /// reports.
    fn list_managed(&self) -> Result<Vec<RegisteredEntry>, AutomationError>;

    /// Compute the next `n` fire times for a given trigger, in
    /// UTC. Pure function — does not touch the OS. Returns an
    /// empty vec if the trigger has no upcoming fire time.
    fn next_runs(
        &self,
        trigger: &Trigger,
        from: DateTime<Utc>,
        n: usize,
    ) -> Result<Vec<DateTime<Utc>>, AutomationError>;

    /// Surface the capability matrix.
    fn capabilities(&self) -> SchedulerCapabilities;
}

/// Construct the active scheduler for the current host. Returns
/// the `Noop` adapter on unsupported platforms (caller can detect
/// this via `capabilities().native_label`).
///
/// In v1 every OS returns `Noop` until each adapter ships. This
/// keeps the rest of the wiring safe to land first.
pub fn active_scheduler() -> Box<dyn Scheduler> {
    Box::new(noop::NoopScheduler)
}

/// Compute the next `n` fire times of a [`Trigger`] starting at
/// `from`. Shared by every adapter — the OS-specific calendar
/// triggers all encode the same cron semantics, so the math lives
/// here.
pub fn cron_next_runs(
    cron_expr: &str,
    from: DateTime<Utc>,
    n: usize,
) -> Result<Vec<DateTime<Utc>>, AutomationError> {
    use super::cron;
    let slots = cron::expand(cron_expr)?;
    if slots.is_empty() || n == 0 {
        return Ok(Vec::new());
    }
    // Walk forward from `from` minute-by-minute up to a year and
    // collect matches. A year is 365 * 24 * 60 = 525_600 minutes;
    // we cap at `n * 1440 * 31` (i.e., n months of search) to keep
    // pathological "every February 29th" expressions bounded.
    let cap_minutes: i64 = (n as i64).saturating_mul(1440 * 31).min(525_600);

    let mut out = Vec::with_capacity(n);
    // Round `from` up to the next minute boundary to mimic
    // launchd/systemd one-minute resolution.
    let mut cursor = from
        .with_timezone(&Utc)
        .checked_add_signed(chrono::Duration::seconds(60))
        .ok_or_else(|| AutomationError::InvalidCron(cron_expr.into(), "time arithmetic overflow".into()))?;
    cursor = truncate_to_minute(cursor);

    for _ in 0..cap_minutes {
        if matches_any_slot(cursor, &slots) {
            out.push(cursor);
            if out.len() >= n {
                break;
            }
        }
        cursor = match cursor.checked_add_signed(chrono::Duration::minutes(1)) {
            Some(x) => x,
            None => break,
        };
    }
    Ok(out)
}

fn truncate_to_minute(dt: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::Timelike;
    dt.with_second(0)
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(dt)
}

fn matches_any_slot(dt: DateTime<Utc>, slots: &[super::cron::LaunchSlot]) -> bool {
    use chrono::{Datelike, Timelike};
    let m = dt.minute() as u8;
    let h = dt.hour() as u8;
    let dom = dt.day() as u8;       // 1..=31
    let mon = dt.month() as u8;     // 1..=12
    // chrono Weekday: Mon=0..Sun=6. We want cron Sun=0..Sat=6.
    let dow = match dt.weekday() {
        chrono::Weekday::Sun => 0u8,
        chrono::Weekday::Mon => 1,
        chrono::Weekday::Tue => 2,
        chrono::Weekday::Wed => 3,
        chrono::Weekday::Thu => 4,
        chrono::Weekday::Fri => 5,
        chrono::Weekday::Sat => 6,
    };
    slots.iter().any(|s| {
        s.minute == m
            && s.hour == h
            && s.month.map(|sm| sm == mon).unwrap_or(true)
            && s.day_of_month.map(|sd| sd == dom).unwrap_or(true)
            && s.day_of_week.map(|sw| sw == dow).unwrap_or(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn cron_next_runs_daily_at_9() {
        // 2026-04-28 was a Tuesday. "0 9 * * *" daily at 09:00 UTC.
        let from = Utc.with_ymd_and_hms(2026, 4, 28, 8, 30, 0).unwrap();
        let next = cron_next_runs("0 9 * * *", from, 3).unwrap();
        assert_eq!(next.len(), 3);
        assert_eq!(next[0], Utc.with_ymd_and_hms(2026, 4, 28, 9, 0, 0).unwrap());
        assert_eq!(next[1], Utc.with_ymd_and_hms(2026, 4, 29, 9, 0, 0).unwrap());
        assert_eq!(next[2], Utc.with_ymd_and_hms(2026, 4, 30, 9, 0, 0).unwrap());
    }

    #[test]
    fn cron_next_runs_weekdays_only() {
        // Mon-Fri at 09:00. From a Friday at 10:00 UTC.
        // 2026-05-01 is Friday → next Monday is 2026-05-04.
        let from = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();
        let next = cron_next_runs("0 9 * * 1-5", from, 3).unwrap();
        assert_eq!(next.len(), 3);
        assert_eq!(next[0], Utc.with_ymd_and_hms(2026, 5, 4, 9, 0, 0).unwrap()); // Mon
        assert_eq!(next[1], Utc.with_ymd_and_hms(2026, 5, 5, 9, 0, 0).unwrap()); // Tue
        assert_eq!(next[2], Utc.with_ymd_and_hms(2026, 5, 6, 9, 0, 0).unwrap()); // Wed
    }

    #[test]
    fn cron_next_runs_step_quarter_hour() {
        // Every 15 minutes during hour 9 only.
        let from = Utc.with_ymd_and_hms(2026, 4, 28, 9, 7, 0).unwrap();
        let next = cron_next_runs("*/15 9 * * *", from, 3).unwrap();
        assert_eq!(next[0].minute(), 15);
        assert_eq!(next[1].minute(), 30);
        assert_eq!(next[2].minute(), 45);

        use chrono::Timelike;
        for r in &next {
            assert_eq!(r.hour(), 9);
        }
    }

    #[test]
    fn cron_next_runs_zero_returns_empty() {
        let from = Utc::now();
        let next = cron_next_runs("0 9 * * *", from, 0).unwrap();
        assert!(next.is_empty());
    }

    #[test]
    fn cron_next_runs_invalid_expr_propagates() {
        let from = Utc::now();
        assert!(cron_next_runs("not a cron", from, 3).is_err());
    }
}
