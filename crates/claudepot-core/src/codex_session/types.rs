//! Public types emitted by the Codex rollout parser.
//!
//! Kept deliberately Rust-native (no `serde::Deserialize` derive on
//! the public types). The parser hand-decodes from `serde_json::Value`
//! so the public types are stable against Codex schema drift —
//! adding a field on the wire never breaks an existing field's
//! decoding here, and missing fields land as `None` rather than as
//! deserialization errors.

use std::path::PathBuf;

use chrono::{DateTime, Utc};

/// Top-of-file metadata for a Codex rollout. Populated from the
/// `session_meta` record (mandatory) and the first `turn_context`
/// record (best-effort) so callers get the working directory and
/// sandbox policy without rewinding the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHead {
    /// `session_meta.payload.id` — the canonical Codex session id.
    pub session_id: String,
    /// `session_meta.payload.cwd`. Codex's `cwd` is always an
    /// absolute path string; we lift to `PathBuf` here.
    pub cwd: Option<PathBuf>,
    /// `session_meta.payload.originator` — e.g. `codex_vscode`,
    /// `codex_cli`. Helps disambiguate where a rollout came from
    /// when surfaces want to group by client.
    pub originator: Option<String>,
    /// `session_meta.payload.cli_version` — e.g. `0.44.0`.
    pub cli_version: Option<String>,
    /// `session_meta.timestamp` (UTC) when the rollout was first
    /// written. Not the same as the first user message — Codex
    /// records the meta line a few ms before the seed turn.
    pub started_at: Option<DateTime<Utc>>,
    /// First non-null `turn_context.payload.approval_policy`
    /// observed in the file. Useful for display; not load-bearing
    /// for indexing.
    pub approval_policy: Option<String>,
    /// First non-null `turn_context.payload.sandbox_policy.mode`.
    /// Same caveat as `approval_policy`.
    pub sandbox_mode: Option<String>,
    /// Reserved for future Codex rollout schema versions. Codex
    /// does not currently stamp a version on every record; when it
    /// does, we read it from `session_meta.payload.schema_version`
    /// (or wherever Codex puts it) into this field.
    pub rollout_schema_version: Option<String>,
}

/// One decoded JSONL record. The variants form a minimal, stable
/// surface: anything the parser cannot classify falls into
/// `Other`, which preserves the `type` tag and physical line
/// number so callers can decide whether to treat the unknown as a
/// soft error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexEvent {
    SessionMeta {
        session_id: String,
        cwd: Option<PathBuf>,
        originator: Option<String>,
        cli_version: Option<String>,
        timestamp: Option<DateTime<Utc>>,
        line: u32,
    },
    TurnContext {
        cwd: Option<PathBuf>,
        approval_policy: Option<String>,
        sandbox_mode: Option<String>,
        timestamp: Option<DateTime<Utc>>,
        line: u32,
    },
    /// `response_item` carrying a user message.
    UserMessage {
        text: String,
        /// Codex synthesises a handful of user messages that aren't
        /// genuine user prompts: `<user_instructions>…</user_instructions>`,
        /// `<environment_context>…</environment_context>`, and the
        /// IDE-context block. These are tagged so callers can filter
        /// them out without re-parsing the text.
        kind: EnvironmentTextKind,
        timestamp: Option<DateTime<Utc>>,
        line: u32,
    },
    /// `response_item` carrying an assistant message.
    AssistantMessage {
        text: String,
        timestamp: Option<DateTime<Utc>>,
        line: u32,
    },
    /// `response_item` carrying a function call.
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
        timestamp: Option<DateTime<Utc>>,
        line: u32,
    },
    /// `response_item` carrying a function call result.
    FunctionCallOutput {
        call_id: String,
        /// Raw `payload.output` string. Codex sometimes emits a
        /// JSON-encoded blob here (e.g.
        /// `{"output":"...","metadata":{"exit_code":0,...}}`); the
        /// parser does not unwrap it because the inner schema is
        /// tool-specific.
        output: String,
        /// Best-effort. Set when the output JSON's
        /// `metadata.exit_code != 0`. False when the field is absent.
        is_error: bool,
        timestamp: Option<DateTime<Utc>>,
        line: u32,
    },
    /// Any other top-level `type` (e.g. `event_msg`,
    /// `compaction`, future variants). Preserved so a stream
    /// consumer can decide what to do.
    Other { type_tag: String, line: u32 },
}

