//! Codex rollout decoding: `parse_head`, `iter_events`,
//! `parse_codex_rollout_jsonl`.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::error::CodexError;
use super::types::{
    CodexConversation, CodexEvent, CodexExchange, CodexHead, CodexToolCall,
    EnvironmentTextKind,
};

/// Read just enough of the file to populate a `CodexHead`. Reads
/// the first `session_meta` record and the first `turn_context`
/// record (if any) and then stops — never walks the whole file.
///
/// Errors:
/// * `Io` for filesystem / read errors.
/// * `MissingSessionMeta` if no `session_meta` record appears before
///   the head-window cap (4096 lines).
/// * `MissingSessionId` if the meta record is present but its
///   `payload.id` is absent.
pub fn parse_head(path: &Path) -> Result<CodexHead, CodexError> {
    const HEAD_WINDOW_LINES: u32 = 4096;

    let iter = iter_events(path)?;
    let mut head: Option<CodexHead> = None;
    let mut scanned: u32 = 0;

    for event in iter {
        scanned += 1;
        match event {
            CodexEvent::SessionMeta {
                session_id,
                cwd,
                originator,
                cli_version,
                timestamp,
                ..
            } => {
                if head.is_none() {
                    head = Some(CodexHead {
                        session_id,
                        cwd,
                        originator,
                        cli_version,
                        started_at: timestamp,
                        approval_policy: None,
                        sandbox_mode: None,
                        rollout_schema_version: None,
                    });
                }
            }
            CodexEvent::TurnContext {
                cwd,
                approval_policy,
                sandbox_mode,
                ..
            } => {
                if let Some(ref mut h) = head {
                    if h.cwd.is_none() {
                        h.cwd = cwd;
                    }
                    if h.approval_policy.is_none() {
                        h.approval_policy = approval_policy;
                    }
                    if h.sandbox_mode.is_none() {
                        h.sandbox_mode = sandbox_mode;
                    }
                    // First turn_context after the meta is
                    // enough; bail out so head parsing stays cheap
                    // on long rollouts.
                    return Ok(h.clone());
                }
            }
            _ => {}
        }
        if scanned >= HEAD_WINDOW_LINES {
            break;
        }
    }

    match head {
        Some(h) => Ok(h),
        None => Err(CodexError::MissingSessionMeta {
            path: path.to_path_buf(),
        }),
    }
}

