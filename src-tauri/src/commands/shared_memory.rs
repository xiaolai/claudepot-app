//! Tauri commands for the Shared Memory section.
//!
//! Mirrors the MCP server's eight read/write tools, plus the two
//! discovery tools, but with friendlier camelCase-or-snake_case
//! JSON shapes for the frontend.
//!
//! Shares a single `SessionIndex` handle across all commands via
//! Tauri state. `SessionIndex` owns a `Mutex<Connection>` so writes
//! serialize internally; reads inside `spawn_blocking` cross the
//! await point without lock contention against the rest of the app
//! (which also `.manage()`s the same Arc).
//!
//! Mutations (`shared_memory_create_memory`,
//! `shared_memory_log_decision`, `shared_memory_archive_*`, etc.)
//! hit the durable tables. Reads (`shared_memory_search`, list_*)
//! hit transcript-derived rows populated by `claudepot codex index`
//! and `claudepot session backfill-exchanges`.

use claudepot_core::redaction::{apply as redact_apply, RedactionPolicy};
use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::{durable, read as smr, search as sms};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

/// Shared `SessionIndex` handle. Opened once at app startup and
/// `.manage()`d. `None` if startup open failed — the matching
/// commands then return a "session index unavailable" error that
/// the UI renders as a banner instead of crashing.
pub struct SharedMemoryIndex(pub Option<Arc<SessionIndex>>);

fn require_idx(state: &State<'_, SharedMemoryIndex>) -> Result<Arc<SessionIndex>, String> {
    state
        .0
        .as_ref()
        .cloned()
        .ok_or_else(|| "session index unavailable (open failed at startup)".to_string())
}

fn join_err(e: tokio::task::JoinError) -> String {
    format!("blocking task failed: {e}")
}

/// Stricter than `RedactionPolicy::default()` — adds emails +
/// env-assignment masking. Matches the MCP boundary policy so the
/// UI and an MCP client see the same redaction discipline.
fn ui_redaction_policy() -> RedactionPolicy {
    RedactionPolicy {
        anthropic_keys: true,
        emails: true,
        env_assignments: true,
        ..Default::default()
    }
}

// ─── search ──────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SearchArgs {
    pub query: String,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub until_ms: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHitDto {
    pub exchange_id: String,
    pub file_path: String,
    pub session_id: String,
    pub source_kind: String,
    pub project_path: String,
    pub git_branch: Option<String>,
    pub timestamp_ms: Option<i64>,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub snippet: String,
    pub turn_index: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResponseDto {
    pub hits: Vec<SearchHitDto>,
    pub has_more: bool,
}

#[tauri::command]
pub async fn shared_memory_search(
    args: SearchArgs,
    state: State<'_, SharedMemoryIndex>,
) -> Result<SearchResponseDto, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        let user_limit = args.limit.unwrap_or(20).clamp(1, 50);
        let q = sms::SearchQuery {
            query: args.query,
            source_kind: args.source_kind,
            project_path: args.project_path,
            git_branch: None,
            model: None,
            since_ms: args.since_ms,
            until_ms: args.until_ms,
            limit: user_limit + 1, // L1: limit+1 for has_more
            offset: args.offset.unwrap_or(0),
            sort: sms::SearchSort::Relevance,
        };
        let policy = ui_redaction_policy();
        let raw = sms::search(idx.as_ref(), &q, &policy).map_err(|e| format!("search: {e}"))?;
        let has_more = raw.len() as u32 > user_limit;
        let hits = raw
            .into_iter()
            .take(user_limit as usize)
            .map(|h| SearchHitDto {
                exchange_id: h.exchange_id,
                file_path: h.file_path,
                session_id: h.session_id,
                source_kind: h.source_kind,
                project_path: h.project_path,
                git_branch: h.git_branch,
                timestamp_ms: h.timestamp_ms,
                line_start: h.line_start,
                line_end: h.line_end,
                snippet: h.snippet,
                turn_index: h.turn_index,
            })
            .collect();
        Ok::<_, String>(SearchResponseDto { hits, has_more })
    })
    .await
    .map_err(join_err)?
}

// ─── read by locator ─────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ReadLocatorArgs {
    pub file_path: String,
    #[serde(default)]
    pub exchange_id: Option<String>,
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationReadDto {
    pub file_path: String,
    pub exchange_id: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
    pub body: String,
    pub truncated: bool,
}

