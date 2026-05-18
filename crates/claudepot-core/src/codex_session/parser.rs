//! Codex rollout decoding: `parse_head`, `iter_events`,
//! `parse_codex_rollout_jsonl`.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::error::CodexError;
use super::types::{
    CodexConversation, CodexEvent, CodexExchange, CodexHead, CodexToolCall, EnvironmentTextKind,
    ParseDiagnostics,
};

/// Maximum bytes the parser will read for a single JSONL line.
/// Codex rollouts in the wild peak at a few kilobytes per line; 1
/// MiB is three orders of magnitude above any real value and well
/// below the OOM threshold for a typical desktop process.
///
/// A line longer than this cap is treated as adversarial input:
/// we drain the rest of the line to the next `\n` without
/// allocating it, emit a malformed-line signal so the indexer can
/// surface the per-file failure, and continue. The file's
/// staleness triple is NOT stamped when a truncated line is seen
/// (per H6 / M14), so the next backfill retries.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

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
                    // Honor the documented contract: an empty
                    // `payload.id` is treated as MissingSessionId.
                    // Previously parse_head returned Ok with an
                    // empty session_id and only parse_head_collecting
                    // caught it, contradicting the doc comment.
                    if session_id.is_empty() {
                        return Err(CodexError::MissingSessionId {
                            path: path.to_path_buf(),
                        });
                    }
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
/// silently skipped (the line counter still advances) but counted
/// in [`EventIter::diagnostics`]. Returns an iterator rather than
/// collecting because rollouts can be tens of megabytes and the
/// indexer can fold over the stream.
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
        diagnostics: ParseDiagnostics::default(),
    })
}

