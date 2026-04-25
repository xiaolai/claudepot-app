//! Inline test module for `classifier.rs`. Lives in this sibling file
//! so `classifier.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "classifier_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;

fn meta() -> SessionMeta {
    SessionMeta {
        session_path: PathBuf::from("/tmp/test.jsonl"),
        cwd: PathBuf::from("/Users/x/proj"),
        git_branch: Some("main".into()),
    }
}

fn parse(line: &str) -> Value {
    serde_json::from_str(line).unwrap()
}

/// The single end-to-end positive case that pins Phase 1's
/// behavior: a real `hook_non_blocking_error` with the
/// plugin_missing pattern → one card with `help.template_id =
/// hook.plugin_missing` and the extracted plugin slug.
#[test]
fn classifies_real_plugin_missing_failure() {
    let line = include_str!("testdata/hook_plugin_missing.jsonl").trim();
    let v = parse(line);
    let mut state = ClassifierState::default();
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards.len(), 1, "exactly one card per failure");
    let c = &cards[0];
    assert_eq!(c.kind, CardKind::HookFailure);
    assert_eq!(c.severity, Severity::Warn);
    assert_eq!(c.title, "Hook failed: PostToolUse:Write");
    let h = c.help.as_ref().expect("plugin_missing must produce help");
    assert_eq!(h.template_id, "hook.plugin_missing");
    assert_eq!(h.args.get("plugin").map(String::as_str), Some("mermaid-preview@xiaolai"));
}

/// Hook failure that doesn't match any known pattern should still
/// produce a card — just without `help`. We never drop a
/// failure on the floor; "no advice" is a valid outcome.
#[test]
fn classifies_unknown_hook_failure_without_help() {
    let line = r#"{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","uuid":"u1","cwd":"/x","gitBranch":"main","attachment":{"type":"hook_non_blocking_error","hookName":"PostToolUse:Edit","hookEvent":"PostToolUse","toolUseID":"t1","command":"node weird-thing.js","exitCode":1,"durationMs":42,"stdout":"","stderr":"node: bad allocation"}}"#;
    let v = parse(line);
    let mut state = ClassifierState::default();
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards.len(), 1);
    let c = &cards[0];
    assert_eq!(c.kind, CardKind::HookFailure);
    assert!(c.help.is_none(), "unknown pattern → no help (not fabricated)");
    assert_eq!(c.subtitle.as_deref(), Some("node: bad allocation"));
}

/// Blocking error → Error severity, distinct title prefix.
#[test]
fn classifies_blocking_error_with_error_severity() {
    let line = r#"{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","uuid":"u2","cwd":"/x","attachment":{"type":"hook_blocking_error","hookName":"PreToolUse:Bash","hookEvent":"PreToolUse","toolUseID":"t1","command":"./block.sh","exitCode":2,"durationMs":10,"stderr":"forbidden command"}}"#;
    let v = parse(line);
    let mut state = ClassifierState::default();
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].severity, Severity::Error);
    assert!(cards[0].title.starts_with("Hook BLOCKED"));
}

/// Negative cases — every non-attachment line type returns zero
/// cards on the v1 fast path.
#[test]
fn ignores_non_attachment_lines() {
    let mut state = ClassifierState::default();
    for sample in [
        r#"{"type":"user","message":{"role":"user","content":"hello"},"timestamp":"2026-04-25T10:00:00Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]},"timestamp":"2026-04-25T10:00:00Z"}"#,
        r#"{"type":"summary","summary":"...","timestamp":"2026-04-25T10:00:00Z"}"#,
        // Pre-2.1.85 hook_progress envelope — explicitly suppressed
        // (also not an attachment).
        r#"{"type":"progress","data":{"type":"hook_progress","hookEvent":"SessionStart"}}"#,
    ] {
        let cards = classify(&parse(sample), 0, &meta(), &mut state);
        assert!(cards.is_empty(), "non-attachment must produce no card: {sample}");
    }
}

