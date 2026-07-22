//! Proactive token refresh for **inactive** CLI accounts.
//!
//! A *behavior* over the existing `account` noun (see
//! `.claude/rules/architecture.md`), not a new domain type: it picks
//! which account to heal next and leaves the healing itself to
//! `services::identity::verify_account_identity`.
//!
//! ## The gap this closes
//!
//! An OAuth access token lives about an hour. Claude Code keeps the
//! *active* account's token fresh on its own schedule, but the accounts
//! parked in Claudepot's private slots have nobody refreshing them —
//! nothing touched them between an explicit "Verify all" and the next
//! account switch. So their tokens sat expired, and every surface that
//! needs a live token (usage windows, the Activity strip, the tray
//! report) showed "Expired" indefinitely for every account except the
//! one in use.
//!
//! ## Why this only selects, and does not refresh
//!
//! The refresh itself is deliberately NOT reimplemented here. Calling
//! `verify_account_identity` on an account whose token has already
//! expired drives the existing, tested path: `/profile` returns 401 →
//! one `refresh_token` exchange → CAS-guarded write back to the private
//! slot → `verify_status` updated. That path already refuses to persist
//! a rotated blob when the profile email drifts from the label, so a
//! background pass cannot entrench a misfiled slot.
//!
//! ## Pacing
//!
//! `reference.md` §III.4.1 records the token endpoint refusing three
//! refreshes from one IP inside a 10-minute window. That was a single
//! observation, not a specification, so the response here is structural
//! rather than tuned: **one account per tick**, on the existing 5-minute
//! orchestrator cadence. That is comfortably inside even the pessimistic
//! reading of that observation, needs no backoff state machine, and
//! costs nothing when no account is expired.
//!
//! Ordering is round-robin by attempt, not by staleness. Staleness alone
//! starves: an account that fails every time stays the most stale and
//! would be picked forever, and no other account would ever be refreshed.

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// One account already established as *eligible* by the caller —
/// has CLI credentials, is not the active CLI account, is not
/// `drift`/`rejected`, and its stored blob is expired.
///
/// Eligibility needs the store and the keychain, so it stays in the
/// orchestrator; this module is pure so the ordering rules can be
/// tested against a clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub uuid: Uuid,
    /// Blob expiry, epoch millis — the `expiresAt` of the private slot.
    pub expires_at_ms: i64,
    /// When a refresh was last attempted for this account in this
    /// process, if ever. `None` sorts first: an account we have never
    /// tried is always more interesting than one we just tried.
    pub last_attempt: Option<DateTime<Utc>>,
}

/// Facts the caller has gathered about one account, for [`is_eligible`].
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    pub has_cli_credentials: bool,
    /// True when this is the account Claude Code is signed in as.
    pub is_active_cli: bool,
    /// The row's `verify_status` (`ok` / `never` / `network_error` /
    /// `drift` / `rejected`).
    pub verify_status: &'a str,
    /// `expiresAt` from the stored blob, epoch millis.
    pub expires_at_ms: i64,
}

/// Is this account worth a background refresh right now?
///
/// Four independent reasons to decline:
///
/// - **No CLI credentials** — nothing in the slot to refresh.
/// - **It is the active CLI account** — that token belongs to Claude
///   Code, which rotates it on its own schedule. Refreshing it from a
///   background tick is the sign-out bug fixed in 0.2.10.
/// - **`drift` or `rejected`** — already known-bad. Drift means the slot
///   holds someone else's credentials, so refreshing would entrench a
///   misfiling; rejected means the refresh token is dead and only a
///   re-login helps. Retrying either every tick is pure noise.
/// - **Not actually expired** — `/profile` would return 200 and the
///   refresh branch would never be reached, so the call would cost a
///   round-trip and heal nothing.
pub fn is_eligible(facts: &Facts<'_>, now_ms: i64) -> bool {
    facts.has_cli_credentials
        && !facts.is_active_cli
        && !matches!(facts.verify_status, "drift" | "rejected")
        && facts.expires_at_ms < now_ms
}