/// Full-file collector. Walks `iter_events`, builds exchanges by
/// pairing genuine user prompts with subsequent assistant messages
/// and tool calls. The head is parsed independently from the same
/// file so callers only need to open it once. The returned
/// `CodexConversation.diagnostics` carries any parse-quality
/// signals; the indexer reads them to decide stamping behavior.
pub fn parse_codex_rollout_jsonl(path: &Path) -> Result<CodexConversation, CodexError> {
    let mut head = parse_head_collecting(path)?;
    let session_id = head.session_id.clone();

    let mut exchanges: Vec<CodexExchange> = Vec::new();
    let mut current: Option<ExchangeBuilder> = None;
    let mut pending_calls: Vec<CodexToolCall> = Vec::new();

    let mut iter = iter_events(path)?;
    // `by_ref()` so we can still read `iter.diagnostics()` after
    // the loop drains it.
    for event in iter.by_ref() {
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
                    // Environment) — extend the current exchange's
                    // physical line range without starting a new
                    // turn. The text itself is dropped; we'll add
                    // a real consumer (separate indexer column or
                    // sidechain table) the day there's one.
                    if let Some(ref mut b) = current {
                        let _ = &text;
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

    let diagnostics = iter.diagnostics().clone();
    if diagnostics.malformed_lines > 0
        || diagnostics.oversize_lines > 0
        || diagnostics.truncated_by_io
    {
        tracing::warn!(
            path = %path.display(),
            malformed = diagnostics.malformed_lines,
            oversize = diagnostics.oversize_lines,
            truncated_by_io = diagnostics.truncated_by_io,
            "codex_session: parse completed with diagnostics"
        );
    }
    Ok(CodexConversation {
        head,
        exchanges,
        diagnostics,
    })
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

/// Streaming iterator over JSONL lines as decoded events. The
/// iterator silently skips malformed and oversized lines but
/// records both in `diagnostics()` — see [`ParseDiagnostics`].
/// Mid-stream I/O errors flip `diagnostics().truncated_by_io` to
/// true; the indexer reads this after iteration to decide whether
/// to stamp the file's staleness triple.
pub struct EventIter {
    reader: BufReader<File>,
    line_no: u32,
    buf: String,
    done: bool,
    diagnostics: ParseDiagnostics,
}

impl EventIter {
    /// Read-only access to the parse quality signals accumulated
    /// during iteration. Stable across `next()` calls; finalize
    /// after the iterator is exhausted.
    pub fn diagnostics(&self) -> &ParseDiagnostics {
        &self.diagnostics
    }
}

impl Iterator for EventIter {
    type Item = CodexEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            self.buf.clear();
            // Cap the per-line read at MAX_LINE_BYTES. `take` here
            // is a logical guard wrapping the underlying reader
            // for one line; we restore the unbounded reader after.
            // SAFETY: `take` consumes its inner; we re-borrow via
            // `by_ref` and `take(N)` for each line.
            let mut limited = (&mut self.reader).take(MAX_LINE_BYTES as u64 + 1);
            match limited.read_line(&mut self.buf) {
                Ok(0) => {
                    self.done = true;
                    return None;
                }
                Ok(n) if n > MAX_LINE_BYTES => {
                    // Oversized line. We've consumed MAX_LINE_BYTES+1
                    // bytes into self.buf already (no newline among
                    // them, since `read_line` only stops at one).
                    // The pre-fix attempt used `read_until(b'\n',
                    // &mut Vec<u8>)`, which accumulates the
                    // discarded bytes — defeating M5's OOM defense
                    // against adversarial multi-GB lines. Replace
                    // with a fixed-size stack-buffer drain that
                    // reads through the file's BufReader (whose
                    // own buffer is bounded by `DEFAULT_BUF_SIZE`,
                    // ~8 KiB) until it sees `\n` or EOF.
                    self.line_no += 1;
                    self.diagnostics.oversize_lines += 1;
                    tracing::warn!(
                        line = self.line_no,
                        bytes_consumed = n,
                        cap = MAX_LINE_BYTES,
                        "codex_session: dropping oversized JSONL line"
                    );
                    match drain_to_newline(&mut self.reader) {
                        Ok(true) => {
                            // Drained past the newline. Next outer
                            // iteration starts a fresh line.
                        }
                        Ok(false) => {
                            // EOF without seeing a newline.
                            self.done = true;
                            return None;
                        }
                        Err(_) => {
                            self.diagnostics.truncated_by_io = true;
                            self.done = true;
                            return None;
                        }
                    }
                    continue;
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
                    // Malformed — skip but count for diagnostics
                    // (the line counter has already advanced).
                    self.diagnostics.malformed_lines += 1;
                }
                Err(_) => {
                    // I/O hiccup mid-stream. Surface as
                    // `truncated_by_io` so the indexer refuses to
                    // stamp the staleness triple — without that,
                    // a partial parse persists indefinitely
                    // because the next backfill sees the file as
                    // unchanged.
                    self.diagnostics.truncated_by_io = true;
                    self.done = true;
                    return None;
                }
            }
        }
    }
}

/// Consume bytes from `reader` up to and including the next `\n`
/// or EOF, discarding everything. Uses `BufRead::fill_buf` +
/// `consume` so memory consumption is O(1) (bounded by the
/// reader's own internal buffer, ~8 KiB for `BufReader`) AND we
/// stop *exactly* at the newline — bytes after the `\n` stay in
/// the reader's buffer for the next `read_line` call.
///
/// This is the M5 OOM defense done right. The earlier
/// `read_until(b'\n', &mut Vec<u8>)` accumulated the discarded
/// bytes (defeating the defense for adversarial multi-GB lines).
/// A naive `read()` into a stack buffer would over-consume past
/// the newline. `fill_buf` + `consume` gives us both bounded
/// memory AND exact line-boundary stops.
///
/// Returns `Ok(true)` if a newline was found and consumed,
/// `Ok(false)` if EOF was reached without seeing one, or
/// `Err(_)` on any I/O error.
// `pub(crate)` so the tests module (a sibling) can exercise this
// helper directly. Without direct access, the M5/H1 OOM defense
// can only be tested through the full parser path, which doesn't
// reliably catch a regression to allocation-heavy drain.
pub(crate) fn drain_to_newline<R: BufRead>(reader: &mut R) -> std::io::Result<bool> {
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(false); // EOF
        }
        match buf.iter().position(|&b| b == b'\n') {
            Some(idx) => {
                // Consume up to AND INCLUDING the newline; bytes
                // after stay in the reader for the next line.
                reader.consume(idx + 1);
                return Ok(true);
            }
            None => {
                // No newline in this slice; consume the whole
                // chunk and loop to refill. Stack memory stays
                // bounded by the reader's internal buffer size.
                let len = buf.len();
                reader.consume(len);
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

fn decode_session_meta(payload: &Value, timestamp: Option<DateTime<Utc>>, line: u32) -> CodexEvent {
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

fn decode_turn_context(payload: &Value, timestamp: Option<DateTime<Utc>>, line: u32) -> CodexEvent {
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
            let call_id = payload.get("call_id").and_then(Value::as_str)?.to_string();
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
            let call_id = payload.get("call_id").and_then(Value::as_str)?.to_string();
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
            assistant_text: String::new(),
            timestamp,
            line_start: Some(line),
            line_end: Some(line),
        }
    }

    fn append_assistant(&mut self, text: &str, timestamp: Option<DateTime<Utc>>, line: u32) {
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
        CodexExchange {
            id: format!("codex:{}:{}", self.session_id, self.turn_index),
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