#[tauri::command]
pub async fn shared_memory_read_locator(
    args: ReadLocatorArgs,
    state: State<'_, SharedMemoryIndex>,
) -> Result<ConversationReadDto, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        let policy = ui_redaction_policy();
        // Same server-side ceiling as MCP: 1 MiB regardless of
        // requested cap.
        let cap = args.max_bytes.unwrap_or(64 * 1024).min(1024 * 1024);
        let locator = smr::ConversationLocator {
            file_path: args.file_path,
            exchange_id: args.exchange_id,
            line_start: None,
            line_end: None,
        };
        let result = smr::read_locator_bounded(idx.as_ref(), &locator, cap, &policy)
            .map_err(|e| format!("read: {e}"))?;
        Ok::<_, String>(ConversationReadDto {
            file_path: result.file_path,
            exchange_id: result.exchange_id,
            line_start: result.line_start,
            line_end: result.line_end,
            body: result.body,
            truncated: result.truncated,
        })
    })
    .await
    .map_err(join_err)?
}

// ─── memories ────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ListMemoriesArgs {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryDto {
    pub id: String,
    pub scope: String,
    pub project_path: Option<String>,
    pub kind: String,
    pub content: String,
    pub created_by_kind: String,
    pub created_by: String,
    pub confidence: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub archived_at_ms: Option<i64>,
}

fn parse_scope(s: Option<&str>) -> Option<durable::Scope> {
    match s {
        Some("global") => Some(durable::Scope::Global),
        Some("project") => Some(durable::Scope::Project),
        _ => None,
    }
}

fn parse_memory_kind(s: Option<&str>) -> Option<durable::MemoryKind> {
    match s {
        Some("fact") => Some(durable::MemoryKind::Fact),
        Some("preference") => Some(durable::MemoryKind::Preference),
        Some("pattern") => Some(durable::MemoryKind::Pattern),
        Some("constraint") => Some(durable::MemoryKind::Constraint),
        Some("summary") => Some(durable::MemoryKind::Summary),
        _ => None,
    }
}

fn parse_decision_status(s: Option<&str>) -> Option<durable::DecisionStatus> {
    match s {
        Some("active") => Some(durable::DecisionStatus::Active),
        Some("superseded") => Some(durable::DecisionStatus::Superseded),
        Some("archived") => Some(durable::DecisionStatus::Archived),
        _ => None,
    }
}

fn scope_str(s: durable::Scope) -> &'static str {
    match s {
        durable::Scope::Global => "global",
        durable::Scope::Project => "project",
    }
}

fn memory_kind_str(k: durable::MemoryKind) -> &'static str {
    match k {
        durable::MemoryKind::Fact => "fact",
        durable::MemoryKind::Preference => "preference",
        durable::MemoryKind::Pattern => "pattern",
        durable::MemoryKind::Constraint => "constraint",
        durable::MemoryKind::Summary => "summary",
    }
}

fn created_by_kind_str(k: durable::CreatedByKind) -> &'static str {
    match k {
        durable::CreatedByKind::User => "user",
        durable::CreatedByKind::Agent => "agent",
        durable::CreatedByKind::Import => "import",
        durable::CreatedByKind::System => "system",
    }
}

fn decision_status_str(s: durable::DecisionStatus) -> &'static str {
    match s {
        durable::DecisionStatus::Active => "active",
        durable::DecisionStatus::Superseded => "superseded",
        durable::DecisionStatus::Archived => "archived",
    }
}