/// Successful hooks (`hook_success`) and rule-load attachments
/// (`nested_memory`) must NOT produce cards in v1 — they're in
/// the suppression list (design v2 §2).
#[test]
fn suppresses_hook_success_and_rule_loads_in_v1() {
    let mut state = ClassifierState::default();
    for sample in [
        r#"{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","attachment":{"type":"hook_success","hookName":"PostToolUse:Edit","hookEvent":"PostToolUse","toolUseID":"t1","content":"ok","exitCode":0,"durationMs":12}}"#,
        r#"{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","attachment":{"type":"nested_memory","path":"/x/.claude/rules/r.md","content":{"type":"Project","content":"..."}}}"#,
    ] {
        let cards = classify(&parse(sample), 0, &meta(), &mut state);
        assert!(cards.is_empty(), "suppressed attachment produced card: {sample}");
    }
}

/// Regression for Codex audit MEDIUM #1: every hook-failure
/// attachment family CC writes must produce a card. Earlier
/// implementations only handled non-blocking + blocking; the
/// other three (cancelled, error_during_execution,
/// stopped_continuation) were silently dropped.
#[test]
fn classifies_every_hook_failure_attachment_family() {
    let cases = [
        (
            "hook_cancelled",
            "Hook cancelled: ",
            Severity::Notice,
        ),
        (
            "hook_error_during_execution",
            "Hook crashed: ",
            Severity::Error,
        ),
        (
            "hook_stopped_continuation",
            "Hook stopped continuation: ",
            Severity::Error,
        ),
    ];
    for (att_type, prefix, expected_severity) in cases {
        let line = format!(
            r#"{{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","uuid":"u1","cwd":"/x","attachment":{{"type":"{att_type}","hookName":"PostToolUse:Edit","hookEvent":"PostToolUse","toolUseID":"t1","command":"./x.sh","exitCode":1,"durationMs":42,"stderr":"oh no"}}}}"#
        );
        let v = parse(&line);
        let mut state = ClassifierState::default();
        let cards = classify(&v, 0, &meta(), &mut state);
        assert_eq!(cards.len(), 1, "{att_type} should produce one card");
        assert_eq!(cards[0].kind, CardKind::HookFailure);
        assert_eq!(
            cards[0].severity, expected_severity,
            "{att_type} severity"
        );
        assert!(
            cards[0].title.starts_with(prefix),
            "{att_type} title prefix mismatch: {:?}",
            cards[0].title
        );
    }
}

/// Regression for Codex audit HIGH #4: stderr text persisted in
/// the card subtitle must pass through `redact_secrets`. A hook
/// that echoes a token in stderr must not leak it into
/// sessions.db. We use a sentinel `sk-ant-` token because
/// `redact_secrets`'s fast-path triggers on that prefix.
#[test]
fn redacts_stderr_secrets_from_subtitle() {
    let line = r#"{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","uuid":"u1","cwd":"/x","attachment":{"type":"hook_non_blocking_error","hookName":"PostToolUse:Edit","hookEvent":"PostToolUse","toolUseID":"t1","command":"./x.sh","exitCode":1,"durationMs":42,"stderr":"failed: token sk-ant-oat01-XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-suffix is invalid"}}"#;
    let v = parse(line);
    let mut state = ClassifierState::default();
    let cards = classify(&v, 0, &meta(), &mut state);
    let sub = cards[0].subtitle.as_deref().unwrap_or("");
    assert!(
        !sub.contains("sk-ant-oat01-XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX-suffix"),
        "raw token must not appear in subtitle: {sub:?}"
    );
}

