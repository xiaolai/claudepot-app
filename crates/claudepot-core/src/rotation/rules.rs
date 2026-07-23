//! Rotation rule schema. Pure data + serde + validation.
//!
//! Hand-edit-friendly: every variant carries an explicit `kind`
//! discriminator, every percent value is integer 1..=100, every list
//! is non-empty when non-trivially required. `serde(deny_unknown_fields)`
//! is **not** used at the top level so old clients don't choke on a
//! file written by a newer Claudepot — but invalid values inside known
//! fields are rejected on validate.

use serde::{Deserialize, Serialize};

use crate::services::usage_alerts::UsageWindowKind;

/// Stable wire name for a window kind, used only in validation error
/// messages. Mirrors the serde `rename_all = "snake_case"` encoding so
/// the message names the same string a hand-editor sees in the file.
fn window_wire(w: UsageWindowKind) -> &'static str {
    match w {
        UsageWindowKind::FiveHour => "five_hour",
        UsageWindowKind::SevenDay => "seven_day",
        UsageWindowKind::SevenDayOpus => "seven_day_opus",
        UsageWindowKind::SevenDaySonnet => "seven_day_sonnet",
    }
}

/// Bumped on schema-breaking changes. The store rejects files with a
/// version it doesn't recognize.
pub const SCHEMA_VERSION: u32 = 1;

/// Top-level on-disk file shape. The wrapping struct gives us a place
/// to attach metadata (schema version, last-edited timestamp, future
/// sync hints) without re-shaping the array of rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RotationRulesFile {
    /// Bumped on schema breakage. Files with an unknown version load as
    /// empty rules and the on-disk file is moved aside to `.corrupt`
    /// (handled by [`super::store`]). `serde(default)` lets a hand-
    /// authored file omit the field; load defaults it to current.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub rules: Vec<RotationRule>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for RotationRulesFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            rules: Vec::new(),
        }
    }
}

/// One rotation rule. `id` is user-facing and must be unique within a
/// file — the store's load path validates this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RotationRule {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub trigger: Trigger,
    pub action: Action,
    #[serde(default)]
    pub mode: RotationMode,
    #[serde(default)]
    pub guards: RotationGuards,
}

fn default_true() -> bool {
    true
}

/// What signal fires the rule. The active CLI account is the one
/// evaluated; selector picks the *target*.
///
/// v1 covers utilization-window thresholds. An extra-usage (monthly
/// $ budget) trigger is sketched in `dev-docs/auto-rotation.md` and
/// deferred to v1.1 — it requires extending `UsageWindows` to carry
/// the `extra_usage.utilization` field, which is a separate change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Active account's utilization on `window` is `>= pct`.
    UtilizationThreshold {
        window: UsageWindowKind,
        /// 1..=100. 0 would fire constantly; 101+ never fires.
        pct: u32,
    },
}

/// What to do when the trigger fires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Pick a target via [`Selector`] and call `cli_use(target,
    /// force=true)`. The watcher's mode (`confirm` vs `auto`) decides
    /// whether the call happens immediately or waits on user
    /// confirmation.
    RotateTo { selector: Selector },
}

/// How the target account is chosen from a candidate list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Selector {
    /// Pick the candidate with the lowest utilization on `window`.
    /// Skips the currently-active account. Returns "no candidate" when
    /// every alternate is also above the trigger's threshold.
    LeastUsed {
        window: UsageWindowKind,
        candidates: Vec<String>,
    },
    /// Pick the next candidate after the active one in list order,
    /// wrapping. Useful when accounts are roughly equivalent and the
    /// user wants deterministic round-robin behavior.
    RoundRobin { candidates: Vec<String> },
    /// Always swap to this email. Rare; used when one account is the
    /// designated overflow target.
    Explicit { email: String },
}

/// Confirmation surface for a fired rule.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RotationMode {
    /// Surface a `rotation-suggested` event; user confirms via toast.
    /// **Default** — protects against misconfigured rules in the
    /// cold-start phase.
    #[default]
    Confirm,
    /// Apply the swap immediately, log it, surface a
    /// `rotation-applied` toast for awareness.
    Auto,
}

