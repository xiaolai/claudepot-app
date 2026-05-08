//! Pure rule evaluator.
//!
//! Inputs: rules, a `UsageSnapshot`, the active CLI uuid, the current
//! audit log, and a wall-clock for guard arithmetic. Output: a list of
//! [`PendingSwap`] that the orchestrator dispatches according to each
//! rule's `mode` (auto / confirm).
//!
//! No I/O, no `Utc::now()` inside — `now` is injected so tests are
//! deterministic.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::rotation::audit::{
    AuditMode, RotationAuditEntry, RotationOutcome, RotationTriggerSummary,
};
use crate::rotation::rules::{Action, RotationRule, Selector, Trigger};
use crate::services::usage_alerts::UsageWindowKind;
use crate::services::usage_snapshot::{
    AccountSnapshot, AccountStatus, UsageSnapshot, UsageWindows, WindowSnapshot,
};

/// One swap waiting for the orchestrator to act on. Carries everything
/// the audit log + UI need without re-querying.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingSwap {
    pub rule_id: String,
    pub from_uuid: Uuid,
    pub from_email: String,
    pub to_uuid: Uuid,
    pub to_email: String,
    pub trigger: RotationTriggerSummary,
    pub mode: AuditMode,
    /// Whether the rule asked the orchestrator to skip when CC is
    /// running. The watcher checks the live process before acting.
    pub skip_when_cc_running: bool,
}

/// Why the evaluator declined to emit a swap for a rule that matched.
/// Surfaced via the audit log for forensics.
#[derive(Debug, Clone, PartialEq)]
pub enum SkipReason {
    /// Active account utilization didn't reach threshold.
    BelowThreshold {
        utilization_pct: f64,
        threshold_pct: u32,
    },
    /// Active account was missing from the snapshot or had no usage
    /// data (status != Ok or window absent).
    NoActiveSnapshot,
    /// Trigger window not present in active account's snapshot.
    NoWindowData,
    /// `min_interval_secs` rejected the fire (too soon after last swap).
    MinIntervalNotElapsed { secs_since_last: i64 },
    /// `max_swaps_per_window` cap reached for the current cycle.
    MaxSwapsHit { swaps_in_cycle: u32 },
    /// No usable rotation target.
    NoCandidate(NoCandidateReason),
}

/// Why no candidate was usable.
#[derive(Debug, Clone, PartialEq)]
pub enum NoCandidateReason {
    /// All listed candidates resolved to the active account.
    OnlyActive,
    /// Every alternate candidate was also at or above the threshold.
    AllAboveThreshold,
    /// No listed candidate could be matched to a real account in the
    /// snapshot.
    UnknownEmails,
    /// Selector type-specific: round-robin couldn't find the active
    /// account in its candidate list.
    ActiveNotInList,
}

/// What the evaluator decided per rule. The orchestrator only acts on
/// `Fire` variants; everything else is logged.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleDecision {
    /// Emit a pending swap. The orchestrator decides whether to apply
    /// or surface a confirm prompt based on `pending.mode`.
    Fire(PendingSwap),
    /// Rule didn't fire — record `reason` if the trigger matched but a
    /// guard or selector blocked. `None` means trigger didn't match
    /// (no audit entry needed).
    Skip {
        rule_id: String,
        reason: Option<SkipReasonRecord>,
    },
}

/// A skip with enough metadata to write an audit entry. Constructed
/// inside `evaluate` and consumed by callers; tests inspect this
/// directly.
#[derive(Debug, Clone, PartialEq)]
pub struct SkipReasonRecord {
    pub reason: SkipReason,
    pub trigger: RotationTriggerSummary,
    pub from_email: String,
    pub to_email: Option<String>,
}

/// Evaluate every enabled rule against the current snapshot + audit
/// state. Pure function; tests inject `now`.
pub fn evaluate(
    rules: &[RotationRule],
    snapshot: &UsageSnapshot,
    active_uuid: Uuid,
    audit: &[RotationAuditEntry],
    now: DateTime<Utc>,
) -> Vec<RuleDecision> {
    let active_key = active_uuid.to_string();
    let active_snap = snapshot.accounts.get(&active_key);
    let from_email = active_snap
        .map(|s| s.email.clone())
        .unwrap_or_else(|| "<unknown>".to_string());

    let mut out = Vec::new();
    for rule in rules.iter().filter(|r| r.enabled) {
        out.push(evaluate_one(
            rule,
            snapshot,
            active_uuid,
            &from_email,
            active_snap,
            audit,
            now,
        ));
    }
    out
}

