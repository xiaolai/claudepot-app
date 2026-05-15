//! Codex rollout JSONL parsing — neutral leaf module.
//!
//! Owns all Codex transcript decoding for the workspace. Both
//! `session_live` (presentation layer) and `shared_memory` (durable
//! indexing) import from here; this module imports from neither.
//! The dependency graph is:
//!
//! ```text
//!   session_live ──┐
//!                  ├──> codex_session
//!   shared_memory ─┘
//! ```
//!
//! Codex rollouts live at `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`
//! (default `~/.codex/sessions/...`). Each line is a JSON object
//! with `{timestamp, type, payload}` shape:
//!
//! * `session_meta` — once per file, first record. Carries the
//!   session id, cwd, originator (e.g. `codex_vscode`), CLI
//!   version, and instructions text.
//! * `turn_context` — per-turn metadata: cwd, approval_policy,
//!   sandbox_policy. Repeats throughout the file.
//! * `response_item` — wraps a model API item:
//!   * `{type: "message", role: "user", content: [{type: "input_text", text}]}`
//!   * `{type: "message", role: "assistant", content: [{type: "output_text", text}]}`
//!   * `{type: "function_call", name, arguments, call_id}`
//!   * `{type: "function_call_output", call_id, output}`
//! * `event_msg` — UI-event log (token counts, internal user
//!   messages). The parser treats these as informational and does
//!   NOT pull user message text from them — `response_item.message`
//!   is the canonical source. Otherwise the IDE-context user
//!   messages would double-count.
//!
//! ## Stable exchange ids
//!
//! `exchange.id = "<session_id>:<turn_index>"`. Turn index is
//! 0-based and counts non-environment user-initiated turns. Every
//! re-parse of an unchanged file produces the same ids in the same
//! order; downstream tables (`exchanges`, `tool_calls`,
//! `exchange_fts`) can rely on this for staleness checks.
//!
//! ## Line numbers
//!
//! All `line_*` fields are 1-based physical JSONL line numbers as
//! the file exists on disk at parse time. Blank lines and
//! malformed-skipped lines still advance the counter.

pub mod error;
pub mod parser;
pub mod types;

#[cfg(test)]
mod tests;

pub use error::CodexError;
pub use parser::{iter_events, parse_codex_rollout_jsonl, parse_head, EventIter};
pub use types::{
    CodexConversation, CodexEvent, CodexExchange, CodexHead, CodexToolCall,
    EnvironmentTextKind,
};