#[tauri::command]
pub async fn shared_memory_list_memories(
    args: ListMemoriesArgs,
    state: State<'_, SharedMemoryIndex>,
) -> Result<Vec<MemoryDto>, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        let policy = ui_redaction_policy();
        let f = durable::MemoryListFilter {
            scope: parse_scope(args.scope.as_deref()),
            project_path: args.project_path,
            kind: parse_memory_kind(args.kind.as_deref()),
            include_archived: args.include_archived.unwrap_or(false),
            limit: args.limit.unwrap_or(0),
        };
        let rows = durable::list_memories(idx.as_ref(), &f).map_err(|e| format!("list: {e}"))?;
        Ok::<_, String>(
            rows.into_iter()
                .map(|m| MemoryDto {
                    id: m.id,
                    scope: scope_str(m.scope).to_string(),
                    project_path: m.project_path,
                    kind: memory_kind_str(m.kind).to_string(),
                    content: redact_apply(&m.content, &policy),
                    created_by_kind: created_by_kind_str(m.created_by_kind).to_string(),
                    created_by: redact_apply(&m.created_by, &policy),
                    confidence: m.confidence,
                    created_at_ms: m.created_at_ms,
                    updated_at_ms: m.updated_at_ms,
                    archived_at_ms: m.archived_at_ms,
                })
                .collect(),
        )
    })
    .await
    .map_err(join_err)?
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateMemoryArgs {
    pub scope: String,
    #[serde(default)]
    pub project_path: Option<String>,
    pub kind: String,
    pub content: String,
    pub created_by: String,
    #[serde(default)]
    pub confidence: Option<i64>,
}

#[tauri::command]
pub async fn shared_memory_create_memory(
    args: CreateMemoryArgs,
    state: State<'_, SharedMemoryIndex>,
) -> Result<MemoryDto, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        let scope = parse_scope(Some(args.scope.as_str()))
            .ok_or_else(|| "invalid scope".to_string())?;
        let kind = parse_memory_kind(Some(args.kind.as_str()))
            .ok_or_else(|| "invalid kind".to_string())?;
        let confidence = args.confidence.map(|c| c.clamp(0, 100));
        let new = durable::NewMemory {
            scope,
            project_path: args.project_path.as_deref(),
            kind,
            content: &args.content,
            created_by_kind: durable::CreatedByKind::User,
            created_by: &args.created_by,
            confidence,
        };
        let m = durable::create_memory(idx.as_ref(), &new).map_err(|e| format!("create: {e}"))?;
        Ok::<_, String>(MemoryDto {
            id: m.id,
            scope: scope_str(m.scope).to_string(),
            project_path: m.project_path,
            kind: memory_kind_str(m.kind).to_string(),
            content: m.content,
            created_by_kind: created_by_kind_str(m.created_by_kind).to_string(),
            created_by: m.created_by,
            confidence: m.confidence,
            created_at_ms: m.created_at_ms,
            updated_at_ms: m.updated_at_ms,
            archived_at_ms: m.archived_at_ms,
        })
    })
    .await
    .map_err(join_err)?
}

#[tauri::command]
pub async fn shared_memory_archive_memory(
    id: String,
    state: State<'_, SharedMemoryIndex>,
) -> Result<bool, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        durable::archive_memory(idx.as_ref(), &id).map_err(|e| format!("archive: {e}"))
    })
    .await
    .map_err(join_err)?
}

// ─── decisions ───────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ListDecisionsArgs {
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionDto {
    pub id: String,
    pub project_path: Option<String>,
    pub topic: Option<String>,
    pub decision: String,
    pub rationale: Option<String>,
    pub status: String,
    pub created_by_kind: String,
    pub created_by: String,
    pub created_at_ms: i64,
    pub supersedes_id: Option<String>,
}

#[tauri::command]
pub async fn shared_memory_list_decisions(
    args: ListDecisionsArgs,
    state: State<'_, SharedMemoryIndex>,
) -> Result<Vec<DecisionDto>, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        let policy = ui_redaction_policy();
        let f = durable::DecisionListFilter {
            project_path: args.project_path,
            status: parse_decision_status(args.status.as_deref()),
            limit: args.limit.unwrap_or(0),
        };
        let rows = durable::list_decisions(idx.as_ref(), &f).map_err(|e| format!("list: {e}"))?;
        Ok::<_, String>(
            rows.into_iter()
                .map(|d| DecisionDto {
                    id: d.id,
                    project_path: d.project_path,
                    topic: d.topic.map(|t| redact_apply(&t, &policy)),
                    decision: redact_apply(&d.decision, &policy),
                    rationale: d.rationale.map(|r| redact_apply(&r, &policy)),
                    status: decision_status_str(d.status).to_string(),
                    created_by_kind: created_by_kind_str(d.created_by_kind).to_string(),
                    created_by: redact_apply(&d.created_by, &policy),
                    created_at_ms: d.created_at_ms,
                    supersedes_id: d.supersedes_id,
                })
                .collect(),
        )
    })
    .await
    .map_err(join_err)?
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogDecisionArgs {
    pub decision: String,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    pub created_by: String,
    #[serde(default)]
    pub supersedes_id: Option<String>,
}

