//! JSONL line → `Card`. The data plane of design v2 §4.
//!
//! Pure function: `classify(line, byte_offset, meta, state) -> Vec<Card>`.
//! No I/O, no allocations on the hot path beyond what the returned
//! vec needs. Most lines emit zero cards; the function short-circuits
//! on type mismatch in the first match arm.
//!
//! `state` is the per-session episode tracker — a write-through
//! accumulator that survives across calls within one session. v1
//! does not yet emit Agent / SessionMilestone cards, so the tracker
//! is essentially inert; the field exists so Phase 3 can add episode
//! pairing without changing the public signature.
//!
//! Why does the classifier consume `serde_json::Value` instead of the
//! existing `SessionEvent` enum? See design v2 §1, call #2: cards
//! and the transcript view have different needs and shouldn't share
//! a parse model. The transcript parser is for *rendering the
//! conversation*; the classifier is for *extracting anomalies*.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::card::{Card, CardKind, ConfigScope, HelpRef, Severity, SourceRef};
use crate::session_live::redact::redact_secrets;

/// Per-session in/out state for the classifier. Phase 1 uses only
/// `seen_rules` (debouncing — rules that haven't been suppressed will
/// land in Phase 2). Phase 3 extends with `open_episodes` for Agent
/// pairing and `cumulative_tokens`, etc., for milestones.
///
/// Default-constructed state is the right starting point for every
/// new session — there is no shared global state.
#[derive(Debug, Default)]
pub struct ClassifierState {
    /// Dedup `nested_memory` loads per session. Reserved for the rule
    /// suppression rule when it's wired up; safe to keep empty for v2.
    pub seen_rules: HashSet<PathBuf>,
    /// Open `Agent` tool_uses keyed by `tool_use.id`. Closed by a
    /// matching `tool_result` (emits `AgentReturn`) or drained at
    /// session-end via `finalize_session` (emits `AgentStranded`).
    pub open_episodes: HashMap<String, OpenEpisode>,
    /// Last `message.model` seen on an assistant turn for this
    /// session. A change between calls emits a `SessionMilestone`
    /// card describing the switch (e.g. "Sonnet 4.6 → Opus 4.7").
    pub last_model: Option<String>,
}

/// A `tool_use` that hasn't seen its matching `tool_result` yet.
/// Closed via `close_agent_episode_if_open` (emits `AgentReturn`)
/// or drained via `finalize_session` (emits `AgentStranded`).
#[derive(Debug, Clone)]
pub struct OpenEpisode {
    pub tool_use_id: String,
    pub tool_name: String,
    pub opened_at: DateTime<Utc>,
    pub byte_offset: u64,
    /// Carried for human-readable card titles. `None` for non-Agent
    /// tool_uses (Phase 2 only opens Agent episodes; later phases
    /// may track other long-running tools).
    pub subagent_type: Option<String>,
    pub description: Option<String>,
}

/// Envelope context carried alongside every line. Filled once per
/// session by the caller so the classifier doesn't need to re-extract
/// it from each line. `cwd` and `git_branch` come from the JSONL
/// itself (CC writes them on every record), so the caller threads
/// the most recent values it has seen.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    /// Absolute path of the JSONL being read.
    pub session_path: PathBuf,
    /// Working directory from the most recent envelope. The classifier
    /// prefers the line's own `cwd` field when present.
    pub cwd: PathBuf,
    /// Git branch from the most recent envelope.
    pub git_branch: Option<String>,
}

/// Classify one JSONL line. Returns 0..N cards (almost always 0 or 1).
///
/// `byte_offset` is the offset of this line's first byte in the
/// JSONL — the caller tracks it as it walks the file. This anchor is
/// what lets the GUI fetch the body lazily without re-scanning.
///
/// `state` is the per-session episode tracker — see `ClassifierState`.
pub fn classify(
    line: &Value,
    byte_offset: u64,
    meta: &SessionMeta,
    state: &mut ClassifierState,
) -> Vec<Card> {
    // Fast-path: route by top-level `type`. Most lines emit zero
    // cards and short-circuit in the first arm.
    let entry_type = line.get("type").and_then(Value::as_str).unwrap_or("");
    match entry_type {
        "attachment" => classify_attachment(line, byte_offset, meta),
        // user lines carry tool_result blocks — that's where ToolError
        // cards come from. Phase 2 also tracks Agent tool_result for
        // episode close (Phase 3 adds the AgentReturn emission).
        "user" => classify_user_line(line, byte_offset, meta, state),
        // assistant lines carry tool_use blocks — Phase 3 opens Agent
        // episodes here and emits SessionMilestone (model switch).
        "assistant" => classify_assistant_line(line, byte_offset, meta, state),
        _ => Vec::new(),
    }
}

