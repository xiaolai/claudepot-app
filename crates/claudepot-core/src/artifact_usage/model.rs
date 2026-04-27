//! Public types for artifact usage tracking.
//!
//! `ArtifactKind` is the cross-cutting noun — every event belongs to
//! exactly one. `Outcome` is the tri-state status (`Ok` / `Error` /
//! `Cancelled`). `UsageEvent` is the row written to `usage_event`.
//! `UsageStats` is the rollup served to UI callers.
//!
//! Marked `#[non_exhaustive]` so adding new kinds (e.g. `Memory` for
//! `nested_memory` tracking) doesn't break exhaustive matches in
//! downstream crates.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ArtifactKind {
    /// Skill invocation — sourced from `attachment.invoked_skills`.
    Skill,
    /// Hook fire — `attachment.hook_success` and the `hook_*_error`
    /// family. Outcome distinguishes the two.
    Hook,
    /// Subagent dispatch — `tool_use.name == "Agent"`. Outcome closes
    /// on the matching `tool_result` (`is_error`).
    Agent,
    /// Slash command — extracted from `<command-name>/foo` markers in
    /// user message content.
    Command,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Hook => "hook",
            Self::Agent => "agent",
            Self::Command => "command",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "skill" => Self::Skill,
            "hook" => Self::Hook,
            "agent" => Self::Agent,
            "command" => Self::Command,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Outcome {
    Ok,
    Error,
    Cancelled,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ok" => Self::Ok,
            "error" => Self::Error,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// One row written to `usage_event`. The extractor produces these from
/// JSONL lines; the index writer flushes them in the same transaction
/// that upserts the parent `SessionRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageEvent {
    pub ts_ms: i64,
    pub session_id: String,
    pub kind: ArtifactKind,
    pub artifact_key: String,
    pub plugin_id: Option<String>,
    pub outcome: Outcome,
    pub duration_ms: Option<u64>,
    /// Kind-specific bag (hookName, subagent description, etc.).
    /// `None` is the common case; a small JSON object when set.
    pub extra_json: Option<String>,
}

/// What the UI gets back. `None` for percentile fields means "no rows
/// in window with a recorded duration" (skills/commands/agents have
/// no duration; only hooks do).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageStats {
    pub count_24h: u64,
    pub count_7d: u64,
    pub count_30d: u64,
    pub error_count_30d: u64,
    pub last_seen_ms: Option<i64>,
    pub p50_ms_24h: Option<u64>,
    pub avg_ms_30d: Option<u64>,
}

impl UsageStats {
    pub fn is_empty(&self) -> bool {
        self.count_30d == 0 && self.last_seen_ms.is_none()
    }
}

/// One row for the "top used / unused" listings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageListRow {
    pub kind: ArtifactKind,
    pub artifact_key: String,
    pub plugin_id: Option<String>,
    pub stats: UsageStats,
}