#[tauri::command]
pub async fn shared_memory_log_decision(
    args: LogDecisionArgs,
    state: State<'_, SharedMemoryIndex>,
) -> Result<DecisionDto, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        let new = durable::NewDecision {
            project_path: args.project_path.as_deref(),
            topic: args.topic.as_deref(),
            decision: &args.decision,
            rationale: args.rationale.as_deref(),
            created_by_kind: durable::CreatedByKind::User,
            created_by: &args.created_by,
        };
        let d = if let Some(ref prior) = args.supersedes_id {
            durable::supersede_decision(idx.as_ref(), prior, &new)
        } else {
            durable::log_decision(idx.as_ref(), &new)
        }
        .map_err(|e| format!("log: {e}"))?;
        Ok::<_, String>(DecisionDto {
            id: d.id,
            project_path: d.project_path,
            topic: d.topic,
            decision: d.decision,
            rationale: d.rationale,
            status: decision_status_str(d.status).to_string(),
            created_by_kind: created_by_kind_str(d.created_by_kind).to_string(),
            created_by: d.created_by,
            created_at_ms: d.created_at_ms,
            supersedes_id: d.supersedes_id,
        })
    })
    .await
    .map_err(join_err)?
}

#[tauri::command]
pub async fn shared_memory_archive_decision(
    id: String,
    state: State<'_, SharedMemoryIndex>,
) -> Result<bool, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        durable::archive_decision(idx.as_ref(), &id).map_err(|e| format!("archive: {e}"))
    })
    .await
    .map_err(join_err)?
}

// ─── discovery ───────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ListSessionsArgs {
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummaryDto {
    pub file_path: String,
    pub session_id: String,
    pub source_kind: String,
    pub project_path: String,
    pub git_branch: Option<String>,
    pub first_ts_ms: Option<i64>,
    pub last_ts_ms: Option<i64>,
    pub message_count: i64,
    pub tokens_input: i64,
    pub tokens_output: i64,
}

#[tauri::command]
pub async fn shared_memory_list_sessions(
    args: ListSessionsArgs,
    state: State<'_, SharedMemoryIndex>,
) -> Result<Vec<SessionSummaryDto>, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        let f = sms::SessionListFilter {
            source_kind: args.source_kind,
            project_path: args.project_path,
            since_ms: args.since_ms,
            limit: args.limit.unwrap_or(0),
            offset: args.offset.unwrap_or(0),
        };
        let rows = sms::list_sessions(idx.as_ref(), &f).map_err(|e| format!("list: {e}"))?;
        Ok::<_, String>(
            rows.into_iter()
                .map(|s| SessionSummaryDto {
                    file_path: s.file_path,
                    session_id: s.session_id,
                    source_kind: s.source_kind,
                    project_path: s.project_path,
                    git_branch: s.git_branch,
                    first_ts_ms: s.first_ts_ms,
                    last_ts_ms: s.last_ts_ms,
                    message_count: s.message_count,
                    tokens_input: s.tokens_input,
                    tokens_output: s.tokens_output,
                })
                .collect(),
        )
    })
    .await
    .map_err(join_err)?
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummaryDto {
    pub project_path: String,
    pub session_count: i64,
    pub last_activity_ms: Option<i64>,
}

#[tauri::command]
pub async fn shared_memory_list_projects(
    limit: Option<u32>,
    state: State<'_, SharedMemoryIndex>,
) -> Result<Vec<ProjectSummaryDto>, String> {
    let idx = require_idx(&state)?;
    tokio::task::spawn_blocking(move || {
        let rows = sms::list_projects(idx.as_ref(), limit.unwrap_or(0))
            .map_err(|e| format!("list: {e}"))?;
        Ok::<_, String>(
            rows.into_iter()
                .map(|p| ProjectSummaryDto {
                    project_path: p.project_path,
                    session_count: p.session_count,
                    last_activity_ms: p.last_activity_ms,
                })
                .collect(),
        )
    })
    .await
    .map_err(join_err)?
}

// ─── installer / MCP health ──────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SnippetInstallResultDto {
    pub path: String,
    pub bytes_written: usize,
    pub include_line: String,
}

