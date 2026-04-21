//! Subagent resolution — pulls `agent-*.jsonl` transcripts that live
//! alongside a parent session and parses them into a structured view.
//!
//! CC supports two on-disk layouts (both observed in the wild against
//! v2.1.x):
//!
//! * **New** (current):
//!   `<config>/projects/<slug>/<session_id>/subagents/agent-<id>.jsonl`
//! * **Legacy**:
//!   `<config>/projects/<slug>/agent-<id>.jsonl` — the file is at the
//!   project root and has no direct filesystem link back to its parent
//!   session. We match those by scanning each file for its
//!   `sessionId` field and keeping only the ones that reference the
//!   target session.
//!
//! Every file is a JSONL transcript with the same shape as a main
//! session, plus `isSidechain: true` and an `agentId` field. We reuse
//! the [`parse_events`](crate::session) logic by opening each file and
//! delegating to the same reader.
//!
//! Ported and adapted from claude-devtools'
//! `SubagentLocator` + `SubagentResolver`.

use crate::session::{SessionError, SessionEvent, TokenUsage};
use crate::session_chunks::ChunkMetrics;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// One resolved subagent.
#[derive(Debug, Clone, Serialize)]
pub struct Subagent {
    /// Short id pulled out of `agent-<id>.jsonl`.
    pub id: String,
    pub file_path: PathBuf,
    pub file_size_bytes: u64,
    pub start_ts: Option<DateTime<Utc>>,
    pub end_ts: Option<DateTime<Utc>>,
    /// Rolled-up metrics across all turns inside this subagent.
    pub metrics: ChunkMetrics,
    /// `tool_use_id` of the spawning `Task` call in the parent session,
    /// if we can link it. `None` when the parent session doesn't have
    /// a matching Task call (e.g. mid-flight adopted transcripts).
    pub parent_task_id: Option<String>,
    /// The `subagent_type` string from the spawning Task call's input,
    /// if available (`Explore`, `Plan`, etc.).
    pub agent_type: Option<String>,
    /// Optional human description of what the subagent was asked to
    /// do — taken from the `description` field on the Task call input,
    /// fallback to the first non-empty user turn of the subagent.
    pub description: Option<String>,
    /// Was at least one other subagent running while this one ran?
    /// Computed from time-window overlap of sibling subagents.
    pub is_parallel: bool,
    /// The subagent's own transcript events.
    pub events: Vec<SessionEvent>,
}

/// Find and parse every subagent file attached to `session_id`.
///
/// Errors bubble up only for the parent path layout issues. A single
/// malformed subagent file counts its bad lines toward its own metric
/// totals but doesn't abort the scan — matches the main parser.
pub fn resolve_subagents(
    config_dir: &Path,
    slug: &str,
    session_id: &str,
) -> Result<Vec<Subagent>, SessionError> {
    let mut files = Vec::new();

    // New structure: projects/<slug>/<session_id>/subagents/agent-*.jsonl
    let new_dir = config_dir
        .join("projects")
        .join(slug)
        .join(session_id)
        .join("subagents");
    collect_agent_files(&new_dir, &mut files);

    // Legacy: projects/<slug>/agent-*.jsonl — narrow by inspecting the
    // sessionId inside each file.
    let slug_dir = config_dir.join("projects").join(slug);
    let mut legacy: Vec<PathBuf> = Vec::new();
    collect_agent_files(&slug_dir, &mut legacy);
    for path in legacy {
        if file_belongs_to_session(&path, session_id) {
            files.push(path);
        }
    }

    if files.is_empty() {
        return Ok(Vec::new());
    }

    let mut agents: Vec<Subagent> = Vec::with_capacity(files.len());
    for path in files {
        match parse_subagent_file(&path) {
            Ok(a) => agents.push(a),
            Err(e) => {
                // A single bad file shouldn't kill the whole list; leave
                // a breadcrumb in the metrics and move on.
                eprintln!("subagent parse error {}: {e}", path.display());
            }
        }
    }

    mark_parallel(&mut agents);
    Ok(agents)
}