/// Guard-rails that apply on top of the trigger logic. Defaults are
/// tuned for "useful out-of-the-box, safe under unexpected utilization
/// spikes."
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RotationGuards {
    /// Minimum seconds since the last swap by **any** rule before this
    /// rule may fire again. Stops a single tick that flips multiple
    /// rules from cascading.
    #[serde(default = "default_min_interval_secs")]
    pub min_interval_secs: u64,
    /// Cap on swaps within the trigger window's current `resets_at`
    /// cycle. Hard ceiling on misconfigured-rule blast radius.
    #[serde(default = "default_max_swaps_per_window")]
    pub max_swaps_per_window: u32,
    /// When true, the rule is a no-op while CC is mid-session; the
    /// orchestrator queues evaluation for the next tick. **Default
    /// false** — most users want the swap to take effect on the next
    /// CC restart, not get stuck waiting.
    #[serde(default)]
    pub skip_when_cc_running: bool,
}

fn default_min_interval_secs() -> u64 {
    60
}

fn default_max_swaps_per_window() -> u32 {
    3
}

impl Default for RotationGuards {
    fn default() -> Self {
        Self {
            min_interval_secs: default_min_interval_secs(),
            max_swaps_per_window: default_max_swaps_per_window(),
            skip_when_cc_running: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("rule id must not be empty")]
    EmptyId,
    #[error("rule id `{0}` appears more than once")]
    DuplicateId(String),
    #[error("threshold pct must be in 1..=100, got {0}")]
    BadThreshold(u32),
    #[error("candidate list must not be empty")]
    EmptyCandidates,
    #[error("candidate email `{0}` appears more than once")]
    DuplicateCandidate(String),
    #[error("explicit selector email must not be empty")]
    EmptyExplicitEmail,
    #[error("max_swaps_per_window must be >= 1, got 0")]
    ZeroMaxSwaps,
    #[error(
        "least_used selector window must equal the trigger window \
         (selector `{selector}`, trigger `{trigger}`)"
    )]
    LeastUsedWindowMismatch {
        selector: &'static str,
        trigger: &'static str,
    },
    #[error("schema version {found} is unsupported (expected {expected})")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },
}

impl RotationRulesFile {
    /// Validate the entire file. Fails on any structural defect — the
    /// caller (typically `store::save`) refuses to persist invalid
    /// data, so on-disk files are always loadable.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnsupportedSchemaVersion {
                found: self.schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.rules {
            r.validate()?;
            if !seen.insert(r.id.clone()) {
                return Err(ValidationError::DuplicateId(r.id.clone()));
            }
        }
        Ok(())
    }
}

impl RotationRule {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.id.trim().is_empty() {
            return Err(ValidationError::EmptyId);
        }
        self.trigger.validate()?;
        self.action.validate()?;
        if self.guards.max_swaps_per_window == 0 {
            return Err(ValidationError::ZeroMaxSwaps);
        }
        // A `least_used` selector filters candidates by the *trigger's*
        // threshold on the *selector's* window (see
        // `eval::select_least_used`). If the two windows differ, that
        // comparison is meaningless — a 90% threshold meant for the
        // 5-hour window would be applied against candidates' 7-day
        // utilization. The GUI keeps them in sync; this rejects
        // hand-edited files that don't, so a nonsensical rule is caught
        // at load/save instead of silently mis-selecting.
        if let Action::RotateTo {
            selector: Selector::LeastUsed { window: sel_w, .. },
        } = &self.action
        {
            let Trigger::UtilizationThreshold { window: trig_w, .. } = &self.trigger;
            if sel_w != trig_w {
                return Err(ValidationError::LeastUsedWindowMismatch {
                    selector: window_wire(*sel_w),
                    trigger: window_wire(*trig_w),
                });
            }
        }
        Ok(())
    }

    /// The threshold percent this rule fires at. Returns `None` for
    /// trigger shapes that don't carry a percent (currently none —
    /// kept as `Option` so future trigger kinds can opt out).
    /// Test-only assertion surface — no production consumer, so it's
    /// cfg(test)-gated.
    #[cfg(test)]
    pub fn trigger_threshold_pct(&self) -> Option<u32> {
        match &self.trigger {
            Trigger::UtilizationThreshold { pct, .. } => Some(*pct),
        }
    }
}

