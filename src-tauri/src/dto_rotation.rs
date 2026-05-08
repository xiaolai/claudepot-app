//! DTOs that cross the Tauri boundary for the rotation feature.
//!
//! Mirror `claudepot_core::rotation::rules` types but re-shaped to be
//! the most ergonomic for the renderer: serde camelCase, `Uuid` ↔
//! `String`, and a couple of thin wrappers (`PendingSwapDto`,
//! `RotationDryRunDto`) the front-end form needs.
//!
//! Round-tripping through DTOs avoids a circular dep where the
//! frontend would otherwise need to know `claudepot-core`'s wire
//! shape verbatim.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use claudepot_core::rotation::audit::{
    AuditMode, RotationAuditEntry, RotationOutcome, RotationTriggerSummary,
};
use claudepot_core::rotation::rules::{
    Action, RotationGuards, RotationMode, RotationRule, RotationRulesFile, Selector, Trigger,
    SCHEMA_VERSION,
};
use claudepot_core::services::usage_alerts::UsageWindowKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationRulesFileDto {
    pub schema_version: u32,
    pub rules: Vec<RotationRuleDto>,
}

impl From<RotationRulesFile> for RotationRulesFileDto {
    fn from(f: RotationRulesFile) -> Self {
        Self {
            schema_version: f.schema_version,
            rules: f.rules.into_iter().map(RotationRuleDto::from).collect(),
        }
    }
}