/// Walk the parent session's events, find every `Task` tool call, and
/// glue it to the subagent whose `id` matches the call's recorded
/// `agentId` (pulled from the tool result's enriched payload when
/// present, or from `agent_type` inference).
///
/// We accept both `agentId` (canonical field on result payloads) and
/// `agent_id` (legacy snake_case) for forward/backward compatibility.
pub fn link_parent_tasks(
    parent_events: &[SessionEvent],
    subagents: &mut [Subagent],
) {
    if subagents.is_empty() {
        return;
    }
    // id -> index into `subagents`
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (i, a) in subagents.iter().enumerate() {
        by_id.insert(a.id.clone(), i);
    }

    // Walk tool calls to learn about subagent_type / description hints.
    for ev in parent_events {
        if let SessionEvent::AssistantToolUse {
            tool_name,
            tool_use_id,
            input_preview,
            ..
        } = ev
        {
            if tool_name != "Task" {
                continue;
            }
            let Some(input) = serde_json::from_str::<Value>(input_preview).ok() else {
                continue;
            };
            let agent_type = input
                .get("subagent_type")
                .and_then(Value::as_str)
                .map(String::from);
            let description = input
                .get("description")
                .and_then(Value::as_str)
                .map(String::from);

            // Matching strategy: the Task call input doesn't always
            // carry the agent_id. Fall back to positional pairing —
            // apply the `tool_use_id` to the first un-linked subagent.
            let mut attached = false;
            if let Some(a_id) = input.get("agent_id").and_then(Value::as_str) {
                if let Some(&idx) = by_id.get(a_id) {
                    let a = &mut subagents[idx];
                    a.parent_task_id.get_or_insert_with(|| tool_use_id.clone());
                    // Explicit link: Task call input is authoritative,
                    // override the parser's best-guess description.
                    if agent_type.is_some() {
                        a.agent_type = agent_type.clone();
                    }
                    if description.is_some() {
                        a.description = description.clone();
                    }
                    attached = true;
                }
            }
            if !attached {
                if let Some(a) = subagents
                    .iter_mut()
                    .find(|a| a.parent_task_id.is_none())
                {
                    a.parent_task_id.get_or_insert_with(|| tool_use_id.clone());
                    // Positional fallback: don't clobber whatever the
                    // subagent parser already derived from its own
                    // first user prompt.
                    if a.agent_type.is_none() {
                        a.agent_type = agent_type;
                    }
                    if a.description.is_none() {
                        a.description = description;
                    }
                }
            }
        }
    }

    // Also walk tool results — CC sometimes writes `agentId` into the
    // result blob, which is the authoritative link when available.
    for ev in parent_events {
        if let SessionEvent::UserToolResult {
            content,
            tool_use_id,
            ..
        } = ev
        {
            if let Some(agent_id) = extract_agent_id_from_result(content) {
                if let Some(&idx) = by_id.get(&agent_id) {
                    // Override any prior positional guess with the exact link.
                    subagents[idx].parent_task_id = Some(tool_use_id.clone());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// File-system helpers
// ---------------------------------------------------------------------------

fn collect_agent_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if (name.starts_with("agent-") || name.starts_with("agent_")) && name.ends_with(".jsonl") {
            out.push(entry.path());
        }
    }
}

fn file_belongs_to_session(path: &Path, session_id: &str) -> bool {
    // Cheap filter — read up to the first JSON line, look at sessionId.
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let reader = BufReader::new(file);
    for line in reader.lines().take(4) {
        let Ok(l) = line else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&l) else {
            continue;
        };
        if let Some(sid) = v.get("sessionId").and_then(Value::as_str) {
            return sid == session_id;
        }
    }
    false
}

fn extract_agent_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    // stem == "agent-<id>" or "agent_<id>"
    let id = stem.strip_prefix("agent-").or_else(|| stem.strip_prefix("agent_"))?;
    Some(id.to_string())
}

fn extract_agent_id_from_result(content: &str) -> Option<String> {
    // Tool result blobs often carry a JSON payload we can sniff; we
    // look for `"agentId":"..."` or `agent_id`. Not all results have
    // one and we tolerate that silently.
    let value = serde_json::from_str::<Value>(content).ok()?;
    value
        .get("agentId")
        .and_then(Value::as_str)
        .or_else(|| value.get("agent_id").and_then(Value::as_str))
        .map(String::from)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse one `agent-*.jsonl` file into a `Subagent`.
///
/// Reuses the tolerant line-by-line JSON reader the main session
/// parser uses, but folds in a metrics pass so the GUI can render a
/// summary without making a second trip through the events.
fn parse_subagent_file(path: &Path) -> Result<Subagent, SessionError> {
    let meta = fs::metadata(path)?;
    let id = extract_agent_id_from_filename(path).unwrap_or_default();

    let events = crate::session::parse_events_public(path)?;

    let mut start_ts: Option<DateTime<Utc>> = None;
    let mut end_ts: Option<DateTime<Utc>> = None;
    let mut metrics = ChunkMetrics::default();
    let mut first_user_prompt: Option<String> = None;

    for ev in &events {
        if let Some(ts) = event_ts(ev) {
            if start_ts.is_none_or(|s| ts < s) {
                start_ts = Some(ts);
            }
            if end_ts.is_none_or(|e| ts > e) {
                end_ts = Some(ts);
            }
        }
        match ev {
            SessionEvent::UserText { text, .. } => {
                metrics.message_count += 1;
                if first_user_prompt.is_none() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        first_user_prompt = Some(truncate(trimmed, 200));
                    }
                }
            }
            SessionEvent::AssistantText { usage, .. } => {
                metrics.message_count += 1;
                if let Some(u) = usage {
                    add_usage(&mut metrics.tokens, u);
                }
            }
            SessionEvent::AssistantThinking { .. } => {
                metrics.message_count += 1;
                metrics.thinking_count += 1;
            }
            SessionEvent::AssistantToolUse { .. } => {
                metrics.message_count += 1;
                metrics.tool_call_count += 1;
            }
            SessionEvent::UserToolResult { .. } => {
                metrics.message_count += 1;
            }
            _ => {}
        }
    }
    metrics.duration_ms = match (start_ts, end_ts) {
        (Some(a), Some(b)) => (b - a).num_milliseconds(),
        _ => 0,
    };

    Ok(Subagent {
        id,
        file_path: path.to_path_buf(),
        file_size_bytes: meta.len(),
        start_ts,
        end_ts,
        metrics,
        parent_task_id: None,
        agent_type: None,
        description: first_user_prompt,
        is_parallel: false,
        events,
    })
}

