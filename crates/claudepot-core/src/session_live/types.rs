//! Public data types for `session_live`.
//!
//! These are the shapes that cross the `claudepot-core` boundary into
//! the Tauri DTO layer. Every `String` field carrying user-authored
//! or model-generated content is understood to have already passed
//! through `redact::redact_secrets` before construction — the type
//! system does not enforce this yet (planned: a `Redacted<String>`
//! newtype in M2), so constructors are responsible.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Canonical status vocabulary. Matches the three values Claude Code
/// itself publishes via `concurrentSessions::SessionStatus` (see
/// `~/github/claude_code_src/src/utils/concurrentSessions.ts:19`).
///
/// Claudepot adds two *overlays* (`errored`, `stuck`) as separate
/// fields on `LiveSessionSummary` — not as extra variants here — so
/// the base status stays aligned with CC's own terminology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The model is producing output or a tool is executing.
    Busy,
    /// The model has finished its turn and is awaiting user input.
    Idle,
    /// Blocked on user approval or a dialog (the PID file's
    /// `waitingFor` string supplies the specific verb).
    Waiting,
}

impl Status {
    /// Parse the string CC writes into the PID file's `status` field.
    /// Unknown values fall back to `Idle` — conservative: don't claim
    /// activity we can't identify.
    pub fn from_pid_field(raw: &str) -> Self {
        match raw {
            "busy" => Self::Busy,
            "waiting" => Self::Waiting,
            _ => Self::Idle,
        }
    }
}

/// One top-level Claude Code process registered under
/// `~/.claude/sessions/<pid>.json`. Fields mirror CC's own writer at
/// `concurrentSessions.ts::registerSession`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidRecord {
    pub pid: u32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    #[serde(rename = "startedAt")]
    pub started_at_ms: i64,
    #[serde(rename = "updatedAt", default)]
    pub updated_at_ms: Option<i64>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// Present only when the `BG_SESSIONS` feature gate is on in CC.
    /// When absent, fall back to transcript-tail status derivation.
    #[serde(default)]
    pub status: Option<String>,
    /// Only populated when `status == "waiting"`. Short verb phrase
    /// like `"approve Bash"` or `"input needed"`.
    #[serde(rename = "waitingFor", default)]
    pub waiting_for: Option<String>,
}

/// Aggregate row published to the `live-all` subscriber channel.
/// One per live session. Intended for tray / strip / status-bar
/// consumers — light enough to ship on every update.
///
/// Secrets invariant: every `String` here is either a non-user-data
/// literal (`cwd`, `session_id`, model id) or has been passed through
/// `redact::redact_secrets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveSessionSummary {
    pub session_id: String,
    pub pid: u32,
    pub cwd: String,
    /// File path of the main transcript when resolvable, else `None`.
    pub transcript_path: Option<String>,
    pub status: Status,
    /// Short phrase describing the current action — prefers the
    /// most recent `task-summary`, falls back to "<tool>: <first-arg>"
    /// when derivable, otherwise `None`.
    pub current_action: Option<String>,
    /// Last model id seen in `assistant.message.model`. In M1 this
    /// is the **raw transcript value** — e.g., you may see
    /// `claude-haiku-4-5-20251001` verbatim. The `pricing` module
    /// (M4) introduces `canonicalize_model_id` that collapses dated
    /// variants to `claude-haiku-4-5` for cost lookup; callers that
    /// just want to display the model should keep using this raw
    /// field.
    pub model: Option<String>,
    /// CC's own `waitingFor` string when status == Waiting.
    pub waiting_for: Option<String>,
    /// Overlay: ≥2 `tool_result.is_error=true` in trailing 60 s.
    pub errored: bool,
    /// Overlay: unmatched `tool_use` older than configured threshold.
    pub stuck: bool,
    /// Milliseconds since the last forward progress. Powers the
    /// elapsed counter without every subscriber having to compute
    /// wall-clock time in the UI thread.
    pub idle_ms: i64,
    /// Monotonic sequence for this session — lets the detail channel
    /// resync against this aggregate snapshot.
    pub seq: u64,
}

/// Per-session detail delivered on the `live::<session_id>` channel.
/// Carries the sequence number so subscribers can detect gaps and
/// trigger resync. `resync_required` is set by the producer when the
/// bounded channel overflows (a slow webview consumer fell behind).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveDelta {
    pub session_id: String,
    pub seq: u64,
    /// Timestamp when the delta was produced (ms since epoch).
    pub produced_at_ms: i64,
    pub kind: LiveDeltaKind,
    /// Set to true on the first delta following a channel overflow.
    /// Subscribers MUST discard local state and call
    /// `LiveRuntime::session_snapshot` before applying more deltas.
    pub resync_required: bool,
}