/// Schema drift defense — a `hook_non_blocking_error` missing the
/// required fields must not panic, and must not emit a malformed
/// card. Returning zero is the conservative answer.
#[test]
fn defensive_against_missing_required_fields() {
    let mut state = ClassifierState::default();
    let v = parse(r#"{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","attachment":{"type":"hook_non_blocking_error"}}"#);
    let cards = classify(&v, 0, &meta(), &mut state);
    assert!(cards.is_empty(), "missing required fields → no card");
}

#[test]
fn extract_plugin_handles_paren_form() {
    let s = "Failed to run: Plugin directory does not exist: /Users/joker/.claude/plugins/cache/xiaolai/mermaid-preview/0.1.1 (mermaid-preview@xiaolai — run /plugin to reinstall)";
    assert_eq!(extract_missing_plugin(s).as_deref(), Some("mermaid-preview@xiaolai"));
}

#[test]
fn extract_plugin_falls_back_to_path_form() {
    // Hypothetical variant that omits the parenthesized hint.
    let s = "Failed to run: Plugin directory does not exist: /Users/joker/.claude/plugins/cache/owner/name/0.1.0";
    assert_eq!(extract_missing_plugin(s).as_deref(), Some("name@owner"));
}

#[test]
fn extract_plugin_returns_none_on_unrelated_stderr() {
    assert_eq!(extract_missing_plugin("permission denied"), None);
    assert_eq!(extract_missing_plugin(""), None);
}

// ── Phase 2: HookSlow ────────────────────────────────────────

/// `hook_success` with `durationMs > 5000` → `HookSlow` card.
#[test]
fn classifies_slow_hook_success() {
    let line = r#"{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","uuid":"u1","cwd":"/x","attachment":{"type":"hook_success","hookName":"PostToolUse:Edit","hookEvent":"PostToolUse","toolUseID":"t1","content":"ok","stdout":"","stderr":"","exitCode":0,"durationMs":7500,"command":"./slow.sh"}}"#;
    let v = parse(line);
    let mut state = ClassifierState::default();
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].kind, CardKind::HookSlow);
    assert_eq!(cards[0].severity, Severity::Notice);
    assert!(
        cards[0].title.contains("7500 ms"),
        "title should include duration: {:?}",
        cards[0].title
    );
}

/// Fast successful hooks stay invisible — that's the suppression
/// rule from design v2 §2 (routine sub-5s success = noise).
#[test]
fn fast_hook_success_is_suppressed() {
    let line = r#"{"type":"attachment","timestamp":"2026-04-25T10:00:00Z","attachment":{"type":"hook_success","hookName":"x","hookEvent":"PostToolUse","toolUseID":"t1","content":"ok","exitCode":0,"durationMs":42}}"#;
    let v = parse(line);
    let mut state = ClassifierState::default();
    let cards = classify(&v, 0, &meta(), &mut state);
    assert!(cards.is_empty());
}

// ── Phase 2: ToolError ───────────────────────────────────────

fn tool_error_line(content: &str) -> Value {
    // Build the JSONL envelope by serializing an object with the
    // content as a string field — escaping is then handled by
    // serde_json. Avoids the embedded-string-escaping fragility of
    // the previous fixture style.
    serde_json::json!({
        "type": "user",
        "timestamp": "2026-04-25T10:00:00Z",
        "uuid": "u1",
        "cwd": "/x",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "t1",
                "is_error": true,
                "content": content,
            }]
        }
    })
}

#[test]
fn classifies_tool_error_unknown_pattern_without_help() {
    let mut state = ClassifierState::default();
    let v = tool_error_line("Exit code 1\nweird novel failure");
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].kind, CardKind::ToolError);
    assert!(cards[0].help.is_none());
}

#[test]
fn classifies_tool_error_read_required_with_help() {
    let mut state = ClassifierState::default();
    let v = tool_error_line(
        "<tool_use_error>File has been modified since read, either by the user or by a linter.</tool_use_error>",
    );
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].kind, CardKind::ToolError);
    // Read-required is Info — model auto-recovers.
    assert_eq!(cards[0].severity, Severity::Info);
    assert_eq!(
        cards[0].help.as_ref().unwrap().template_id,
        "tool.read_required"
    );
}

#[test]
fn classifies_tool_error_ssh_timeout_extracts_host() {
    let mut state = ClassifierState::default();
    let v = tool_error_line("Exit code 255\nssh: connect to host 192.0.2.7 port 22: Operation timed out");
    let cards = classify(&v, 0, &meta(), &mut state);
    let h = cards[0].help.as_ref().unwrap();
    assert_eq!(h.template_id, "tool.ssh_timeout");
    assert_eq!(h.args.get("host").map(String::as_str), Some("192.0.2.7"));
}

