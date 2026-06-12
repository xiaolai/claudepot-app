//! Visible-context token attribution.
//!
//! For every turn in the session, bucket the token cost into one of
//! six categories so the UI can tell the user *what's eating the
//! context window*:
//!
//! | Category          | Sources                                         |
//! |-------------------|-------------------------------------------------|
//! | `ClaudeMd`        | `Read` calls / `@` mentions whose path ends in `CLAUDE.md`. |
//! | `MentionedFile`   | Other `@path` mentions in user text.            |
//! | `ToolOutput`      | Any tool result content (excluding team tools). |
//! | `ThinkingText`    | `AssistantThinking` blocks + assistant text out. |
//! | `TeamCoordination`| `TaskCreate`/`TaskUpdate`/`TaskList`/`TaskGet`/`SendMessage`/`TeamCreate`/`TeamDelete`. |
//! | `UserMessage`     | User-typed text itself.                         |
//!
//! This is a pure derivation from `SessionEvent`s; no filesystem access.
//!
//! Adapted from claude-devtools' `contextTracker` with the
//! category-level math preserved and the UI-specific injection-list
//! plumbing removed. We compute per-category totals and a per-turn
//! breakdown; the UI is free to render categories as bars, stacks, or
//! rankings.

use crate::session::SessionEvent;
use crate::session_chunks::should_charge_usage;
use crate::session_phases::{compute_phases, ContextPhase};
use crate::session_tool_link::link_tools;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const TEAM_COORDINATION_TOOLS: &[&str] = &[
    "SendMessage",
    "TeamCreate",
    "TeamDelete",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    // `Task` spawns a subagent — the call + result crosses the
    // context boundary as "team-coordination overhead", not a regular
    // tool call whose output is rendered to the user.
    "Task",
];

/// Six buckets used for token attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextCategory {
    ClaudeMd,
    MentionedFile,
    ToolOutput,
    ThinkingText,
    TeamCoordination,
    UserMessage,
}

/// Per-category token totals.
#[derive(Debug, Default, Clone, Serialize)]
pub struct TokensByCategory {
    pub claude_md: u64,
    pub mentioned_file: u64,
    pub tool_output: u64,
    pub thinking_text: u64,
    pub team_coordination: u64,
    pub user_message: u64,
}

impl TokensByCategory {
    pub fn total(&self) -> u64 {
        self.claude_md
            + self.mentioned_file
            + self.tool_output
            + self.thinking_text
            + self.team_coordination
            + self.user_message
    }

    fn add(&mut self, category: ContextCategory, tokens: u64) {
        match category {
            ContextCategory::ClaudeMd => self.claude_md += tokens,
            ContextCategory::MentionedFile => self.mentioned_file += tokens,
            ContextCategory::ToolOutput => self.tool_output += tokens,
            ContextCategory::ThinkingText => self.thinking_text += tokens,
            ContextCategory::TeamCoordination => self.team_coordination += tokens,
            ContextCategory::UserMessage => self.user_message += tokens,
        }
    }
}

/// One per-turn attribution row — a turn here is defined loosely as
/// "the run of events belonging to a single user→assistant exchange",
/// but we express it in event indices for simplicity.
#[derive(Debug, Clone, Serialize)]
pub struct ContextInjection {
    pub event_index: usize,
    pub category: ContextCategory,
    /// Short human tag — filename, tool name, or `"user"`.
    pub label: String,
    pub tokens: u64,
    /// When the injection entered the context. Sourced from the event
    /// timestamp when available.
    pub ts: Option<chrono::DateTime<chrono::Utc>>,
    /// Phase number (from `session_phases`) — UI can filter by phase.
    pub phase: usize,
}

/// Aggregate view returned by [`attribute_context`].
#[derive(Debug, Clone, Serialize)]
pub struct ContextStats {
    pub totals: TokensByCategory,
    pub injections: Vec<ContextInjection>,
    pub phases: Vec<ContextPhase>,
    /// Total tokens reported in usage headers on assistant turns —
    /// used as a sanity check / second source when renderers want to
    /// show "visible X of Y total".
    pub reported_total_tokens: u64,
}