/// The shape of an individual live event. Deliberately narrow for
/// M1 — covers only what the sidebar strip needs (status transitions
/// and light summary updates). M2 adds transcript-row appends for
/// the live pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveDeltaKind {
    /// Status or waiting-for changed.
    StatusChanged {
        status: Status,
        waiting_for: Option<String>,
    },
    /// A new `task-summary` was written to the transcript.
    TaskSummaryChanged { summary: String },
    /// Model id for the current turn changed (e.g., user switched
    /// from Sonnet to Opus via /model).
    ModelChanged { model: String },
    /// Overlay transition — errored or stuck state flipped.
    OverlayChanged { errored: bool, stuck: bool },
    /// An activity card was extracted from this session's tail.
    /// Carries the assigned `id` from the index plus the payload
    /// fields the GUI needs to render an inline strip without a
    /// follow-up query. Subscribers should debounce by `id` —
    /// rare double-emit (rotation race) is the only case where
    /// the same id might appear twice.
    ///
    /// `card_kind` (not `kind`) avoids colliding with the serde
    /// discriminator tag on this enum.
    CardEmitted {
        id: i64,
        card_kind: String,
        severity: String,
        title: String,
        ts_ms: i64,
        plugin: Option<String>,
        cwd: String,
    },
    /// Session has ended (process died or PID file removed). The
    /// subscriber should discard state and unsubscribe.
    Ended,
}

/// Utility: current monotonic-ish wall clock in ms since epoch. Tests
/// replace this via `produced_at_ms` on fixtures; production callers
/// use `LiveDelta::now_ms` to stamp deltas at emission time.
impl LiveDelta {
    pub fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parses_cc_vocabulary() {
        assert_eq!(Status::from_pid_field("busy"), Status::Busy);
        assert_eq!(Status::from_pid_field("idle"), Status::Idle);
        assert_eq!(Status::from_pid_field("waiting"), Status::Waiting);
    }

    /// Unknown strings must not be silently mapped to Busy — that
    /// would misreport activity. Idle is the safe conservative.
    #[test]
    fn status_unknown_falls_back_to_idle() {
        assert_eq!(Status::from_pid_field("running"), Status::Idle);
        assert_eq!(Status::from_pid_field(""), Status::Idle);
        assert_eq!(Status::from_pid_field("BUSY"), Status::Idle);
    }

    #[test]
    fn status_serde_is_lowercase_for_ts_boundary() {
        let s = serde_json::to_string(&Status::Busy).unwrap();
        assert_eq!(s, r#""busy""#);
        let back: Status = serde_json::from_str(r#""waiting""#).unwrap();
        assert_eq!(back, Status::Waiting);
    }

    #[test]
    fn pid_record_parses_live_fixture_without_status() {
        let raw = include_str!("testdata/pid/24813-fixture.json");
        let r: PidRecord = serde_json::from_str(raw).unwrap();
        assert_eq!(r.pid, 99835);
        assert_eq!(r.kind.as_deref(), Some("interactive"));
        assert!(r.status.is_none(), "BG_SESSIONS off → no status field");
        assert!(r.updated_at_ms.is_some());
    }

    #[test]
    fn pid_record_parses_bg_busy_fixture() {
        let raw = include_str!("testdata/pid/99001-bg-busy.json");
        let r: PidRecord = serde_json::from_str(raw).unwrap();
        assert_eq!(r.status.as_deref(), Some("busy"));
        assert!(r.waiting_for.is_none());
    }

    #[test]
    fn pid_record_parses_bg_waiting_with_verb() {
        let raw = include_str!("testdata/pid/99002-bg-waiting.json");
        let r: PidRecord = serde_json::from_str(raw).unwrap();
        assert_eq!(r.status.as_deref(), Some("waiting"));
        assert_eq!(r.waiting_for.as_deref(), Some("approve Bash"));
    }

    #[test]
    fn pid_record_rejects_malformed() {
        let raw = include_str!("testdata/pid/99006-malformed.json");
        assert!(serde_json::from_str::<PidRecord>(raw).is_err());
    }

    #[test]
    fn live_delta_serde_roundtrip() {
        let d = LiveDelta {
            session_id: "s1".into(),
            seq: 42,
            produced_at_ms: 1_776_755_000_000,
            kind: LiveDeltaKind::StatusChanged {
                status: Status::Waiting,
                waiting_for: Some("approve Bash".into()),
            },
            resync_required: false,
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: LiveDelta = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn live_delta_kind_tag_matches_ts_convention() {
        // The tag MUST stay snake_case — the TS side discriminates on
        // `kind` and will silently miss a `TaskSummaryChanged` payload
        // if the case drifts.
        let d = LiveDelta {
            session_id: "s1".into(),
            seq: 1,
            produced_at_ms: 0,
            kind: LiveDeltaKind::TaskSummaryChanged {
                summary: "running tests".into(),
            },
            resync_required: false,
        };
        let s = serde_json::to_string(&d).unwrap();
        assert!(s.contains(r#""kind":"task_summary_changed""#));
    }

    #[test]
    fn live_delta_now_ms_is_positive_and_roughly_current() {
        let now = LiveDelta::now_ms();
        // 2024-01-01 = 1_704_067_200_000 ms. Any sooner would mean the
        // clock is broken.
        assert!(now > 1_704_067_200_000, "now_ms suspiciously small: {now}");
    }
}