#[test]
fn classifies_tool_error_edit_drift() {
    let mut state = ClassifierState::default();
    let v = tool_error_line(
        "<tool_use_error>String to replace not found in file.\nString: foo</tool_use_error>",
    );
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(
        cards[0].help.as_ref().unwrap().template_id,
        "tool.edit_drift"
    );
}

#[test]
fn classifies_tool_error_user_rejected() {
    let mut state = ClassifierState::default();
    let v = tool_error_line(
        "The user doesn't want to proceed with this tool use. The tool use was rejected.",
    );
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards[0].severity, Severity::Notice);
    assert_eq!(
        cards[0].help.as_ref().unwrap().template_id,
        "tool.user_rejected"
    );
}

#[test]
fn classifies_tool_error_bash_cmd_not_found() {
    let mut state = ClassifierState::default();
    let v = tool_error_line("Exit code 127\npyenv: python: command not found");
    let cards = classify(&v, 0, &meta(), &mut state);
    let h = cards[0].help.as_ref().unwrap();
    assert_eq!(h.template_id, "tool.bash_cmd_not_found");
    assert_eq!(h.args.get("command").map(String::as_str), Some("python"));
}

/// Successful tool calls (`is_error: false`) must NOT produce
/// cards — only the failure path is interesting.
#[test]
fn successful_tool_results_are_suppressed() {
    let mut state = ClassifierState::default();
    let v = serde_json::json!({
        "type": "user",
        "timestamp": "2026-04-25T10:00:00Z",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "t1",
                "is_error": false,
                "content": "fine",
            }]
        }
    });
    let cards = classify(&v, 0, &meta(), &mut state);
    assert!(cards.is_empty());
}

// ── Phase 3: Episode tracker (Agent return / stranded) ───────

fn agent_open_line(tool_use_id: &str) -> Value {
    serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-04-25T10:00:00Z",
        "uuid": "u-open",
        "cwd": "/x",
        "message": {
            "role": "assistant",
            "model": "claude-opus-4-7",
            "content": [{
                "type": "tool_use",
                "id": tool_use_id,
                "name": "Agent",
                "input": {
                    "subagent_type": "Explore",
                    "description": "Find the leak"
                }
            }]
        }
    })
}

fn agent_close_line(tool_use_id: &str, is_error: bool, ts: &str) -> Value {
    serde_json::json!({
        "type": "user",
        "timestamp": ts,
        "uuid": "u-close",
        "cwd": "/x",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "is_error": is_error,
                "content": "done",
            }]
        }
    })
}

/// Slow successful agent close → AgentReturn card (Notice).
/// Phase 2/3 design: only emit on failure OR duration > 60 s.
#[test]
fn agent_open_then_slow_close_emits_one_agent_return_card() {
    let mut state = ClassifierState::default();
    let cards1 = classify(&agent_open_line("t1"), 0, &meta(), &mut state);
    // Opening the episode produces no card by itself (the assistant
    // turn is the trigger; the card lands on close).
    assert!(cards1.is_empty());
    assert_eq!(state.open_episodes.len(), 1);

    // 90 s elapsed — above the 60 s threshold, so the card emits.
    let cards2 = classify(
        &agent_close_line("t1", false, "2026-04-25T10:01:30Z"),
        100,
        &meta(),
        &mut state,
    );
    assert_eq!(cards2.len(), 1);
    assert_eq!(cards2[0].kind, CardKind::AgentReturn);
    assert_eq!(cards2[0].severity, Severity::Notice);
    assert!(
        cards2[0].title.starts_with("Agent Explore returned"),
        "title {:?}",
        cards2[0].title
    );
    assert_eq!(state.open_episodes.len(), 0, "episode closed");
}