/// Classification for synthetic user-message text. The parser
/// inspects the leading bytes of the user-message body, so a
/// message that *starts with* `<user_instructions>` is classified
/// as `Instructions` regardless of whatever follows. Reduces
/// false positives at the cost of missing exotic phrasings; this
/// is acceptable for indexing because the underlying text is still
/// captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentTextKind {
    /// Genuine user prompt (default).
    UserPrompt,
    /// `<user_instructions>…</user_instructions>` system seed.
    Instructions,
    /// `<environment_context>…</environment_context>` system seed.
    Environment,
    /// IDE / Codex-IDE context dump (starts with `# Context from`).
    /// For Codex-VSCode sessions, the actual user typed prompt
    /// lives *inside* this block (after `## My request for Codex:`).
    /// The parser keeps the whole block as one message and lets
    /// downstream consumers strip the chrome if they want — the
    /// raw text is searchable either way.
    IdeContext,
}

impl EnvironmentTextKind {
    /// True when this user-message kind should open a new
    /// exchange. `UserPrompt` and `IdeContext` both qualify:
    /// codex_cli emits `UserPrompt`, codex_vscode wraps the
    /// prompt in `IdeContext`. `Instructions` and `Environment`
    /// are synthetic seeds and never open a turn.
    pub fn is_turn_seed(self) -> bool {
        matches!(self, Self::UserPrompt | Self::IdeContext)
    }
}

/// A normalized user/assistant turn plus its tool calls.
///
/// Pairing rule: a new exchange starts at every
/// `UserMessage { kind: UserPrompt, .. }`. Everything emitted before
/// the next genuine user prompt (and after this one) belongs to
/// this exchange. Tool calls are linked by `call_id`. The
/// assistant text is the concatenation of all assistant messages
/// in the turn — Codex sometimes streams multiple final messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexExchange {
    /// `<session_id>:<turn_index>` — stable across re-parses of
    /// the same file.
    pub id: String,
    pub turn_index: u32,
    pub user_text: String,
    pub assistant_text: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub tool_calls: Vec<CodexToolCall>,
}

/// A paired `function_call` + (optional) `function_call_output`.
/// If the rollout ends before the output is recorded (the agent
/// crashed or the run is mid-flight), `output` is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub output: Option<String>,
    pub is_error: bool,
    pub timestamp: Option<DateTime<Utc>>,
    pub call_line: u32,
    pub output_line: Option<u32>,
}

/// Full-file parse result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexConversation {
    pub head: CodexHead,
    pub exchanges: Vec<CodexExchange>,
    /// Quality signals from the parse pass. The indexer uses these
    /// to decide whether to stamp the file's staleness triple
    /// (refuses to stamp when `truncated_by_io` so the next
    /// backfill retries) and to surface per-file warnings.
    pub diagnostics: ParseDiagnostics,
}

/// Per-file parse-quality signals. All fields default to zero/false
/// on a clean parse. Non-default values do NOT make the parse
/// "fail" — the caller still gets a valid `CodexConversation` with
/// whatever was decoded — but they DO change the indexer's
/// stamping behavior and surface as per-file warnings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParseDiagnostics {
    /// Number of lines that didn't decode (bad JSON, unknown
    /// shape, etc.). Each was skipped; the line counter advanced.
    pub malformed_lines: u32,
    /// Number of lines exceeding `MAX_LINE_BYTES` that were drained
    /// without allocation. Defends against adversarial input that
    /// would otherwise OOM the indexer.
    pub oversize_lines: u32,
    /// True if the parser stopped mid-stream due to an I/O error
    /// (not EOF). The indexer must refuse to stamp the staleness
    /// triple in this case — otherwise a transient read failure
    /// leaves a partial transcript cached as if complete, and the
    /// (size, mtime, inode) tuple makes it look "unchanged" on
    /// subsequent backfills.
    pub truncated_by_io: bool,
}