/// Compute per-category token attribution over the entire session.
///
/// Token estimation heuristic: `len(str) / 4` rounded up, matching the
/// cheap char-count used by claude-devtools for anything the assistant
/// `usage` field doesn't cover directly. When an event carries an
/// actual `usage` block we prefer it.
pub fn attribute_context(events: &[SessionEvent]) -> ContextStats {
    let phase_info = compute_phases(events);
    let phase_of = build_phase_lookup(events, &phase_info.phases);

    let linked = link_tools(events);
    let tool_name_by_result_idx: HashMap<usize, String> = linked
        .iter()
        .filter_map(|lt| lt.result_index.map(|i| (i, lt.tool_name.clone())))
        .collect();
    let team_tool_call_indices: HashSet<usize> = linked
        .iter()
        .filter(|lt| TEAM_COORDINATION_TOOLS.contains(&lt.tool_name.as_str()))
        .map(|lt| lt.call_index)
        .collect();
    // For tool results: inherit the "team" designation from their call.
    let team_tool_result_indices: HashSet<usize> = linked
        .iter()
        .filter(|lt| TEAM_COORDINATION_TOOLS.contains(&lt.tool_name.as_str()))
        .filter_map(|lt| lt.result_index)
        .collect();

    let mut totals = TokensByCategory::default();
    let mut injections: Vec<ContextInjection> = Vec::new();
    let mut reported_total_tokens: u64 = 0;
    // See session_chunks::should_charge_usage — assistant turns fan out
    // into multiple events that share the same `usage`. Charge once per
    // uuid so the reported-total number is honest.
    let mut usage_counted: HashSet<String> = HashSet::new();

    for (idx, ev) in events.iter().enumerate() {
        let phase = phase_of.get(&idx).copied().unwrap_or(0);
        let ts = event_ts(ev);
        match ev {
            SessionEvent::UserText { text, .. } => {
                attribute_user_text(idx, text, ts, phase, &mut totals, &mut injections);
            }
            SessionEvent::UserToolResult { content, .. } => {
                let tokens = estimate_tokens(content);
                let (category, label) = if team_tool_result_indices.contains(&idx) {
                    let name = tool_name_by_result_idx
                        .get(&idx)
                        .cloned()
                        .unwrap_or_else(|| "team".into());
                    (ContextCategory::TeamCoordination, name)
                } else {
                    let label = tool_name_by_result_idx
                        .get(&idx)
                        .cloned()
                        .unwrap_or_else(|| "tool".into());
                    (ContextCategory::ToolOutput, label)
                };
                totals.add(category, tokens);
                injections.push(ContextInjection {
                    event_index: idx,
                    category,
                    label,
                    tokens,
                    ts,
                    phase,
                });
            }
            SessionEvent::AssistantText {
                text, usage, uuid, ..
            } => {
                // Per-message usage charges only once per source UUID —
                // same rule as reported_total_tokens and chunks. When a
                // turn fans out into multiple text fragments we charge
                // the aggregate `usage.output` once, then estimate the
                // remaining fragments from their length only. Missing
                // usage always falls back to estimate_tokens.
                let tokens = match usage {
                    Some(u) => {
                        if should_charge_usage(uuid, &mut usage_counted) {
                            reported_total_tokens += u.total();
                            u.output.max(estimate_tokens(text))
                        } else {
                            estimate_tokens(text)
                        }
                    }
                    None => estimate_tokens(text),
                };
                totals.add(ContextCategory::ThinkingText, tokens);
                injections.push(ContextInjection {
                    event_index: idx,
                    category: ContextCategory::ThinkingText,
                    label: "assistant".into(),
                    tokens,
                    ts,
                    phase,
                });
            }
            SessionEvent::AssistantThinking { text, .. } => {
                let tokens = estimate_tokens(text);
                totals.add(ContextCategory::ThinkingText, tokens);
                injections.push(ContextInjection {
                    event_index: idx,
                    category: ContextCategory::ThinkingText,
                    label: "thinking".into(),
                    tokens,
                    ts,
                    phase,
                });
            }
            SessionEvent::AssistantToolUse {
                tool_name,
                input_preview,
                ..
            } => {
                let tokens = estimate_tokens(input_preview);
                let category = if team_tool_call_indices.contains(&idx) {
                    ContextCategory::TeamCoordination
                } else {
                    ContextCategory::ToolOutput
                };
                // `Read`/`@` mentions of CLAUDE.md get re-bucketed to
                // the CLAUDE.md category when we can detect them.
                let effective_category =
                    if tool_name == "Read" && input_preview_mentions_claude_md(input_preview) {
                        ContextCategory::ClaudeMd
                    } else {
                        category
                    };
                totals.add(effective_category, tokens);
                injections.push(ContextInjection {
                    event_index: idx,
                    category: effective_category,
                    label: tool_name.clone(),
                    tokens,
                    ts,
                    phase,
                });
            }
            _ => {}
        }
    }

    ContextStats {
        totals,
        injections,
        phases: phase_info.phases,
        reported_total_tokens,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_phase_lookup(events: &[SessionEvent], phases: &[ContextPhase]) -> HashMap<usize, usize> {
    let mut m = HashMap::with_capacity(events.len());
    for p in phases {
        for i in p.start_index..p.end_index {
            m.insert(i, p.phase_number);
        }
    }
    m
}

fn attribute_user_text(
    idx: usize,
    text: &str,
    ts: Option<chrono::DateTime<chrono::Utc>>,
    phase: usize,
    totals: &mut TokensByCategory,
    injections: &mut Vec<ContextInjection>,
) {
    let mentions = extract_at_mentions(text);
    let mut remaining = text.to_string();
    for m in &mentions {
        remaining = remaining.replace(&format!("@{m}"), "");
    }
    let user_tokens = estimate_tokens(remaining.trim());
    if user_tokens > 0 {
        totals.add(ContextCategory::UserMessage, user_tokens);
        injections.push(ContextInjection {
            event_index: idx,
            category: ContextCategory::UserMessage,
            label: "user".into(),
            tokens: user_tokens,
            ts,
            phase,
        });
    }
    for m in mentions {
        let category = if path_is_claude_md(&m) {
            ContextCategory::ClaudeMd
        } else {
            ContextCategory::MentionedFile
        };
        // Estimate mention file size as ~500 tokens heuristically — we
        // don't open the file (core is filesystem-aware but we keep
        // this path pure for testability). UI can enrich later.
        let tokens: u64 = 500;
        totals.add(category, tokens);
        injections.push(ContextInjection {
            event_index: idx,
            category,
            label: m,
            tokens,
            ts,
            phase,
        });
    }
}

fn extract_at_mentions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' && (i == 0 || is_mention_separator(bytes[i - 1])) {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && is_path_char(bytes[end]) {
                end += 1;
            }
            if end > start {
                if let Ok(s) = std::str::from_utf8(&bytes[start..end]) {
                    out.push(s.to_string());
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn is_mention_separator(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\n' | b'\t' | b'(' | b'[' | b'{' | b',' | b';' | b'"' | b'\''
    )
}

fn is_path_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.' | b'~')
}

fn path_is_claude_md(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with("claude.md") || lower.ends_with("claude_md") || lower.ends_with("/claude.md")
}

fn input_preview_mentions_claude_md(preview: &str) -> bool {
    let lower = preview.to_ascii_lowercase();
    lower.contains("claude.md")
}

fn estimate_tokens(s: &str) -> u64 {
    let len = s.len() as u64;
    if len == 0 {
        0
    } else {
        len.div_ceil(4)
    }
}

fn event_ts(ev: &SessionEvent) -> Option<chrono::DateTime<chrono::Utc>> {
    match ev {
        SessionEvent::UserText { ts, .. }
        | SessionEvent::UserToolResult { ts, .. }
        | SessionEvent::AssistantText { ts, .. }
        | SessionEvent::AssistantToolUse { ts, .. }
        | SessionEvent::AssistantThinking { ts, .. }
        | SessionEvent::Summary { ts, .. }
        | SessionEvent::System { ts, .. }
        | SessionEvent::Attachment { ts, .. }
        | SessionEvent::FileHistorySnapshot { ts, .. }
        | SessionEvent::TaskSummary { ts, .. }
        | SessionEvent::Other { ts, .. } => *ts,
        SessionEvent::Malformed { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::TokenUsage;

    fn user(text: &str) -> SessionEvent {
        SessionEvent::UserText {
            ts: None,
            uuid: None,
            text: text.into(),
        }
    }

    fn assistant(text: &str, output_tokens: u64) -> SessionEvent {
        SessionEvent::AssistantText {
            ts: None,
            uuid: None,
            model: None,
            text: text.into(),
            usage: Some(TokenUsage {
                input: 10,
                output: output_tokens,
                ..TokenUsage::default()
            }),
            stop_reason: None,
        }
    }

    fn assistant_with_uuid(uuid: &str, text: &str, output_tokens: u64) -> SessionEvent {
        SessionEvent::AssistantText {
            ts: None,
            uuid: Some(uuid.into()),
            model: None,
            text: text.into(),
            usage: Some(TokenUsage {
                input: 10,
                output: output_tokens,
                ..TokenUsage::default()
            }),
            stop_reason: None,
        }
    }

    fn thinking(text: &str) -> SessionEvent {
        SessionEvent::AssistantThinking {
            ts: None,
            uuid: None,
            text: text.into(),
        }
    }

    fn tool_use(id: &str, name: &str, preview: &str) -> SessionEvent {
        SessionEvent::AssistantToolUse {
            ts: None,
            uuid: None,
            model: None,
            tool_name: name.into(),
            tool_use_id: id.into(),
            input_preview: preview.into(),
            input_full: preview.into(),
        }
    }

    fn tool_result(id: &str, content: &str) -> SessionEvent {
        SessionEvent::UserToolResult {
            ts: None,
            uuid: None,
            tool_use_id: id.into(),
            content: content.into(),
            is_error: false,
        }
    }

    #[test]
    fn empty_session_has_zero_totals() {
        let stats = attribute_context(&[]);
        assert_eq!(stats.totals.total(), 0);
        assert!(stats.injections.is_empty());
    }

    #[test]
    fn user_text_goes_to_user_bucket() {
        let events = vec![user("please investigate the deadlock")];
        let stats = attribute_context(&events);
        assert!(stats.totals.user_message > 0);
        assert_eq!(stats.totals.claude_md, 0);
    }

    #[test]
    fn at_mention_bucketed_as_file() {
        let events = vec![user("look at @src/lib.rs and @README.md please")];
        let stats = attribute_context(&events);
        assert!(stats.totals.mentioned_file > 0);
        // Two @ files, 500 each = 1000.
        assert_eq!(stats.totals.mentioned_file, 1000);
    }

    #[test]
    fn claude_md_mention_bucketed_as_claude_md() {
        let events = vec![user("update @CLAUDE.md and @src/main.rs")];
        let stats = attribute_context(&events);
        assert_eq!(stats.totals.claude_md, 500);
        assert_eq!(stats.totals.mentioned_file, 500);
    }

    #[test]
    fn read_tool_targeting_claude_md_goes_to_claude_md_bucket() {
        let events = vec![tool_use(
            "t1",
            "Read",
            r#"{"file_path":"/repo/CLAUDE.md","limit":200}"#,
        )];
        let stats = attribute_context(&events);
        assert!(stats.totals.claude_md > 0);
        assert_eq!(stats.totals.tool_output, 0);
    }

    #[test]
    fn tool_results_land_in_tool_output() {
        let events = vec![
            tool_use("t1", "Bash", r#"{"cmd":"ls"}"#),
            tool_result("t1", "file1.txt\nfile2.txt"),
        ];
        let stats = attribute_context(&events);
        assert!(stats.totals.tool_output > 0);
    }

    #[test]
    fn task_tool_is_team_coordination() {
        let events = vec![
            tool_use(
                "t1",
                "Task",
                r#"{"subagent_type":"Explore","description":"find thing"}"#,
            ),
            tool_result("t1", "done"),
        ];
        let stats = attribute_context(&events);
        assert!(stats.totals.team_coordination > 0);
        assert_eq!(stats.totals.tool_output, 0);
    }

    #[test]
    fn team_tools_land_in_team_coordination() {
        let events = vec![
            tool_use("t1", "SendMessage", r#"{"to":"teammate"}"#),
            tool_result("t1", "delivered"),
        ];
        let stats = attribute_context(&events);
        assert!(stats.totals.team_coordination > 0);
        assert_eq!(stats.totals.tool_output, 0);
    }

    #[test]
    fn thinking_and_assistant_text_both_count_as_thinking_text() {
        let events = vec![
            thinking("let me think about this carefully"),
            assistant("here's the answer", 20),
        ];
        let stats = attribute_context(&events);
        assert!(stats.totals.thinking_text > 0);
    }

    #[test]
    fn reported_total_tokens_sums_usage_headers() {
        let events = vec![assistant("one", 100), assistant("two", 50)];
        let stats = attribute_context(&events);
        // Each assistant turn adds 10 input + output + 0 cache = 110 and 60.
        assert_eq!(stats.reported_total_tokens, 10 + 100 + 10 + 50);
    }

    #[test]
    fn multi_fragment_assistant_counts_reported_usage_once() {
        // Same uuid across two fragments — reported_total_tokens must
        // count usage exactly once.
        let events = vec![
            assistant_with_uuid("a1", "first", 100),
            assistant_with_uuid("a1", "second", 100),
        ];
        let stats = attribute_context(&events);
        // 10 input + 100 output = 110, exactly once.
        assert_eq!(stats.reported_total_tokens, 110);
    }

    #[test]
    fn multi_fragment_assistant_thinking_text_not_doubled() {
        // Same uuid across three text fragments of a single turn: the
        // first fragment is charged the full `usage.output`, the
        // remaining fragments only pay their text-length estimate.
        // The test ensures we don't 3x-count the `usage.output` value.
        let events = vec![
            assistant_with_uuid("a1", "aaa", 1_000),
            assistant_with_uuid("a1", "bbb", 1_000),
            assistant_with_uuid("a1", "ccc", 1_000),
        ];
        let stats = attribute_context(&events);
        // First fragment: max(1000 output, ceil("aaa".len/4)) = 1000
        // Second + third: estimate_tokens("bbb") = ceil(3/4) = 1 each.
        assert_eq!(stats.totals.thinking_text, 1000 + 1 + 1);
    }

    #[test]
    fn injections_are_stamped_with_phase_number() {
        let events = vec![
            user("round1"),
            SessionEvent::Summary {
                ts: None,
                uuid: None,
                text: "compacted".into(),
            },
            user("round2"),
        ];
        let stats = attribute_context(&events);
        assert_eq!(stats.injections.len(), 2);
        assert_eq!(stats.injections[0].phase, 0);
        assert_eq!(stats.injections[1].phase, 1);
    }

    #[test]
    fn at_mention_scanner_ignores_email_like_tokens() {
        // "me@example.com" should NOT be treated as a file mention
        // because the '@' is preceded by an alphanumeric char.
        let events = vec![user("email me@example.com about @src/lib.rs")];
        let stats = attribute_context(&events);
        // Only one mention: @src/lib.rs → 500 tokens.
        assert_eq!(stats.totals.mentioned_file, 500);
    }

    #[test]
    fn long_user_text_accumulates_more_user_tokens() {
        let short = attribute_context(&[user("hi")]);
        let long = attribute_context(&[user(&"x".repeat(400))]);
        assert!(long.totals.user_message > short.totals.user_message);
        assert_eq!(long.totals.user_message, 100);
    }
}