/// Route `attachment` records by `attachment.type`. Most types are
/// suppressed in v1 (rule loads, skill listings, MCP additions).
fn classify_attachment(line: &Value, byte_offset: u64, meta: &SessionMeta) -> Vec<Card> {
    // Bind in the match to avoid the unwrap-after-Some-check pattern
    // (rust-conventions: no `unwrap` in core).
    let attachment = match line.get("attachment") {
        Some(v @ Value::Object(_)) => v,
        _ => return Vec::new(),
    };

    let attachment_type = attachment.get("type").and_then(Value::as_str).unwrap_or("");
    match attachment_type {
        // Every hook-failure attachment family CC writes. Severities
        // and labels differ; the shared `classify_hook_failure` does
        // the heavy lifting and the per-arm metadata is a small
        // override map. Keep this list in sync with `CardKind::HookFailure`'s
        // doc comment so the public contract stays honest.
        "hook_non_blocking_error" => classify_hook_failure(
            line,
            attachment,
            byte_offset,
            meta,
            HookFailureFlavor::NonBlocking,
        )
        .map(|c| vec![c])
        .unwrap_or_default(),
        "hook_blocking_error" => classify_hook_failure(
            line,
            attachment,
            byte_offset,
            meta,
            HookFailureFlavor::Blocking,
        )
        .map(|c| vec![c])
        .unwrap_or_default(),
        "hook_cancelled" => classify_hook_failure(
            line,
            attachment,
            byte_offset,
            meta,
            HookFailureFlavor::Cancelled,
        )
        .map(|c| vec![c])
        .unwrap_or_default(),
        "hook_error_during_execution" => classify_hook_failure(
            line,
            attachment,
            byte_offset,
            meta,
            HookFailureFlavor::ExecutionError,
        )
        .map(|c| vec![c])
        .unwrap_or_default(),
        "hook_stopped_continuation" => classify_hook_failure(
            line,
            attachment,
            byte_offset,
            meta,
            HookFailureFlavor::StoppedContinuation,
        )
        .map(|c| vec![c])
        .unwrap_or_default(),
        // Phase 2: surface successful hooks that exceeded a slowness
        // threshold. Routine fast hooks stay invisible per design v2
        // §2 suppression rules.
        "hook_success" => classify_hook_slow(line, attachment, byte_offset, meta)
            .map(|c| vec![c])
            .unwrap_or_default(),
        // Other attachment types (additional_context, system_message,
        // nested_memory, skill_listing, mcp_*_delta, …) are
        // suppressed in v1 — they're routine activity, not anomalies.
        _ => Vec::new(),
    }
}

/// Slow-hook threshold. Hooks above this duration get a `HookSlow`
/// card even on success — the duration itself is the signal.
const HOOK_SLOW_THRESHOLD_MS: i64 = 5_000;

/// Slow-agent threshold. Successful Agent runs above this get an
/// `AgentReturn` card; below it, the episode silently closes (the
/// design's noise-suppression rule for routine fast subagents).
const AGENT_RETURN_SLOW_THRESHOLD_MS: i64 = 60_000;

/// `hook_success` with `durationMs > 5000` → `HookSlow` card. Cheap
/// hooks below the threshold stay silent (the design's suppression
/// rule for routine activity).
fn classify_hook_slow(
    line: &Value,
    attachment: &Value,
    byte_offset: u64,
    meta: &SessionMeta,
) -> Option<Card> {
    let duration_ms = attachment.get("durationMs").and_then(Value::as_i64)?;
    // Design v2 §6 / §5: durationMs > 5000 emits, exactly 5000 stays
    // silent. `<=` (not `<`) implements the strictly-greater rule.
    if duration_ms <= HOOK_SLOW_THRESHOLD_MS {
        return None;
    }
    let hook_name = attachment.get("hookName").and_then(Value::as_str)?;
    let _hook_event = attachment.get("hookEvent").and_then(Value::as_str)?;
    let command = attachment
        .get("command")
        .and_then(Value::as_str)
        .map(|c| redact_secrets(c));

    let plugin = plugin_from_hook(attachment);
    Some(Card {
        id: None,
        session_path: meta.session_path.clone(),
        event_uuid: line.get("uuid").and_then(Value::as_str).map(String::from),
        byte_offset,
        kind: CardKind::HookSlow,
        ts: parse_ts(line)?,
        severity: Severity::Notice,
        title: format!("Slow hook: {hook_name} ({duration_ms} ms)"),
        subtitle: command.as_deref().map(|s| truncate(s, 120)),
        help: None,
        source_ref: None,
        cwd: extract_cwd(line, meta),
        git_branch: extract_git_branch(line, meta),
        plugin,
    })
}

