//! Pure extraction: a single parsed JSONL `Value` → zero or more
//! `UsageEvent`s.
//!
//! This module is the success-side counterpart to
//! `activity::classifier`, which is failure-only. Both share the
//! same JSONL grammar; both are pure (no I/O) so they can be tested
//! with literal fixtures.
//!
//! Extractor responsibilities:
//!
//! 1. Skill invocation → `ArtifactKind::Skill`
//!    Source: `attachment.invoked_skills.skills[].path`
//!    Key:    the `path` itself (already canonical: `plugin:<id>:<name>`,
//!            `userSettings:<name>`, `projectSettings:<name>`)
//! 2. Hook fire → `ArtifactKind::Hook`
//!    Sources:
//!      - `attachment.hook_success`            → outcome=Ok
//!      - `attachment.hook_non_blocking_error` → outcome=Error
//!      - `attachment.hook_blocking_error`     → outcome=Error
//!      - `attachment.hook_cancelled`          → outcome=Cancelled
//!      - `attachment.hook_error_during_execution` → outcome=Error
//!      - `attachment.hook_stopped_continuation`   → outcome=Error
//!    Key: the `command` field (the actual exec'd shell). Stable per
//!    installation; renames with the hook config.
//! 3. Subagent dispatch → `ArtifactKind::Agent`
//!    Source: `assistant.message.content[].tool_use` where `name == "Agent"`
//!    Key: `input.subagent_type` (e.g. `Explore`, `loc-guardian:counter`)
//!    Outcome: defaults to Ok at extraction. The matching `tool_result`
//!    (later in the same session) flips to Error when `is_error: true`.
//!    `extract_assistant_with_ids` returns each event paired with its
//!    `tool_use.id`; `session::scan_session` registers those ids during
//!    the same streaming pass and flips outcomes when the matching
//!    `tool_result` arrives.
//! 4. Slash command → `ArtifactKind::Command`
//!    Source: user message content carrying `<command-name>/foo</command-name>`
//!    Key: the command name including the leading slash (e.g. `/foo`,
//!    `/codex-toolkit:audit`).
//!
//! Plugin attribution: `parse_plugin_id` derives `plugin_id` from the
//! artifact key when present (skills, agents, commands all use the
//! `plugin:<id>:<rest>` or `<plugin-id>:<rest>` convention). Hook
//! commands carry `${CLAUDE_PLUGIN_ROOT}` or
//! `~/.claude/plugins/cache/<owner>/<id>/...`; we extract the `<id>`
//! when the cache pattern matches.

use crate::artifact_usage::extract_helpers::{
    extract_slash_commands, hook_artifact_key, parse_colon_plugin_id, parse_command_plugin_id,
    parse_hook_plugin_id, parse_skill_plugin_id,
};
use crate::artifact_usage::model::{ArtifactKind, Outcome, UsageEvent};
use chrono::DateTime;
use serde_json::Value;

/// Parse `timestamp` (RFC3339) into ms-since-epoch. Returns `None` on
/// missing or malformed timestamps so the caller can drop the line —
/// usage rows without a timestamp are useless for rollups.
pub fn parse_ts_ms(v: &Value) -> Option<i64> {
    let s = v.get("timestamp").and_then(Value::as_str)?;
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// Extract all usage events from a single parsed JSONL line.
///
/// Returns an empty Vec for lines that aren't usage-relevant (most
/// lines). Allocates only when a real event is found.
pub fn extract_from_line(v: &Value, session_id: &str) -> Vec<UsageEvent> {
    let Some(ts_ms) = parse_ts_ms(v) else {
        return Vec::new();
    };
    let event_type = v.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "attachment" => extract_attachment(v, ts_ms, session_id),
        "assistant" => extract_assistant(v, ts_ms, session_id),
        "user" => extract_user(v, ts_ms, session_id),
        _ => Vec::new(),
    }
}

// ---------- attachment dispatch -----------------------------------------

fn extract_attachment(v: &Value, ts_ms: i64, session_id: &str) -> Vec<UsageEvent> {
    let Some(att) = v.get("attachment") else {
        return Vec::new();
    };
    let att_type = att.get("type").and_then(Value::as_str).unwrap_or("");
    match att_type {
        "invoked_skills" => extract_invoked_skills(att, ts_ms, session_id),
        "hook_success" => extract_hook(att, ts_ms, session_id, Outcome::Ok),
        "hook_non_blocking_error"
        | "hook_blocking_error"
        | "hook_error_during_execution"
        | "hook_stopped_continuation" => extract_hook(att, ts_ms, session_id, Outcome::Error),
        "hook_cancelled" => extract_hook(att, ts_ms, session_id, Outcome::Cancelled),
        _ => Vec::new(),
    }
}

