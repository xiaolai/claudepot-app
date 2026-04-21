//! Cross-project session index + transcript reader.
//!
//! CC persists every REPL turn to `<config>/projects/<slug>/<session>.jsonl`
//! as newline-delimited JSON. This module gives the GUI two surfaces:
//!
//!   - `list_all_sessions` — one pass over every JSONL under every slug,
//!     producing lightweight metadata rows (token totals, first prompt,
//!     models used). Ordered newest-first. Parallelized via rayon.
//!
//!   - `read_session_detail` — full parse of one file into normalized
//!     `SessionEvent`s the UI can render as a transcript.
//!
//! The JSONL layout was empirically derived from live transcripts
//! against CC v2.1.97 and crosschecked against the field map in
//! `dev-docs/kannon/reference.md §V.2 (Source 2)`. CC tolerates
//! malformed lines — so do we.

use chrono::{DateTime, Utc};
#[cfg(test)]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("invalid session path: {0}")]
    InvalidPath(String),
    /// Stringified `SessionIndexError` — kept as `String` to avoid a
    /// cycle with `session_index::SessionIndexError`, which already
    /// wraps `SessionError` for its scan-failure variant.
    #[error("session index: {0}")]
    Index(String),
}

// ---------------------------------------------------------------------------
// Public types — cross the Tauri boundary via DTO conversion.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_creation + self.cache_read
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    /// CC's session UUID (also the filename stem).
    pub session_id: String,
    /// Sanitized on-disk slug (`~/.claude/projects/<slug>/`).
    pub slug: String,
    /// Absolute transcript path on disk.
    pub file_path: PathBuf,
    pub file_size_bytes: u64,
    /// ms-since-epoch of file mtime. `None` only on filesystems that
    /// don't report mtime (virtually never in practice).
    pub last_modified: Option<SystemTime>,
    /// First line's `cwd` field — the project this session belongs to.
    /// Falls back to `unsanitize(slug)` when the JSONL is empty/malformed
    /// so the UI always has something to group by.
    pub project_path: String,
    /// True iff `project_path` came from the live JSONL (not decoded
    /// from the slug). Drives the "recovered" tag in the UI.
    pub project_from_transcript: bool,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    /// Total newline-delimited events of any type (user, assistant,
    /// system, attachment, …). Captures the true "size" of the session.
    pub event_count: usize,
    /// Only user + assistant lines — the turn count the user cares about.
    pub message_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    /// First user message that was NOT a tool result. Truncated to 240
    /// chars so the list view stays predictable. `None` when the session
    /// never got past tool-result-only traffic.
    pub first_user_prompt: Option<String>,
    /// Sorted, deduped list of `message.model` values seen on assistant
    /// events. Empty when the session has no assistant turns yet.
    pub models: Vec<String>,
    pub tokens: TokenUsage,
    /// `gitBranch` field from the last event that carried it — CC
    /// writes the current branch into every line, so we take the most
    /// recent reading.
    pub git_branch: Option<String>,
    /// CC `version` field from the most recent event.
    pub cc_version: Option<String>,
    /// CC's internal display slug (`message.slug` on any line). Lets us
    /// offer a stable human name in the list.
    pub display_slug: Option<String>,
    /// True iff any assistant event included `is_error: true` or
    /// `stop_reason == "error"`. Surfaces a badge in the list.
    pub has_error: bool,
    /// True iff ANY event had `isSidechain: true` — pure-agent
    /// subsession (e.g. subagent transcripts bundled alongside the
    /// parent). Lets the UI filter these out.
    pub is_sidechain: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum SessionEvent {
    #[serde(rename = "userText")]
    UserText {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        text: String,
    },
    #[serde(rename = "userToolResult")]
    UserToolResult {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "assistantText")]
    AssistantText {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        model: Option<String>,
        text: String,
        usage: Option<TokenUsage>,
        stop_reason: Option<String>,
    },
    #[serde(rename = "assistantToolUse")]
    AssistantToolUse {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        model: Option<String>,
        tool_name: String,
        tool_use_id: String,
        input_preview: String,
    },
    #[serde(rename = "assistantThinking")]
    AssistantThinking {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        text: String,
    },
    #[serde(rename = "summary")]
    Summary {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        text: String,
    },
    #[serde(rename = "system")]
    System {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        subtype: Option<String>,
        detail: String,
    },
    #[serde(rename = "attachment")]
    Attachment {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        name: Option<String>,
        mime: Option<String>,
    },
    /// `task-summary` — CC's periodic fork-generated natural-language
    /// summary of what the agent is currently doing. Written every
    /// min(5 steps, 2 min) specifically for `claude ps` / the
    /// Activity current-action card. See
    /// ~/github/claude_code_src/src/types/logs.ts (TaskSummaryMessage)
    /// and sessionStorage.ts (saveTaskSummary).
    #[serde(rename = "taskSummary")]
    TaskSummary {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        summary: String,
    },
    #[serde(rename = "fileSnapshot")]
    FileHistorySnapshot {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        file_count: usize,
    },
    #[serde(rename = "other")]
    Other {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        raw_type: String,
    },
    #[serde(rename = "malformed")]
    Malformed {
        line_number: usize,
        error: String,
        preview: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail {
    pub row: SessionRow,
    pub events: Vec<SessionEvent>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Enumerate every session under `<config_dir>/projects/*/`, parsing
/// the JSONL to produce rich metadata. Returns rows sorted newest-first
/// by `last_ts` (falling back to file mtime).
///
/// Delegates to the persistent `SessionIndex` cache at
/// `<claudepot_data_dir>/sessions.db`. Cold first run folds every
/// transcript; subsequent calls touch only `stat()` + the rows whose
/// `(size, mtime_ns)` changed. A session directory with no JSONL
/// files is silently skipped — those are handled by the
/// orphan-detection pipeline, not here.
pub fn list_all_sessions(config_dir: &Path) -> Result<Vec<SessionRow>, SessionError> {
    let data_dir = crate::paths::claudepot_data_dir();
    let db_path = data_dir.join("sessions.db");
    let idx = crate::session_index::SessionIndex::open(&db_path)
        .map_err(|e| SessionError::Index(e.to_string()))?;
    idx.list_all(config_dir)
        .map_err(|e| SessionError::Index(e.to_string()))
}

/// Direct (uncached) scan — used by tests that want to verify the
/// JSONL-folding logic without pulling SQLite and the global data-dir
/// lock into every unit test. Production callers go through
/// `list_all_sessions`, which wraps the persistent index.
#[cfg(test)]
pub(crate) fn scan_all_sessions_uncached(
    config_dir: &Path,
) -> Result<Vec<SessionRow>, SessionError> {
    let projects_dir = config_dir.join("projects");
    if !projects_dir.exists() {
        return Ok(vec![]);
    }

    // Collect (slug, session_file) pairs first so rayon can parallelize
    // across files, not just slugs. Large accounts have hundreds of
    // small files; per-file parallelism is the right granularity.
    let mut work: Vec<(String, PathBuf)> = Vec::new();
    for slug_entry in fs::read_dir(&projects_dir)? {
        let slug_entry = slug_entry?;
        if !slug_entry.file_type()?.is_dir() {
            continue;
        }
        let slug = slug_entry.file_name().to_string_lossy().to_string();
        for session_entry in fs::read_dir(slug_entry.path())? {
            let session_entry = session_entry?;
            let name = session_entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jsonl") {
                continue;
            }
            work.push((slug.clone(), session_entry.path()));
        }
    }

    let mut rows: Vec<SessionRow> = work
        .par_iter()
        .filter_map(|(slug, path)| scan_session(slug, path).ok())
        .collect();

    rows.sort_by(|a, b| {
        let ak = a.last_ts.map(|t| t.timestamp_millis()).unwrap_or_else(|| {
            a.last_modified
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        });
        let bk = b.last_ts.map(|t| t.timestamp_millis()).unwrap_or_else(|| {
            b.last_modified
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        });
        bk.cmp(&ak)
    });

    Ok(rows)
}

/// Read a single session transcript end-to-end. Returns both the row
/// (same data as `list_all_sessions` would produce for this file) and
/// the normalized event stream.
///
/// `session_id` is the filename stem; we find the slug by scanning
/// `<config>/projects/*/<session_id>.jsonl`. This is O(number_of_slugs)
/// which is fine for typical accounts (<1000 slugs).
///
/// When two files on disk share a session_id (can happen after an
/// interrupted rescue/adopt), callers that need to target a specific
/// one should use `read_session_detail_at_path` instead.
pub fn read_session_detail(
    config_dir: &Path,
    session_id: &str,
) -> Result<SessionDetail, SessionError> {
    let (slug, path) = locate_session(config_dir, session_id)?;
    let row = scan_session(&slug, &path)?;
    let events = parse_events(&path)?;
    Ok(SessionDetail { row, events })
}

/// Read a specific transcript file by absolute path. Used by the GUI
/// when the listing surfaced a row that points at a concrete file —
/// looking the session up by id is both slower and ambiguous when
/// duplicates exist.
///
/// Guards against paths that don't sit under `<config>/projects/`: we
/// don't want a stray Tauri command turning into an arbitrary-file
/// reader.
pub fn read_session_detail_at_path(
    config_dir: &Path,
    file_path: &Path,
) -> Result<SessionDetail, SessionError> {
    let projects_dir = config_dir.join("projects");
    let canonical_projects = fs::canonicalize(&projects_dir).unwrap_or(projects_dir);
    let canonical_file = fs::canonicalize(file_path)
        .map_err(|_| SessionError::NotFound(file_path.display().to_string()))?;
    if !canonical_file.starts_with(&canonical_projects) {
        return Err(SessionError::InvalidPath(file_path.display().to_string()));
    }
    if canonical_file
        .extension()
        .map(|e| e != "jsonl")
        .unwrap_or(true)
    {
        return Err(SessionError::InvalidPath(file_path.display().to_string()));
    }
    let slug = canonical_file
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let row = scan_session(&slug, &canonical_file)?;
    let events = parse_events(&canonical_file)?;
    Ok(SessionDetail { row, events })
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn locate_session(config_dir: &Path, session_id: &str) -> Result<(String, PathBuf), SessionError> {
    if session_id.contains('/')
        || session_id.contains('\\')
        || session_id.contains("..")
        || session_id.is_empty()
    {
        return Err(SessionError::InvalidPath(session_id.to_string()));
    }
    let filename = format!("{session_id}.jsonl");
    let projects_dir = config_dir.join("projects");
    for slug_entry in fs::read_dir(&projects_dir)? {
        let slug_entry = slug_entry?;
        if !slug_entry.file_type()?.is_dir() {
            continue;
        }
        let candidate = slug_entry.path().join(&filename);
        if candidate.is_file() {
            let slug = slug_entry.file_name().to_string_lossy().to_string();
            return Ok((slug, candidate));
        }
    }
    Err(SessionError::NotFound(session_id.to_string()))
}

/// Single streaming scan that folds every field we care about into a
/// `SessionRow`. Malformed lines are counted toward `event_count` but
/// contribute nothing else — matching CC's own tolerance.
///
/// Exposed to `session_index` so the persistent cache can reuse the
/// same JSONL-folding logic without copy-pasting the match tree.
pub(crate) fn scan_session(slug: &str, path: &Path) -> Result<SessionRow, SessionError> {
    let meta = fs::metadata(path)?;
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);

    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| slug.to_string());

    let mut event_count: usize = 0;
    let mut message_count: usize = 0;
    let mut user_message_count: usize = 0;
    let mut assistant_message_count: usize = 0;
    let mut first_ts: Option<DateTime<Utc>> = None;
    let mut last_ts: Option<DateTime<Utc>> = None;
    let mut first_user_prompt: Option<String> = None;
    let mut models: BTreeSet<String> = BTreeSet::new();
    let mut tokens = TokenUsage::default();
    let mut cwd_from_transcript: Option<String> = None;
    let mut git_branch: Option<String> = None;
    let mut cc_version: Option<String> = None;
    let mut display_slug: Option<String> = None;
    let mut has_error = false;
    let mut any_sidechain = false;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        event_count += 1;

        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        // --- ambient fields (most lines carry these) -------------------
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));

        if let Some(t) = ts {
            if first_ts.map_or(true, |cur| t < cur) {
                first_ts = Some(t);
            }
            if last_ts.map_or(true, |cur| t > cur) {
                last_ts = Some(t);
            }
        }

        if cwd_from_transcript.is_none() {
            if let Some(c) = v.get("cwd").and_then(Value::as_str) {
                if !c.is_empty() {
                    cwd_from_transcript = Some(c.to_string());
                }
            }
        }
        if let Some(b) = v.get("gitBranch").and_then(Value::as_str) {
            if !b.is_empty() {
                git_branch = Some(b.to_string());
            }
        }
        if let Some(s) = v.get("version").and_then(Value::as_str) {
            if !s.is_empty() {
                cc_version = Some(s.to_string());
            }
        }
        if display_slug.is_none() {
            if let Some(s) = v.get("slug").and_then(Value::as_str) {
                if !s.is_empty() {
                    display_slug = Some(s.to_string());
                }
            }
        }
        if v.get("isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            any_sidechain = true;
        }

        // --- event type --------------------------------------------------
        let event_type = v.get("type").and_then(Value::as_str).unwrap_or("");
        match event_type {
            "user" => {
                message_count += 1;
                user_message_count += 1;
                if first_user_prompt.is_none() {
                    if let Some(text) = extract_user_text(&v) {
                        first_user_prompt = Some(truncate_prompt(&text));
                    }
                }
            }
            "assistant" => {
                message_count += 1;
                assistant_message_count += 1;
                if let Some(msg) = v.get("message") {
                    if let Some(model) = msg.get("model").and_then(Value::as_str) {
                        if !model.is_empty() {
                            models.insert(model.to_string());
                        }
                    }
                    if let Some(usage) = msg.get("usage") {
                        tokens.input += usage
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        tokens.output += usage
                            .get("output_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        tokens.cache_creation += usage
                            .get("cache_creation_input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        tokens.cache_read += usage
                            .get("cache_read_input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                    }
                    if msg.get("stop_reason").and_then(Value::as_str) == Some("error") {
                        has_error = true;
                    }
                }
            }
            _ => {}
        }

        if v.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            has_error = true;
        }
    }

    let project_from_transcript = cwd_from_transcript.is_some();
    let project_path = cwd_from_transcript
        .unwrap_or_else(|| crate::project_sanitize::unsanitize_path(slug));

    Ok(SessionRow {
        session_id,
        slug: slug.to_string(),
        file_path: path.to_path_buf(),
        file_size_bytes: meta.len(),
        last_modified: meta.modified().ok(),
        project_path,
        project_from_transcript,
        first_ts,
        last_ts,
        event_count,
        message_count,
        user_message_count,
        assistant_message_count,
        first_user_prompt,
        models: models.into_iter().collect(),
        tokens,
        git_branch,
        cc_version,
        display_slug,
        has_error,
        is_sidechain: any_sidechain,
    })
}

/// Crate-visible wrapper so `session_subagents` can parse an
/// `agent-*.jsonl` without duplicating the tolerant reader. Exposed
/// only within the crate — external callers go through
/// `read_session_detail_at_path`.
pub(crate) fn parse_events_public(path: &Path) -> Result<Vec<SessionEvent>, SessionError> {
    parse_events(path)
}

/// Full-fidelity parse. Lines we don't recognize land in `Other`;
/// invalid JSON lands in `Malformed` with the line number so the UI
/// can show "CC wrote a bad line on turn 42" without hiding it.
fn parse_events(path: &Path) -> Result<Vec<SessionEvent>, SessionError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();

    for (idx, line) in reader.lines().enumerate() {
        let line_number = idx + 1;
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                events.push(SessionEvent::Malformed {
                    line_number,
                    error: e.to_string(),
                    preview: String::new(),
                });
                continue;
            }
        };
        parse_line_into(&mut events, &line, line_number);
    }

    Ok(events)
}