/// `type: user` records carry the model's `tool_result` responses
/// (in `message.content[*]` with `type: tool_result`). Any with
/// `is_error: true` becomes a `ToolError` card.
///
/// Phase 3 will also close `Agent` episodes here when a tool_result
/// matches an open `Agent` tool_use_id (emitting `AgentReturn`
/// instead of `ToolError`). For Phase 2 we treat agent-tool errors
/// the same as any other tool error — there's no Agent open-set yet.
fn classify_user_line(
    line: &Value,
    byte_offset: u64,
    meta: &SessionMeta,
    state: &mut ClassifierState,
) -> Vec<Card> {
    let message = match line.get("message").and_then(Value::as_object) {
        Some(m) => m,
        None => return Vec::new(),
    };
    let content = match message.get("content").and_then(Value::as_array) {
        Some(c) => c,
        None => return Vec::new(),
    };

    let mut cards = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let is_error = block
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let tool_use_id = block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .map(String::from);

        // Phase 3 hook: close the matching Agent episode if one's open.
        // For now (Phase 2) we just log it — the Agent close path
        // emits its own card kind and lives in
        // `close_agent_episode_if_open`. Always called so the
        // open_episodes map gets drained even on Phase 2 paths.
        if let Some(id) = tool_use_id.as_deref() {
            if let Some(card) =
                close_agent_episode_if_open(state, id, line, byte_offset, meta, is_error)
            {
                cards.push(card);
                // The agent return card supersedes a generic tool
                // error card on the same `tool_use_id`.
                continue;
            }
        }

        if !is_error {
            continue;
        }

        let body = extract_tool_result_text(block);
        if let Some(card) =
            build_tool_error_card(line, byte_offset, meta, tool_use_id.clone(), &body)
        {
            cards.push(card);
        }
    }
    cards
}

/// `type: assistant` records carry `tool_use` blocks in
/// `message.content`. Phase 3 opens `Agent` episodes here and emits
/// `SessionMilestone` cards on model changes; Phase 2 already
/// supports both since the cost is one HashMap insert + one model-id
/// compare per assistant turn.
fn classify_assistant_line(
    line: &Value,
    byte_offset: u64,
    meta: &SessionMeta,
    state: &mut ClassifierState,
) -> Vec<Card> {
    let message = match line.get("message").and_then(Value::as_object) {
        Some(m) => m,
        None => return Vec::new(),
    };
    let mut cards = Vec::new();

    // Milestone: model change. Compare the model id seen on this
    // turn to the last one we recorded for the session. Skip the
    // first turn (when last_model is None) — there's nothing to
    // "switch from."
    if let Some(model) = message.get("model").and_then(Value::as_str) {
        let model_string = model.to_string();
        match &state.last_model {
            None => state.last_model = Some(model_string),
            Some(prev) if prev != model => {
                cards.push(build_milestone_card(
                    line,
                    byte_offset,
                    meta,
                    format!("Model switched: {prev} → {model}"),
                ));
                state.last_model = Some(model_string);
            }
            _ => {}
        }
    }

    // Open Agent episodes for every `Agent` tool_use in this turn.
    // The matching close is in `close_agent_episode_if_open`.
    if let Some(content) = message.get("content").and_then(Value::as_array) {
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let tool_name = match block.get("name").and_then(Value::as_str) {
                Some(n) => n,
                None => continue,
            };
            if tool_name != "Agent" {
                continue;
            }
            let tool_use_id = match block.get("id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => continue,
            };
            let subagent_type = block
                .get("input")
                .and_then(|i| i.get("subagent_type"))
                .and_then(Value::as_str)
                .map(String::from);
            let description = block
                .get("input")
                .and_then(|i| i.get("description"))
                .and_then(Value::as_str)
                .map(String::from);
            state.open_episodes.insert(
                tool_use_id.clone(),
                OpenEpisode {
                    tool_use_id,
                    tool_name: tool_name.to_string(),
                    opened_at: parse_ts(line).unwrap_or_else(Utc::now),
                    byte_offset,
                    subagent_type,
                    description,
                },
            );
        }
    }

    cards
}

/// Drain every still-open episode in `state` into `AgentStranded`
/// cards. The caller invokes this when the session ends (PID gone OR
/// JSONL idle 5 min). Mutates state — drained map is empty after.
///
/// Returned cards reference the *opening* tool_use's byte_offset and
/// timestamp — they describe the agent that never returned, not the
/// session end itself.
pub fn finalize_session(state: &mut ClassifierState, meta: &SessionMeta) -> Vec<Card> {
    let drained: Vec<OpenEpisode> = state.open_episodes.drain().map(|(_, ep)| ep).collect();
    drained
        .into_iter()
        .map(|ep| {
            let label = ep
                .subagent_type
                .as_deref()
                .map(|s| format!("Agent {s}"))
                .unwrap_or_else(|| format!("{} tool", ep.tool_name));
            let subtitle = ep
                .description
                .as_deref()
                .map(|s| truncate(&redact_secrets(s), 120));
            let plugin = ep
                .subagent_type
                .as_deref()
                .and_then(plugin_from_namespaced_name);
            Card {
                id: None,
                session_path: meta.session_path.clone(),
                event_uuid: None,
                byte_offset: ep.byte_offset,
                kind: CardKind::AgentStranded,
                ts: ep.opened_at,
                severity: Severity::Warn,
                title: format!("{label} did not return"),
                subtitle,
                help: Some(HelpRef {
                    template_id: "agent.no_return".to_string(),
                    args: Default::default(),
                }),
                source_ref: None,
                cwd: meta.cwd.clone(),
                git_branch: meta.git_branch.clone(),
                plugin,
            }
        })
        .collect()
}