/// Stream every JSONL line as a `CodexEvent`. Malformed lines are
/// silently skipped (the line counter still advances). Returns an
/// iterator rather than collecting because rollouts can be tens of
/// megabytes and the indexer can fold over the stream.
pub fn iter_events(path: &Path) -> Result<EventIter, CodexError> {
    let file = File::open(path).map_err(|source| CodexError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(EventIter {
        reader: BufReader::new(file),
        line_no: 0,
        buf: String::new(),
        done: false,
    })
}

/// Full-file collector. Walks `iter_events`, builds exchanges by
/// pairing genuine user prompts with subsequent assistant messages
/// and tool calls. The head is parsed independently from the same
/// file so callers only need to open it once.
pub fn parse_codex_rollout_jsonl(path: &Path) -> Result<CodexConversation, CodexError> {
    let mut head = parse_head_collecting(path)?;
    let session_id = head.session_id.clone();

    let mut exchanges: Vec<CodexExchange> = Vec::new();
    let mut current: Option<ExchangeBuilder> = None;
    let mut pending_calls: Vec<CodexToolCall> = Vec::new();

    for event in iter_events(path)? {
        match event {
            CodexEvent::SessionMeta { .. } | CodexEvent::Other { .. } => {}
            CodexEvent::TurnContext {
                cwd,
                approval_policy,
                sandbox_mode,
                ..
            } => {
                if head.cwd.is_none() {
                    head.cwd = cwd;
                }
                if head.approval_policy.is_none() {
                    head.approval_policy = approval_policy;
                }
                if head.sandbox_mode.is_none() {
                    head.sandbox_mode = sandbox_mode;
                }
            }
            CodexEvent::UserMessage {
                text,
                kind,
                timestamp,
                line,
            } => {
                if kind.is_turn_seed() {
                    // Genuine user prompt (cli) or IDE-wrapped
                    // user prompt (codex_vscode) — close the
                    // previous exchange and start a new one.
                    if let Some(b) = current.take() {
                        exchanges.push(b.finish(&pending_calls));
                        pending_calls.clear();
                    }
                    current = Some(ExchangeBuilder::new(
                        &session_id,
                        exchanges.len() as u32,
                        text,
                        timestamp,
                        line,
                    ));
                } else {
                    // Synthetic seed message (Instructions /
                    // Environment) — fold into the current
                    // exchange's environment trail without
                    // starting a new turn.
                    if let Some(ref mut b) = current {
                        b.extend_user_environment(&text);
                        b.line_end = Some(line);
                    }
                }
            }
            CodexEvent::AssistantMessage {
                text,
                timestamp,
                line,
            } => {
                if let Some(ref mut b) = current {
                    b.append_assistant(&text, timestamp, line);
                }
            }
            CodexEvent::FunctionCall {
                call_id,
                name,
                arguments,
                timestamp,
                line,
            } => {
                pending_calls.push(CodexToolCall {
                    call_id,
                    name,
                    arguments,
                    output: None,
                    is_error: false,
                    timestamp,
                    call_line: line,
                    output_line: None,
                });
                if let Some(ref mut b) = current {
                    b.line_end = Some(line);
                }
            }
            CodexEvent::FunctionCallOutput {
                call_id,
                output,
                is_error,
                line,
                ..
            } => {
                if let Some(slot) = pending_calls
                    .iter_mut()
                    .rev()
                    .find(|c| c.call_id == call_id && c.output.is_none())
                {
                    slot.output = Some(output);
                    slot.is_error = is_error;
                    slot.output_line = Some(line);
                }
                if let Some(ref mut b) = current {
                    b.line_end = Some(line);
                }
            }
        }
    }

    if let Some(b) = current.take() {
        exchanges.push(b.finish(&pending_calls));
    }

    Ok(CodexConversation { head, exchanges })
}

/// Identical contract to `parse_head` but returns a fresh
/// `CodexHead` we own (not a clone) so the caller can mutate
/// `cwd` / `approval_policy` / `sandbox_mode` as it discovers
/// later `turn_context` records.
fn parse_head_collecting(path: &Path) -> Result<CodexHead, CodexError> {
    let head = parse_head(path)?;
    if head.session_id.is_empty() {
        return Err(CodexError::MissingSessionId {
            path: path.to_path_buf(),
        });
    }
    Ok(head)
}

/// Streaming iterator over JSONL lines as decoded events.
pub struct EventIter {
    reader: BufReader<File>,
    line_no: u32,
    buf: String,
    done: bool,
}

impl Iterator for EventIter {
    type Item = CodexEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            self.buf.clear();
            match self.reader.read_line(&mut self.buf) {
                Ok(0) => {
                    self.done = true;
                    return None;
                }
                Ok(_) => {
                    self.line_no += 1;
                    let trimmed = self.buf.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Some(event) = decode_line(trimmed, self.line_no) {
                        return Some(event);
                    }
                    // Malformed — skip silently, the line counter
                    // has already advanced.
                }
                Err(_) => {
                    // I/O hiccup mid-stream. The iterator surface
                    // doesn't propagate errors; we end the stream
                    // and trust the indexer to detect via the
                    // staleness triple that the file should be
                    // re-scanned next time.
                    self.done = true;
                    return None;
                }
            }
        }
    }
}

fn decode_line(raw: &str, line: u32) -> Option<CodexEvent> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let type_tag = v.get("type").and_then(Value::as_str)?.to_string();
    let timestamp = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp);
    let payload = v.get("payload");

    match type_tag.as_str() {
        "session_meta" => Some(decode_session_meta(payload?, timestamp, line)),
        "turn_context" => Some(decode_turn_context(payload?, timestamp, line)),
        "response_item" => decode_response_item(payload?, timestamp, line),
        other => Some(CodexEvent::Other {
            type_tag: other.to_string(),
            line,
        }),
    }
}

fn decode_session_meta(
    payload: &Value,
    timestamp: Option<DateTime<Utc>>,
    line: u32,
) -> CodexEvent {
    let session_id = payload
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let originator = payload
        .get("originator")
        .and_then(Value::as_str)
        .map(String::from);
    let cli_version = payload
        .get("cli_version")
        .and_then(Value::as_str)
        .map(String::from);
    let timestamp = payload
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
        .or(timestamp);
    CodexEvent::SessionMeta {
        session_id,
        cwd,
        originator,
        cli_version,
        timestamp,
        line,
    }
}

fn decode_turn_context(
    payload: &Value,
    timestamp: Option<DateTime<Utc>>,
    line: u32,
) -> CodexEvent {
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let approval_policy = payload
        .get("approval_policy")
        .and_then(Value::as_str)
        .map(String::from);
    let sandbox_mode = payload
        .get("sandbox_policy")
        .and_then(|p| p.get("mode"))
        .and_then(Value::as_str)
        .map(String::from);
    CodexEvent::TurnContext {
        cwd,
        approval_policy,
        sandbox_mode,
        timestamp,
        line,
    }
}

