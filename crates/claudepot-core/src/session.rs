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

use crate::artifact_usage::extract as usage_extract;
use crate::artifact_usage::model::{Outcome, UsageEvent};
use crate::path_utils::canonicalize_simplified;
use chrono::{DateTime, Utc};
#[cfg(test)]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
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
        /// Display-only preview: trimmed, newlines collapsed, capped at
        /// 240 chars. Render this in lists / compact UI.
        input_preview: String,
        /// Raw serialized JSON of the tool input (untruncated). Powers
        /// the detail-level substring search so a match deeper than
        /// `input_preview`'s 240-char cap (e.g. inside a long Write
        /// `content` or Edit `new_string`) is reachable. Never render
        /// verbatim — preview is the display form.
        #[serde(default)]
        input_full: String,
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
        .filter_map(|(slug, path)| scan_session(slug, path).ok().map(|s| s.row))
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
    let row = scan_session(&slug, &path)?.row;
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
    // Both sides of the containment check must be canonicalized —
    // silently falling back on failure turns this security gate off.
    // On Windows we also strip the `\\?\` verbatim prefix so string-
    // based `starts_with` comparisons line up on both paths.
    let canonical_projects = canonicalize_simplified(&projects_dir)
        .map_err(|_| SessionError::InvalidPath(projects_dir.display().to_string()))?;
    let canonical_file = canonicalize_simplified(file_path)
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
    let row = scan_session(&slug, &canonical_file)?.row;
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

// Per-turn record — public so external callers (CLI, future GUI
// surfaces) can consume the shape without a re-export shuffle.

/// One assistant turn's token usage, as extracted from a `.jsonl`
/// transcript. Per-turn data is the building block for "top-N
/// costliest prompts" and "per-turn pacing" surfaces; aggregating
/// across turns reproduces the totals already cached on the session
/// row.
///
/// `turn_index` is the 0-based ordinal of the assistant message
/// within the transcript (skipping user / system / sidechain lines).
/// It pairs with `file_path` to form the persistent index's primary
/// key, and is stable as long as the transcript is append-only —
/// which CC guarantees outside `session move` (which writes a new
/// file with new turn rows) and `slim --strip-images` (which rewrites
/// content but preserves line ordering, so turn indices remain valid).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_index: usize,
    /// Server-side timestamp; `None` when the message line lacks a
    /// usable `timestamp` field.
    pub ts_ms: Option<i64>,
    /// Model id stamped on this turn's `message.model`. Empty string
    /// when missing — kept as a String (not Option<String>) so the
    /// SQL primary surface is uniform; consumer SQL filters on
    /// `model != ''` to drop unmatched turns from per-model breakdowns.
    pub model: String,
    pub tokens: TokenUsage,
    /// Truncated copy of the user prompt that drove this turn. The
    /// nearest preceding `user` message in the stream wins; multiple
    /// assistant turns produced from one prompt all carry the same
    /// preview. `None` when no user prompt has been seen yet (e.g.
    /// transcripts that start with a system or assistant line).
    pub user_prompt_preview: Option<String>,
}

/// Combined output of a session scan — the indexed metadata row plus
/// the usage events and per-turn records extracted from the same
/// pass. Returned by `scan_session` so callers that only want
/// metadata can drop the auxiliary fields, while `session_index`
/// consumes all three in one transaction.
pub struct SessionScan {
    pub row: SessionRow,
    pub usage: Vec<UsageEvent>,
    pub turns: Vec<TurnRecord>,
}