/// Try to close an open `Agent` episode keyed by `tool_use_id`. If
/// the id is in `open_episodes`, remove it and return an
/// `AgentReturn` card describing the close. If not present (the
/// tool_result was for a non-Agent tool, or the open record was
/// never created), return `None` so the caller falls through to the
/// generic `ToolError` path.
fn close_agent_episode_if_open(
    state: &mut ClassifierState,
    tool_use_id: &str,
    line: &Value,
    byte_offset: u64,
    meta: &SessionMeta,
    is_error: bool,
) -> Option<Card> {
    let ep = state.open_episodes.remove(tool_use_id)?;
    let now = parse_ts(line).unwrap_or_else(Utc::now);
    let duration = (now - ep.opened_at).num_milliseconds().max(0);
    // Design v2 §5: AgentReturn cards only emit on failure or when
    // the agent ran longer than 60 s. Routine successful agents
    // returning quickly are noise. Episode is still drained from the
    // map (so it can't strand later); we just suppress the card.
    if !is_error && duration <= AGENT_RETURN_SLOW_THRESHOLD_MS {
        return None;
    }
    let severity = if is_error {
        Severity::Error
    } else {
        Severity::Notice
    };
    let label = ep
        .subagent_type
        .as_deref()
        .map(|s| format!("Agent {s}"))
        .unwrap_or_else(|| format!("{} tool", ep.tool_name));
    let title = if is_error {
        format!("{label} failed ({duration} ms)")
    } else {
        format!("{label} returned ({duration} ms)")
    };
    // Description from the parent's tool_use input is user-typed
    // text — same redaction discipline as hook stderr.
    let subtitle = ep
        .description
        .as_deref()
        .map(|s| truncate(&redact_secrets(s), 120));
    let help = if is_error {
        Some(HelpRef {
            template_id: "agent.error_return".to_string(),
            args: Default::default(),
        })
    } else {
        None
    };
    // Agent return cards inherit the subagent's plugin namespace
    // when the subagent_type carries one (e.g. "grill:roast").
    let plugin = ep
        .subagent_type
        .as_deref()
        .and_then(plugin_from_namespaced_name);
    Some(Card {
        id: None,
        session_path: meta.session_path.clone(),
        event_uuid: line.get("uuid").and_then(Value::as_str).map(String::from),
        byte_offset,
        kind: CardKind::AgentReturn,
        ts: now,
        severity,
        title,
        subtitle,
        help,
        source_ref: None,
        cwd: extract_cwd(line, meta),
        git_branch: extract_git_branch(line, meta),
        plugin,
    })
}

/// Build a `SessionMilestone` card with a fixed Notice severity.
/// Used by the milestone-detection arms in `classify_assistant_line`.
fn build_milestone_card(line: &Value, byte_offset: u64, meta: &SessionMeta, title: String) -> Card {
    Card {
        id: None,
        session_path: meta.session_path.clone(),
        event_uuid: line.get("uuid").and_then(Value::as_str).map(String::from),
        byte_offset,
        kind: CardKind::SessionMilestone,
        ts: parse_ts(line).unwrap_or_else(Utc::now),
        severity: Severity::Notice,
        title,
        subtitle: None,
        help: None,
        source_ref: None,
        cwd: extract_cwd(line, meta),
        git_branch: extract_git_branch(line, meta),
        plugin: None,
    }
}

/// Build a `ToolError` card. Tries to match the body against the
/// Phase 2 tool-error templates; emits the card with no help when
/// none match. Tool name is best-effort — the parent message has it
/// in a sibling field, but most callers only need the body for the
/// error message. Defer parent lookup to v2 if it matters.
fn build_tool_error_card(
    line: &Value,
    byte_offset: u64,
    meta: &SessionMeta,
    tool_use_id: Option<String>,
    body: &str,
) -> Option<Card> {
    let _ = tool_use_id; // reserved for Phase 4 tool-name lookup
    let cwd_for_help = extract_cwd(line, meta);
    let help = match_help_for_tool_error(body, &cwd_for_help);
    let severity = severity_for_tool_error_help(help.as_ref());
    let title = title_for_tool_error(body);
    let subtitle = first_line(body).map(|s| truncate(&redact_secrets(s), 120));

    Some(Card {
        id: None,
        session_path: meta.session_path.clone(),
        event_uuid: line.get("uuid").and_then(Value::as_str).map(String::from),
        byte_offset,
        kind: CardKind::ToolError,
        ts: parse_ts(line)?,
        severity,
        title,
        subtitle,
        help,
        source_ref: None,
        cwd: extract_cwd(line, meta),
        git_branch: extract_git_branch(line, meta),
        // Tool errors do not have an obvious plugin signal at this
        // layer (the parent assistant's tool_use carries the name).
        // Phase 4 source-ref work could thread plugin attribution
        // through here when the tool name is known.
        plugin: None,
    })
}

