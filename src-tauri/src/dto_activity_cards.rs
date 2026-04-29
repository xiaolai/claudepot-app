//! DTOs for the activity *cards* surface (per-event forensic
//! stream). Distinct from `dto_activity.rs` which serves the
//! live-session aggregate strip — see the design split in
//! `dev-docs/activity-cards-design.md` §9.
//!
//! Stable wire shape: every enum variant uses `serde_camelCase`
//! values that match the JS-side discriminated union. Adding new
//! kinds appends; never reorder, never rename.

use claudepot_core::activity::{Card, CardKind, ConfigScope, HelpRef, Severity, SourceRef};
use serde::{Deserialize, Serialize};

/// One activity card, JSON-serializable for the JS bridge. Mirrors
/// `claudepot_core::activity::Card` with: `PathBuf` flattened to
/// strings (webview can't hold OsString), `DateTime<Utc>` projected
/// to `ts_ms` (ms since epoch), every Option that's `None` omitted
/// from the wire frame.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityCardDto {
    pub id: i64,
    pub session_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_uuid: Option<String>,
    pub byte_offset: u64,
    pub kind: String,
    pub ts_ms: i64,
    pub severity: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<HelpRefDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRefDto>,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HelpRefDto {
    pub template_id: String,
    pub args: std::collections::BTreeMap<String, String>,
    /// Pre-rendered English text from the template catalog. Saves
    /// the JS side from having to ship a parallel template registry
    /// (which would also drift). `None` means the template id was
    /// unknown to this binary — the renderer should hide the help
    /// line rather than guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendered: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRefDto {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub scope: String,
}

impl From<&Card> for ActivityCardDto {
    fn from(c: &Card) -> Self {
        Self {
            id: c.id.unwrap_or(0),
            session_path: c.session_path.to_string_lossy().into_owned(),
            event_uuid: c.event_uuid.clone(),
            byte_offset: c.byte_offset,
            kind: c.kind.label().to_string(),
            ts_ms: c.ts.timestamp_millis(),
            severity: c.severity.label().to_string(),
            title: c.title.clone(),
            subtitle: c.subtitle.clone(),
            help: c.help.as_ref().map(HelpRefDto::from),
            source_ref: c.source_ref.as_ref().map(SourceRefDto::from),
            cwd: c.cwd.to_string_lossy().into_owned(),
            git_branch: c.git_branch.clone(),
            plugin: c.plugin.clone(),
        }
    }
}

impl From<&HelpRef> for HelpRefDto {
    fn from(h: &HelpRef) -> Self {
        Self {
            template_id: h.template_id.clone(),
            args: h.args.clone(),
            rendered: claudepot_core::activity::render_help(h),
        }
    }
}

impl From<&SourceRef> for SourceRefDto {
    fn from(s: &SourceRef) -> Self {
        Self {
            path: s.path.to_string_lossy().into_owned(),
            line: s.line,
            scope: match s.scope {
                ConfigScope::Project => "project".to_string(),
                ConfigScope::Local => "local".to_string(),
                ConfigScope::User => "user".to_string(),
                ConfigScope::Managed => "managed".to_string(),
                _ => "unknown".to_string(),
            },
        }
    }
}

/// Filter set for `cards_recent` and `cards_count_new_since`. Every
/// field is optional; absent = no constraint on that dimension.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardsRecentQueryDto {
    /// Cards with `ts_ms >= since_ms`. Use absolute ms-since-epoch.
    pub since_ms: Option<i64>,
    /// String labels (camelCase wire form) — parsed via
    /// `parse_card_kind` per element.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// `info | notice | warn | error`. Returns cards with severity
    /// at or above this level.
    pub min_severity: Option<String>,
    /// Substring-prefix filter on `cwd`. Plain prefix match (no
    /// glob semantics) — safe for paths containing `%` or `_`.
    pub project_path_prefix: Option<String>,
    /// `<plugin>` or `<plugin>@<owner>` form.
    pub plugin: Option<String>,
    /// Default 200, max 10_000.
    pub limit: Option<u32>,
}

impl CardsRecentQueryDto {
    pub fn into_core(self) -> Result<claudepot_core::activity::RecentQuery, String> {
        // Resolve the fallible fields first so the struct literal
        // below is a clean one-shot — avoids the
        // field_reassign_with_default lint on top of `Default::default()`.
        let kinds = self
            .kinds
            .iter()
            .map(|k| parse_card_kind(k))
            .collect::<Result<Vec<_>, _>>()?;
        let min_severity = self
            .min_severity
            .as_deref()
            .map(parse_severity)
            .transpose()?;
        Ok(claudepot_core::activity::RecentQuery {
            since_ms: self.since_ms,
            kinds,
            min_severity,
            project_path_prefix: self.project_path_prefix.map(std::path::PathBuf::from),
            plugin: self.plugin,
            limit: self.limit.map(|n| n as usize),
        })
    }
}

fn parse_card_kind(s: &str) -> Result<CardKind, String> {
    Ok(match s {
        "hook" => CardKind::HookFailure,
        "hook-slow" => CardKind::HookSlow,
        "hook-info" => CardKind::HookGuidance,
        "agent" => CardKind::AgentReturn,
        "agent-stranded" => CardKind::AgentStranded,
        "tool-error" => CardKind::ToolError,
        "command" => CardKind::CommandFailure,
        "milestone" => CardKind::SessionMilestone,
        other => {
            return Err(format!(
                "unknown card kind {other:?}; valid: hook, hook-slow, hook-info, agent, agent-stranded, tool-error, command, milestone"
            ))
        }
    })
}

fn parse_severity(s: &str) -> Result<Severity, String> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "info" => Severity::Info,
        "notice" => Severity::Notice,
        "warn" | "warning" => Severity::Warn,
        "error" | "err" => Severity::Error,
        other => {
            return Err(format!(
                "unknown severity {other:?}; valid: info, notice, warn, error"
            ))
        }
    })
}

/// Click-through navigation payload. The JS side uses this to
/// switch to the Sessions section and seek to the right line.
/// `event_uuid` is the preferred anchor (stable across re-parses);
/// `byte_offset` is the fallback when the JSONL has no uuid on that
/// line (rare but observed in pre-2.1.85 records).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardNavigateDto {
    pub session_path: String,
    pub byte_offset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_uuid: Option<String>,
}

/// Counts payload for the "N new since you were away" badge plus
/// total. The renderer compares `total` to `new` to decide whether
/// to show the badge at all (no point on a fresh-install zero).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardsCountDto {
    pub total: i64,
    pub new: i64,
    pub last_seen_id: Option<i64>,
}

/// Backfill outcome. Same shape as the CLI prints, plus a
/// truncated failures list (cap at 50 to keep IPC frames small).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardsReindexResultDto {
    pub files_scanned: usize,
    pub cards_inserted: usize,
    pub cards_skipped_duplicates: usize,
    pub cards_pruned: usize,
    pub failed: Vec<CardsReindexFailureDto>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardsReindexFailureDto {
    pub path: String,
    pub error: String,
}