/// Single streaming scan that folds every field we care about into a
/// `SessionRow`, and at the same time extracts artifact usage events.
/// Malformed lines are counted toward `event_count` but contribute
/// nothing else — matching CC's own tolerance.
///
/// Exposed to `session_index` so the persistent cache can reuse the
/// same JSONL-folding logic without copy-pasting the match tree.
pub(crate) fn scan_session(slug: &str, path: &Path) -> Result<SessionScan, SessionError> {
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

    // Usage extraction state — one Vec accumulates all events from
    // this transcript; `agent_use_index` lets us flip an Agent event's
    // outcome to Error when its matching tool_result lands later in
    // the stream.
    let mut usage_events: Vec<UsageEvent> = Vec::new();
    let mut agent_use_index: HashMap<String, usize> = HashMap::new();

    // Per-turn extraction state. `turns` collects one record per
    // assistant message; `last_user_prompt` carries the nearest
    // preceding user prompt forward so each turn carries the
    // prompt that drove it. The truncation rule matches
    // `first_user_prompt` so consumer surfaces render uniform text.
    let mut turns: Vec<TurnRecord> = Vec::new();
    let mut last_user_prompt: Option<String> = None;
    let mut assistant_ordinal: usize = 0;

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

        // Extract usage events from this line. Agent events come
        // through a paired API so we capture each tool_use.id at
        // construction — no positional re-match needed (the previous
        // index-based pairing skipped malformed Agent blocks
        // inconsistently between the two passes).
        collect_usage_events(
            &v,
            ts_for_usage(&v),
            &session_id,
            &mut usage_events,
            &mut agent_use_index,
        );
        flip_agent_outcomes_from_user_line(&v, &agent_use_index, &mut usage_events);

        // --- ambient fields (most lines carry these) -------------------
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));

        if let Some(t) = ts {
            if first_ts.is_none_or(|cur| t < cur) {
                first_ts = Some(t);
            }
            if last_ts.is_none_or(|cur| t > cur) {
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
                let prompt_text = extract_user_text(&v).map(|t| truncate_prompt(&t));
                // Always replace the carry-over, even when the new user
                // line has no extractable text (image-only message,
                // tool-result-only line, or a caveat-stripped CLI
                // command). Carrying a stale text prompt forward would
                // mis-attribute it to a later assistant turn and the
                // top-costly-prompts panel would show the wrong text
                // alongside the right cost.
                last_user_prompt = prompt_text.clone();
                if first_user_prompt.is_none() {
                    if let Some(p) = prompt_text {
                        first_user_prompt = Some(p);
                    }
                }
            }
            "assistant" => {
                message_count += 1;
                assistant_message_count += 1;
                if let Some(msg) = v.get("message") {
                    let turn_model = msg
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !turn_model.is_empty() {
                        models.insert(turn_model.clone());
                    }
                    let mut turn_tokens = TokenUsage::default();
                    if let Some(usage) = msg.get("usage") {
                        let inp = usage
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        let out = usage
                            .get("output_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        let cw = usage
                            .get("cache_creation_input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        let cr = usage
                            .get("cache_read_input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        tokens.input += inp;
                        tokens.output += out;
                        tokens.cache_creation += cw;
                        tokens.cache_read += cr;
                        turn_tokens = TokenUsage {
                            input: inp,
                            output: out,
                            cache_creation: cw,
                            cache_read: cr,
                        };
                    }
                    if msg.get("stop_reason").and_then(Value::as_str) == Some("error") {
                        has_error = true;
                    }
                    // Emit a per-turn record. `turn_index` follows
                    // the ordering of assistant lines as they appear
                    // in the transcript; that ordering is stable for
                    // append-only writes (CC's normal mode).
                    turns.push(TurnRecord {
                        turn_index: assistant_ordinal,
                        ts_ms: ts.map(|t| t.timestamp_millis()),
                        model: turn_model,
                        tokens: turn_tokens,
                        user_prompt_preview: last_user_prompt.clone(),
                    });
                    assistant_ordinal += 1;
                }
            }
            _ => {}
        }

        if v.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            has_error = true;
        }
    }

    let project_from_transcript = cwd_from_transcript.is_some();
    let project_path =
        cwd_from_transcript.unwrap_or_else(|| crate::project_sanitize::unsanitize_path(slug));

    let row = SessionRow {
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
    };
    Ok(SessionScan {
        row,
        usage: usage_events,
        turns,
    })
}

/// Parse a line's timestamp into ms-since-epoch for the usage
/// extractor, mirroring `extract::parse_ts_ms`. Returns `None` for
/// lines without a usable timestamp; the caller treats that as
/// "skip usage extraction for this line."
fn ts_for_usage(v: &Value) -> Option<i64> {
    usage_extract::parse_ts_ms(v)
}

/// Collect usage events from one JSONL line, registering any Agent
/// tool_use_id encountered for later outcome-flip pairing.
///
/// Agent events come from `extract_assistant_with_ids` so the id is
/// captured at construction — no positional re-match between two
/// passes (which previously diverged when malformed Agent blocks
/// appeared before valid ones).
///
/// Non-Agent events go straight to `usage_events` via
/// `extract_from_line`'s normal route.
fn collect_usage_events(
    v: &Value,
    ts_ms: Option<i64>,
    session_id: &str,
    usage_events: &mut Vec<UsageEvent>,
    agent_use_index: &mut HashMap<String, usize>,
) {
    let Some(ts_ms) = ts_ms else {
        return;
    };
    let event_type = v.get("type").and_then(Value::as_str).unwrap_or("");
    if event_type == "assistant" {
        for (ev, id) in usage_extract::extract_assistant_with_ids(v, ts_ms, session_id) {
            let idx = usage_events.len();
            usage_events.push(ev);
            if let Some(id) = id {
                agent_use_index.insert(id, idx);
            }
        }
        return;
    }
    // user / attachment lines and anything else — no Agent ids to
    // pair, so the simple extractor path is fine.
    let new_events = usage_extract::extract_from_line(v, session_id);
    usage_events.extend(new_events);
}

/// Walk a `user` line's `tool_result` blocks; for each one with
/// `is_error: true` whose `tool_use_id` we've recorded, flip the
/// matching Agent event's outcome to `Outcome::Error` and forget the
/// id (so a subsequent re-issue can't double-flip).
fn flip_agent_outcomes_from_user_line(
    v: &Value,
    agent_use_index: &HashMap<String, usize>,
    usage_events: &mut [UsageEvent],
) {
    if v.get("type").and_then(Value::as_str) != Some("user") {
        return;
    }
    let Some(content) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let is_error = block
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !is_error {
            continue;
        }
        let Some(use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(&idx) = agent_use_index.get(use_id) {
            if let Some(ev) = usage_events.get_mut(idx) {
                ev.outcome = Outcome::Error;
            }
        }
    }
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
    let uuid = v.get("uuid").and_then(Value::as_str).map(|s| s.to_string());

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
                let input_raw = part.get("input").map(|i| i.to_string()).unwrap_or_default();
                let input_preview = truncate_prompt(&input_raw);
                out.push(SessionEvent::AssistantToolUse {
                    ts,
                    uuid: uuid.clone(),
                    model: model.clone(),
                    tool_name,
                    tool_use_id,
                    input_preview,
                    input_full: input_raw,
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
#[path = "session_tests.rs"]
mod tests;