/// Map a tool-error body to a help template. Returns `None` when no
/// template matches — better to ship the card without help than to
/// fabricate guidance.
///
/// `cwd` is the session's current working directory; some templates
/// (e.g. `tool.no_such_file`) include it in the rendered help so
/// the user knows where the path lookup happened.
fn match_help_for_tool_error(body: &str, cwd: &Path) -> Option<HelpRef> {
    use std::collections::BTreeMap;
    // Read-required: most common tool error on the reference machine
    // (~400 instances). Two variants — "not been read" and "modified
    // since read" — both diagnose the same situation: model needs to
    // re-read and retry, no user action.
    if body.contains("File has not been read yet")
        || body.contains("File has been modified since read")
    {
        let mut args = BTreeMap::new();
        if let Some(file) = extract_path_from_body(body) {
            args.insert("file".to_string(), redact_secrets(&file));
        }
        return Some(HelpRef {
            template_id: "tool.read_required".to_string(),
            args,
        });
    }
    if body.contains("Cancelled: parallel tool call") {
        return Some(HelpRef {
            template_id: "tool.parallel_cancelled".to_string(),
            args: BTreeMap::new(),
        });
    }
    if body.contains("ssh: connect to host") && body.contains("timed out") {
        let mut args = BTreeMap::new();
        if let Some(host) = extract_ssh_host(body) {
            args.insert("host".to_string(), host);
        }
        return Some(HelpRef {
            template_id: "tool.ssh_timeout".to_string(),
            args,
        });
    }
    if body.contains("String to replace not found") {
        let mut args = BTreeMap::new();
        if let Some(file) = extract_path_from_body(body) {
            args.insert("file".to_string(), redact_secrets(&file));
        }
        return Some(HelpRef {
            template_id: "tool.edit_drift".to_string(),
            args,
        });
    }
    if body.contains("user doesn't want to proceed") {
        return Some(HelpRef {
            template_id: "tool.user_rejected".to_string(),
            args: BTreeMap::new(),
        });
    }
    if body.contains("command not found") {
        let mut args = BTreeMap::new();
        if let Some(cmd) = extract_missing_command(body) {
            let cmd_redacted = redact_secrets(&cmd);
            if let Some(pkg) = brew_install_hint(&cmd_redacted) {
                args.insert("brew_install_hint".to_string(), pkg.to_string());
            }
            args.insert("command".to_string(), cmd_redacted);
        }
        return Some(HelpRef {
            template_id: "tool.bash_cmd_not_found".to_string(),
            args,
        });
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("no such file or directory") {
        let mut args = BTreeMap::new();
        if let Some(path) = extract_no_such_file_path(body) {
            args.insert("path".to_string(), redact_secrets(&path));
        }
        // Always carry the cwd context — the renderer omits it when
        // the path looks absolute, but the template prefers having
        // it for a relative path so the user can disambiguate.
        let cwd_str = cwd.to_string_lossy();
        if !cwd_str.is_empty() {
            args.insert("cwd".to_string(), redact_secrets(&cwd_str));
        }
        return Some(HelpRef {
            template_id: "tool.no_such_file".to_string(),
            args,
        });
    }
    None
}

/// Pull the offending path from a "no such file or directory" body.
/// CC-shaped messages typically end with `... no such file or
/// directory: <path>` (e.g. `(eval):2: no such file or directory:
/// /tmp/missing`). Returns `None` when no path is parseable.
fn extract_no_such_file_path(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let needle = "no such file or directory";
    let idx = lower.find(needle)?;
    let rest = &body[idx + needle.len()..];
    // Skip the colon-and-space delimiter.
    let trimmed = rest.trim_start_matches([':', ' ']).trim();
    if trimmed.is_empty() {
        return None;
    }
    // Stop at first newline.
    let end = trimmed.find('\n').unwrap_or(trimmed.len());
    let candidate = trimmed[..end].trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

fn severity_for_tool_error_help(help: Option<&HelpRef>) -> Severity {
    match help.map(|h| h.template_id.as_str()) {
        // Read-required and parallel-cancelled are pure noise — model
        // recovers automatically. Render as Info so the default view
        // can hide them with a "warn or above" filter.
        Some("tool.read_required") | Some("tool.parallel_cancelled") => Severity::Info,
        Some("tool.user_rejected") => Severity::Notice,
        // Everything else (ssh timeout, no such file, edit drift, cmd
        // not found, unmatched) is a real failure the user might
        // want to act on.
        _ => Severity::Warn,
    }
}

fn title_for_tool_error(body: &str) -> String {
    // First line of the body is usually the most informative — the
    // <tool_use_error> tag wraps a sentence the model wrote for
    // itself ("File has been modified since read…"). Strip the tag
    // so the title is human-readable. Redact BEFORE truncation so a
    // tool error that echoes a token (e.g. an HTTP failure with
    // an Authorization header) doesn't surface the secret in the
    // card title — the title is persisted to sessions.db.
    let head = first_line(body).unwrap_or("Tool error");
    let stripped = head
        .trim_start_matches("<tool_use_error>")
        .trim_end_matches("</tool_use_error>")
        .trim();
    let one_line = if stripped.is_empty() {
        "Tool error".to_string()
    } else {
        redact_secrets(stripped)
    };
    truncate(&one_line, 80)
}

/// Best-effort extractor: pulls `file_path: "..."` or "in <path>" out
/// of a tool-error body. Returns `None` when no obvious path is found.
fn extract_path_from_body(body: &str) -> Option<String> {
    // "in <path>" pattern (used by edit_drift)
    if let Some(idx) = body.find(" in ") {
        let rest = &body[idx + 4..];
        let end = rest
            .find(|c: char| c == '\n' || c == '.' || c == ')' || c == ',')
            .unwrap_or(rest.len());
        let candidate = rest[..end].trim();
        if candidate.starts_with('/') || candidate.starts_with('~') {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Pull the host from `ssh: connect to host <host>` (with or without
/// a trailing port).
fn extract_ssh_host(body: &str) -> Option<String> {
    let needle = "ssh: connect to host ";
    let idx = body.find(needle)?;
    let rest = &body[idx + needle.len()..];
    let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    let host = rest[..end].trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Map a missing command name to its Homebrew package name when the
/// two differ. Most CLI tools install under their own name, so the
/// table only carries the well-known exceptions. `None` is the
/// honest "we don't know" signal — the renderer falls back to a
/// generic install message rather than guess wrong.
fn brew_install_hint(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        // Tools whose binary name matches their brew formula —
        // safe to suggest unconditionally.
        "rg" => "ripgrep",
        "fd" => "fd",
        "fzf" => "fzf",
        "jq" => "jq",
        "yq" => "yq",
        "gh" => "gh",
        "tree" => "tree",
        "wget" => "wget",
        "tmux" => "tmux",
        "htop" => "htop",
        "ncdu" => "ncdu",
        "bat" => "bat",
        "exa" => "eza",
        "eza" => "eza",
        "delta" => "git-delta",
        "shellcheck" => "shellcheck",
        "ffmpeg" => "ffmpeg",
        "imagemagick" | "convert" => "imagemagick",
        "pandoc" => "pandoc",
        "qpdf" => "qpdf",
        "pdftk" => "pdftk-java",
        "gs" => "ghostscript",
        _ => return None,
    })
}

/// Pull the missing command from `bash: <cmd>: command not found` or
/// `<cmd>: command not found`.
fn extract_missing_command(body: &str) -> Option<String> {
    let needle = ": command not found";
    let idx = body.find(needle)?;
    let prefix = &body[..idx];
    // Trim back to the start of the line, then strip an optional
    // shell-name prefix like "bash:" / "zsh:" / "(eval):2:".
    let line_start = prefix.rfind('\n').map(|p| p + 1).unwrap_or(0);
    let raw = &prefix[line_start..];
    // Common shell prefixes: "bash: foo", "(eval):1: foo", "zsh: foo".
    let stripped = raw
        .splitn(2, ':')
        .nth(1)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(raw.trim());
    if stripped.is_empty() {
        None
    } else {
        // If the rest still has colon-prefixed garbage, take the last
        // word — that's typically the actual command.
        let last_word = stripped.split_whitespace().last().unwrap_or(stripped);
        Some(last_word.to_string())
    }
}

/// Pull the textual content out of a `tool_result` block. CC writes
/// either a string or an array of `{type:"text", text:"..."}` blocks
/// — we handle both shapes.
fn extract_tool_result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn extract_cwd(line: &Value, meta: &SessionMeta) -> PathBuf {
    line.get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| meta.cwd.clone())
}

fn extract_git_branch(line: &Value, meta: &SessionMeta) -> Option<String> {
    line.get("gitBranch")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| meta.git_branch.clone())
}

/// Best-effort plugin attribution from a hook attachment. CC's hook
/// `command` field on plugin-supplied hooks contains the literal
/// `${CLAUDE_PLUGIN_ROOT}` substring, and the path that follows
/// resolves under `~/.claude/plugins/cache/<owner>/<name>/<version>/`
/// at runtime. Both forms work as a signature: the env-var literal
/// or the cache path. We take the first signal we find.
///
/// Returns `Some("<name>@<owner>")` when extractable, `None` when
/// the command is from outside the plugin system (e.g. a bare
/// `bash …` script).
fn plugin_from_hook(attachment: &Value) -> Option<String> {
    let command = attachment.get("command").and_then(Value::as_str)?;
    plugin_from_command_string(command)
}

/// Pull plugin slug from a hook `command` string. Two real-world
/// shapes seen in CC's stderr:
/// 1. `bash ${CLAUDE_PLUGIN_ROOT}/scripts/foo.sh` — the CLAUDE_PLUGIN_ROOT
///    literal alone doesn't name the plugin (CC resolves it at
///    runtime). Returns None — caller can fall back to other
///    signals when present (e.g. the corresponding plugin_missing
///    stderr already carries the slug).
/// 2. `bash /Users/<user>/.claude/plugins/cache/<owner>/<name>/<ver>/scripts/foo.sh`
///    — the path encodes owner + name. Returns `<name>@<owner>`.
fn plugin_from_command_string(command: &str) -> Option<String> {
    if let Some(idx) = command.find("/plugins/cache/") {
        let rest = &command[idx + "/plugins/cache/".len()..];
        let mut parts = rest.split('/');
        let owner = parts.next()?;
        let name = parts.next()?;
        if !owner.is_empty() && !name.is_empty() {
            return Some(format!("{name}@{owner}"));
        }
    }
    None
}

/// Plugin attribution from a slash-command name (`/plugin:cmd`). CC
/// namespaces plugin commands with the plugin's installed name as
/// the prefix, separated by `:`. Returns just the plugin name (no
/// `@<owner>` suffix — that lookup requires the installed plugins
/// registry, which lives outside this hot path).
fn plugin_from_namespaced_name(name: &str) -> Option<String> {
    let (prefix, rest) = name.split_once(':')?;
    if prefix.is_empty() || rest.is_empty() {
        return None;
    }
    Some(prefix.to_string())
}

/// Per-attachment-type flavor for hook failure classification.
/// Decouples the title/severity selection from the JSON shape parsing.
#[derive(Debug, Clone, Copy)]
enum HookFailureFlavor {
    NonBlocking,
    Blocking,
    Cancelled,
    ExecutionError,
    StoppedContinuation,
}

impl HookFailureFlavor {
    fn severity(self) -> Severity {
        match self {
            // Blocking error and execution-error mean CC actually
            // halted or the hook itself crashed — both warrant Error.
            // Stopped-continuation also halts the assistant.
            Self::Blocking | Self::ExecutionError | Self::StoppedContinuation => Severity::Error,
            // Non-blocking failures and cancellations are warn —
            // CC kept running.
            Self::NonBlocking => Severity::Warn,
            Self::Cancelled => Severity::Notice,
        }
    }
    fn title_prefix(self) -> &'static str {
        match self {
            Self::NonBlocking => "Hook failed: ",
            Self::Blocking => "Hook BLOCKED: ",
            Self::Cancelled => "Hook cancelled: ",
            Self::ExecutionError => "Hook crashed: ",
            Self::StoppedContinuation => "Hook stopped continuation: ",
        }
    }
}

/// Build a `HookFailure` card from any of the five hook-failure
/// attachment families. Returns `None` if the line is missing the
/// fields we depend on (defensive against schema drift).
fn classify_hook_failure(
    line: &Value,
    attachment: &Value,
    byte_offset: u64,
    meta: &SessionMeta,
    flavor: HookFailureFlavor,
) -> Option<Card> {
    // `hookEvent` and `hookName` are both required across every
    // failure variant. If either is absent the record is malformed
    // — return None rather than fabricate a card.
    let _hook_event = attachment.get("hookEvent").and_then(Value::as_str)?;
    let hook_name = attachment.get("hookName").and_then(Value::as_str)?;
    let stderr = attachment
        .get("stderr")
        .and_then(Value::as_str)
        .unwrap_or("");
    let exit_code = attachment.get("exitCode").and_then(Value::as_i64);
    let command = attachment
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string);

    // CC's `hookName` is the informative form — already encodes the
    // event prefix when a tool matcher exists ("PostToolUse:Write")
    // and stands on its own otherwise ("SessionStart:clear",
    // "UserPromptSubmit"). Use it directly.
    let title = format!("{}{hook_name}", flavor.title_prefix());

    // Subtitle: prefer the first non-empty line of stderr (capped),
    // otherwise the command. Both must be redacted before crossing
    // into the persistence boundary — design v2 §14 makes this
    // non-negotiable. A hook script that echoes a token in its
    // stderr would otherwise leak the token into sessions.db.
    let subtitle = first_line(stderr)
        .map(|s| truncate(&redact_secrets(s), 120))
        .or_else(|| {
            command
                .as_deref()
                .map(|c| truncate(&redact_secrets(c), 120))
        });

    let help = match_help_for_hook(stderr, exit_code);

    // Plugin attribution: prefer the slug extracted from a
    // plugin_missing stderr (always present in the parenthesized
    // form), fall back to the cache-path signature in `command`.
    let plugin = extract_missing_plugin(stderr).or_else(|| plugin_from_hook(attachment));

    let ts = parse_ts(line)?;
    let event_uuid = line.get("uuid").and_then(Value::as_str).map(str::to_string);
    let cwd = line
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| meta.cwd.clone());
    let git_branch = line
        .get("gitBranch")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| meta.git_branch.clone());

    Some(Card {
        id: None,
        session_path: meta.session_path.clone(),
        event_uuid,
        byte_offset,
        kind: CardKind::HookFailure,
        ts,
        severity: flavor.severity(),
        title,
        subtitle,
        help,
        source_ref: derive_hook_source_ref(&cwd),
        cwd,
        git_branch,
        plugin,
    })
}