fn evaluate_one(
    rule: &RotationRule,
    snapshot: &UsageSnapshot,
    active_uuid: Uuid,
    from_email: &str,
    active_snap: Option<&AccountSnapshot>,
    audit: &[RotationAuditEntry],
    now: DateTime<Utc>,
) -> RuleDecision {
    let active_snap = match active_snap {
        Some(s) if matches!(s.status, AccountStatus::Ok) => s,
        _ => {
            return RuleDecision::Skip {
                rule_id: rule.id.clone(),
                reason: Some(SkipReasonRecord {
                    reason: SkipReason::NoActiveSnapshot,
                    trigger: trigger_summary_below(rule, 0.0),
                    from_email: from_email.to_string(),
                    to_email: None,
                }),
            };
        }
    };

    // 1. Trigger evaluation: get the active account's utilization for
    //    the trigger's window.
    let (window, threshold_pct) = match &rule.trigger {
        Trigger::UtilizationThreshold { window, pct } => (*window, *pct),
    };
    let usage = match &active_snap.usage {
        Some(u) => u,
        None => {
            return RuleDecision::Skip {
                rule_id: rule.id.clone(),
                reason: Some(SkipReasonRecord {
                    reason: SkipReason::NoWindowData,
                    trigger: trigger_summary_below(rule, 0.0),
                    from_email: from_email.to_string(),
                    to_email: None,
                }),
            };
        }
    };
    let active_pct = match window_pct(usage, window) {
        Some(p) => p,
        None => {
            return RuleDecision::Skip {
                rule_id: rule.id.clone(),
                reason: Some(SkipReasonRecord {
                    reason: SkipReason::NoWindowData,
                    trigger: trigger_summary_below(rule, 0.0),
                    from_email: from_email.to_string(),
                    to_email: None,
                }),
            };
        }
    };

    if active_pct < threshold_pct as f64 {
        // Trigger didn't match. No audit entry — this is the steady
        // state, not a noteworthy event.
        return RuleDecision::Skip {
            rule_id: rule.id.clone(),
            reason: None,
        };
    }

    let trigger_summary = RotationTriggerSummary {
        window: Some(window),
        utilization_pct: active_pct,
        threshold_pct,
        is_extra_usage: false,
    };

    // 2. Guard: min_interval_secs since the last swap by ANY rule.
    //    Only `applied` swaps count — a `suggested` is still
    //    awaiting user input; a `skipped_*` didn't disturb the user.
    let guard_min_interval = rule.guards.min_interval_secs;
    if let Some(secs) = secs_since_last_applied(audit, now) {
        if secs < guard_min_interval as i64 {
            return RuleDecision::Skip {
                rule_id: rule.id.clone(),
                reason: Some(SkipReasonRecord {
                    reason: SkipReason::MinIntervalNotElapsed {
                        secs_since_last: secs,
                    },
                    trigger: trigger_summary.clone(),
                    from_email: from_email.to_string(),
                    to_email: None,
                }),
            };
        }
    }

    // 3. Guard: max_swaps_per_window for THIS rule + the current
    //    cycle of the trigger window. A cycle is bounded by the
    //    window's `resets_at` — entries from a prior cycle don't
    //    count.
    let cycle_start_ts = window_resets_at(usage, window).map(|ts| ts.with_timezone(&Utc));
    // The "current cycle" is everything between (cycle_start_ts -
    // window-length) and now. We don't know the exact window length,
    // so we use a generous lower bound: any audit entry whose
    // trigger.window matches AND whose ts is after the most recent
    // swap that referenced this rule beyond a 24h ago bound. The
    // simpler and equally correct rule: count audit entries for THIS
    // rule_id within the last `cycle_length` of `window` (5h or 7d).
    // We hardcode the lower bound here — exposing window-length out
    // of UsageWindowKind would be cleaner but is purely an
    // ergonomic expansion the user doesn't see. A 5h trigger looks
    // back 5h, a 7d trigger looks back 7d.
    let cycle_length = cycle_length_for(window);
    let cycle_floor = now - cycle_length;
    let swaps_in_cycle = count_applied_for_rule(audit, &rule.id, cycle_floor);
    if swaps_in_cycle >= rule.guards.max_swaps_per_window {
        // Use cycle_start_ts in the audit entry if we have it for
        // forensic clarity.
        let _ = cycle_start_ts;
        return RuleDecision::Skip {
            rule_id: rule.id.clone(),
            reason: Some(SkipReasonRecord {
                reason: SkipReason::MaxSwapsHit { swaps_in_cycle },
                trigger: trigger_summary.clone(),
                from_email: from_email.to_string(),
                to_email: None,
            }),
        };
    }

    // 4. Selector: pick the target.
    let target = match &rule.action {
        Action::RotateTo { selector } => {
            select_target(selector, snapshot, active_uuid, threshold_pct)
        }
    };
    let (to_uuid, to_email) = match target {
        Ok(t) => t,
        Err(reason) => {
            return RuleDecision::Skip {
                rule_id: rule.id.clone(),
                reason: Some(SkipReasonRecord {
                    reason: SkipReason::NoCandidate(reason),
                    trigger: trigger_summary.clone(),
                    from_email: from_email.to_string(),
                    to_email: None,
                }),
            };
        }
    };

    RuleDecision::Fire(PendingSwap {
        rule_id: rule.id.clone(),
        from_uuid: active_uuid,
        from_email: from_email.to_string(),
        to_uuid,
        to_email,
        trigger: trigger_summary,
        mode: rule.mode.into(),
        skip_when_cc_running: rule.guards.skip_when_cc_running,
    })
}