/// Fast successful agent close → episode closed silently (no card).
/// Routine fast subagents are noise per design v2 §5.
#[test]
fn agent_fast_successful_close_suppresses_card_but_drains_episode() {
    let mut state = ClassifierState::default();
    classify(&agent_open_line("t1"), 0, &meta(), &mut state);
    let cards = classify(
        // 30 s elapsed — below the 60 s threshold.
        &agent_close_line("t1", false, "2026-04-25T10:00:30Z"),
        100,
        &meta(),
        &mut state,
    );
    assert!(cards.is_empty(), "fast successful agent suppressed");
    assert_eq!(state.open_episodes.len(), 0, "episode still drained");
}

#[test]
fn agent_close_with_error_emits_error_severity_card() {
    let mut state = ClassifierState::default();
    classify(&agent_open_line("t1"), 0, &meta(), &mut state);
    let cards = classify(
        &agent_close_line("t1", true, "2026-04-25T10:00:01Z"),
        0,
        &meta(),
        &mut state,
    );
    assert_eq!(cards[0].kind, CardKind::AgentReturn);
    assert_eq!(cards[0].severity, Severity::Error);
    assert!(cards[0].title.contains("failed"));
    assert_eq!(
        cards[0].help.as_ref().unwrap().template_id,
        "agent.error_return"
    );
}

/// Open agent episodes drained at session end → AgentStranded.
#[test]
fn finalize_session_drains_open_episodes_into_stranded_cards() {
    let mut state = ClassifierState::default();
    classify(&agent_open_line("t1"), 0, &meta(), &mut state);
    classify(&agent_open_line("t2"), 50, &meta(), &mut state);
    let cards = finalize_session(&mut state, &meta());
    assert_eq!(cards.len(), 2);
    for card in &cards {
        assert_eq!(card.kind, CardKind::AgentStranded);
        assert_eq!(card.severity, Severity::Warn);
        assert!(card.title.contains("did not return"));
    }
    assert_eq!(state.open_episodes.len(), 0, "drained");
}

/// A non-Agent `tool_result` whose id matches an open episode
/// (shouldn't happen in CC but defensive): fall through to the
/// generic ToolError path. The Agent close path checks the
/// open_episodes map by id alone, so this test pins what happens
/// when a stale id collides.
#[test]
fn non_agent_tool_result_falls_through_to_tool_error() {
    let mut state = ClassifierState::default();
    let v = tool_error_line("some random failure");
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards[0].kind, CardKind::ToolError);
}

// ── Phase 3: SessionMilestone (model switch) ─────────────────

fn assistant_with_model(model: &str) -> Value {
    serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-04-25T10:00:00Z",
        "uuid": "u-a",
        "cwd": "/x",
        "message": {
            "role": "assistant",
            "model": model,
            "content": [{"type": "text", "text": "hi"}],
        }
    })
}

#[test]
fn first_assistant_turn_does_not_emit_milestone() {
    let mut state = ClassifierState::default();
    let cards = classify(
        &assistant_with_model("claude-opus-4-7"),
        0,
        &meta(),
        &mut state,
    );
    assert!(
        cards.is_empty(),
        "first model sighting is a baseline, not a switch"
    );
    assert_eq!(state.last_model.as_deref(), Some("claude-opus-4-7"));
}

#[test]
fn model_switch_emits_milestone_card() {
    let mut state = ClassifierState::default();
    classify(
        &assistant_with_model("claude-opus-4-7"),
        0,
        &meta(),
        &mut state,
    );
    let cards = classify(
        &assistant_with_model("claude-sonnet-4-6"),
        100,
        &meta(),
        &mut state,
    );
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].kind, CardKind::SessionMilestone);
    assert!(
        cards[0]
            .title
            .contains("claude-opus-4-7 → claude-sonnet-4-6"),
        "title {:?}",
        cards[0].title
    );
}

#[test]
fn same_model_repeated_does_not_emit_milestone() {
    let mut state = ClassifierState::default();
    classify(
        &assistant_with_model("claude-opus-4-7"),
        0,
        &meta(),
        &mut state,
    );
    let cards = classify(
        &assistant_with_model("claude-opus-4-7"),
        100,
        &meta(),
        &mut state,
    );
    assert!(cards.is_empty());
}