impl Trigger {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Trigger::UtilizationThreshold { pct, .. } => {
                if *pct == 0 || *pct > 100 {
                    return Err(ValidationError::BadThreshold(*pct));
                }
                Ok(())
            }
        }
    }
}

impl Action {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Action::RotateTo { selector } => selector.validate(),
        }
    }
}

impl Selector {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Selector::LeastUsed { candidates, .. } | Selector::RoundRobin { candidates } => {
                if candidates.is_empty() {
                    return Err(ValidationError::EmptyCandidates);
                }
                let mut seen = std::collections::HashSet::new();
                for c in candidates {
                    // Dedupe case-insensitively (and ignoring surrounding
                    // whitespace) to match `eval::resolve_email_to_uuid`,
                    // which resolves emails with `eq_ignore_ascii_case`.
                    // A case-sensitive check here would accept
                    // `["a@x.com", "A@x.com"]` — two list slots that
                    // resolve to the *same* account, skewing round-robin
                    // order and (pre-guard) risking a self-swap.
                    let key = c.trim().to_ascii_lowercase();
                    if !seen.insert(key) {
                        return Err(ValidationError::DuplicateCandidate(c.clone()));
                    }
                }
                Ok(())
            }
            Selector::Explicit { email } => {
                if email.trim().is_empty() {
                    return Err(ValidationError::EmptyExplicitEmail);
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule_least_used() -> RotationRule {
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
                    candidates: vec!["a@x.com".into(), "b@x.com".into()],
                },
            },
            mode: RotationMode::Confirm,
            guards: RotationGuards::default(),
        }
    }

    #[test]
    fn round_trip_least_used() {
        let r = rule_least_used();
        let s = serde_json::to_string(&r).unwrap();
        let back: RotationRule = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn round_trip_round_robin() {
        let r = RotationRule {
            id: "rr".into(),
            enabled: true,
            trigger: Trigger::UtilizationThreshold {
                window: UsageWindowKind::SevenDay,
                pct: 95,
            },
            action: Action::RotateTo {
                selector: Selector::RoundRobin {
                    candidates: vec!["a@x.com".into(), "b@x.com".into(), "c@x.com".into()],
                },
            },
            mode: RotationMode::Auto,
            guards: RotationGuards::default(),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<RotationRule>(&s).unwrap(), r);
    }

    #[test]
    fn round_trip_explicit_with_custom_guards() {
        let r = RotationRule {
            id: "overflow".into(),
            enabled: true,
            trigger: Trigger::UtilizationThreshold {
                window: UsageWindowKind::SevenDayOpus,
                pct: 80,
            },
            action: Action::RotateTo {
                selector: Selector::Explicit {
                    email: "overflow@x.com".into(),
                },
            },
            mode: RotationMode::Confirm,
            guards: RotationGuards {
                min_interval_secs: 120,
                max_swaps_per_window: 5,
                skip_when_cc_running: true,
            },
        };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<RotationRule>(&s).unwrap(), r);
    }

    #[test]
    fn defaults_apply_on_partial_read() {
        // mode + guards omitted; enabled omitted.
        let json = r#"{
            "id": "min",
            "trigger": { "kind": "utilization_threshold", "window": "five_hour", "pct": 90 },
            "action": {
                "kind": "rotate_to",
                "selector": {
                    "kind": "least_used",
                    "window": "five_hour",
                    "candidates": ["a@x.com"]
                }
            }
        }"#;
        let r: RotationRule = serde_json::from_str(json).unwrap();
        assert!(r.enabled);
        assert_eq!(r.mode, RotationMode::Confirm);
        assert_eq!(r.guards.min_interval_secs, 60);
        assert_eq!(r.guards.max_swaps_per_window, 3);
        assert!(!r.guards.skip_when_cc_running);
    }

    #[test]
    fn validate_rejects_empty_id() {
        let mut r = rule_least_used();
        r.id = "".into();
        assert_eq!(r.validate(), Err(ValidationError::EmptyId));
    }

    #[test]
    fn validate_rejects_bad_threshold() {
        let mut r = rule_least_used();
        r.trigger = Trigger::UtilizationThreshold {
            window: UsageWindowKind::FiveHour,
            pct: 0,
        };
        assert_eq!(r.validate(), Err(ValidationError::BadThreshold(0)));
        r.trigger = Trigger::UtilizationThreshold {
            window: UsageWindowKind::FiveHour,
            pct: 101,
        };
        assert_eq!(r.validate(), Err(ValidationError::BadThreshold(101)));
    }

    #[test]
    fn validate_rejects_empty_candidates() {
        let mut r = rule_least_used();
        r.action = Action::RotateTo {
            selector: Selector::LeastUsed {
                window: UsageWindowKind::FiveHour,
                candidates: vec![],
            },
        };
        assert_eq!(r.validate(), Err(ValidationError::EmptyCandidates));
    }

    #[test]
    fn validate_rejects_duplicate_candidates() {
        let mut r = rule_least_used();
        r.action = Action::RotateTo {
            selector: Selector::LeastUsed {
                window: UsageWindowKind::FiveHour,
                candidates: vec!["a@x.com".into(), "a@x.com".into()],
            },
        };
        assert_eq!(
            r.validate(),
            Err(ValidationError::DuplicateCandidate("a@x.com".into()))
        );
    }

    #[test]
    fn validate_rejects_empty_explicit_email() {
        let mut r = rule_least_used();
        r.action = Action::RotateTo {
            selector: Selector::Explicit { email: "".into() },
        };
        assert_eq!(r.validate(), Err(ValidationError::EmptyExplicitEmail));
    }

    #[test]
    fn validate_rejects_zero_max_swaps() {
        let mut r = rule_least_used();
        r.guards.max_swaps_per_window = 0;
        assert_eq!(r.validate(), Err(ValidationError::ZeroMaxSwaps));
    }

    #[test]
    fn validate_rejects_case_insensitive_duplicate_candidates() {
        // "a@x.com" and "A@x.com" resolve to the same account
        // (case-insensitive), so the validator must reject the pair
        // even though the raw strings differ.
        let mut r = rule_least_used();
        r.action = Action::RotateTo {
            selector: Selector::LeastUsed {
                window: UsageWindowKind::FiveHour,
                candidates: vec!["a@x.com".into(), "A@x.com".into()],
            },
        };
        assert!(matches!(
            r.validate(),
            Err(ValidationError::DuplicateCandidate(_))
        ));
    }

    #[test]
    fn validate_rejects_least_used_window_mismatch() {
        // Trigger fires on the 5-hour window but the selector picks by
        // the 7-day window — a nonsensical threshold comparison.
        let mut r = rule_least_used();
        r.trigger = Trigger::UtilizationThreshold {
            window: UsageWindowKind::FiveHour,
            pct: 90,
        };
        r.action = Action::RotateTo {
            selector: Selector::LeastUsed {
                window: UsageWindowKind::SevenDay,
                candidates: vec!["a@x.com".into(), "b@x.com".into()],
            },
        };
        assert!(matches!(
            r.validate(),
            Err(ValidationError::LeastUsedWindowMismatch {
                selector: "seven_day",
                trigger: "five_hour",
            })
        ));
    }

    #[test]
    fn validate_accepts_matching_least_used_window() {
        // The default helper already matches (5h/5h) — assert it passes
        // so the mismatch check can't over-reject the common case.
        assert!(rule_least_used().validate().is_ok());
    }

    #[test]
    fn validate_ignores_window_for_round_robin_and_explicit() {
        // The window-match rule only applies to `least_used`. A
        // round-robin or explicit selector carries no window and must
        // validate regardless of the trigger window.
        let mut rr = rule_least_used();
        rr.trigger = Trigger::UtilizationThreshold {
            window: UsageWindowKind::SevenDayOpus,
            pct: 80,
        };
        rr.action = Action::RotateTo {
            selector: Selector::RoundRobin {
                candidates: vec!["a@x.com".into(), "b@x.com".into()],
            },
        };
        assert!(rr.validate().is_ok());
    }

    #[test]
    fn validate_file_rejects_duplicate_ids() {
        let r = rule_least_used();
        let file = RotationRulesFile {
            schema_version: SCHEMA_VERSION,
            rules: vec![r.clone(), r.clone()],
        };
        assert_eq!(
            file.validate(),
            Err(ValidationError::DuplicateId("5h-near-cap".into()))
        );
    }

    #[test]
    fn validate_file_rejects_unknown_schema_version() {
        let file = RotationRulesFile {
            schema_version: 99,
            rules: vec![],
        };
        assert!(matches!(
            file.validate(),
            Err(ValidationError::UnsupportedSchemaVersion {
                found: 99,
                expected: 1
            })
        ));
    }

    #[test]
    fn empty_file_validates() {
        let f = RotationRulesFile::default();
        assert!(f.validate().is_ok());
    }

    #[test]
    fn trigger_threshold_pct_extracts_value() {
        let r = rule_least_used();
        assert_eq!(r.trigger_threshold_pct(), Some(90));
    }

    /// Golden round-trip: a representative file shape decodes,
    /// re-encodes, and the second decode equals the first. Catches
    /// drift if a future refactor changes serialization defaults.
    #[test]
    fn golden_round_trip_full_file() {
        let golden = r#"{
  "schema_version": 1,
  "rules": [
    {
      "id": "5h-near-cap",
      "enabled": true,
      "trigger": { "kind": "utilization_threshold", "window": "five_hour", "pct": 90 },
      "action": {
        "kind": "rotate_to",
        "selector": {
          "kind": "least_used",
          "window": "five_hour",
          "candidates": ["a@x.com", "b@x.com"]
        }
      },
      "mode": "confirm",
      "guards": {
        "min_interval_secs": 60,
        "max_swaps_per_window": 3,
        "skip_when_cc_running": false
      }
    }
  ]
}"#;
        let file: RotationRulesFile = serde_json::from_str(golden).unwrap();
        assert!(file.validate().is_ok());
        // Re-serialize, re-parse, compare to first parse — any
        // drift in field defaults or serde renames trips the
        // assertion.
        let encoded = serde_json::to_string(&file).unwrap();
        let back: RotationRulesFile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(back, file);
    }

    /// Reject an unknown window value (typo in a hand-edited file).
    /// `UsageWindowKind` is a closed enum on the wire — serde
    /// errors on unknown variants, which the store's load path
    /// handles as corruption (rename-aside). This test pins the
    /// schema-level rejection behavior independent of the store.
    #[test]
    fn reject_unknown_window_value() {
        let json = r#"{
            "id": "r",
            "trigger": { "kind": "utilization_threshold", "window": "not_a_window", "pct": 90 },
            "action": { "kind": "rotate_to", "selector": { "kind": "explicit", "email": "a@x.com" } }
        }"#;
        let r = serde_json::from_str::<RotationRule>(json);
        assert!(r.is_err(), "unknown window must be a parse error");
    }

    /// Reject an unknown trigger kind (forward-incompatible client
    /// or hand-edit typo). The error is bubbled by serde rather
    /// than silently dropping the rule.
    #[test]
    fn reject_unknown_trigger_kind() {
        let json = r#"{
            "schema_version": 1,
            "rules": [{
                "id": "r",
                "trigger": { "kind": "future_kind", "window": "five_hour", "pct": 90 },
                "action": { "kind": "rotate_to", "selector": { "kind": "explicit", "email": "a@x.com" } }
            }]
        }"#;
        let r = serde_json::from_str::<RotationRulesFile>(json);
        assert!(r.is_err(), "unknown trigger kind must be a parse error");
    }
}