fn decode_response_item(
    payload: &Value,
    timestamp: Option<DateTime<Utc>>,
    line: u32,
) -> Option<CodexEvent> {
    let inner_type = payload.get("type").and_then(Value::as_str)?;
    match inner_type {
        "message" => {
            let role = payload.get("role").and_then(Value::as_str)?;
            let text = extract_message_text(payload.get("content")?)?;
            match role {
                "user" => {
                    let kind = classify_user_text(&text);
                    Some(CodexEvent::UserMessage {
                        text,
                        kind,
                        timestamp,
                        line,
                    })
                }
                "assistant" => Some(CodexEvent::AssistantMessage {
                    text,
                    timestamp,
                    line,
                }),
                _ => None,
            }
        }
        "function_call" => {
            let call_id = payload
                .get("call_id")
                .and_then(Value::as_str)?
                .to_string();
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let arguments = payload
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(CodexEvent::FunctionCall {
                call_id,
                name,
                arguments,
                timestamp,
                line,
            })
        }
        "function_call_output" => {
            let call_id = payload
                .get("call_id")
                .and_then(Value::as_str)?
                .to_string();
            let output = payload
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let is_error = detect_function_error(&output);
            Some(CodexEvent::FunctionCallOutput {
                call_id,
                output,
                is_error,
                timestamp,
                line,
            })
        }
        _ => None,
    }
}

fn extract_message_text(content: &Value) -> Option<String> {
    let arr = content.as_array()?;
    let mut buf = String::new();
    let mut first = true;
    for item in arr {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "input_text" | "output_text" | "text") {
            continue;
        }
        if let Some(t) = item.get("text").and_then(Value::as_str) {
            if !first {
                buf.push('\n');
            }
            buf.push_str(t);
            first = false;
        }
    }
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

fn classify_user_text(text: &str) -> EnvironmentTextKind {
    let head = text.trim_start();
    if head.starts_with("<user_instructions>") {
        EnvironmentTextKind::Instructions
    } else if head.starts_with("<environment_context>") {
        EnvironmentTextKind::Environment
    } else if head.starts_with("# Context from") {
        EnvironmentTextKind::IdeContext
    } else {
        EnvironmentTextKind::UserPrompt
    }
}

fn detect_function_error(output: &str) -> bool {
    let v: Value = match serde_json::from_str(output) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let exit_code = v
        .get("metadata")
        .and_then(|m| m.get("exit_code"))
        .and_then(Value::as_i64);
    matches!(exit_code, Some(c) if c != 0)
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

// ─── Exchange builder ─────────────────────────────────────────

struct ExchangeBuilder {
    session_id: String,
    turn_index: u32,
    user_text: String,
    environment_trail: String,
    assistant_text: String,
    timestamp: Option<DateTime<Utc>>,
    line_start: Option<u32>,
    line_end: Option<u32>,
}

impl ExchangeBuilder {
    fn new(
        session_id: &str,
        turn_index: u32,
        user_text: String,
        timestamp: Option<DateTime<Utc>>,
        line: u32,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            turn_index,
            user_text,
            environment_trail: String::new(),
            assistant_text: String::new(),
            timestamp,
            line_start: Some(line),
            line_end: Some(line),
        }
    }

    fn extend_user_environment(&mut self, text: &str) {
        if !self.environment_trail.is_empty() {
            self.environment_trail.push('\n');
        }
        self.environment_trail.push_str(text);
    }

    fn append_assistant(
        &mut self,
        text: &str,
        timestamp: Option<DateTime<Utc>>,
        line: u32,
    ) {
        if !self.assistant_text.is_empty() {
            self.assistant_text.push('\n');
        }
        self.assistant_text.push_str(text);
        if self.timestamp.is_none() {
            self.timestamp = timestamp;
        }
        self.line_end = Some(line);
    }

    fn finish(self, pending: &[CodexToolCall]) -> CodexExchange {
        // Combine the user prompt with any environment trail so
        // downstream FTS can still surface the IDE-context block,
        // but the displayable "user said" stays in `user_text`.
        // We keep them separate at this layer; the durable
        // indexer (WI-003) chooses whether to concatenate them
        // into the FTS text columns.
        let _ = &self.environment_trail; // reserved for indexer use
        CodexExchange {
            id: format!("{}:{}", self.session_id, self.turn_index),
            turn_index: self.turn_index,
            user_text: self.user_text,
            assistant_text: self.assistant_text,
            timestamp: self.timestamp,
            line_start: self.line_start,
            line_end: self.line_end,
            tool_calls: pending.to_vec(),
        }
    }
}