// ── Helper extractors ─────────────────────────────────────────

#[test]
fn extract_ssh_host_handles_real_message() {
    let s = "Exit code 255\nssh: connect to host 192.0.2.7 port 22: Operation timed out\n";
    assert_eq!(extract_ssh_host(s).as_deref(), Some("192.0.2.7"));
}

#[test]
fn extract_missing_command_handles_pyenv_form() {
    let s = "Exit code 127\npyenv: python: command not found";
    assert_eq!(extract_missing_command(s).as_deref(), Some("python"));
}

#[test]
fn extract_missing_command_handles_bash_form() {
    let s = "bash: fzf: command not found";
    assert_eq!(extract_missing_command(s).as_deref(), Some("fzf"));
}

// ── Phase 4: Plugin attribution ──────────────────────────────

#[test]
fn plugin_from_command_extracts_from_cache_path() {
    // Real shape from the audit fixture.
    let s = "bash /Users/joker/.claude/plugins/cache/xiaolai/mermaid-preview/0.1.1/scripts/foo.sh";
    assert_eq!(
        plugin_from_command_string(s).as_deref(),
        Some("mermaid-preview@xiaolai")
    );
}

#[test]
fn plugin_from_command_returns_none_for_bare_env_var() {
    // CLAUDE_PLUGIN_ROOT alone doesn't name the plugin — caller
    // falls back to other signals (extract_missing_plugin from
    // stderr is the most common one).
    let s = "bash ${CLAUDE_PLUGIN_ROOT}/scripts/foo.sh";
    assert_eq!(plugin_from_command_string(s), None);
}

#[test]
fn plugin_from_namespaced_name_pulls_prefix() {
    assert_eq!(
        plugin_from_namespaced_name("grill:roast").as_deref(),
        Some("grill")
    );
    assert_eq!(
        plugin_from_namespaced_name("nlpm:scorer").as_deref(),
        Some("nlpm")
    );
}

#[test]
fn plugin_from_namespaced_name_returns_none_for_bare_name() {
    // Built-in agents like "Explore" / "general-purpose" have no
    // plugin namespace.
    assert_eq!(plugin_from_namespaced_name("Explore"), None);
    assert_eq!(plugin_from_namespaced_name("general-purpose"), None);
    // Empty halves rejected.
    assert_eq!(plugin_from_namespaced_name(":roast"), None);
    assert_eq!(plugin_from_namespaced_name("grill:"), None);
}

/// End-to-end: a real plugin_missing hook failure card carries
/// the extracted plugin slug as an attribution.
#[test]
fn plugin_missing_hook_attributes_to_plugin() {
    let line = include_str!("testdata/hook_plugin_missing.jsonl").trim();
    let v = parse(line);
    let mut state = ClassifierState::default();
    let cards = classify(&v, 0, &meta(), &mut state);
    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0].plugin.as_deref(),
        Some("mermaid-preview@xiaolai"),
        "plugin attribution missing"
    );
}

/// Agent return cards inherit the plugin namespace from
/// `subagent_type` when present.
#[test]
fn plugin_namespaced_subagent_attributes_to_plugin() {
    let mut state = ClassifierState::default();
    let open = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-04-25T10:00:00Z",
        "uuid": "u-open",
        "cwd": "/x",
        "message": {
            "role": "assistant",
            "model": "claude-opus-4-7",
            "content": [{
                "type": "tool_use",
                "id": "t1",
                "name": "Agent",
                "input": {
                    "subagent_type": "grill:roast",
                    "description": "audit",
                }
            }]
        }
    });
    classify(&open, 0, &meta(), &mut state);
    // Use a slow close (>60s) so the card actually emits.
    let close = agent_close_line("t1", false, "2026-04-25T10:01:30Z");
    let cards = classify(&close, 100, &meta(), &mut state);
    assert_eq!(cards[0].plugin.as_deref(), Some("grill"));
}
