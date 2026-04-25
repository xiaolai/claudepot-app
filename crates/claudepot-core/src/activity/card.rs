//! Public card types — see `dev-docs/activity-cards-design.md` §2.
//!
//! These are the shapes that cross the `claudepot-core` boundary into
//! the CLI presenter and (Phase 2+) the Tauri DTO layer. Public enums
//! (`CardKind`, `Severity`, `ConfigScope`) are `#[non_exhaustive]` so
//! later phases can add variants without breaking downstream
//! exhaustive matches; callers MUST include a `_ =>` arm.
//!
//! Cards never carry stdout/stderr bodies. The body is fetched lazily
//! from the source JSONL using `byte_offset` (design v2 §1, call #3).
//! Storing the body would: (a) duplicate the canonical store,
//! (b) require a redaction pass at the storage boundary, (c) add
//! migration cost on every payload-shape change.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One activity card. See module docs for the design rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    /// SQLite rowid. `None` for cards that have not yet been persisted
    /// (the classifier emits this shape; the index assigns the id on
    /// insert).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Absolute path of the source JSONL. Pairs with `byte_offset` for
    /// O(1) seek when fetching the body.
    pub session_path: PathBuf,
    /// JSONL `uuid` field. Optional because some lines (notably
    /// pre-2.1.85 `progress` envelopes) omit the canonical uuid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_uuid: Option<String>,
    /// Byte offset of the line's first byte in the JSONL file.
    /// Direct-seek anchor for `activity_card_body`.
    pub byte_offset: u64,
    pub kind: CardKind,
    pub ts: DateTime<Utc>,
    pub severity: Severity,
    /// ≤80 chars; ASCII safe. Used as the primary list label.
    pub title: String,
    /// ≤120 chars; one-line context shown under the title (e.g. the
    /// failing command, the agent description). `None` when there is
    /// nothing meaningful to add — never a placeholder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Diagnosis + remediation pointer. `None` is an honest answer: we
    /// could not deterministically diagnose this failure. **Never**
    /// fabricate a help line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<HelpRef>,
    /// File:line into the user's config that caused this card. `line`
    /// is `None` until the parser tracks byte offsets (Phase 4 in the
    /// design); v1 ships file path + scope only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    /// Working directory the session was running in.
    pub cwd: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
}

/// What kind of activity this card represents. Marked
/// `#[non_exhaustive]` so adding new variants in later phases does
/// not break downstream callers' exhaustive `match`. Callers MUST
/// have a `_ =>` arm.
///
/// Suppression rules — see design v2 §2:
/// - Successful sub-5s hooks with empty stdout: invisible.
/// - `nested_memory` rule loads: invisible.
/// - `skill_listing`: invisible.
/// - Built-in commands: invisible unless they fail.
/// - Successful tool calls: invisible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CardKind {
    /// `attachment.type` ∈ {hook_non_blocking_error, hook_blocking_error,
    /// hook_cancelled, hook_error_during_execution, hook_stopped_continuation}.
    HookFailure,
    /// `hook_success` with `durationMs > 5000`. Visible because the
    /// duration itself is the signal.
    HookSlow,
    /// `hook_system_message` / `hook_additional_context` that warrants
    /// surfacing (Phase 3+ heuristic; v1 emits none of these).
    HookGuidance,
    /// `Agent` tool_use ↔ matching `tool_result`, emitted only on
    /// failure or when `durationMs > 60000`. v1 does not emit these
    /// (episode tracker lands in Phase 3).
    AgentReturn,
    /// `Agent` tool_use without a closing `tool_result` at session end.
    /// Phase 3.
    AgentStranded,
    /// Any non-Agent `tool_result` with `is_error: true`.
    ToolError,
    /// User-typed slash command followed by an error system message.
    /// v1 emits only the failure case for plugin commands.
    CommandFailure,
    /// SessionStart, model switch via /model, plan-mode entered, large
    /// compact triggered. Phase 3.
    SessionMilestone,
}