/// Parse a single JSONL line and append the resulting zero-or-more
/// `SessionEvent`s to `out`. Empty lines are silently skipped;
/// malformed JSON yields a `Malformed` event with the line number.
///
/// Exposed crate-visible so `session_live::runtime` can feed events
/// one line at a time as the tail reader surfaces them, without
/// re-parsing the whole transcript.
pub(crate) fn parse_line_into(out: &mut Vec<SessionEvent>, line: &str, line_number: usize) {
    if line.trim().is_empty() {
        return;
    }
    let v = match serde_json::from_str::<Value>(line) {
        Ok(v) => v,
        Err(e) => {
            out.push(SessionEvent::Malformed {
                line_number,
                error: e.to_string(),
                preview: truncate_prompt(line),
            });
            return;
        }
    };

    let ts = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let uuid = v
        .get("uuid")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let event_type = v.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "user" => emit_user_events(out, &v, ts, uuid),
        "assistant" => emit_assistant_events(out, &v, ts, uuid),
        "summary" => {
            let text = v
                .get("summary")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .unwrap_or_default();
            out.push(SessionEvent::Summary { ts, uuid, text });
        }
        "system" => {
            let subtype = v
                .get("subtype")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let detail = v
                .get("level")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .unwrap_or_default();
            out.push(SessionEvent::System {
                ts,
                uuid,
                subtype,
                detail,
            });
        }
        "attachment" => {
            let name = v
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| v.get("filename").and_then(Value::as_str))
                .map(|s| s.to_string());
            let mime = v
                .get("mimeType")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            out.push(SessionEvent::Attachment {
                ts,
                uuid,
                name,
                mime,
            });
        }
        "file-history-snapshot" => {
            let file_count = v
                .get("files")
                .and_then(Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            out.push(SessionEvent::FileHistorySnapshot {
                ts,
                uuid,
                file_count,
            });
        }
        "task-summary" => {
            let summary = v
                .get("summary")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .unwrap_or_default();
            out.push(SessionEvent::TaskSummary { ts, uuid, summary });
        }
        other => out.push(SessionEvent::Other {
            ts,
            uuid,
            raw_type: other.to_string(),
        }),
    }
}