/// Best-effort `SourceRef` for a hook failure. Walks CC's known
/// settings cascade for the project cwd and returns the first file
/// that exists, ranked by match probability:
///
///   1. `<cwd>/.claude/settings.local.json` (Local scope)
///   2. `<cwd>/.claude/settings.json` (Project scope)
///   3. `~/.claude/settings.json` (User scope) — most common
///
/// Line number is unset — JSON byte-offset tracking lives behind
/// a parser refactor (Phase 6 in the design doc). The GUI's
/// click-through opens the file at line 1; users see the file and
/// can search for the offending hook by name.
///
/// Returns `None` only when no settings file at any layer exists,
/// which is genuinely the case (hook came from a since-removed
/// settings file) — never fabricates a path.
fn derive_hook_source_ref(cwd: &Path) -> Option<SourceRef> {
    let candidates: [(PathBuf, ConfigScope); 3] = [
        (
            cwd.join(".claude").join("settings.local.json"),
            ConfigScope::Local,
        ),
        (
            cwd.join(".claude").join("settings.json"),
            ConfigScope::Project,
        ),
        (
            crate::paths::claude_config_dir().join("settings.json"),
            ConfigScope::User,
        ),
    ];
    for (path, scope) in candidates {
        if path.exists() {
            return Some(SourceRef {
                path,
                line: None,
                scope,
            });
        }
    }
    None
}

