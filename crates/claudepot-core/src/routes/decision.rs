//! The recorded outcome of the per-run route gate.
//!
//! [`RouteDecision`] is the *templates/routes* concern of "which
//! backend did this agent run end up using, and why" — it belongs to
//! the routes domain, not to the agent type module. It is recorded on
//! `agent::AgentRun` as one cross-domain field (the run's route
//! outcome), which `agent::types` re-exports for that single use.
//!
//! The pre-run gate (`agent::prerun`) produces a `PrerunDecision`
//! carrying wrapper paths; `agent::run` projects that into this
//! user-facing `RouteDecision` for persistence in `result.json`.

use serde::{Deserialize, Serialize};

/// Decision recorded by the pre-run gate before invoking
/// `claude -p`. The gate runs route-reachability probes and
/// applies the blueprint's `fallback_policy`.
///
/// See `dev-docs/templates-implementation-plan.md` §5.3 for the
/// truth table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteDecision {
    /// Run proceeded against the assigned route (or the default
    /// route if `route_id` is `None`).
    Ran { route_id: Option<String> },
    /// Assigned route was unreachable; fell back to the default
    /// route. Only legal when `privacy != local`.
    Fallback {
        from: String,
        to: Option<String>,
        reason: String,
    },
    /// Run skipped silently (assigned route unreachable + policy
    /// = `skip`, or the route was outright invalid).
    Skipped { reason: String },
    /// Run skipped and a notification was posted (policy =
    /// `alert`, or `privacy = local` and the local route is
    /// unreachable).
    SkippedAlerted { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_decision_round_trips_tagged_snake_case() {
        let cases = [
            (RouteDecision::Ran { route_id: None }, "ran"),
            (
                RouteDecision::Fallback {
                    from: "r1".into(),
                    to: None,
                    reason: "ctx".into(),
                },
                "fallback",
            ),
            (
                RouteDecision::Skipped {
                    reason: "no route".into(),
                },
                "skipped",
            ),
            (
                RouteDecision::SkippedAlerted {
                    reason: "alerted".into(),
                },
                "skipped_alerted",
            ),
        ];
        for (d, kind) in cases {
            let v = serde_json::to_value(&d).unwrap();
            assert_eq!(v["kind"], kind);
            let back: RouteDecision = serde_json::from_value(v).unwrap();
            assert_eq!(d, back);
        }
    }
}