fn event_ts(ev: &SessionEvent) -> Option<DateTime<Utc>> {
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
        | SessionEvent::Other { ts, .. } => *ts,
        SessionEvent::Malformed { .. } => None,
    }
}

fn add_usage(acc: &mut TokenUsage, u: &TokenUsage) {
    acc.input += u.input;
    acc.output += u.output;
    acc.cache_creation += u.cache_creation;
    acc.cache_read += u.cache_read;
}

fn truncate(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max + 1));
    for (idx, ch) in s.chars().enumerate() {
        if idx >= max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// Overlapping time windows → `is_parallel = true` on both sides.
fn mark_parallel(agents: &mut [Subagent]) {
    let spans: Vec<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> =
        agents.iter().map(|a| (a.start_ts, a.end_ts)).collect();
    for i in 0..agents.len() {
        let (ai_s, ai_e) = spans[i];
        let (Some(ai_s), Some(ai_e)) = (ai_s, ai_e) else {
            continue;
        };
        for (j, (bj_s, bj_e)) in spans.iter().enumerate() {
            if i == j {
                continue;
            }
            let (Some(bj_s), Some(bj_e)) = (*bj_s, *bj_e) else {
                continue;
            };
            if ai_s <= bj_e && bj_s <= ai_e {
                agents[i].is_parallel = true;
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn mkdir(p: &Path) {
        fs::create_dir_all(p).unwrap();
    }

    fn write_file(path: &Path, lines: &[&str]) {
        let mut f = fs::File::create(path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    fn user_line(ts: &str, text: &str, sid: &str) -> String {
        format!(
            r#"{{"type":"user","isSidechain":true,"sessionId":"{sid}","message":{{"role":"user","content":"{text}"}},"timestamp":"{ts}"}}"#
        )
    }

    fn asst_line(ts: &str, text: &str, sid: &str, in_tokens: u64, out_tokens: u64) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":true,"sessionId":"{sid}","message":{{"role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"{text}"}}],"usage":{{"input_tokens":{in_tokens},"output_tokens":{out_tokens}}}}},"timestamp":"{ts}"}}"#
        )
    }

    #[test]
    fn empty_session_returns_no_subagents() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp.path().join("projects").join("-repo"));
        let agents = resolve_subagents(tmp.path(), "-repo", "S1").unwrap();
        assert!(agents.is_empty());
    }

    #[test]
    fn new_structure_picks_up_agent_files() {
        let tmp = TempDir::new().unwrap();
        let sub_dir = tmp
            .path()
            .join("projects")
            .join("-repo")
            .join("S1")
            .join("subagents");
        mkdir(&sub_dir);
        write_file(
            &sub_dir.join("agent-a1.jsonl"),
            &[
                &user_line("2026-04-10T10:00:00Z", "explore repo", "S1"),
                &asst_line("2026-04-10T10:00:02Z", "found 3 files", "S1", 10, 5),
            ],
        );
        let agents = resolve_subagents(tmp.path(), "-repo", "S1").unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "a1");
        assert_eq!(agents[0].metrics.message_count, 2);
        assert_eq!(agents[0].metrics.tokens.input, 10);
        assert_eq!(agents[0].metrics.tokens.output, 5);
        assert_eq!(agents[0].metrics.duration_ms, 2000);
        assert_eq!(
            agents[0].description.as_deref(),
            Some("explore repo")
        );
    }

    #[test]
    fn legacy_structure_filters_by_session_id() {
        let tmp = TempDir::new().unwrap();
        let slug_dir = tmp.path().join("projects").join("-repo");
        mkdir(&slug_dir);
        write_file(
            &slug_dir.join("agent-keep.jsonl"),
            &[&user_line("2026-04-10T10:00:00Z", "mine", "S1")],
        );
        write_file(
            &slug_dir.join("agent-drop.jsonl"),
            &[&user_line("2026-04-10T10:00:00Z", "other session", "S2")],
        );
        let agents = resolve_subagents(tmp.path(), "-repo", "S1").unwrap();
        let ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
        assert_eq!(ids, vec!["keep".to_string()]);
    }

    #[test]
    fn both_structures_coexist() {
        let tmp = TempDir::new().unwrap();
        let slug_dir = tmp.path().join("projects").join("-repo");
        let sub_dir = slug_dir.join("S1").join("subagents");
        mkdir(&slug_dir);
        mkdir(&sub_dir);
        write_file(
            &sub_dir.join("agent-new.jsonl"),
            &[&user_line("2026-04-10T10:00:00Z", "new", "S1")],
        );
        write_file(
            &slug_dir.join("agent-legacy.jsonl"),
            &[&user_line("2026-04-10T10:00:00Z", "legacy", "S1")],
        );
        let agents = resolve_subagents(tmp.path(), "-repo", "S1").unwrap();
        let mut ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["legacy".to_string(), "new".to_string()]);
    }

    #[test]
    fn parallel_flag_set_when_windows_overlap() {
        let tmp = TempDir::new().unwrap();
        let sub_dir = tmp.path().join("projects").join("-r").join("S1").join("subagents");
        mkdir(&sub_dir);
        write_file(
            &sub_dir.join("agent-a.jsonl"),
            &[
                &user_line("2026-04-10T10:00:00Z", "a start", "S1"),
                &asst_line("2026-04-10T10:00:30Z", "a end", "S1", 1, 1),
            ],
        );
        write_file(
            &sub_dir.join("agent-b.jsonl"),
            &[
                &user_line("2026-04-10T10:00:10Z", "b start", "S1"),
                &asst_line("2026-04-10T10:00:40Z", "b end", "S1", 1, 1),
            ],
        );
        let agents = resolve_subagents(tmp.path(), "-r", "S1").unwrap();
        assert_eq!(agents.len(), 2);
        assert!(agents.iter().all(|a| a.is_parallel));
    }

    #[test]
    fn non_overlapping_subagents_are_sequential() {
        let tmp = TempDir::new().unwrap();
        let sub_dir = tmp.path().join("projects").join("-r").join("S1").join("subagents");
        mkdir(&sub_dir);
        write_file(
            &sub_dir.join("agent-a.jsonl"),
            &[
                &user_line("2026-04-10T10:00:00Z", "a start", "S1"),
                &asst_line("2026-04-10T10:00:05Z", "a end", "S1", 1, 1),
            ],
        );
        write_file(
            &sub_dir.join("agent-b.jsonl"),
            &[
                &user_line("2026-04-10T10:00:10Z", "b start", "S1"),
                &asst_line("2026-04-10T10:00:15Z", "b end", "S1", 1, 1),
            ],
        );
        let agents = resolve_subagents(tmp.path(), "-r", "S1").unwrap();
        assert!(agents.iter().all(|a| !a.is_parallel));
    }

    #[test]
    fn link_parent_tasks_matches_explicit_agent_id() {
        use serde_json::json;
        let tmp = TempDir::new().unwrap();
        let sub_dir = tmp.path().join("projects").join("-r").join("S1").join("subagents");
        mkdir(&sub_dir);
        write_file(
            &sub_dir.join("agent-aaa.jsonl"),
            &[&user_line("2026-04-10T10:00:00Z", "go", "S1")],
        );
        let mut agents = resolve_subagents(tmp.path(), "-r", "S1").unwrap();
        let parent_events = vec![SessionEvent::AssistantToolUse {
            ts: None,
            uuid: None,
            model: None,
            tool_name: "Task".into(),
            tool_use_id: "toolu_parent".into(),
            input_preview: json!({
                "subagent_type": "Explore",
                "description": "Find thing",
                "agent_id": "aaa",
            })
            .to_string(),
        }];
        link_parent_tasks(&parent_events, &mut agents);
        assert_eq!(agents[0].parent_task_id.as_deref(), Some("toolu_parent"));
        assert_eq!(agents[0].agent_type.as_deref(), Some("Explore"));
        assert_eq!(agents[0].description.as_deref(), Some("Find thing"));
    }

    #[test]
    fn link_parent_tasks_falls_back_to_positional_match() {
        let tmp = TempDir::new().unwrap();
        let sub_dir = tmp.path().join("projects").join("-r").join("S1").join("subagents");
        mkdir(&sub_dir);
        write_file(
            &sub_dir.join("agent-zz.jsonl"),
            &[&user_line("2026-04-10T10:00:00Z", "go", "S1")],
        );
        let mut agents = resolve_subagents(tmp.path(), "-r", "S1").unwrap();
        let parent_events = vec![SessionEvent::AssistantToolUse {
            ts: None,
            uuid: None,
            model: None,
            tool_name: "Task".into(),
            tool_use_id: "toolu_only".into(),
            input_preview: r#"{"subagent_type":"Plan","description":"Plan it"}"#.into(),
        }];
        link_parent_tasks(&parent_events, &mut agents);
        assert_eq!(agents[0].parent_task_id.as_deref(), Some("toolu_only"));
        assert_eq!(agents[0].agent_type.as_deref(), Some("Plan"));
    }

    #[test]
    fn link_parent_tasks_uses_tool_result_agent_id_when_present() {
        let tmp = TempDir::new().unwrap();
        let sub_dir = tmp.path().join("projects").join("-r").join("S1").join("subagents");
        mkdir(&sub_dir);
        write_file(
            &sub_dir.join("agent-xyz.jsonl"),
            &[&user_line("2026-04-10T10:00:00Z", "go", "S1")],
        );
        let mut agents = resolve_subagents(tmp.path(), "-r", "S1").unwrap();
        let parent_events = vec![
            SessionEvent::AssistantToolUse {
                ts: None,
                uuid: None,
                model: None,
                tool_name: "Task".into(),
                tool_use_id: "call_outer".into(),
                input_preview: r#"{"description":"X"}"#.into(),
            },
            SessionEvent::UserToolResult {
                ts: None,
                uuid: None,
                tool_use_id: "call_outer".into(),
                content: r#"{"agentId":"xyz","status":"ok"}"#.into(),
                is_error: false,
            },
        ];
        link_parent_tasks(&parent_events, &mut agents);
        assert_eq!(agents[0].parent_task_id.as_deref(), Some("call_outer"));
    }

    #[test]
    fn agent_underscore_filename_is_also_recognized() {
        let tmp = TempDir::new().unwrap();
        let sub_dir = tmp.path().join("projects").join("-r").join("S1").join("subagents");
        mkdir(&sub_dir);
        write_file(
            &sub_dir.join("agent_legacy.jsonl"),
            &[&user_line("2026-04-10T10:00:00Z", "hi", "S1")],
        );
        let agents = resolve_subagents(tmp.path(), "-r", "S1").unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "legacy");
    }
}