/// Pick the next account to refresh, or `None` when nothing is due.
///
/// Rules, in order:
///
/// 1. Skip anything attempted within `min_retry_gap` — a persistently
///    failing account must not consume every tick.
/// 2. Never-attempted accounts win over attempted ones.
/// 3. Among attempted, the oldest attempt wins (round-robin).
/// 4. Ties break on the oldest expiry, then on uuid so the choice is
///    deterministic for a given input.
pub fn select_next(
    candidates: &[Candidate],
    now: DateTime<Utc>,
    min_retry_gap: Duration,
) -> Option<Uuid> {
    candidates
        .iter()
        .filter(|c| match c.last_attempt {
            None => true,
            Some(at) => now.signed_duration_since(at) >= min_retry_gap,
        })
        // `min_by_key` keeps the FIRST minimum, and the key below is a
        // total order, so the result does not depend on input order.
        .min_by_key(|c| {
            (
                // None → 0 sorts ahead of every Some → 1.
                c.last_attempt.map_or(0, |_| 1),
                c.last_attempt.map(|at| at.timestamp_millis()).unwrap_or(0),
                c.expires_at_ms,
                c.uuid,
            )
        })
        .map(|c| c.uuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn candidate(n: u8, expires_at_ms: i64, last_attempt: Option<DateTime<Utc>>) -> Candidate {
        Candidate {
            uuid: uuid(n),
            expires_at_ms,
            last_attempt,
        }
    }

    /// An eligible baseline; each test flips exactly one field so the
    /// rule under test is the only thing that can explain the result.
    fn eligible_facts() -> Facts<'static> {
        Facts {
            has_cli_credentials: true,
            is_active_cli: false,
            verify_status: "ok",
            expires_at_ms: 500,
        }
    }

    #[test]
    fn baseline_expired_inactive_account_is_eligible() {
        assert!(is_eligible(&eligible_facts(), 1_000));
    }

    #[test]
    fn account_without_cli_credentials_is_not_eligible() {
        let f = Facts {
            has_cli_credentials: false,
            ..eligible_facts()
        };
        assert!(!is_eligible(&f, 1_000));
    }

    /// The 0.2.10 sign-out bug in selector form: the active account's
    /// token belongs to Claude Code and must never be refreshed by a
    /// background tick.
    #[test]
    fn the_active_cli_account_is_never_eligible() {
        let f = Facts {
            is_active_cli: true,
            ..eligible_facts()
        };
        assert!(!is_eligible(&f, 1_000));
    }

    #[test]
    fn drifted_and_rejected_accounts_are_not_eligible() {
        for status in ["drift", "rejected"] {
            let f = Facts {
                verify_status: status,
                ..eligible_facts()
            };
            assert!(!is_eligible(&f, 1_000), "{status} must be skipped");
        }
    }

    #[test]
    fn unverified_and_transiently_failed_accounts_stay_eligible() {
        // "never" and "network_error" are not evidence of a bad slot —
        // excluding them would strand a fresh account forever.
        for status in ["never", "network_error", "ok"] {
            let f = Facts {
                verify_status: status,
                ..eligible_facts()
            };
            assert!(is_eligible(&f, 1_000), "{status} must stay eligible");
        }
    }

    #[test]
    fn a_token_with_time_left_is_not_eligible() {
        let f = Facts {
            expires_at_ms: 2_000,
            ..eligible_facts()
        };
        assert!(!is_eligible(&f, 1_000));
    }

    #[test]
    fn expiry_exactly_now_is_not_yet_eligible() {
        // Boundary: strict `<`, so the tick that lands exactly on expiry
        // leaves it; the next one picks it up.
        let f = Facts {
            expires_at_ms: 1_000,
            ..eligible_facts()
        };
        assert!(!is_eligible(&f, 1_000));
    }

    #[test]
    fn empty_candidate_list_selects_nothing() {
        assert_eq!(
            select_next(&[], at(1_000), Duration::seconds(600)),
            None,
            "no eligible accounts must cost nothing and select nobody"
        );
    }

    #[test]
    fn never_attempted_wins_over_recently_attempted() {
        let cands = vec![candidate(1, 0, Some(at(900))), candidate(2, 0, None)];
        assert_eq!(
            select_next(&cands, at(1_000), Duration::seconds(60)),
            Some(uuid(2))
        );
    }

    #[test]
    fn oldest_attempt_wins_among_attempted() {
        let cands = vec![
            candidate(1, 0, Some(at(800))),
            candidate(2, 0, Some(at(500))),
            candidate(3, 0, Some(at(700))),
        ];
        assert_eq!(
            select_next(&cands, at(1_000), Duration::seconds(60)),
            Some(uuid(2))
        );
    }

    #[test]
    fn attempt_inside_the_retry_gap_is_skipped() {
        // Attempted 30s ago with a 60s gap — not due yet.
        let cands = vec![candidate(1, 0, Some(at(970)))];
        assert_eq!(select_next(&cands, at(1_000), Duration::seconds(60)), None);
    }

    #[test]
    fn attempt_exactly_at_the_retry_gap_is_due() {
        // Boundary: `>=` means 60s after a 60s gap is eligible.
        let cands = vec![candidate(1, 0, Some(at(940)))];
        assert_eq!(
            select_next(&cands, at(1_000), Duration::seconds(60)),
            Some(uuid(1))
        );
    }

    /// The starvation guard, and the reason ordering is by attempt
    /// rather than by staleness. Account 1 is permanently broken and is
    /// always the most stale; account 2 must still get a turn.
    #[test]
    fn a_permanently_failing_account_does_not_starve_the_others() {
        let gap = Duration::seconds(60);
        let mut last: std::collections::HashMap<Uuid, DateTime<Utc>> = Default::default();

        let mut picks = Vec::new();
        for tick in 0..4 {
            let now = at(1_000 + tick * 300);
            let cands = vec![
                // Never refreshes, so its expiry stays ancient.
                candidate(1, 0, last.get(&uuid(1)).copied()),
                candidate(2, 5_000, last.get(&uuid(2)).copied()),
            ];
            let picked = select_next(&cands, now, gap).unwrap();
            last.insert(picked, now);
            picks.push(picked);
        }

        assert!(
            picks.contains(&uuid(2)),
            "round-robin must reach account 2; got {picks:?}"
        );
        assert_eq!(
            picks.iter().filter(|p| **p == uuid(1)).count(),
            2,
            "and must not hand every tick to the failing account: {picks:?}"
        );
    }

    #[test]
    fn selection_is_deterministic_regardless_of_input_order() {
        let a = candidate(1, 100, None);
        let b = candidate(2, 100, None);
        let forward = select_next(&[a.clone(), b.clone()], at(1_000), Duration::seconds(60));
        let reverse = select_next(&[b, a], at(1_000), Duration::seconds(60));
        assert_eq!(forward, reverse, "uuid tie-break must be order-independent");
    }

    #[test]
    fn oldest_expiry_breaks_ties_between_never_attempted() {
        let cands = vec![candidate(1, 9_000, None), candidate(2, 1_000, None)];
        assert_eq!(
            select_next(&cands, at(1_000), Duration::seconds(60)),
            Some(uuid(2)),
            "the more stale token should be healed first"
        );
    }
}
