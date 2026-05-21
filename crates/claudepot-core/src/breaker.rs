//! Consecutive-failure circuit breaker — pure logic.
//!
//! Claudepot's background orchestrators (`rotation_orchestrator`,
//! `permission_orchestrator`) drive a fixed action every
//! `usage_snapshot::run_tick` (~5 min). When an action fails they
//! leave it to retry next tick — *forever*. A permanently-broken
//! rotation rule re-fires and re-fails every 5 minutes; a permission
//! revert that keeps failing retries every tick. Nothing bounds the
//! retry.
//!
//! This module is the bound. A [`FailureLedger`] counts *consecutive*
//! failures for one action. After [`THRESHOLD`] of them the breaker
//! [`evaluate`]s as [`BreakerVerdict::Tripped`] and the orchestrator
//! quarantines the action instead of retrying it. After [`COOLDOWN`]
//! has passed since the last failure, one *probe* retry is allowed —
//! if it succeeds the ledger resets; if it fails the ledger keeps
//! climbing and the breaker re-trips immediately.
//!
//! Pure: every function takes the wall-clock as an injected `now`
//! parameter, performs no I/O, and is golden-tested on all host OSs.
//! The orchestrators own the *persistence* of a [`FailureLedger`]
//! (rotation in `rotation::breaker_store`, permission inline on the
//! `Grant` struct); this module only does the arithmetic.

use chrono::{DateTime, Duration, Utc};

/// Consecutive failures that trip the breaker. The third failure in a
/// row quarantines the action. Chosen to mirror `octoally`'s
/// session-manager lesson — two failures can be a transient
/// network/identity flap; three in a row across ~15 minutes
/// (3 ticks) is a real fault worth surfacing instead of retrying
/// silently. Hardcoded in the style of
/// `rotation_orchestrator::PENDING_TTL_SECS` — no config surface
/// until one is asked for.
pub const THRESHOLD: u32 = 3;

/// Cooldown after the last failure of a tripped breaker, after which
/// exactly one probe retry is permitted. Six hours: long enough that
/// a tripped action stops spamming the 5-minute tick (a tripped
/// breaker would otherwise be re-evaluated ~72 times before the
/// probe), short enough that a transient outage that has since
/// healed recovers within a working day without the user manually
/// clearing the breaker.
pub const COOLDOWN: Duration = Duration::hours(6);

/// Per-action failure history. Plain data — the orchestrators embed
/// this (rotation persists it in `rotation-breaker.json` keyed by
/// `rule_id`; permission rides the two new `Grant` fields). `Default`
/// is the clean "no failures yet" state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FailureLedger {
    /// Number of failures in an unbroken run. Reset to zero by
    /// [`record_success`].
    pub consecutive: u32,
    /// When the most recent failure happened. `None` iff
    /// `consecutive == 0`.
    pub last_failure: Option<DateTime<Utc>>,
}

/// What the breaker says about an action right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerVerdict {
    /// Run the action. Either the failure count is below threshold,
    /// or it is at/above threshold but the cooldown has elapsed and
    /// this run is the permitted probe retry.
    Allow,
    /// Quarantine the action. The threshold has been crossed and the
    /// cooldown since the last failure has not yet elapsed.
    Tripped,
}

/// Decide whether an action may run, given its failure history and
/// the current time. Pure — `now` is injected.
///
/// - Below [`THRESHOLD`] consecutive failures → always [`Allow`].
/// - At/above threshold, within [`COOLDOWN`] of the last failure →
///   [`Tripped`].
/// - At/above threshold, but [`COOLDOWN`] has elapsed since the last
///   failure → [`Allow`] (the single probe retry). The ledger is not
///   mutated here; if the probe fails, [`record_failure`] advances
///   `last_failure` and the next [`evaluate`] trips again.
///
/// [`Allow`]: BreakerVerdict::Allow
/// [`Tripped`]: BreakerVerdict::Tripped
pub fn evaluate(ledger: &FailureLedger, now: DateTime<Utc>) -> BreakerVerdict {
    if ledger.consecutive < THRESHOLD {
        return BreakerVerdict::Allow;
    }
    match ledger.last_failure {
        // Tripped, but a probe is due once the cooldown elapses.
        Some(last) if now - last >= COOLDOWN => BreakerVerdict::Allow,
        Some(_) => BreakerVerdict::Tripped,
        // Threshold reached but no timestamp — a corrupt/hand-edited
        // ledger. Allow rather than quarantine forever on bad data;
        // the next failure will set `last_failure` and re-trip.
        None => BreakerVerdict::Allow,
    }
}

