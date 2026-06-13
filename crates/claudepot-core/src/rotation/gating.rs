//! Circuit-breaker gating of the rule set — the pure pre-filter the
//! rotation orchestrator applies before [`super::eval::evaluate`]
//! runs. Sibling of [`crate::breaker`] (the pure ledger verdicts):
//! the breaker decides *whether one rule is tripped*, the gate
//! decides *which rules survive into this tick*.

use chrono::{DateTime, Utc};

use crate::breaker;
use crate::rotation::breaker_store::BreakerFile;
use crate::rotation::rules::RotationRule;

/// Filter the rule set down to those the circuit breaker permits to
/// run this tick.
///
/// The gate only *excludes* — never includes. An **enabled** rule
/// whose breaker is tripped is dropped (it is quarantined; there is
/// no pending entry to skip, so it must be excluded before the
/// evaluator runs). A **disabled** rule passes through via the
/// `!r.enabled` short-circuit — its breaker is irrelevant and
/// `evaluate` drops it for being disabled anyway. Keeping disabled
/// rules in the list means a rule re-enabled while still tripped is
/// correctly gated again on the next tick.
pub fn breaker_gated_rules(
    rules: &[RotationRule],
    breaker_file: &BreakerFile,
    now: DateTime<Utc>,
) -> Vec<RotationRule> {
    rules
        .iter()
        .filter(|r| !r.enabled || !breaker::is_tripped(&breaker_file.ledger_for(&r.id), now))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::breaker::FailureLedger;
    use crate::rotation::rules::{Action, RotationGuards, RotationMode, Selector, Trigger};
    use crate::services::usage_alerts::UsageWindowKind;

    fn rule(id: &str, enabled: bool) -> RotationRule {
        RotationRule {
            id: id.to_string(),
            enabled,
            trigger: Trigger::UtilizationThreshold {
                window: UsageWindowKind::FiveHour,
                pct: 80,
            },
            action: Action::RotateTo {
                selector: Selector::Explicit {
                    email: "to@example.com".to_string(),
                },
            },
            mode: RotationMode::Auto,
            guards: RotationGuards::default(),
        }
    }

    fn tripped_ledger(now: DateTime<Utc>) -> FailureLedger {
        FailureLedger {
            consecutive: breaker::THRESHOLD,
            last_failure: Some(now),
        }
    }

    #[test]
    fn test_breaker_gate_excludes_enabled_tripped_rule() {
        let now = Utc::now();
        let mut bf = BreakerFile::default();
        bf.set_ledger("tripped", tripped_ledger(now));
        let rules = vec![rule("tripped", true), rule("healthy", true)];
        let gated = breaker_gated_rules(&rules, &bf, now);
        let ids: Vec<&str> = gated.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["healthy"],
            "an enabled, tripped rule must be gated out before evaluation"
        );
    }

    #[test]
    fn test_breaker_gate_keeps_disabled_tripped_rule() {
        // A disabled rule passes the gate (its breaker is moot) —
        // `evaluate` drops it for being disabled. Keeping it here
        // means re-enabling it while still tripped re-gates it.
        let now = Utc::now();
        let mut bf = BreakerFile::default();
        bf.set_ledger("off", tripped_ledger(now));
        let gated = breaker_gated_rules(&[rule("off", false)], &bf, now);
        assert_eq!(
            gated.len(),
            1,
            "a disabled rule is not gated by the breaker"
        );
    }

    #[test]
    fn test_breaker_gate_keeps_rule_with_clean_ledger() {
        let now = Utc::now();
        let bf = BreakerFile::default(); // no ledgers — every rule clean
        let gated = breaker_gated_rules(&[rule("fresh", true)], &bf, now);
        assert_eq!(gated.len(), 1, "a rule with no failure history runs");
    }

    #[test]
    fn test_breaker_gate_keeps_rule_below_threshold() {
        // Two failures — one short of THRESHOLD — must not gate.
        let now = Utc::now();
        let mut bf = BreakerFile::default();
        bf.set_ledger(
            "flapping",
            FailureLedger {
                consecutive: breaker::THRESHOLD - 1,
                last_failure: Some(now),
            },
        );
        let gated = breaker_gated_rules(&[rule("flapping", true)], &bf, now);
        assert_eq!(
            gated.len(),
            1,
            "below-threshold failures do not gate the rule"
        );
    }
}