/// Write the canonical `claudepot-mcp-instructions.md` snippet to
/// `~/.claude/claudepot-mcp-instructions.md` (or override). Mirrors
/// the `claudepot mcp install-snippet` CLI verb so the GUI
/// installer pane can call it without shelling out.
///
/// The snippet body is fetched from the CLI module's
/// `snippet_body()` so there's only one canonical source.
#[tauri::command]
pub async fn shared_memory_install_snippet(
    out: Option<String>,
) -> Result<SnippetInstallResultDto, String> {
    tokio::task::spawn_blocking(move || {
        let path = match out {
            Some(p) => std::path::PathBuf::from(p),
            None => dirs::home_dir()
                .ok_or_else(|| "no home dir".to_string())?
                .join(".claude")
                .join("claudepot-mcp-instructions.md"),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create parent: {e}"))?;
        }
        let body = canonical_snippet();
        std::fs::write(&path, &body).map_err(|e| format!("write: {e}"))?;
        Ok::<_, String>(SnippetInstallResultDto {
            include_line: format!("@include {}", path.display()),
            path: path.display().to_string(),
            bytes_written: body.len(),
        })
    })
    .await
    .map_err(join_err)?
}

/// Returns the snippet body without writing. UI shows this in a
/// preview panel so the user can copy or read before installing.
#[tauri::command]
pub async fn shared_memory_snippet_body() -> Result<String, String> {
    Ok(canonical_snippet())
}

/// Lightweight MCP-server probe. Spawns `claudepot mcp memory-server`
/// in a subprocess, sends an `initialize` + `tools/list`, counts
/// the tools, and returns a structured health report. Used by the
/// Settings → MCP pane to render a "tool_visible" badge.
#[derive(Debug, Clone, Serialize)]
pub struct McpHealthDto {
    pub tool_visible: bool,
    pub tool_count: usize,
    pub error: Option<String>,
}