/// `true` exactly when [`evaluate`] returns [`BreakerVerdict::Tripped`].
/// Convenience for the orchestrators' filter predicates.
pub fn is_tripped(ledger: &FailureLedger, now: DateTime<Utc>) -> bool {
    matches!(evaluate(ledger, now), BreakerVerdict::Tripped)
}

/// `true` when recording one more failure at `now` would make the
/// breaker newly cross [`THRESHOLD`] — i.e. the ledger was below
/// threshold and the next failure puts it at threshold. The
/// orchestrators use this to fire the `*-breaker-tripped` event
/// exactly once, on the transition, not on every subsequent failed
/// tick.
pub fn trips_on_next_failure(ledger: &FailureLedger) -> bool {
    ledger.consecutive + 1 == THRESHOLD
}

/// Record one failure at `now`. Returns the advanced ledger —
/// `consecutive` is incremented (saturating) and `last_failure` is
/// set to `now`. Pure; the caller persists the result.
pub fn record_failure(ledger: &FailureLedger, now: DateTime<Utc>) -> FailureLedger {
    FailureLedger {
        consecutive: ledger.consecutive.saturating_add(1),
        last_failure: Some(now),
    }
}

/// Record one success. Returns the cleared ledger
/// ([`FailureLedger::default`]) — a success ends any failure run, so
/// the breaker is fully reset. Pure; the caller persists the result.
pub fn record_success(_ledger: &FailureLedger) -> FailureLedger {
    FailureLedger::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 21, 12, 0, 0).unwrap()
    }

    #[test]
    fn test_breaker_evaluate_clean_ledger_allows() {
        let l = FailureLedger::default();
        assert_eq!(evaluate(&l, now()), BreakerVerdict::Allow);
    }

    #[test]
    fn test_breaker_evaluate_below_threshold_allows() {
        // Two consecutive failures — one short of THRESHOLD (3).
        let l = FailureLedger {
            consecutive: THRESHOLD - 1,
            last_failure: Some(now()),
        };
        assert_eq!(evaluate(&l, now()), BreakerVerdict::Allow);
    }

    #[test]
    fn test_breaker_evaluate_at_threshold_trips() {
        let l = FailureLedger {
            consecutive: THRESHOLD,
            last_failure: Some(now()),
        };
        assert_eq!(evaluate(&l, now()), BreakerVerdict::Tripped);
        assert!(is_tripped(&l, now()));
    }

    #[test]
    fn test_breaker_evaluate_above_threshold_trips() {
        let l = FailureLedger {
            consecutive: THRESHOLD + 5,
            last_failure: Some(now()),
        };
        assert_eq!(evaluate(&l, now()), BreakerVerdict::Tripped);
    }

    #[test]
    fn test_breaker_evaluate_within_cooldown_stays_tripped() {
        // Last failure was COOLDOWN minus one minute ago — still tripped.
        let last = now() - COOLDOWN + Duration::minutes(1);
        let l = FailureLedger {
            consecutive: THRESHOLD,
            last_failure: Some(last),
        };
        assert_eq!(evaluate(&l, now()), BreakerVerdict::Tripped);
    }

    #[test]
    fn test_breaker_evaluate_after_cooldown_allows_probe() {
        // Last failure was exactly COOLDOWN ago — the probe is due.
        let last = now() - COOLDOWN;
        let l = FailureLedger {
            consecutive: THRESHOLD,
            last_failure: Some(last),
        };
        assert_eq!(evaluate(&l, now()), BreakerVerdict::Allow);
    }

    #[test]
    fn test_breaker_evaluate_well_past_cooldown_allows_probe() {
        let last = now() - COOLDOWN - Duration::hours(12);
        let l = FailureLedger {
            consecutive: THRESHOLD + 9,
            last_failure: Some(last),
        };
        assert_eq!(evaluate(&l, now()), BreakerVerdict::Allow);
    }

    #[test]
    fn test_breaker_evaluate_threshold_without_timestamp_allows() {
        // A hand-edited/corrupt ledger: count is high but no
        // timestamp. Don't quarantine forever on bad data.
        let l = FailureLedger {
            consecutive: THRESHOLD + 1,
            last_failure: None,
        };
        assert_eq!(evaluate(&l, now()), BreakerVerdict::Allow);
    }

    #[test]
    fn test_breaker_record_failure_increments_and_stamps() {
        let l = FailureLedger::default();
        let l = record_failure(&l, now());
        assert_eq!(l.consecutive, 1);
        assert_eq!(l.last_failure, Some(now()));
        let later = now() + Duration::minutes(5);
        let l = record_failure(&l, later);
        assert_eq!(l.consecutive, 2);
        assert_eq!(l.last_failure, Some(later));
    }

    #[test]
    fn test_breaker_record_failure_saturates() {
        let l = FailureLedger {
            consecutive: u32::MAX,
            last_failure: Some(now()),
        };
        let l = record_failure(&l, now());
        assert_eq!(l.consecutive, u32::MAX);
    }

    #[test]
    fn test_breaker_record_success_resets_to_default() {
        let l = FailureLedger {
            consecutive: THRESHOLD + 4,
            last_failure: Some(now()),
        };
        let l = record_success(&l);
        assert_eq!(l, FailureLedger::default());
        assert_eq!(l.consecutive, 0);
        assert_eq!(l.last_failure, None);
    }

    #[test]
    fn test_breaker_full_cycle_trips_then_recovers() {
        // Three failures one tick apart trip the breaker; a success
        // after the cooldown probe fully clears it.
        let mut l = FailureLedger::default();
        for i in 0..THRESHOLD {
            let t = now() + Duration::minutes(5 * i as i64);
            l = record_failure(&l, t);
        }
        let trip_time = now() + Duration::minutes(5 * (THRESHOLD - 1) as i64);
        assert_eq!(evaluate(&l, trip_time), BreakerVerdict::Tripped);

        // Probe is due after the cooldown.
        let probe_time = trip_time + COOLDOWN;
        assert_eq!(evaluate(&l, probe_time), BreakerVerdict::Allow);

        // Probe succeeds → ledger clears, breaker stays allowed.
        l = record_success(&l);
        assert_eq!(evaluate(&l, probe_time), BreakerVerdict::Allow);
    }

    #[test]
    fn test_breaker_failed_probe_retrips_immediately() {
        // A tripped breaker whose probe also fails must re-trip at
        // once — the failure advances `last_failure` to the probe
        // time, restarting the cooldown clock.
        let trip = FailureLedger {
            consecutive: THRESHOLD,
            last_failure: Some(now() - COOLDOWN),
        };
        assert_eq!(evaluate(&trip, now()), BreakerVerdict::Allow); // probe due
        let after_failed_probe = record_failure(&trip, now());
        assert_eq!(after_failed_probe.consecutive, THRESHOLD + 1);
        assert_eq!(evaluate(&after_failed_probe, now()), BreakerVerdict::Tripped);
    }

    #[test]
    fn test_breaker_trips_on_next_failure_detects_transition() {
        // Below the trip edge: the next failure crosses it.
        let edge = FailureLedger {
            consecutive: THRESHOLD - 1,
            last_failure: Some(now()),
        };
        assert!(trips_on_next_failure(&edge));
        // One short of the edge: the next failure does not yet trip.
        let before_edge = FailureLedger {
            consecutive: THRESHOLD - 2,
            last_failure: Some(now()),
        };
        assert!(!trips_on_next_failure(&before_edge));
        // Already tripped: a further failure is not a *new* trip.
        let tripped = FailureLedger {
            consecutive: THRESHOLD,
            last_failure: Some(now()),
        };
        assert!(!trips_on_next_failure(&tripped));
    }
}