fn extract_invoked_skills(att: &Value, ts_ms: i64, session_id: &str) -> Vec<UsageEvent> {
    let Some(skills) = att.get("skills").and_then(Value::as_array) else {
        return Vec::new();
    };
    skills
        .iter()
        .filter_map(|s| {
            let path = s.get("path").and_then(Value::as_str)?;
            let plugin_id = parse_skill_plugin_id(path);
            Some(UsageEvent {
                ts_ms,
                session_id: session_id.to_string(),
                kind: ArtifactKind::Skill,
                artifact_key: path.to_string(),
                plugin_id,
                outcome: Outcome::Ok,
                duration_ms: None,
                extra_json: None,
            })
        })
        .collect()
}

fn extract_hook(att: &Value, ts_ms: i64, session_id: &str, outcome: Outcome) -> Vec<UsageEvent> {
    let command = att
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string);
    let hook_name = att
        .get("hookName")
        .and_then(Value::as_str)
        .map(str::to_string);
    let artifact_key = match hook_artifact_key(hook_name.as_deref(), command.as_deref()) {
        Some(k) => k,
        None => return Vec::new(),
    };
    let duration_ms = att.get("durationMs").and_then(Value::as_u64);
    let plugin_id = command.as_deref().and_then(parse_hook_plugin_id);
    let extra_json = build_hook_extra(hook_name.as_deref(), command.as_deref());
    vec![UsageEvent {
        ts_ms,
        session_id: session_id.to_string(),
        kind: ArtifactKind::Hook,
        artifact_key,
        plugin_id,
        outcome,
        duration_ms,
        extra_json,
    }]
}

fn build_hook_extra(hook_name: Option<&str>, command: Option<&str>) -> Option<String> {
    if hook_name.is_none() && command.is_none() {
        return None;
    }
    let mut obj = serde_json::Map::new();
    if let Some(h) = hook_name {
        obj.insert("hookName".into(), Value::String(h.to_string()));
    }
    // Only echo `command` separately when artifact_key fell back to
    // hook_name (else we'd duplicate it in the row).
    if let (Some(c), None) = (command, hook_name) {
        obj.insert("command".into(), Value::String(c.to_string()));
    }
    serde_json::to_string(&Value::Object(obj)).ok()
}

// ---------- assistant: Agent tool_use -----------------------------------

fn extract_assistant(v: &Value, ts_ms: i64, session_id: &str) -> Vec<UsageEvent> {
    extract_assistant_with_ids(v, ts_ms, session_id)
        .into_iter()
        .map(|(ev, _id)| ev)
        .collect()
}

/// Same as `extract_assistant` but also returns the originating
/// `tool_use.id` so the caller can pair agents to their later
/// `tool_result` outcomes without an error-prone positional match.
///
/// Returns `(event, Some(id))` when the tool_use carried an `id`,
/// `(event, None)` for malformed blocks. The second arm of the tuple
/// is what `session::collect_agent_ids` consumes.
pub fn extract_assistant_with_ids(
    v: &Value,
    ts_ms: i64,
    session_id: &str,
) -> Vec<(UsageEvent, Option<String>)> {
    let Some(content) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    content
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                return None;
            }
            if block.get("name").and_then(Value::as_str) != Some("Agent") {
                return None;
            }
            let input = block.get("input")?;
            let subagent = input
                .get("subagent_type")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())?;
            let plugin_id = parse_colon_plugin_id(subagent);
            let extra_json = input
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .and_then(|d| {
                    let obj = serde_json::json!({ "description": d });
                    serde_json::to_string(&obj).ok()
                });
            let tool_use_id = block.get("id").and_then(Value::as_str).map(str::to_string);
            Some((
                UsageEvent {
                    ts_ms,
                    session_id: session_id.to_string(),
                    kind: ArtifactKind::Agent,
                    artifact_key: subagent.to_string(),
                    plugin_id,
                    outcome: Outcome::Ok,
                    duration_ms: None,
                    extra_json,
                },
                tool_use_id,
            ))
        })
        .collect()
}

// ---------- user: slash commands ---------------------------------------