fn trigger_summary_below(rule: &RotationRule, util: f64) -> RotationTriggerSummary {
    let (window, threshold_pct) = match &rule.trigger {
        Trigger::UtilizationThreshold { window, pct } => (*window, *pct),
    };
    RotationTriggerSummary {
        window: Some(window),
        utilization_pct: util,
        threshold_pct,
        is_extra_usage: false,
    }
}

fn window_pct(u: &UsageWindows, kind: UsageWindowKind) -> Option<f64> {
    pick_window(u, kind).map(|w| w.utilization)
}

fn window_resets_at(
    u: &UsageWindows,
    kind: UsageWindowKind,
) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    pick_window(u, kind).and_then(|w| w.resets_at)
}

fn pick_window(u: &UsageWindows, kind: UsageWindowKind) -> Option<&WindowSnapshot> {
    match kind {
        UsageWindowKind::FiveHour => u.five_hour.as_ref(),
        UsageWindowKind::SevenDay => u.seven_day.as_ref(),
        UsageWindowKind::SevenDayOpus => u.seven_day_opus.as_ref(),
        UsageWindowKind::SevenDaySonnet => u.seven_day_sonnet.as_ref(),
    }
}

fn cycle_length_for(kind: UsageWindowKind) -> chrono::Duration {
    match kind {
        UsageWindowKind::FiveHour => chrono::Duration::hours(5),
        UsageWindowKind::SevenDay
        | UsageWindowKind::SevenDayOpus
        | UsageWindowKind::SevenDaySonnet => chrono::Duration::days(7),
    }
}

fn secs_since_last_applied(audit: &[RotationAuditEntry], now: DateTime<Utc>) -> Option<i64> {
    audit
        .iter()
        .filter(|e| matches!(e.outcome, RotationOutcome::Applied))
        .map(|e| (now - e.ts).num_seconds())
        .min()
}

fn count_applied_for_rule(
    audit: &[RotationAuditEntry],
    rule_id: &str,
    floor: DateTime<Utc>,
) -> u32 {
    audit
        .iter()
        .filter(|e| {
            e.rule_id == rule_id
                && matches!(e.outcome, RotationOutcome::Applied)
                && e.ts >= floor
        })
        .count() as u32
}

fn select_target(
    selector: &Selector,
    snapshot: &UsageSnapshot,
    active_uuid: Uuid,
    threshold_pct: u32,
) -> Result<(Uuid, String), NoCandidateReason> {
    match selector {
        Selector::LeastUsed { window, candidates } => {
            select_least_used(snapshot, active_uuid, candidates, *window, threshold_pct)
        }
        Selector::RoundRobin { candidates } => {
            select_round_robin(snapshot, active_uuid, candidates)
        }
        Selector::Explicit { email } => select_explicit(snapshot, active_uuid, email),
    }
}