impl TryFrom<RotationRulesFileDto> for RotationRulesFile {
    type Error = String;
    fn try_from(d: RotationRulesFileDto) -> Result<Self, Self::Error> {
        Ok(RotationRulesFile {
            schema_version: d.schema_version,
            rules: d
                .rules
                .into_iter()
                .map(RotationRule::try_from)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl Default for RotationRulesFileDto {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationRuleDto {
    pub id: String,
    pub enabled: bool,
    pub trigger: TriggerDto,
    pub action: ActionDto,
    pub mode: String, // "confirm" | "auto"
    pub guards: GuardsDto,
}

impl From<RotationRule> for RotationRuleDto {
    fn from(r: RotationRule) -> Self {
        Self {
            id: r.id,
            enabled: r.enabled,
            trigger: r.trigger.into(),
            action: r.action.into(),
            mode: match r.mode {
                RotationMode::Confirm => "confirm".into(),
                RotationMode::Auto => "auto".into(),
            },
            guards: r.guards.into(),
        }
    }
}

impl TryFrom<RotationRuleDto> for RotationRule {
    type Error = String;
    fn try_from(d: RotationRuleDto) -> Result<Self, Self::Error> {
        let mode = match d.mode.as_str() {
            "confirm" | "" => RotationMode::Confirm,
            "auto" => RotationMode::Auto,
            other => return Err(format!("unknown rotation mode: {other}")),
        };
        Ok(RotationRule {
            id: d.id,
            enabled: d.enabled,
            trigger: d.trigger.try_into()?,
            action: d.action.try_into()?,
            mode,
            guards: d.guards.into(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDto {
    /// Always `"utilization_threshold"` in v1. Kept as a free-form
    /// string so v1.1's `extra_usage_threshold` only needs an
    /// additive serde change here, not a versioned migration.
    pub kind: String,
    /// Required for `utilization_threshold`. Empty string for
    /// trigger kinds that don't carry a window.
    #[serde(default)]
    pub window: String,
    pub pct: u32,
}

impl From<Trigger> for TriggerDto {
    fn from(t: Trigger) -> Self {
        match t {
            Trigger::UtilizationThreshold { window, pct } => Self {
                kind: "utilization_threshold".into(),
                window: window_kind_str(window),
                pct,
            },
        }
    }
}

impl TryFrom<TriggerDto> for Trigger {
    type Error = String;
    fn try_from(d: TriggerDto) -> Result<Self, Self::Error> {
        match d.kind.as_str() {
            "utilization_threshold" => Ok(Trigger::UtilizationThreshold {
                window: window_kind_from_str(&d.window)?,
                pct: d.pct,
            }),
            other => Err(format!("unknown trigger kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDto {
    /// Always `"rotate_to"` in v1.
    pub kind: String,
    pub selector: SelectorDto,
}

impl From<Action> for ActionDto {
    fn from(a: Action) -> Self {
        match a {
            Action::RotateTo { selector } => Self {
                kind: "rotate_to".into(),
                selector: selector.into(),
            },
        }
    }
}

impl TryFrom<ActionDto> for Action {
    type Error = String;
    fn try_from(d: ActionDto) -> Result<Self, Self::Error> {
        match d.kind.as_str() {
            "rotate_to" => Ok(Action::RotateTo {
                selector: d.selector.try_into()?,
            }),
            other => Err(format!("unknown action kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectorDto {
    pub kind: String, // "least_used" | "round_robin" | "explicit"
    /// Used by `least_used`. Empty for the others.
    #[serde(default)]
    pub window: String,
    /// Used by `least_used` and `round_robin`. Empty for `explicit`.
    #[serde(default)]
    pub candidates: Vec<String>,
    /// Used by `explicit`. Empty for the others.
    #[serde(default)]
    pub email: String,
}

impl From<Selector> for SelectorDto {
    fn from(s: Selector) -> Self {
        match s {
            Selector::LeastUsed { window, candidates } => Self {
                kind: "least_used".into(),
                window: window_kind_str(window),
                candidates,
                email: String::new(),
            },
            Selector::RoundRobin { candidates } => Self {
                kind: "round_robin".into(),
                window: String::new(),
                candidates,
                email: String::new(),
            },
            Selector::Explicit { email } => Self {
                kind: "explicit".into(),
                window: String::new(),
                candidates: Vec::new(),
                email,
            },
        }
    }
}

impl TryFrom<SelectorDto> for Selector {
    type Error = String;
    fn try_from(d: SelectorDto) -> Result<Self, Self::Error> {
        match d.kind.as_str() {
            "least_used" => Ok(Selector::LeastUsed {
                window: window_kind_from_str(&d.window)?,
                candidates: d.candidates,
            }),
            "round_robin" => Ok(Selector::RoundRobin {
                candidates: d.candidates,
            }),
            "explicit" => Ok(Selector::Explicit { email: d.email }),
            other => Err(format!("unknown selector kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardsDto {
    pub min_interval_secs: u64,
    pub max_swaps_per_window: u32,
    pub skip_when_cc_running: bool,
}

impl From<RotationGuards> for GuardsDto {
    fn from(g: RotationGuards) -> Self {
        Self {
            min_interval_secs: g.min_interval_secs,
            max_swaps_per_window: g.max_swaps_per_window,
            skip_when_cc_running: g.skip_when_cc_running,
        }
    }
}

impl From<GuardsDto> for RotationGuards {
    fn from(d: GuardsDto) -> Self {
        Self {
            min_interval_secs: d.min_interval_secs,
            max_swaps_per_window: d.max_swaps_per_window,
            skip_when_cc_running: d.skip_when_cc_running,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationAuditEntryDto {
    pub id: u64,
    pub ts: DateTime<Utc>,
    pub rule_id: String,
    pub trigger: TriggerSummaryDto,
    pub from_email: String,
    pub to_email: Option<String>,
    pub mode: String,
    pub outcome: String,
    pub reason: String,
}

impl From<RotationAuditEntry> for RotationAuditEntryDto {
    fn from(e: RotationAuditEntry) -> Self {
        Self {
            id: e.id,
            ts: e.ts,
            rule_id: e.rule_id,
            trigger: e.trigger.into(),
            from_email: e.from_email,
            to_email: e.to_email,
            mode: match e.mode {
                AuditMode::Confirm => "confirm".into(),
                AuditMode::Auto => "auto".into(),
            },
            outcome: rotation_outcome_str(e.outcome).into(),
            reason: e.reason,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSummaryDto {
    pub window: Option<String>,
    pub utilization_pct: f64,
    pub threshold_pct: u32,
    pub is_extra_usage: bool,
}

impl From<RotationTriggerSummary> for TriggerSummaryDto {
    fn from(s: RotationTriggerSummary) -> Self {
        Self {
            window: s.window.map(window_kind_str),
            utilization_pct: s.utilization_pct,
            threshold_pct: s.threshold_pct,
            is_extra_usage: s.is_extra_usage,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSwapDto {
    pub swap_id: String,
    pub rule_id: String,
    pub from_email: String,
    pub to_email: String,
    pub queued_at: DateTime<Utc>,
    /// Trigger details from the moment the swap was queued. Lets
    /// the renderer's hydration path show a meaningful toast
    /// instead of a stripped-down placeholder.
    pub trigger: TriggerSummaryDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationDryRunDto {
    /// True iff the rule, given the current snapshot + audit, would
    /// emit a `Fire` decision on the next tick.
    pub would_fire: bool,
    /// Email of the target the selector would pick. `None` when the
    /// rule wouldn't fire or no candidate is selectable.
    pub target_email: Option<String>,
    /// Why the rule would or wouldn't fire. Free-form, reader-friendly.
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Window-kind helpers
// ---------------------------------------------------------------------------

pub(crate) fn window_kind_str(k: UsageWindowKind) -> String {
    match k {
        UsageWindowKind::FiveHour => "five_hour".into(),
        UsageWindowKind::SevenDay => "seven_day".into(),
        UsageWindowKind::SevenDayOpus => "seven_day_opus".into(),
        UsageWindowKind::SevenDaySonnet => "seven_day_sonnet".into(),
    }
}

pub(crate) fn window_kind_from_str(s: &str) -> Result<UsageWindowKind, String> {
    match s {
        "five_hour" => Ok(UsageWindowKind::FiveHour),
        "seven_day" => Ok(UsageWindowKind::SevenDay),
        "seven_day_opus" => Ok(UsageWindowKind::SevenDayOpus),
        "seven_day_sonnet" => Ok(UsageWindowKind::SevenDaySonnet),
        other => Err(format!("unknown window: {other}")),
    }
}

fn rotation_outcome_str(o: RotationOutcome) -> &'static str {
    match o {
        RotationOutcome::Applied => "applied",
        RotationOutcome::Suggested => "suggested",
        RotationOutcome::SkippedGuard => "skipped_guard",
        RotationOutcome::SkippedCcRunning => "skipped_cc_running",
        RotationOutcome::NoCandidate => "no_candidate",
        RotationOutcome::Failed => "failed",
    }
}
