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
        // Cap at 49 (not 50): the `+1` probe below must fit within
        // `shared_memory::search::search`'s internal `clamp(1, 50)`
        // ceiling, otherwise the probe is silently truncated and
        // `has_more` is unreachable at the maximum page size.
        let user_limit = args.limit.unwrap_or(20).clamp(1, 49);
        let q = sms::SearchQuery {
            query: args.query,
            source_kind: args.source_kind,
            project_path: args.project_path,
            // No confinement in the GUI. `project_path_exact` carries
            // the MCP server's cross-project boundary (see
            // shared_memory::scope) — that boundary exists to stop an
            // *agent* in one project reading another. The user
            // browsing their own machine in their own app is not that
            // threat, and the Memory tab is expressly cross-project.
            project_path_exact: None,
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
        let scope =
            parse_scope(Some(args.scope.as_str())).ok_or_else(|| "invalid scope".to_string())?;
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
        // `None` = unconfined; see the note in shared_memory_search.
        let rows = sms::list_projects(idx.as_ref(), limit.unwrap_or(0), None)
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
    pub scope: String,
    pub path: String,
    pub bytes_written: usize,
    pub include_line: String,
    /// Files the user is expected to paste `include_line` into. For
    /// user scope: the three agent home configs that auto-load every
    /// session. For project scope: only `AGENTS.md` per
    /// `/init-workspace`'s rule (CLAUDE.md / GEMINI.md just import
    /// AGENTS.md and shouldn't be hand-edited).
    pub target_files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallSnippetArgs {
    /// "user" (default) or "project".
    #[serde(default)]
    pub scope: Option<String>,
    /// Required when scope = "project". Must be an existing
    /// directory; `<project_path>/.claude/` is created if missing
    /// but the project root itself is not.
    #[serde(default)]
    pub project_path: Option<String>,
    /// Power-user escape hatch: write to this exact path instead
    /// of the scope-derived default. Bypasses scope/project_path.
    #[serde(default)]
    pub out: Option<String>,
}

/// Write the canonical `claudepot-mcp-instructions.md` snippet to
/// either user-scope (`~/.claude/`) or project-scope
/// (`<project>/.claude/`). Project scope honors
/// `/init-workspace`'s convention — the recommended paste target
/// is the project's `AGENTS.md` (the canonical source-of-truth);
/// `CLAUDE.md` / `GEMINI.md` are `@AGENTS.md` re-exports and
/// shouldn't be hand-edited.
#[tauri::command]
pub async fn shared_memory_install_snippet(
    args: InstallSnippetArgs,
) -> Result<SnippetInstallResultDto, String> {
    tokio::task::spawn_blocking(move || {
        use claudepot_core::mcp_snippet::{install, InstallScope};

        let scope = match args.scope.as_deref().unwrap_or("user") {
            "user" => InstallScope::User,
            "project" => InstallScope::Project,
            other => {
                return Err(format!(
                    "invalid scope: {other:?} (expected \"user\" or \"project\")"
                ))
            }
        };
        // Renderer-supplied paths must be absolute before they reach
        // the shared installer (its `out` arm is the trusted
        // CLI-flag escape hatch and does no validation itself).
        let out = args.out.as_deref().map(std::path::Path::new);
        if let Some(o) = out {
            if !o.is_absolute() {
                return Err(format!("out path must be absolute: {}", o.display()));
            }
        }
        let project_root = args.project_path.as_deref().map(std::path::Path::new);

        // Path policy, validation (absolute + existing dir for
        // project scope), the write, and the @-import line all live
        // in `claudepot_core::mcp_snippet::install` — shared with
        // `claudepot mcp install-snippet` so the two can't drift.
        let report = install(scope, project_root, out).map_err(|e| e.to_string())?;

        Ok::<_, String>(SnippetInstallResultDto {
            scope: report.scope.as_str().to_string(),
            path: report.path.display().to_string(),
            bytes_written: report.bytes_written,
            include_line: report.include_line,
            target_files: report
                .target_files
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
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

/// The probe itself (JSON-RPC framing, read loop, failure
/// classification) and the sibling-CLI resolution policy live in
/// `claudepot_core::mcp_probe` — unit-tested there. This command
/// only resolves the optional override, supplies `current_exe()`'s
/// dir (the one thing core can't know), and maps to the DTO.
#[tauri::command]
pub async fn shared_memory_mcp_health(
    claudepot_binary: Option<String>,
) -> Result<McpHealthDto, String> {
    use claudepot_core::mcp_probe;

    let bin = match claudepot_binary {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let dir = exe
                .parent()
                .ok_or_else(|| "current_exe has no parent".to_string())?;
            match mcp_probe::resolve_sibling_cli(dir, cfg!(debug_assertions)) {
                Ok(p) => p,
                Err(e) => {
                    // "No binary found" is a renderable badge state,
                    // not a hard IPC error — matches the old shape.
                    return Ok(McpHealthDto {
                        tool_visible: false,
                        tool_count: 0,
                        error: Some(e.to_string()),
                    });
                }
            }
        }
    };

    let report = mcp_probe::probe_memory_server(&bin, std::time::Duration::from_secs(8))
        .await
        .map_err(|e| e.to_string())?;
    Ok(McpHealthDto {
        tool_visible: report.tool_visible,
        tool_count: report.tool_count,
        error: report.error,
    })
}

// Canonical snippet body lives in `claudepot_core::mcp_snippet`
// so the CLI and the Tauri installer emit the same bytes. Audit
// 2026-05 found these had drifted within a single release.
use claudepot_core::mcp_snippet::snippet_body as canonical_snippet;