impl CardKind {
    /// Human-readable short label for CLI output. Stable wire name —
    /// don't reword without a migration plan for callers that filter
    /// by exact match on the JSON form.
    pub fn label(self) -> &'static str {
        match self {
            Self::HookFailure => "hook",
            Self::HookSlow => "hook-slow",
            Self::HookGuidance => "hook-info",
            Self::AgentReturn => "agent",
            Self::AgentStranded => "agent-stranded",
            Self::ToolError => "tool-error",
            Self::CommandFailure => "command",
            Self::SessionMilestone => "milestone",
        }
    }
}

/// One severity scale, four levels. Notifications fire on Warn and
/// above (opt-in per `activity-implementation-plan.md §5.6`). Cards
/// render color only at Warn or Error.
///
/// Marked `#[non_exhaustive]` to leave room for additional levels
/// (e.g. `Critical`) without breaking exhaustive matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Severity {
    Info,
    Notice,
    Warn,
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Notice => "NOTICE",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Reference into the help template catalog, plus the substitution
/// args needed to render the template's text. The template itself
/// lives in `templates::render` — keeping the English text out of
/// SQLite makes catalog edits a code change, not a data migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpRef {
    pub template_id: String,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

/// Pointer into the user's config that originated this event. v1
/// ships file path + scope; line numbers land when the merged-settings
/// parser tracks byte offsets (Phase 4 in the design).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub scope: ConfigScope,
}

/// Which settings layer the source reference points into. Mirrors
/// CC's own four-layer cascade — Project / Local / User / Managed.
/// Unrelated to `claudepot_core::keys` scope vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ConfigScope {
    Project,
    Local,
    User,
    Managed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_kind_label_is_stable_wire_name() {
        // If you find yourself wanting to change one of these, add a
        // new variant instead — these strings show up in CLI scripts,
        // SQLite rows, and (Phase 2) the JS bridge.
        assert_eq!(CardKind::HookFailure.label(), "hook");
        assert_eq!(CardKind::ToolError.label(), "tool-error");
        assert_eq!(CardKind::AgentStranded.label(), "agent-stranded");
    }

    #[test]
    fn severity_orders_info_below_error() {
        assert!(Severity::Info < Severity::Notice);
        assert!(Severity::Notice < Severity::Warn);
        assert!(Severity::Warn < Severity::Error);
    }

    #[test]
    fn card_serde_roundtrips_minimal() {
        let c = Card {
            id: None,
            session_path: PathBuf::from("/tmp/x.jsonl"),
            event_uuid: Some("u1".into()),
            byte_offset: 0,
            kind: CardKind::HookFailure,
            ts: Utc::now(),
            severity: Severity::Warn,
            title: "Hook failed: PostToolUse:Edit".into(),
            subtitle: None,
            help: None,
            source_ref: None,
            cwd: PathBuf::from("/Users/x/proj"),
            git_branch: None,
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: Card = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn card_serde_omits_optional_fields_when_none() {
        let c = Card {
            id: None,
            session_path: PathBuf::from("/tmp/x.jsonl"),
            event_uuid: None,
            byte_offset: 42,
            kind: CardKind::ToolError,
            ts: Utc::now(),
            severity: Severity::Error,
            title: "Bash failed".into(),
            subtitle: None,
            help: None,
            source_ref: None,
            cwd: PathBuf::from("/x"),
            git_branch: None,
        };
        let s = serde_json::to_string(&c).unwrap();
        // Cleanliness check: no nullable noise crosses the boundary.
        assert!(!s.contains("\"id\""));
        assert!(!s.contains("\"event_uuid\""));
        assert!(!s.contains("\"subtitle\""));
        assert!(!s.contains("\"help\""));
        assert!(!s.contains("\"source_ref\""));
        assert!(!s.contains("\"git_branch\""));
    }
}