/// Map a hook failure's stderr/exit-code to a help template.
///
/// Phase 2 catalog: `hook.plugin_missing` and `hook.json_invalid`.
/// Other patterns surface as cards without help — honest "we don't
/// have advice for this yet" rather than fabricated guidance.
///
/// Every arg value derived from stderr is run through the redactor
/// before crossing the persistence boundary.
fn match_help_for_hook(stderr: &str, _exit_code: Option<i64>) -> Option<HelpRef> {
    use std::collections::BTreeMap;
    if let Some(plugin) = extract_missing_plugin(stderr) {
        let mut args = BTreeMap::new();
        args.insert("plugin".to_string(), redact_secrets(&plugin));
        return Some(HelpRef {
            template_id: "hook.plugin_missing".to_string(),
            args,
        });
    }
    if stderr.contains("Hook JSON output validation failed")
        || stderr.contains("hookSpecificOutput")
    {
        let mut args = BTreeMap::new();
        if let Some(detail) = extract_json_validation_detail(stderr) {
            args.insert("detail".to_string(), redact_secrets(&detail));
        }
        return Some(HelpRef {
            template_id: "hook.json_invalid".to_string(),
            args,
        });
    }
    None
}

/// Pull the most informative line out of a CC schema-validation
/// stderr ("- : Invalid input" / "- /hookSpecificOutput: ..."). The
/// raw stderr can run several lines and includes a JSON dump of
/// what the hook produced; we want the one-line "what the schema
/// rejected." Returns `None` when no specific line stands out.
fn extract_json_validation_detail(stderr: &str) -> Option<String> {
    // Match the typical CC line: "- /<json-pointer>: <message>" or
    // "- : <message>". Prefer the first line that looks shaped.
    for raw in stderr.lines() {
        let line = raw.trim();
        if line.starts_with("- ") {
            let rest = line.trim_start_matches("- ").trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Extract the plugin slug from a CC "Plugin directory does not exist"
/// stderr message. Returns `None` if the pattern doesn't match.
///
/// Two real-world shapes seen on this machine:
/// 1. `Failed to run: Plugin directory does not exist: /Users/joker/.claude/plugins/cache/xiaolai/mermaid-preview/0.1.1 (mermaid-preview@xiaolai — run /plugin to reinstall)`
/// 2. The same with a different path / plugin slug.
///
/// We prefer the parenthesized "<slug>@<owner>" form over the path
/// derivation because that's what /plugin commands accept verbatim.
fn extract_missing_plugin(stderr: &str) -> Option<String> {
    if !stderr.contains("Plugin directory does not exist") {
        return None;
    }
    // Pull `<slug>@<owner>` out of `(mermaid-preview@xiaolai — run /plugin to reinstall)`.
    if let Some(open) = stderr.find('(') {
        let rest = &stderr[open + 1..];
        if let Some(dash) = rest.find('—').or_else(|| rest.find(" - ")) {
            let inner = rest[..dash].trim().trim_end_matches(',');
            if !inner.is_empty() {
                return Some(inner.to_string());
            }
        }
    }
    // Fallback: derive from the cache path
    // (`/.../plugins/cache/<owner>/<name>/<version>`).
    if let Some(idx) = stderr.find("/plugins/cache/") {
        let rest = &stderr[idx + "/plugins/cache/".len()..];
        let mut parts = rest.split('/');
        let owner = parts.next()?;
        let name = parts.next()?;
        if !owner.is_empty() && !name.is_empty() {
            return Some(format!("{name}@{owner}"));
        }
    }
    None
}

fn parse_ts(line: &Value) -> Option<DateTime<Utc>> {
    let raw = line.get("timestamp").and_then(Value::as_str)?;
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn first_line(s: &str) -> Option<&str> {
    s.lines().find(|l| !l.trim().is_empty())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
#[path = "classifier_tests.rs"]
mod tests;