/// Resolve the `claudepot` CLI binary the user would configure as
/// the MCP command. The GUI binary (`current_exe()`) doesn't have
/// the `mcp memory-server` subcommand — that's only on the CLI
/// crate. Probe order:
///
///   1. Explicit override.
///   2. Sibling of `current_exe()`. In dev: `target/debug/claudepot`
///      (the CLI's `[[bin]] name`). In a prod bundle:
///      `Contents/MacOS/claudepot-cli` (the externalBin-resolved
///      sidecar that `tauri-cli` copies in at bundle time).
///   3. Sibling with the platform-triple suffix that `externalBin`
///      uses pre-bundle (`claudepot-cli-aarch64-apple-darwin` etc.).
fn resolve_cli_binary(
    override_path: Option<String>,
) -> Result<std::path::PathBuf, String> {
    if let Some(p) = override_path {
        return Ok(std::path::PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "current_exe has no parent".to_string())?;
    // In dev builds, prefer `claudepot` (the CLI crate's
    // `[[bin]] name`). A stale `claudepot-cli` artifact from a
    // pre-rename build may still sit next to it in `target/debug/`
    // — probing that first would spawn the wrong binary.
    // In release builds the sidecar is named `claudepot-cli`.
    let mut candidates: Vec<std::path::PathBuf> = if cfg!(debug_assertions) {
        vec![
            dir.join("claudepot"),     // dev: target/debug/claudepot
            dir.join("claudepot-cli"), // dev: stale rename fallback
        ]
    } else {
        vec![
            dir.join("claudepot-cli"), // prod bundle: Contents/MacOS/claudepot-cli
            dir.join("claudepot"),     // fallback if installed manually
        ]
    };
    // Pre-bundle externalBin candidates carry the host target triple.
    #[cfg(target_os = "macos")]
    {
        candidates.push(dir.join("claudepot-cli-aarch64-apple-darwin"));
        candidates.push(dir.join("claudepot-cli-x86_64-apple-darwin"));
    }
    #[cfg(target_os = "linux")]
    {
        candidates.push(dir.join("claudepot-cli-x86_64-unknown-linux-gnu"));
        candidates.push(dir.join("claudepot-cli-aarch64-unknown-linux-gnu"));
    }
    #[cfg(target_os = "windows")]
    {
        candidates.push(dir.join("claudepot-cli-x86_64-pc-windows-msvc.exe"));
    }
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    Err(format!(
        "no claudepot CLI binary found next to {} (tried: {})",
        exe.display(),
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[tauri::command]
pub async fn shared_memory_mcp_health(claudepot_binary: Option<String>) -> Result<McpHealthDto, String> {
    tokio::task::spawn_blocking(move || {
        use std::io::{BufRead, BufReader, Write};
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let bin = match resolve_cli_binary(claudepot_binary) {
            Ok(p) => p,
            Err(e) => {
                return Ok::<_, String>(McpHealthDto {
                    tool_visible: false,
                    tool_count: 0,
                    error: Some(e),
                });
            }
        };

        let mut child = Command::new(&bin)
            .args(["mcp", "memory-server"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("RUST_LOG", "warn")
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", bin.display()))?;

        let stdin_handle = child.stdin.take();
        let stdout_handle = child.stdout.take().ok_or_else(|| "no stdout".to_string())?;

        if let Some(mut stdin) = stdin_handle {
            let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"claudepot-health\",\"version\":\"0\"}}}\n\
                          {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                          {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";
            let _ = stdin.write_all(frames.as_bytes());
            drop(stdin); // EOF → server processes queued + exits
        }

        let stderr_handle = child.stderr.take();

        let mut reader = BufReader::new(stdout_handle);
        let mut tool_count = 0usize;
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let v: serde_json::Value = match serde_json::from_str(trimmed) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if v.get("id").and_then(|i| i.as_u64()) == Some(2) {
                        if let Some(tools) = v
                            .get("result")
                            .and_then(|r| r.get("tools"))
                            .and_then(|t| t.as_array())
                        {
                            tool_count = tools.len();
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();

        // On failure, drain stderr (first 1 KiB) so the UI can show
        // *why* the probe didn't see any tools — wrong binary,
        // missing subcommand, panic, etc. — instead of a bare
        // "failed" badge.
        let error = if tool_count == 0 {
            let mut buf = String::new();
            if let Some(mut s) = stderr_handle {
                use std::io::Read;
                let mut take = (&mut s).take(1024);
                let _ = take.read_to_string(&mut buf);
            }
            let trimmed = buf.trim();
            Some(if trimmed.is_empty() {
                format!(
                    "spawned {} but no tools/list response within 8s",
                    bin.display()
                )
            } else {
                format!("stderr from {}: {trimmed}", bin.display())
            })
        } else {
            None
        };

        Ok::<_, String>(McpHealthDto {
            tool_visible: tool_count > 0,
            tool_count,
            error,
        })
    })
    .await
    .map_err(join_err)?
}

// ─── shared snippet body ─────────────────────────────────────

const SNIPPET_VERSION: &str = "1";

/// Same content as `claudepot-cli/src/commands/mcp.rs::snippet_body()`.
/// Duplicated here so the Tauri command doesn't take an unwanted dep
/// on the CLI crate; the canonical text is checked in to both
/// locations and a CI gate could enforce drift (out of scope for
/// this commit).
fn canonical_snippet() -> String {
    format!(
        r#"<!-- claudepot-mcp-instructions v{version} — managed by `claudepot mcp install-snippet` or Claudepot GUI -->

# Claudepot shared memory

You have access to the Claudepot MCP memory server via tools
prefixed `claudepot_*`. Use them. Specifically:

- **At the start of a session in a project**, call
  `claudepot_list_decisions(project_path=cwd(), status="active")`
  and `claudepot_list_memories(scope="project", project_path=cwd())`.

- **Before asking the user a question that history might
  answer**, call `claudepot_search_memory(query)` and follow up
  with `claudepot_read_conversation` on the hit's file_path.

- **When the user states a durable fact**, call
  `claudepot_remember(scope="project", project_path=cwd(),
  kind="...", content="...", created_by="...")`.

- **On a non-trivial design decision**, call
  `claudepot_log_decision(...)`. Pass `supersedes_id` if replacing.

- **After an audit/fix loop**, call `claudepot_submit_evidence(...)`.

- **For discovery**, `claudepot_list_sessions(project_path=cwd())`
  and `claudepot_list_projects()`.

All `created_by` ids should identify YOU. Future regenerations of
this file (re-running install-snippet) refresh the content; the
`@include` line in your CLAUDE.md / AGENTS.md never has to change.
"#,
        version = SNIPPET_VERSION,
    )
}