fn resolve_email_to_uuid(snapshot: &UsageSnapshot, email: &str) -> Option<Uuid> {
    for (k, v) in &snapshot.accounts {
        if v.email.eq_ignore_ascii_case(email) {
            if let Ok(u) = Uuid::parse_str(k) {
                return Some(u);
            }
        }
    }
    None
}

fn snapshot_for(snapshot: &UsageSnapshot, uuid: Uuid) -> Option<&AccountSnapshot> {
    snapshot.accounts.get(&uuid.to_string())
}

fn select_least_used(
    snapshot: &UsageSnapshot,
    active_uuid: Uuid,
    candidates: &[String],
    window: UsageWindowKind,
    threshold_pct: u32,
) -> Result<(Uuid, String), NoCandidateReason> {
    let mut resolved: Vec<(Uuid, &AccountSnapshot)> = Vec::new();
    let mut any_unknown_count = 0;
    let mut any_active_match = false;
    for c in candidates {
        match resolve_email_to_uuid(snapshot, c) {
            Some(u) if u == active_uuid => {
                any_active_match = true;
            }
            Some(u) => {
                if let Some(s) = snapshot_for(snapshot, u) {
                    if matches!(s.status, AccountStatus::Ok) {
                        resolved.push((u, s));
                    }
                }
            }
            None => any_unknown_count += 1,
        }
    }

    if resolved.is_empty() {
        if any_active_match && any_unknown_count == 0 {
            return Err(NoCandidateReason::OnlyActive);
        }
        return Err(NoCandidateReason::UnknownEmails);
    }

    // Filter to candidates strictly below threshold.
    let mut below: Vec<(Uuid, &AccountSnapshot, f64)> = resolved
        .iter()
        .filter_map(|(u, s)| {
            let p = s.usage.as_ref().and_then(|w| window_pct(w, window))?;
            Some((*u, *s, p))
        })
        .filter(|(_, _, p)| *p < threshold_pct as f64)
        .collect();

    if below.is_empty() {
        return Err(NoCandidateReason::AllAboveThreshold);
    }

    // Lowest utilization first, deterministic on ties via uuid order.
    below.sort_by(|a, b| {
        a.2.partial_cmp(&b.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    let (uuid, snap, _) = below[0];
    Ok((uuid, snap.email.clone()))
}

fn select_round_robin(
    snapshot: &UsageSnapshot,
    active_uuid: Uuid,
    candidates: &[String],
) -> Result<(Uuid, String), NoCandidateReason> {
    // Resolve every candidate first so we know which slot the active
    // account sits in. Unknown emails are kept as `None` — they
    // count as positions in the order, but you can't rotate INTO
    // them.
    let resolved: Vec<Option<Uuid>> = candidates
        .iter()
        .map(|c| resolve_email_to_uuid(snapshot, c))
        .collect();
    let active_idx = resolved
        .iter()
        .position(|u| matches!(u, Some(uu) if *uu == active_uuid))
        .ok_or(NoCandidateReason::ActiveNotInList)?;

    let n = resolved.len();
    if n == 0 {
        return Err(NoCandidateReason::UnknownEmails);
    }
    // Walk forward from active+1, wrap, stop before active again.
    for i in 1..n {
        let idx = (active_idx + i) % n;
        if let Some(u) = resolved[idx] {
            if let Some(s) = snapshot_for(snapshot, u) {
                if matches!(s.status, AccountStatus::Ok) {
                    return Ok((u, s.email.clone()));
                }
            }
        }
    }
    Err(NoCandidateReason::OnlyActive)
}

fn select_explicit(
    snapshot: &UsageSnapshot,
    active_uuid: Uuid,
    email: &str,
) -> Result<(Uuid, String), NoCandidateReason> {
    let u = resolve_email_to_uuid(snapshot, email)
        .ok_or(NoCandidateReason::UnknownEmails)?;
    if u == active_uuid {
        return Err(NoCandidateReason::OnlyActive);
    }
    let s = snapshot_for(snapshot, u).ok_or(NoCandidateReason::UnknownEmails)?;
    if !matches!(s.status, AccountStatus::Ok) {
        return Err(NoCandidateReason::UnknownEmails);
    }
    Ok((u, s.email.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::audit::{AuditMode, RotationOutcome};
    use crate::rotation::rules::{
        Action, RotationGuards, RotationMode, RotationRule, Selector, Trigger,
    };
    use crate::services::usage_snapshot::{
        AccountSnapshot, AccountStatus, UsageSnapshot, UsageWindows, WindowSnapshot,
    };
    use chrono::{Duration, FixedOffset, TimeZone, Utc};
    use std::collections::BTreeMap;

    fn fixed_now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 8, 12, 0, 0).unwrap()
    }

    fn resets_at(hours_ahead: i64) -> chrono::DateTime<FixedOffset> {
        let utc = fixed_now() + Duration::hours(hours_ahead);
        utc.with_timezone(&FixedOffset::east_opt(0).unwrap())
    }

    fn snap_account(
        email: &str,
        five_hour_pct: Option<f64>,
        seven_day_pct: Option<f64>,
        cli_active: bool,
    ) -> AccountSnapshot {
        AccountSnapshot {
            email: email.into(),
            subscription_type: Some("max".into()),
            cli_active,
            desktop_active: false,
            status: AccountStatus::Ok,
            fetched_at: fixed_now(),
            ttl_secs: 60,
            usage: Some(UsageWindows {
                five_hour: five_hour_pct.map(|p| WindowSnapshot {
                    utilization: p,
                    resets_at: Some(resets_at(3)),
                }),
                seven_day: seven_day_pct.map(|p| WindowSnapshot {
                    utilization: p,
                    resets_at: Some(resets_at(72)),
                }),
                seven_day_opus: None,
                seven_day_sonnet: None,
            }),
            retry_after_secs: None,
            error: None,
        }
    }

    fn build_snapshot(entries: Vec<(Uuid, AccountSnapshot)>) -> UsageSnapshot {
        let mut map = BTreeMap::new();
        for (u, s) in entries {
            map.insert(u.to_string(), s);
        }
        UsageSnapshot {
            schema_version: 1,
            written_at: fixed_now(),
            accounts: map,
        }
    }

    fn rule_5h_least_used(candidates: Vec<String>) -> RotationRule {
        RotationRule {
            id: "5h-near-cap".into(),
            enabled: true,
            trigger: Trigger::UtilizationThreshold {
                window: UsageWindowKind::FiveHour,
                pct: 90,
            },
            action: Action::RotateTo {
                selector: Selector::LeastUsed {
                    window: UsageWindowKind::FiveHour,
                    candidates,
                },
            },
            mode: RotationMode::Confirm,
            guards: RotationGuards::default(),
        }
    }

    #[test]
    fn below_threshold_does_not_fire() {
        let active = Uuid::new_v4();
        let other = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (active, snap_account("a@x.com", Some(89.9), Some(40.0), true)),
            (other, snap_account("b@x.com", Some(20.0), Some(10.0), false)),
        ]);
        let rule = rule_5h_least_used(vec!["a@x.com".into(), "b@x.com".into()]);
        let decisions = evaluate(&[rule], &snap, active, &[], fixed_now());
        assert_eq!(decisions.len(), 1);
        match &decisions[0] {
            RuleDecision::Skip {
                reason: None,
                ..
            } => {}
            d => panic!("expected silent skip, got {d:?}"),
        }
    }

    #[test]
    fn at_threshold_fires_picks_lowest() {
        let active = Uuid::new_v4();
        let target_low = Uuid::new_v4();
        let target_mid = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (active, snap_account("a@x.com", Some(91.0), Some(40.0), true)),
            (
                target_low,
                snap_account("low@x.com", Some(10.0), Some(5.0), false),
            ),
            (
                target_mid,
                snap_account("mid@x.com", Some(50.0), Some(25.0), false),
            ),
        ]);
        let rule = rule_5h_least_used(vec![
            "a@x.com".into(),
            "low@x.com".into(),
            "mid@x.com".into(),
        ]);
        let decisions = evaluate(&[rule], &snap, active, &[], fixed_now());
        match &decisions[0] {
            RuleDecision::Fire(p) => {
                assert_eq!(p.to_email, "low@x.com");
                assert_eq!(p.to_uuid, target_low);
                assert_eq!(p.from_email, "a@x.com");
                assert!((p.trigger.utilization_pct - 91.0).abs() < 1e-6);
                assert_eq!(p.trigger.threshold_pct, 90);
            }
            d => panic!("expected fire, got {d:?}"),
        }
    }

    #[test]
    fn all_alternates_above_threshold_returns_no_candidate() {
        let active = Uuid::new_v4();
        let other = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (active, snap_account("a@x.com", Some(95.0), Some(40.0), true)),
            (
                other,
                snap_account("b@x.com", Some(92.0), Some(45.0), false),
            ),
        ]);
        let rule = rule_5h_least_used(vec!["a@x.com".into(), "b@x.com".into()]);
        let decisions = evaluate(&[rule], &snap, active, &[], fixed_now());
        match &decisions[0] {
            RuleDecision::Skip {
                reason: Some(rec), ..
            } => match &rec.reason {
                SkipReason::NoCandidate(NoCandidateReason::AllAboveThreshold) => {}
                r => panic!("expected AllAboveThreshold, got {r:?}"),
            },
            d => panic!("expected skip with no-candidate, got {d:?}"),
        }
    }

    #[test]
    fn min_interval_blocks_repeat_fire() {
        let active = Uuid::new_v4();
        let other = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (active, snap_account("a@x.com", Some(95.0), Some(40.0), true)),
            (
                other,
                snap_account("b@x.com", Some(20.0), Some(10.0), false),
            ),
        ]);
        let rule = rule_5h_least_used(vec!["a@x.com".into(), "b@x.com".into()]);
        // Audit shows a fresh applied 30s ago; default min_interval is 60s.
        let recent_ts = fixed_now() - Duration::seconds(30);
        let entry = RotationAuditEntry {
            id: 1,
            ts: recent_ts,
            rule_id: "any-other".into(),
            trigger: RotationTriggerSummary {
                window: Some(UsageWindowKind::FiveHour),
                utilization_pct: 91.0,
                threshold_pct: 90,
                is_extra_usage: false,
            },
            from_email: "a@x.com".into(),
            to_email: Some("b@x.com".into()),
            mode: AuditMode::Confirm,
            outcome: RotationOutcome::Applied,
            reason: "".into(),
        };
        let decisions = evaluate(&[rule], &snap, active, &[entry], fixed_now());
        match &decisions[0] {
            RuleDecision::Skip {
                reason: Some(rec), ..
            } => match rec.reason {
                SkipReason::MinIntervalNotElapsed { secs_since_last } => {
                    assert_eq!(secs_since_last, 30);
                }
                _ => panic!("expected MinIntervalNotElapsed"),
            },
            _ => panic!("expected skip"),
        }
    }

    #[test]
    fn max_swaps_per_window_blocks_after_cap() {
        let active = Uuid::new_v4();
        let other = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (active, snap_account("a@x.com", Some(95.0), Some(40.0), true)),
            (
                other,
                snap_account("b@x.com", Some(20.0), Some(10.0), false),
            ),
        ]);
        let mut rule = rule_5h_least_used(vec!["a@x.com".into(), "b@x.com".into()]);
        rule.guards.max_swaps_per_window = 2;
        rule.guards.min_interval_secs = 0; // exclude the min-interval guard

        let mk = |secs_ago: i64| RotationAuditEntry {
            id: 0,
            ts: fixed_now() - Duration::seconds(secs_ago),
            rule_id: "5h-near-cap".into(),
            trigger: RotationTriggerSummary {
                window: Some(UsageWindowKind::FiveHour),
                utilization_pct: 91.0,
                threshold_pct: 90,
                is_extra_usage: false,
            },
            from_email: "a@x.com".into(),
            to_email: Some("b@x.com".into()),
            mode: AuditMode::Auto,
            outcome: RotationOutcome::Applied,
            reason: "".into(),
        };
        // Two recent applied entries inside the 5h cycle.
        let audit = vec![mk(60), mk(120)];
        let decisions = evaluate(&[rule], &snap, active, &audit, fixed_now());
        match &decisions[0] {
            RuleDecision::Skip {
                reason: Some(rec), ..
            } => match rec.reason {
                SkipReason::MaxSwapsHit { swaps_in_cycle } => {
                    assert_eq!(swaps_in_cycle, 2);
                }
                _ => panic!("expected MaxSwapsHit"),
            },
            _ => panic!("expected skip"),
        }
    }

    #[test]
    fn old_audit_entries_outside_cycle_are_ignored() {
        let active = Uuid::new_v4();
        let other = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (active, snap_account("a@x.com", Some(95.0), Some(40.0), true)),
            (
                other,
                snap_account("b@x.com", Some(20.0), Some(10.0), false),
            ),
        ]);
        let mut rule = rule_5h_least_used(vec!["a@x.com".into(), "b@x.com".into()]);
        rule.guards.max_swaps_per_window = 1;
        rule.guards.min_interval_secs = 0;

        // Audit entry from 6h ago — outside the 5h cycle.
        let old = RotationAuditEntry {
            id: 1,
            ts: fixed_now() - Duration::hours(6),
            rule_id: "5h-near-cap".into(),
            trigger: RotationTriggerSummary {
                window: Some(UsageWindowKind::FiveHour),
                utilization_pct: 91.0,
                threshold_pct: 90,
                is_extra_usage: false,
            },
            from_email: "a@x.com".into(),
            to_email: Some("b@x.com".into()),
            mode: AuditMode::Auto,
            outcome: RotationOutcome::Applied,
            reason: "".into(),
        };
        let decisions = evaluate(&[rule], &snap, active, &[old], fixed_now());
        // Should fire — the old entry is outside the cycle floor.
        assert!(matches!(decisions[0], RuleDecision::Fire(_)));
    }

    #[test]
    fn round_robin_picks_next() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (a, snap_account("a@x.com", Some(95.0), Some(40.0), true)),
            (b, snap_account("b@x.com", Some(50.0), Some(20.0), false)),
            (c, snap_account("c@x.com", Some(10.0), Some(5.0), false)),
        ]);
        let rule = RotationRule {
            id: "rr".into(),
            enabled: true,
            trigger: Trigger::UtilizationThreshold {
                window: UsageWindowKind::FiveHour,
                pct: 90,
            },
            action: Action::RotateTo {
                selector: Selector::RoundRobin {
                    candidates: vec!["a@x.com".into(), "b@x.com".into(), "c@x.com".into()],
                },
            },
            mode: RotationMode::Auto,
            guards: RotationGuards::default(),
        };
        let decisions = evaluate(&[rule], &snap, a, &[], fixed_now());
        match &decisions[0] {
            RuleDecision::Fire(p) => assert_eq!(p.to_uuid, b),
            d => panic!("expected fire to b, got {d:?}"),
        }
    }

    #[test]
    fn explicit_selector_picks_named() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (a, snap_account("a@x.com", Some(95.0), Some(40.0), true)),
            (b, snap_account("overflow@x.com", Some(5.0), Some(2.0), false)),
        ]);
        let rule = RotationRule {
            id: "exp".into(),
            enabled: true,
            trigger: Trigger::UtilizationThreshold {
                window: UsageWindowKind::FiveHour,
                pct: 90,
            },
            action: Action::RotateTo {
                selector: Selector::Explicit {
                    email: "overflow@x.com".into(),
                },
            },
            mode: RotationMode::Confirm,
            guards: RotationGuards::default(),
        };
        let decisions = evaluate(&[rule], &snap, a, &[], fixed_now());
        match &decisions[0] {
            RuleDecision::Fire(p) => assert_eq!(p.to_uuid, b),
            d => panic!("expected fire, got {d:?}"),
        }
    }

    #[test]
    fn disabled_rule_is_ignored() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let snap = build_snapshot(vec![
            (a, snap_account("a@x.com", Some(95.0), Some(40.0), true)),
            (b, snap_account("b@x.com", Some(5.0), Some(2.0), false)),
        ]);
        let mut rule = rule_5h_least_used(vec!["a@x.com".into(), "b@x.com".into()]);
        rule.enabled = false;
        let decisions = evaluate(&[rule], &snap, a, &[], fixed_now());
        assert!(decisions.is_empty());
    }

    #[test]
    fn missing_active_snapshot_skips_with_reason() {
        let unknown_active = Uuid::new_v4();
        let b = Uuid::new_v4();
        let snap = build_snapshot(vec![(
            b,
            snap_account("b@x.com", Some(5.0), Some(2.0), false),
        )]);
        let rule = rule_5h_least_used(vec!["b@x.com".into()]);
        let decisions = evaluate(&[rule], &snap, unknown_active, &[], fixed_now());
        match &decisions[0] {
            RuleDecision::Skip {
                reason: Some(rec), ..
            } => assert!(matches!(rec.reason, SkipReason::NoActiveSnapshot)),
            _ => panic!("expected skip"),
        }
    }
}