fn emit_user_events(
    out: &mut Vec<SessionEvent>,
    v: &Value,
    ts: Option<DateTime<Utc>>,
    uuid: Option<String>,
) {
    let Some(msg) = v.get("message") else {
        return;
    };
    match msg.get("content") {
        Some(Value::String(s)) => out.push(SessionEvent::UserText {
            ts,
            uuid,
            text: s.clone(),
        }),
        Some(Value::Array(parts)) => {
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                match kind {
                    "text" => {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            out.push(SessionEvent::UserText {
                                ts,
                                uuid: uuid.clone(),
                                text: text.to_string(),
                            });
                        }
                    }
                    "tool_result" => {
                        let tool_use_id = part
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let is_error = part
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let content = match part.get("content") {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Array(inner)) => inner
                                .iter()
                                .filter_map(|p| p.get("text").and_then(Value::as_str))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            Some(other) => other.to_string(),
                            None => String::new(),
                        };
                        out.push(SessionEvent::UserToolResult {
                            ts,
                            uuid: uuid.clone(),
                            tool_use_id,
                            content,
                            is_error,
                        });
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn emit_assistant_events(
    out: &mut Vec<SessionEvent>,
    v: &Value,
    ts: Option<DateTime<Utc>>,
    uuid: Option<String>,
) {
    let Some(msg) = v.get("message") else {
        return;
    };
    let model = msg
        .get("model")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let stop_reason = msg
        .get("stop_reason")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let usage = msg.get("usage").map(|u| TokenUsage {
        input: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
        output: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        cache_creation: u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read: u
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    });

    let Some(Value::Array(parts)) = msg.get("content") else {
        return;
    };

    for part in parts {
        let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "text" => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    out.push(SessionEvent::AssistantText {
                        ts,
                        uuid: uuid.clone(),
                        model: model.clone(),
                        text: text.to_string(),
                        usage: usage.clone(),
                        stop_reason: stop_reason.clone(),
                    });
                }
            }
            "thinking" => {
                if let Some(text) = part.get("thinking").and_then(Value::as_str) {
                    out.push(SessionEvent::AssistantThinking {
                        ts,
                        uuid: uuid.clone(),
                        text: text.to_string(),
                    });
                }
            }
            "tool_use" => {
                let tool_name = part
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let tool_use_id = part
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let input_preview = part
                    .get("input")
                    .map(|i| truncate_prompt(&i.to_string()))
                    .unwrap_or_default();
                out.push(SessionEvent::AssistantToolUse {
                    ts,
                    uuid: uuid.clone(),
                    model: model.clone(),
                    tool_name,
                    tool_use_id,
                    input_preview,
                });
            }
            _ => {}
        }
    }
}

