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
use std::path::PathBuf;

use super::card::{Card, CardKind, HelpRef, Severity};
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
    /// Phase 2+: dedup `nested_memory` loads per session. Already
    /// present so the suppression rule has somewhere to record state
    /// when it's enabled.
    pub seen_rules: HashSet<PathBuf>,
    /// Phase 3+: open `Agent` tool_uses keyed by `tool_use.id`,
    /// drained at session-end into `AgentStranded` cards.
    pub open_episodes: HashMap<String, OpenEpisode>,
}

/// Phase 3 placeholder — a tool_use that hasn't seen its matching
/// tool_result yet.
#[derive(Debug, Clone)]
pub struct OpenEpisode {
    pub tool_use_id: String,
    pub tool_name: String,
    pub opened_at: DateTime<Utc>,
    pub byte_offset: u64,
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
    _state: &mut ClassifierState,
) -> Vec<Card> {
    // Fast-path: only `attachment` records can produce v1 cards.
    // Every other type returns immediately, keeping the per-line
    // cost ≤1µs on 99% of input.
    let entry_type = line.get("type").and_then(Value::as_str).unwrap_or("");
    if entry_type != "attachment" {
        return Vec::new();
    }

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
        // Phase 2+ adds the remaining attachment.type variants
        // (success-slow, additional_context, system_message, etc.).
        // Unknown attachment types are silently skipped — never emit
        // a placeholder.
        _ => Vec::new(),
    }
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

    let ts = parse_ts(line)?;
    let event_uuid = line
        .get("uuid")
        .and_then(Value::as_str)
        .map(str::to_string);
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
        source_ref: None, // Phase 2 adds settings-layer resolution
        cwd,
        git_branch,
    })
}

/// Map a hook failure's stderr/exit-code to a help template.
///
/// v1 catalog: `hook.plugin_missing` only. Other patterns surface as
/// cards without help — honest "we don't have advice for this yet"
/// rather than fabricated guidance.
///
/// Every arg value derived from stderr is run through the redactor
/// before crossing the persistence boundary. The current extractor
/// can only return a `<slug>@<owner>` pair which is safe by shape,
/// but the redact pass keeps the contract honest under future
/// extractors that pull more freeform text.
fn match_help_for_hook(stderr: &str, _exit_code: Option<i64>) -> Option<HelpRef> {
    if let Some(plugin) = extract_missing_plugin(stderr) {
        let mut args = std::collections::BTreeMap::new();
        args.insert("plugin".to_string(), redact_secrets(&plugin));
        return Some(HelpRef {
            template_id: "hook.plugin_missing".to_string(),
            args,
        });
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
mod tests {
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
}