fn extract_user(v: &Value, ts_ms: i64, session_id: &str) -> Vec<UsageEvent> {
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return Vec::new();
    };
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return Vec::new(),
    };
    extract_slash_commands(&text)
        .into_iter()
        .map(|cmd| {
            let plugin_id = parse_command_plugin_id(&cmd);
            UsageEvent {
                ts_ms,
                session_id: session_id.to_string(),
                kind: ArtifactKind::Command,
                artifact_key: cmd,
                plugin_id,
                outcome: Outcome::Ok,
                duration_ms: None,
                extra_json: None,
            }
        })
        .collect()
}

// Note: an earlier `link_agent_outcomes` two-pass linker lived here;
// it was superseded by the streaming pair-aware approach in
// `session::scan_session` (which uses `extract_assistant_with_ids`)
// and removed to drop the second JSONL pass + the in-memory line cache
// it required.
//
// Plugin attribution + slash-command parsing helpers used to live in
// this file too; they're now in `extract_helpers.rs` so this file
// stays under the loc-guardian limit.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    // ----------- skill -------------------------------------------------

    #[test]
    fn invoked_skills_one_skill_yields_one_event_with_plugin_id() {
        let line = parse(
            r#"{
              "type":"attachment","timestamp":"2026-04-21T11:48:43.002Z",
              "attachment":{"type":"invoked_skills","skills":[
                {"name":"codex-toolkit:audit-fix","path":"plugin:codex-toolkit:audit-fix"}
              ]}
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.kind, ArtifactKind::Skill);
        assert_eq!(ev.artifact_key, "plugin:codex-toolkit:audit-fix");
        assert_eq!(ev.plugin_id.as_deref(), Some("codex-toolkit"));
        assert_eq!(ev.outcome, Outcome::Ok);
        assert!(ev.duration_ms.is_none());
    }

    #[test]
    fn invoked_skills_user_settings_has_no_plugin_id() {
        let line = parse(
            r#"{
              "type":"attachment","timestamp":"2026-04-21T11:48:43.002Z",
              "attachment":{"type":"invoked_skills","skills":[
                {"name":"hands-off","path":"userSettings:hands-off"}
              ]}
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].plugin_id, None);
        assert_eq!(events[0].artifact_key, "userSettings:hands-off");
    }

    #[test]
    fn invoked_skills_multi_yields_multiple_events_in_order() {
        let line = parse(
            r#"{
              "type":"attachment","timestamp":"2026-04-21T11:48:43.002Z",
              "attachment":{"type":"invoked_skills","skills":[
                {"name":"a","path":"plugin:p:a"},
                {"name":"b","path":"plugin:p:b"},
                {"name":"c","path":"userSettings:c"}
              ]}
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].artifact_key, "plugin:p:a");
        assert_eq!(events[1].artifact_key, "plugin:p:b");
        assert_eq!(events[2].artifact_key, "userSettings:c");
        assert_eq!(events[2].plugin_id, None);
    }

    // ----------- hook --------------------------------------------------

    #[test]
    fn hook_success_yields_ok_event_with_duration_and_plugin() {
        let line = parse(
            r#"{
              "type":"attachment","timestamp":"2026-04-24T10:07:27.898Z",
              "attachment":{
                "type":"hook_success",
                "hookName":"PreToolUse:Bash",
                "hookEvent":"PreToolUse",
                "command":"node /Users/me/.claude/plugins/cache/xiaolai/tdd-guardian/0.1.0/scripts/guard.js",
                "durationMs":45,
                "exitCode":0
              }
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.kind, ArtifactKind::Hook);
        // Key is `<hookName>|<command>` so two hooks sharing a command
        // but firing on different events are distinct artifacts.
        assert!(ev.artifact_key.starts_with("PreToolUse:Bash|node "));
        assert_eq!(ev.outcome, Outcome::Ok);
        assert_eq!(ev.duration_ms, Some(45));
        assert_eq!(ev.plugin_id.as_deref(), Some("tdd-guardian"));
        // hookName lands in extra_json too for renderers that want it
        // surfaced separately from the artifact key.
        let extra = ev.extra_json.as_deref().unwrap();
        assert!(extra.contains("PreToolUse:Bash"));
    }

    #[test]
    fn two_hooks_sharing_a_command_have_distinct_keys() {
        // Regression for the audit Medium finding: previously the
        // artifact key was the command alone, so two hooks declared on
        // different events but pointing at the same shell command got
        // merged into one row.
        let pre = parse(
            r#"{"type":"attachment","timestamp":"2026-04-24T10:00:00Z",
              "attachment":{"type":"hook_success","hookName":"PreToolUse:Bash","command":"true","durationMs":1,"exitCode":0}}"#,
        );
        let post = parse(
            r#"{"type":"attachment","timestamp":"2026-04-24T10:00:01Z",
              "attachment":{"type":"hook_success","hookName":"PostToolUse:Edit","command":"true","durationMs":1,"exitCode":0}}"#,
        );
        let pre_events = extract_from_line(&pre, "S1");
        let post_events = extract_from_line(&post, "S1");
        assert_ne!(
            pre_events[0].artifact_key, post_events[0].artifact_key,
            "hooks on different events must have distinct keys even when the command is identical"
        );
    }

    // hook_artifact_key + the plugin-id parsers are unit-tested in
    // `extract_helpers::tests`; the integration coverage above
    // exercises the same code paths through extract_from_line.

    #[test]
    fn hook_blocking_error_yields_error_event() {
        let line = parse(
            r#"{
              "type":"attachment","timestamp":"2026-04-24T10:07:27.898Z",
              "attachment":{
                "type":"hook_blocking_error",
                "hookName":"PostToolUse:Edit",
                "command":"node /tmp/h.js",
                "durationMs":120
              }
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, Outcome::Error);
        assert_eq!(events[0].duration_ms, Some(120));
    }

    #[test]
    fn hook_cancelled_yields_cancelled_event() {
        let line = parse(
            r#"{
              "type":"attachment","timestamp":"2026-04-24T10:07:27.898Z",
              "attachment":{
                "type":"hook_cancelled",
                "hookName":"Stop",
                "command":"true"
              }
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, Outcome::Cancelled);
    }

    #[test]
    fn hook_with_env_templated_command_has_no_plugin_id() {
        let line = parse(
            r#"{
              "type":"attachment","timestamp":"2026-04-24T10:07:27.898Z",
              "attachment":{
                "type":"hook_success",
                "hookName":"PreToolUse:Bash",
                "command":"node ${CLAUDE_PLUGIN_ROOT}/scripts/guard.js",
                "durationMs":45,
                "exitCode":0
              }
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events[0].plugin_id, None);
    }

    // ----------- agent -------------------------------------------------

    #[test]
    fn agent_tool_use_with_plugin_subagent_yields_event_with_plugin_id() {
        let line = parse(
            r#"{
              "type":"assistant","timestamp":"2026-04-24T10:00:00Z",
              "message":{"role":"assistant","content":[
                {"type":"tool_use","id":"toolu_X","name":"Agent",
                 "input":{"subagent_type":"loc-guardian:counter","description":"Count LOC"}}
              ]}
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.kind, ArtifactKind::Agent);
        assert_eq!(ev.artifact_key, "loc-guardian:counter");
        assert_eq!(ev.plugin_id.as_deref(), Some("loc-guardian"));
        assert_eq!(ev.outcome, Outcome::Ok);
        assert!(ev.extra_json.as_deref().unwrap().contains("Count LOC"));
    }

    #[test]
    fn agent_tool_use_builtin_subagent_has_no_plugin_id() {
        let line = parse(
            r#"{
              "type":"assistant","timestamp":"2026-04-24T10:00:00Z",
              "message":{"content":[
                {"type":"tool_use","id":"toolu_X","name":"Agent",
                 "input":{"subagent_type":"Explore"}}
              ]}
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].artifact_key, "Explore");
        assert_eq!(events[0].plugin_id, None);
    }

    #[test]
    fn assistant_without_agent_tool_use_yields_nothing() {
        let line = parse(
            r#"{
              "type":"assistant","timestamp":"2026-04-24T10:00:00Z",
              "message":{"content":[
                {"type":"tool_use","name":"Bash","input":{"command":"ls"}},
                {"type":"text","text":"hi"}
              ]}
            }"#,
        );
        assert!(extract_from_line(&line, "S1").is_empty());
    }

    // ----------- slash command -----------------------------------------

    #[test]
    fn user_slash_command_with_close_tag_extracts() {
        let line = parse(
            r#"{
              "type":"user","timestamp":"2026-04-24T10:00:00Z",
              "message":{"role":"user","content":"<command-name>/loc-guardian:scan</command-name>\n<command-args></command-args>"}
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ArtifactKind::Command);
        assert_eq!(events[0].artifact_key, "/loc-guardian:scan");
        assert_eq!(events[0].plugin_id.as_deref(), Some("loc-guardian"));
    }

    #[test]
    fn user_slash_command_open_only_form_still_extracts() {
        let line = parse(
            r#"{
              "type":"user","timestamp":"2026-04-24T10:00:00Z",
              "message":{"role":"user","content":"<command-name>/clear\n<command-message>"}
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].artifact_key, "/clear");
        assert_eq!(events[0].plugin_id, None);
    }

    #[test]
    fn user_with_array_content_still_extracts_slash_command() {
        let line = parse(
            r#"{
              "type":"user","timestamp":"2026-04-24T10:00:00Z",
              "message":{"role":"user","content":[
                {"type":"text","text":"<command-name>/foo</command-name>"}
              ]}
            }"#,
        );
        let events = extract_from_line(&line, "S1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].artifact_key, "/foo");
    }

    #[test]
    fn user_without_slash_command_yields_nothing() {
        let line = parse(
            r#"{
              "type":"user","timestamp":"2026-04-24T10:00:00Z",
              "message":{"role":"user","content":"just a regular message"}
            }"#,
        );
        assert!(extract_from_line(&line, "S1").is_empty());
    }

    // ----------- non-events --------------------------------------------

    #[test]
    fn line_without_timestamp_yields_nothing() {
        let line = parse(
            r#"{"type":"attachment","attachment":{"type":"invoked_skills","skills":[{"path":"plugin:p:a"}]}}"#,
        );
        assert!(extract_from_line(&line, "S1").is_empty());
    }

    #[test]
    fn unrelated_attachment_types_yield_nothing() {
        for att in &[
            r#"{"type":"task_reminder"}"#,
            r#"{"type":"skill_listing","skillCount":5,"isInitial":true}"#,
            r#"{"type":"diagnostics"}"#,
            r#"{"type":"command_permissions","allowedTools":[]}"#,
            r#"{"type":"nested_memory","path":"/x"}"#,
        ] {
            let line = json!({
                "type": "attachment",
                "timestamp": "2026-04-24T10:00:00Z",
                "attachment": serde_json::from_str::<Value>(att).unwrap()
            });
            assert!(
                extract_from_line(&line, "S1").is_empty(),
                "expected empty for {att}"
            );
        }
    }

    // ----------- agent outcome linking ---------------------------------

    #[test]
    fn extract_assistant_with_ids_pairs_event_to_tool_use_id() {
        let line = parse(
            r#"{
              "type":"assistant","timestamp":"2026-04-24T10:00:00Z",
              "message":{"content":[
                {"type":"tool_use","id":"toolu_AAA","name":"Agent","input":{"subagent_type":"Explore"}},
                {"type":"tool_use","id":"toolu_BBB","name":"Agent","input":{"subagent_type":"Plan"}}
              ]}
            }"#,
        );
        let pairs = extract_assistant_with_ids(&line, 1, "S1");
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0.artifact_key, "Explore");
        assert_eq!(pairs[0].1.as_deref(), Some("toolu_AAA"));
        assert_eq!(pairs[1].0.artifact_key, "Plan");
        assert_eq!(pairs[1].1.as_deref(), Some("toolu_BBB"));
    }

    #[test]
    fn extract_assistant_with_ids_skips_malformed_blocks_without_id_drift() {
        // Regression for the audit finding: a malformed Agent block
        // (no subagent_type) was previously skipped by extract_assistant
        // but counted by collect_agent_ids, causing the second valid
        // Agent's id to be paired with the wrong event. The new paired
        // API can't drift — id and event come from the same iteration.
        let line = parse(
            r#"{
              "type":"assistant","timestamp":"2026-04-24T10:00:00Z",
              "message":{"content":[
                {"type":"tool_use","id":"toolu_BAD","name":"Agent","input":{}},
                {"type":"tool_use","id":"toolu_GOOD","name":"Agent","input":{"subagent_type":"Explore"}}
              ]}
            }"#,
        );
        let pairs = extract_assistant_with_ids(&line, 1, "S1");
        assert_eq!(pairs.len(), 1, "malformed block must be skipped");
        assert_eq!(pairs[0].0.artifact_key, "Explore");
        assert_eq!(
            pairs[0].1.as_deref(),
            Some("toolu_GOOD"),
            "id must match the event whose block produced it, not a positional sibling"
        );
    }

    // (link_agent_outcomes was removed; outcome flipping now happens
    // streaming inside `session::scan_session` via the paired API.
    // The end-to-end test for that path lives in `session_index/mod.rs`.)

    // Slash-command parser + plugin-id parser unit tests live in
    // `extract_helpers::tests` — that's where the implementations now
    // are. Integration coverage above hits the same code through
    // extract_from_line (e.g. `user_slash_command_with_close_tag_extracts`).
}