/// First user-typed prompt. Skips tool results (those come back
/// automatically from the previous turn). Returns the first plain-text
/// user content, or the text portion of a structured user message.
fn extract_user_text(v: &Value) -> Option<String> {
    let msg = v.get("message")?;
    match msg.get("content")? {
        Value::String(s) => {
            if looks_like_local_command_caveat(s) {
                return None;
            }
            Some(s.clone())
        }
        Value::Array(parts) => {
            for part in parts {
                let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                if kind != "text" {
                    continue;
                }
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if looks_like_local_command_caveat(text) {
                        continue;
                    }
                    return Some(text.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

/// CC inserts synthetic `<command-name>/<command-message>/
/// <local-command-caveat>` wrappers for slash-command turns and
/// internal notes. They're technically user messages but aren't what
/// the user typed, so they're lousy list-view previews. Skip past them
/// to find the first "real" prompt.
fn looks_like_local_command_caveat(s: &str) -> bool {
    let trimmed = s.trim_start();
    trimmed.starts_with("<local-command-caveat>")
        || trimmed.starts_with("<command-name>")
        || trimmed.starts_with("<command-message>")
        || trimmed.starts_with("<command-args>")
}

fn truncate_prompt(s: &str) -> String {
    const MAX: usize = 240;
    let cleaned = s.trim().replace(['\n', '\r'], " ");
    let mut out = String::with_capacity(cleaned.len().min(MAX + 1));
    for (idx, ch) in cleaned.chars().enumerate() {
        if idx >= MAX {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_session(dir: &Path, slug: &str, session_id: &str, lines: &[&str]) -> PathBuf {
        let slug_dir = dir.join("projects").join(slug);
        fs::create_dir_all(&slug_dir).unwrap();
        let path = slug_dir.join(format!("{session_id}.jsonl"));
        let mut f = fs::File::create(&path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        path
    }

    #[test]
    fn empty_projects_dir_is_ok() {
        let tmp = TempDir::new().unwrap();
        let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn single_session_scan_captures_everything() {
        let tmp = TempDir::new().unwrap();
        let user1 = r#"{"type":"user","message":{"role":"user","content":"Fix the build"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/repo/foo","gitBranch":"main","version":"2.1.97","sessionId":"AAA","slug":"brave-otter"}"#;
        let asst1 = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"OK"}],"usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":10,"cache_read_input_tokens":200}},"timestamp":"2026-04-10T10:00:05Z","cwd":"/repo/foo","gitBranch":"main","version":"2.1.97","sessionId":"AAA"}"#;
        let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"done","is_error":false}]},"timestamp":"2026-04-10T10:00:10Z","cwd":"/repo/foo","sessionId":"AAA"}"#;

        write_session(tmp.path(), "-repo-foo", "AAA", &[user1, asst1, tool]);

        let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.session_id, "AAA");
        assert_eq!(r.slug, "-repo-foo");
        assert_eq!(r.project_path, "/repo/foo");
        assert!(r.project_from_transcript);
        assert_eq!(r.event_count, 3);
        assert_eq!(r.message_count, 3);
        assert_eq!(r.user_message_count, 2);
        assert_eq!(r.assistant_message_count, 1);
        assert_eq!(r.first_user_prompt.as_deref(), Some("Fix the build"));
        assert_eq!(r.models, vec!["claude-opus-4-7".to_string()]);
        assert_eq!(r.tokens.input, 100);
        assert_eq!(r.tokens.output, 50);
        assert_eq!(r.tokens.cache_creation, 10);
        assert_eq!(r.tokens.cache_read, 200);
        assert_eq!(r.git_branch.as_deref(), Some("main"));
        assert_eq!(r.cc_version.as_deref(), Some("2.1.97"));
        assert_eq!(r.display_slug.as_deref(), Some("brave-otter"));
        assert!(!r.has_error);
    }

    #[test]
    fn first_user_prompt_skips_tool_result_and_caveat() {
        let tmp = TempDir::new().unwrap();
        let caveat = r#"{"type":"user","message":{"role":"user","content":"<local-command-caveat>ignore</local-command-caveat>"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/a","sessionId":"S1"}"#;
        let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"x","is_error":false}]},"timestamp":"2026-04-10T10:00:01Z","cwd":"/a","sessionId":"S1"}"#;
        let real = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"the real question"}]},"timestamp":"2026-04-10T10:00:02Z","cwd":"/a","sessionId":"S1"}"#;
        write_session(tmp.path(), "-a", "S1", &[caveat, tool, real]);
        let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
        assert_eq!(rows[0].first_user_prompt.as_deref(), Some("the real question"));
    }

    #[test]
    fn malformed_line_does_not_poison_scan() {
        let tmp = TempDir::new().unwrap();
        let bad = "{not valid json";
        let good = r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/z","sessionId":"S1"}"#;
        write_session(tmp.path(), "-z", "S1", &[bad, good]);
        let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
        assert_eq!(rows.len(), 1);
        // event_count counts ALL non-empty lines, including malformed.
        assert_eq!(rows[0].event_count, 2);
        assert_eq!(rows[0].user_message_count, 1);
        assert_eq!(rows[0].first_user_prompt.as_deref(), Some("hi"));
    }

    #[test]
    fn sort_newest_first() {
        let tmp = TempDir::new().unwrap();
        let older = r#"{"type":"user","message":{"role":"user","content":"old"},"timestamp":"2026-04-01T00:00:00Z","cwd":"/a","sessionId":"A"}"#;
        let newer = r#"{"type":"user","message":{"role":"user","content":"new"},"timestamp":"2026-04-20T00:00:00Z","cwd":"/b","sessionId":"B"}"#;
        write_session(tmp.path(), "-a", "A", &[older]);
        write_session(tmp.path(), "-b", "B", &[newer]);
        let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].session_id, "B");
        assert_eq!(rows[1].session_id, "A");
    }

    #[test]
    fn read_session_detail_parses_event_kinds() {
        let tmp = TempDir::new().unwrap();
        let user = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hello"}]},"timestamp":"2026-04-10T10:00:00Z","cwd":"/r","sessionId":"D1","uuid":"u1"}"#;
        let asst = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4-7","content":[{"type":"text","text":"hi back"},{"type":"tool_use","id":"t1","name":"Bash","input":{"cmd":"ls"}}],"usage":{"input_tokens":1,"output_tokens":2}},"timestamp":"2026-04-10T10:00:01Z","cwd":"/r","sessionId":"D1","uuid":"u2"}"#;
        let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"a\nb","is_error":false}]},"timestamp":"2026-04-10T10:00:02Z","cwd":"/r","sessionId":"D1","uuid":"u3"}"#;
        let summary = r#"{"type":"summary","summary":"compacted","timestamp":"2026-04-10T10:00:03Z","uuid":"u4"}"#;
        write_session(tmp.path(), "-r", "D1", &[user, asst, tool, summary]);

        let detail = read_session_detail(tmp.path(), "D1").unwrap();
        assert_eq!(detail.row.session_id, "D1");
        assert_eq!(detail.events.len(), 5);
        match &detail.events[0] {
            SessionEvent::UserText { text, .. } => assert_eq!(text, "hello"),
            e => panic!("expected UserText, got {e:?}"),
        }
        match &detail.events[1] {
            SessionEvent::AssistantText { text, .. } => assert_eq!(text, "hi back"),
            e => panic!("expected AssistantText, got {e:?}"),
        }
        match &detail.events[2] {
            SessionEvent::AssistantToolUse { tool_name, .. } => assert_eq!(tool_name, "Bash"),
            e => panic!("expected AssistantToolUse, got {e:?}"),
        }
        match &detail.events[3] {
            SessionEvent::UserToolResult { content, .. } => assert_eq!(content, "a\nb"),
            e => panic!("expected UserToolResult, got {e:?}"),
        }
        match &detail.events[4] {
            SessionEvent::Summary { text, .. } => assert_eq!(text, "compacted"),
            e => panic!("expected Summary, got {e:?}"),
        }
    }

    #[test]
    fn read_session_detail_at_path_rejects_outside_projects() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("projects")).unwrap();
        let outside = tmp.path().join("rogue.jsonl");
        fs::write(&outside, "{}\n").unwrap();
        assert!(matches!(
            read_session_detail_at_path(tmp.path(), &outside),
            Err(SessionError::InvalidPath(_))
        ));
    }

    #[test]
    fn read_session_detail_at_path_rejects_non_jsonl() {
        let tmp = TempDir::new().unwrap();
        let slug_dir = tmp.path().join("projects").join("-repo");
        fs::create_dir_all(&slug_dir).unwrap();
        let wrong = slug_dir.join("notes.md");
        fs::write(&wrong, "hi\n").unwrap();
        assert!(matches!(
            read_session_detail_at_path(tmp.path(), &wrong),
            Err(SessionError::InvalidPath(_))
        ));
    }

    #[test]
    fn read_session_detail_at_path_reads_the_targeted_file_among_dupes() {
        let tmp = TempDir::new().unwrap();
        let a_line = r#"{"type":"user","message":{"role":"user","content":"from A"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/a","sessionId":"DUP"}"#;
        let b_line = r#"{"type":"user","message":{"role":"user","content":"from B"},"timestamp":"2026-04-10T10:00:00Z","cwd":"/b","sessionId":"DUP"}"#;
        let a_path = write_session(tmp.path(), "-a", "DUP", &[a_line]);
        let b_path = write_session(tmp.path(), "-b", "DUP", &[b_line]);

        let read_a = read_session_detail_at_path(tmp.path(), &a_path).unwrap();
        let read_b = read_session_detail_at_path(tmp.path(), &b_path).unwrap();
        assert_eq!(read_a.row.project_path, "/a");
        assert_eq!(read_b.row.project_path, "/b");
        assert_eq!(read_a.row.slug, "-a");
        assert_eq!(read_b.row.slug, "-b");
    }

    #[test]
    fn locate_session_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("projects")).unwrap();
        assert!(matches!(
            read_session_detail(tmp.path(), "../../etc/passwd"),
            Err(SessionError::InvalidPath(_))
        ));
    }

    #[test]
    fn read_session_detail_not_found() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("projects")).unwrap();
        assert!(matches!(
            read_session_detail(tmp.path(), "missing"),
            Err(SessionError::NotFound(_))
        ));
    }

    #[test]
    fn fallback_project_path_from_slug_when_cwd_missing() {
        let tmp = TempDir::new().unwrap();
        let asst = r#"{"type":"assistant","message":{"role":"assistant","model":"m","content":[{"type":"text","text":"x"}]},"timestamp":"2026-04-10T10:00:00Z","sessionId":"S"}"#;
        write_session(tmp.path(), "-Users-joker-repo", "S", &[asst]);
        let rows = scan_all_sessions_uncached(tmp.path()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].project_from_transcript);
        // unsanitize_path turns "-Users-joker-repo" back into an absolute path
        assert!(rows[0].project_path.contains("Users") && rows[0].project_path.contains("joker"));
    }
}
