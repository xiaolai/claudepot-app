//! Claudepot MCP memory server (WI-008).
//!
//! `claudepot mcp memory-server` starts a stdio MCP server exposing
//! seven tools backed by the shared_memory module:
//!
//! * `claudepot_search_memory`
//! * `claudepot_read_conversation`
//! * `claudepot_remember`
//! * `claudepot_log_decision`
//! * `claudepot_submit_evidence`
//! * `claudepot_list_memories`
//! * `claudepot_list_decisions`
//!
//! All emission paths run through `claudepot_core::redaction::apply`
//! before crossing the MCP boundary. Server logs go to stderr only;
//! stdout is reserved for JSON-RPC frames.
//!
//! Spike verdict in `dev-docs/reports/rmcp-spike-2026-05-15.md`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{schemars, tool, tool_router, ServiceExt};
use serde::{Deserialize, Serialize};

use claudepot_core::redaction::{apply as redact_apply, RedactionPolicy};
use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::{durable, read as smr, search as sms};

const SCHEMA_VERSION: u32 = 1;

/// Default DB path: `~/.claudepot/sessions.db`.
fn default_db_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claudepot")
        .join("sessions.db")
}

/// Build the redaction policy used at every MCP emission. Stricter
/// than `RedactionPolicy::default()` because the MCP boundary is
/// the riskiest emission surface — LLM clients see exactly what
/// the server returns, and prompt-injected agents can ask for
/// arbitrary content. Masks `sk-ant-*` tokens, email addresses,
/// and `FOO=bar` env-assignment lines.
///
/// At-rest data in `~/.claudepot/sessions.db` is unredacted per R9;
/// this policy applies only to emission.
fn mcp_redaction_policy() -> RedactionPolicy {
    RedactionPolicy {
        anthropic_keys: true,
        emails: true,
        env_assignments: true,
        ..Default::default()
    }
}

// The canonical snippet body lives in `claudepot_core::mcp_snippet`
// so the CLI installer and the Tauri Settings → MCP pane both emit
// the same bytes. Re-export the public surface here to keep the
// CLI's call sites short.

pub use claudepot_core::mcp_snippet::snippet_body;

/// Default install path: `~/.claude/claudepot-mcp-instructions.md`.
/// Sits next to CLAUDE.md so an `@include
/// ~/.claude/claudepot-mcp-instructions.md` line in CLAUDE.md
/// resolves cleanly.
fn default_snippet_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("claudepot-mcp-instructions.md")
}

/// Print the snippet to stdout. For users who want to paste it
/// manually instead of using `install-snippet`.
pub fn print_snippet() -> Result<()> {
    print!("{}", snippet_body());
    Ok(())
}

/// Write the snippet to `~/.claude/claudepot-mcp-instructions.md`
/// (or the override). Idempotent — re-running overwrites the file
/// with the current canonical content. After writing, optionally
/// print the recommended `@include` line.
pub fn install_snippet(out: Option<PathBuf>, print_include: bool) -> Result<()> {
    let path = out.unwrap_or_else(default_snippet_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create parent of {}", path.display()))?;
    }
    std::fs::write(&path, snippet_body())
        .with_context(|| format!("write {}", path.display()))?;
    eprintln!("Wrote {} ({} bytes)", path.display(), snippet_body().len());
    if print_include {
        eprintln!();
        eprintln!("Add this single line to your CLAUDE.md and/or AGENTS.md:");
        eprintln!();
        eprintln!("    @include {}", path.display());
        eprintln!();
        eprintln!("Re-run `claudepot mcp install-snippet` to refresh the snippet content.");
        eprintln!("The @include line never needs to change.");
    }
    Ok(())
}

/// Entry point for the `mcp memory-server` subcommand.
pub async fn run(db_path: Option<PathBuf>) -> Result<()> {
    let path = db_path.unwrap_or_else(default_db_path);
    let idx = Arc::new(SessionIndex::open(&path)?);
    tracing::info!(db = %path.display(), "claudepot mcp memory-server starting");
    let server = MemoryServer {
        idx,
        policy: Arc::new(mcp_redaction_policy()),
    };
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[derive(Clone)]
struct MemoryServer {
    idx: Arc<SessionIndex>,
    policy: Arc<RedactionPolicy>,
}

// ─── tool input/output structs ────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchMemoryRequest {
    /// Free-text query. Phrase-escaped before reaching FTS5; safe to
    /// pass user-supplied secrets, operators, or quotes.
    query: String,
    /// Restrict by transcript origin: `claude_code` or `codex`.
    #[serde(default)]
    source_kind: Option<String>,
    /// Substring match on project path.
    #[serde(default)]
    project_path: Option<String>,
    /// Page size. Defaults to 20; capped at 50.
    #[serde(default)]
    limit: Option<u32>,
    /// Pagination offset.
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SearchHitOut {
    exchange_id: String,
    file_path: String,
    session_id: String,
    source_kind: String,
    project_path: String,
    timestamp_ms: Option<i64>,
    line_start: Option<i64>,
    line_end: Option<i64>,
    snippet: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SearchPayload {
    schema_version: u32,
    hits: Vec<SearchHitOut>,
    /// True when more results exist past this page.
    has_more: bool,
}

/// Server-side ceiling on the body size a single
/// `claudepot_read_conversation` call can return. Even with a
/// huge `max_bytes` from the client, the server will not allocate
/// more than this. 1 MiB is far above any reasonable transcript
/// excerpt and matches `MAX_LINE_BYTES` for the parser side.
const READ_MAX_BYTES_CEILING: usize = 1024 * 1024;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadConversationRequest {
    /// `sessions.file_path` returned in a SearchHitOut.
    file_path: String,
    /// Optional exchange id (`<session_id>:<turn_index>`); when
    /// present, the read is bounded to that exchange's line range.
    #[serde(default)]
    exchange_id: Option<String>,
    /// Byte cap on the returned body. Defaults to 16 KiB; the
    /// server clamps to 1 MiB regardless of what the client asks
    /// for.
    #[serde(default)]
    max_bytes: Option<usize>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ReadPayload {
    schema_version: u32,
    file_path: String,
    exchange_id: Option<String>,
    line_start: u32,
    line_end: u32,
    /// Already redacted; emit verbatim.
    body: String,
    truncated: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RememberRequest {
    /// `global` or `project`.
    scope: String,
    /// Required when scope=`project`; must be omitted when scope=`global`.
    #[serde(default)]
    project_path: Option<String>,
    /// `fact` | `preference` | `pattern` | `constraint` | `summary`.
    kind: String,
    content: String,
    /// Free-form actor id (e.g. `codex@2026-05-15`, `claude-code`).
    #[serde(default)]
    created_by: Option<String>,
    /// 0..=100. Optional self-rating.
    #[serde(default)]
    confidence: Option<i64>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct RememberPayload {
    schema_version: u32,
    id: String,
    scope: String,
    kind: String,
    created_at_ms: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LogDecisionRequest {
    decision: String,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    project_path: Option<String>,
    #[serde(default)]
    supersedes_id: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct LogDecisionPayload {
    schema_version: u32,
    id: String,
    status: String,
    supersedes_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SubmitEvidenceRequest {
    summary: String,
    verification: String,
    /// JSON-encoded array of relative file paths.
    files_changed: String,
    /// 0..=100.
    confidence: i64,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    project_path: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SubmitEvidencePayload {
    schema_version: u32,
    id: String,
    created_at_ms: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListMemoriesRequest {
    /// Optional. When absent, all memories returned (capped).
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    project_path: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    include_archived: Option<bool>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct MemoryOut {
    id: String,
    scope: String,
    project_path: Option<String>,
    kind: String,
    content: String,
    created_by_kind: String,
    created_by: String,
    created_at_ms: i64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ListMemoriesPayload {
    schema_version: u32,
    memories: Vec<MemoryOut>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ArchiveDecisionRequest {
    /// The decision id to flip to `archived`. Use when a decision
    /// is no longer in force but wasn't replaced by a specific
    /// successor (use `claudepot_log_decision` with `supersedes_id`
    /// when there's a replacement).
    id: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ArchiveDecisionPayload {
    schema_version: u32,
    /// True if the decision transitioned from `active` to
    /// `archived`. False if the id didn't reference an active
    /// decision (already archived, superseded, or doesn't exist).
    archived: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListDecisionsRequest {
    #[serde(default)]
    project_path: Option<String>,
    /// `active`, `superseded`, or `archived`.
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct DecisionOut {
    id: String,
    project_path: Option<String>,
    topic: Option<String>,
    decision: String,
    rationale: Option<String>,
    status: String,
    created_by_kind: String,
    created_by: String,
    created_at_ms: i64,
    supersedes_id: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ListDecisionsPayload {
    schema_version: u32,
    decisions: Vec<DecisionOut>,
}

// ─── server impl ──────────────────────────────────────────────

#[tool_router(server_handler)]
impl MemoryServer {
    #[tool(description = "Search Claudepot's shared memory across Claude Code and Codex transcripts. Returns compact hits with locators; raw secrets are redacted in snippets.")]
    fn claudepot_search_memory(
        &self,
        Parameters(req): Parameters<SearchMemoryRequest>,
    ) -> String {
        let user_limit = req.limit.unwrap_or(20).clamp(1, 50);
        // Query one extra row so we can report has_more without
        // the boundary-false-positive of `len >= limit`. Slice
        // back to user_limit before returning.
        let q = sms::SearchQuery {
            query: req.query,
            source_kind: req.source_kind,
            project_path: req.project_path,
            git_branch: None,
            model: None,
            since_ms: None,
            until_ms: None,
            limit: user_limit + 1,
            offset: req.offset.unwrap_or(0),
            sort: sms::SearchSort::Relevance,
        };
        let raw_hits = match sms::search(&self.idx, &q, &self.policy) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "claudepot_search_memory: search failed");
                return to_json(&error_with(error_code::SEARCH_FAILED, &e, &self.policy));
            }
        };
        let has_more = raw_hits.len() as u32 > user_limit;
        let payload = SearchPayload {
            schema_version: SCHEMA_VERSION,
            hits: raw_hits
                .into_iter()
                .take(user_limit as usize)
                .map(|h| SearchHitOut {
                    exchange_id: h.exchange_id,
                    file_path: h.file_path,
                    session_id: h.session_id,
                    source_kind: h.source_kind,
                    project_path: h.project_path,
                    timestamp_ms: h.timestamp_ms,
                    line_start: h.line_start,
                    line_end: h.line_end,
                    snippet: h.snippet,
                })
                .collect(),
            has_more,
        };
        to_json(&payload)
    }

    #[tool(description = "Read a transcript excerpt by locator. file_path must be a value previously returned by claudepot_search_memory; raw paths are rejected.")]
    fn claudepot_read_conversation(
        &self,
        Parameters(req): Parameters<ReadConversationRequest>,
    ) -> String {
        let locator = smr::ConversationLocator {
            file_path: req.file_path,
            exchange_id: req.exchange_id,
            line_start: None,
            line_end: None,
        };
        // Clamp client-supplied max_bytes to the server ceiling
        // so a single MCP call can't force unbounded allocation.
        let cap = req
            .max_bytes
            .unwrap_or(16 * 1024)
            .min(READ_MAX_BYTES_CEILING);
        match smr::read_locator_bounded(&self.idx, &locator, cap, &self.policy) {
            Ok(r) => to_json(&ReadPayload {
                schema_version: SCHEMA_VERSION,
                file_path: r.file_path,
                exchange_id: r.exchange_id,
                line_start: r.line_start,
                line_end: r.line_end,
                body: r.body,
                truncated: r.truncated,
            }),
            Err(e) => {
                let code = match &e {
                    smr::ReadError::NotIndexed(_) => error_code::LOCATOR_NOT_INDEXED,
                    smr::ReadError::Io { .. } | smr::ReadError::Sql(_) => error_code::READ_FAILED,
                };
                tracing::warn!(error = %e, "claudepot_read_conversation: read failed");
                to_json(&error_with(code, &e, &self.policy))
            }
        }
    }

    #[tool(description = "Store a durable memory (fact / preference / pattern / constraint / summary). Survives transcript rebuilds.")]
    fn claudepot_remember(
        &self,
        Parameters(req): Parameters<RememberRequest>,
    ) -> String {
        let scope = match req.scope.as_str() {
            "global" => durable::Scope::Global,
            "project" => durable::Scope::Project,
            _ => {
                return to_json(&error_static(
                    error_code::INVALID_SCOPE,
                    "scope must be 'global' or 'project'",
                ));
            }
        };
        let kind = match req.kind.as_str() {
            "fact" => durable::MemoryKind::Fact,
            "preference" => durable::MemoryKind::Preference,
            "pattern" => durable::MemoryKind::Pattern,
            "constraint" => durable::MemoryKind::Constraint,
            "summary" => durable::MemoryKind::Summary,
            _ => {
                return to_json(&error_static(
                    error_code::INVALID_KIND,
                    "kind must be one of fact|preference|pattern|constraint|summary",
                ));
            }
        };
        let created_by = req
            .created_by
            .clone()
            .unwrap_or_else(|| "agent:unknown".to_string());
        let pp = req.project_path.as_deref();
        // L4: clamp confidence to [0, 100] for consistency with
        // submit_evidence and to keep the column's semantic
        // interpretation (a percentage) intact.
        let confidence = req.confidence.map(|c| c.clamp(0, 100));
        let new = durable::NewMemory {
            scope,
            project_path: pp,
            kind,
            content: &req.content,
            created_by_kind: durable::CreatedByKind::Agent,
            created_by: &created_by,
            confidence,
        };
        match durable::create_memory(&self.idx, &new) {
            Ok(m) => to_json(&RememberPayload {
                schema_version: SCHEMA_VERSION,
                id: m.id,
                scope: req.scope,
                kind: req.kind,
                created_at_ms: m.created_at_ms,
            }),
            Err(e) => {
                let code = match &e {
                    durable::DurableError::InvalidScope { .. } => error_code::INVALID_SCOPE,
                    _ => error_code::WRITE_FAILED,
                };
                tracing::warn!(error = %e, "claudepot_remember: create failed");
                to_json(&error_with(code, &e, &self.policy))
            }
        }
    }

    #[tool(description = "Log a durable decision with rationale. If supersedes_id is set, the prior decision flips to 'superseded' atomically.")]
    fn claudepot_log_decision(
        &self,
        Parameters(req): Parameters<LogDecisionRequest>,
    ) -> String {
        let created_by = req
            .created_by
            .clone()
            .unwrap_or_else(|| "agent:unknown".to_string());
        let new = durable::NewDecision {
            project_path: req.project_path.as_deref(),
            topic: req.topic.as_deref(),
            decision: &req.decision,
            rationale: req.rationale.as_deref(),
            created_by_kind: durable::CreatedByKind::Agent,
            created_by: &created_by,
        };
        let result = if let Some(ref prior) = req.supersedes_id {
            durable::supersede_decision(&self.idx, prior, &new)
        } else {
            durable::log_decision(&self.idx, &new)
        };
        match result {
            Ok(d) => to_json(&LogDecisionPayload {
                schema_version: SCHEMA_VERSION,
                id: d.id,
                status: "active".to_string(),
                supersedes_id: d.supersedes_id,
            }),
            Err(e) => {
                let code = match &e {
                    durable::DurableError::DecisionNotFound(_) => error_code::DECISION_NOT_FOUND,
                    _ => error_code::WRITE_FAILED,
                };
                tracing::warn!(error = %e, "claudepot_log_decision: write failed");
                to_json(&error_with(code, &e, &self.policy))
            }
        }
    }

    #[tool(description = "Submit evidence for a task or audit-fix run. Records the verification step and the file changes so future sessions don't re-discover the same finding.")]
    fn claudepot_submit_evidence(
        &self,
        Parameters(req): Parameters<SubmitEvidenceRequest>,
    ) -> String {
        let created_by = req
            .created_by
            .clone()
            .unwrap_or_else(|| "agent:unknown".to_string());
        let new = durable::NewEvidence {
            project_path: req.project_path.as_deref(),
            topic: req.topic.as_deref(),
            summary: &req.summary,
            verification: &req.verification,
            files_changed_json: &req.files_changed,
            confidence: req.confidence.clamp(0, 100),
            created_by_kind: durable::CreatedByKind::Agent,
            created_by: &created_by,
        };
        match durable::submit_evidence(&self.idx, &new) {
            Ok(e) => to_json(&SubmitEvidencePayload {
                schema_version: SCHEMA_VERSION,
                id: e.id,
                created_at_ms: e.created_at_ms,
            }),
            Err(e) => {
                tracing::warn!(error = %e, "claudepot_submit_evidence: write failed");
                to_json(&error_with(error_code::WRITE_FAILED, &e, &self.policy))
            }
        }
    }

    #[tool(description = "List durable memories. Without filters, returns the most recent (up to 100).")]
    fn claudepot_list_memories(
        &self,
        Parameters(req): Parameters<ListMemoriesRequest>,
    ) -> String {
        let scope = match req.scope.as_deref() {
            None => None,
            Some("global") => Some(durable::Scope::Global),
            Some("project") => Some(durable::Scope::Project),
            Some(other) => {
                return to_json(&error_static(
                    error_code::INVALID_SCOPE,
                    &format!(
                        "scope must be 'global' or 'project' (or omitted); got '{other}'",
                    ),
                ));
            }
        };
        let kind = match req.kind.as_deref() {
            None => None,
            Some("fact") => Some(durable::MemoryKind::Fact),
            Some("preference") => Some(durable::MemoryKind::Preference),
            Some("pattern") => Some(durable::MemoryKind::Pattern),
            Some("constraint") => Some(durable::MemoryKind::Constraint),
            Some("summary") => Some(durable::MemoryKind::Summary),
            Some(other) => {
                return to_json(&error_static(
                    error_code::INVALID_KIND,
                    &format!(
                        "kind must be one of 'fact'|'preference'|'pattern'|'constraint'|'summary' (or omitted); got '{other}'",
                    ),
                ));
            }
        };
        let f = durable::MemoryListFilter {
            scope,
            project_path: req.project_path,
            kind,
            include_archived: req.include_archived.unwrap_or(false),
            limit: req.limit.unwrap_or(0),
        };
        match durable::list_memories(&self.idx, &f) {
            Ok(rows) => to_json(&ListMemoriesPayload {
                schema_version: SCHEMA_VERSION,
                memories: rows
                    .into_iter()
                    .map(|m| MemoryOut {
                        id: m.id,
                        scope: scope_str(m.scope).to_string(),
                        project_path: m.project_path,
                        kind: memory_kind_str(m.kind).to_string(),
                        content: redact_apply(&m.content, &self.policy),
                        created_by_kind: created_by_kind_str(m.created_by_kind).to_string(),
                        // created_by is agent-supplied (e.g. via the
                        // `remember` tool); an injected agent could
                        // stash a secret here. Redact before emission.
                        created_by: redact_apply(&m.created_by, &self.policy),
                        created_at_ms: m.created_at_ms,
                    })
                    .collect(),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "claudepot_list_memories: query failed");
                to_json(&error_with(error_code::LIST_FAILED, &e, &self.policy))
            }
        }
    }

    #[tool(description = "List indexed sessions (Claude Code + Codex transcripts) ordered by most-recent activity. A discovery primitive — agents that want to browse what's indexed instead of search-by-text can call this first, then read_conversation on a chosen file_path.")]
    fn claudepot_list_sessions(
        &self,
        Parameters(req): Parameters<ListSessionsRequest>,
    ) -> String {
        let f = sms::SessionListFilter {
            source_kind: req.source_kind,
            project_path: req.project_path,
            since_ms: req.since_ms,
            limit: req.limit.unwrap_or(0),
            offset: req.offset.unwrap_or(0),
        };
        match sms::list_sessions(&self.idx, &f) {
            Ok(rows) => to_json(&ListSessionsPayload {
                schema_version: SCHEMA_VERSION,
                sessions: rows
                    .into_iter()
                    .map(|s| SessionSummaryOut {
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
            }),
            Err(e) => {
                tracing::warn!(error = %e, "claudepot_list_sessions: query failed");
                to_json(&error_with(error_code::LIST_FAILED, &e, &self.policy))
            }
        }
    }

    #[tool(description = "List distinct project paths in the cache with per-project session count and most-recent-activity stamp. Use this to discover which projects have indexed transcripts before drilling in with claudepot_list_sessions(project_path=...).")]
    fn claudepot_list_projects(
        &self,
        Parameters(req): Parameters<ListProjectsRequest>,
    ) -> String {
        match sms::list_projects(&self.idx, req.limit.unwrap_or(0)) {
            Ok(rows) => to_json(&ListProjectsPayload {
                schema_version: SCHEMA_VERSION,
                projects: rows
                    .into_iter()
                    .map(|p| ProjectSummaryOut {
                        project_path: p.project_path,
                        session_count: p.session_count,
                        last_activity_ms: p.last_activity_ms,
                    })
                    .collect(),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "claudepot_list_projects: query failed");
                to_json(&error_with(error_code::LIST_FAILED, &e, &self.policy))
            }
        }
    }

    #[tool(description = "Mark a decision as archived. Use when a decision is no longer in force but wasn't replaced by a specific successor (use claudepot_log_decision with supersedes_id when there's a replacement). Returns archived=false if the id didn't reference an active decision.")]
    fn claudepot_archive_decision(
        &self,
        Parameters(req): Parameters<ArchiveDecisionRequest>,
    ) -> String {
        match durable::archive_decision(&self.idx, &req.id) {
            Ok(archived) => to_json(&ArchiveDecisionPayload {
                schema_version: SCHEMA_VERSION,
                archived,
            }),
            Err(e) => {
                tracing::warn!(error = %e, "claudepot_archive_decision: write failed");
                to_json(&error_with(error_code::WRITE_FAILED, &e, &self.policy))
            }
        }
    }

    #[tool(description = "List decisions for a project (or all). Filter by status: active | superseded | archived.")]
    fn claudepot_list_decisions(
        &self,
        Parameters(req): Parameters<ListDecisionsRequest>,
    ) -> String {
        let status = match req.status.as_deref() {
            None => None,
            Some("active") => Some(durable::DecisionStatus::Active),
            Some("superseded") => Some(durable::DecisionStatus::Superseded),
            Some("archived") => Some(durable::DecisionStatus::Archived),
            Some(other) => {
                return to_json(&error_static(
                    error_code::INVALID_STATUS,
                    &format!(
                        "status must be 'active'|'superseded'|'archived' (or omitted); got '{other}'",
                    ),
                ));
            }
        };
        let f = durable::DecisionListFilter {
            project_path: req.project_path,
            status,
            limit: req.limit.unwrap_or(0),
        };
        match durable::list_decisions(&self.idx, &f) {
            Ok(rows) => to_json(&ListDecisionsPayload {
                schema_version: SCHEMA_VERSION,
                decisions: rows
                    .into_iter()
                    .map(|d| DecisionOut {
                        id: d.id,
                        project_path: d.project_path,
                        // topic is agent-supplied; redact.
                        topic: d.topic.map(|t| redact_apply(&t, &self.policy)),
                        decision: redact_apply(&d.decision, &self.policy),
                        rationale: d
                            .rationale
                            .map(|r| redact_apply(&r, &self.policy)),
                        status: decision_status_str(d.status).to_string(),
                        created_by_kind: created_by_kind_str(d.created_by_kind).to_string(),
                        // created_by is agent-supplied; redact.
                        created_by: redact_apply(&d.created_by, &self.policy),
                        created_at_ms: d.created_at_ms,
                        supersedes_id: d.supersedes_id,
                    })
                    .collect(),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "claudepot_list_decisions: query failed");
                to_json(&error_with(error_code::LIST_FAILED, &e, &self.policy))
            }
        }
    }
}

// ─── shared helpers ───────────────────────────────────────────

/// MCP error envelope. Carries a stable `error_code` so callers
/// can branch programmatically (e.g. retry on `sql_error`, abort
/// on `invalid_scope`). The `error` string is always run through
/// the MCP redaction policy before reaching this struct so a
/// failed write of secret-bearing content can't echo the secret
/// back in the error message.
#[derive(Serialize, schemars::JsonSchema)]
struct ErrorPayload {
    schema_version: u32,
    /// Stable category code. See `ErrorCode::*` constants.
    error_code: String,
    /// Human-readable error description. Already redacted.
    error: String,
}

// ─── list_sessions / list_projects (discovery) ────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListSessionsRequest {
    /// `claude_code` | `codex`. Omit to list both.
    #[serde(default)]
    source_kind: Option<String>,
    /// Exact match. Use `claudepot_list_projects` to discover
    /// the available values.
    #[serde(default)]
    project_path: Option<String>,
    /// Inclusive lower bound on the session's last activity in
    /// epoch ms.
    #[serde(default)]
    since_ms: Option<i64>,
    /// Default 50; capped at 200.
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SessionSummaryOut {
    file_path: String,
    session_id: String,
    source_kind: String,
    project_path: String,
    git_branch: Option<String>,
    first_ts_ms: Option<i64>,
    last_ts_ms: Option<i64>,
    message_count: i64,
    tokens_input: i64,
    tokens_output: i64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ListSessionsPayload {
    schema_version: u32,
    sessions: Vec<SessionSummaryOut>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListProjectsRequest {
    /// Default 100; capped at 500.
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ProjectSummaryOut {
    project_path: String,
    session_count: i64,
    last_activity_ms: Option<i64>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ListProjectsPayload {
    schema_version: u32,
    projects: Vec<ProjectSummaryOut>,
}

/// Stable error codes. Documented as part of the MCP contract; do
/// not rename or remove without a schema bump.
mod error_code {
    pub const INVALID_SCOPE: &str = "invalid_scope";
    pub const INVALID_KIND: &str = "invalid_kind";
    pub const INVALID_STATUS: &str = "invalid_status";
    pub const LOCATOR_NOT_INDEXED: &str = "locator_not_indexed";
    pub const DECISION_NOT_FOUND: &str = "decision_not_found";
    pub const SEARCH_FAILED: &str = "search_failed";
    pub const READ_FAILED: &str = "read_failed";
    pub const WRITE_FAILED: &str = "write_failed";
    pub const LIST_FAILED: &str = "list_failed";
}

/// Build an error envelope with the given category and the
/// `Display`-rendered cause. The message is redacted before
/// emission so a `Sql` error that echoes a row value (e.g. UNIQUE
/// constraint failure on `memories.content`) can't leak a secret
/// the caller just tried to write.
fn error_with(code: &str, cause: &dyn std::fmt::Display, policy: &RedactionPolicy) -> ErrorPayload {
    let raw = format!("{cause}");
    ErrorPayload {
        schema_version: SCHEMA_VERSION,
        error_code: code.to_string(),
        error: redact_apply(&raw, policy),
    }
}

/// Build an error envelope for a static message that doesn't need
/// redaction (input-validation errors authored by us).
fn error_static(code: &str, message: &str) -> ErrorPayload {
    ErrorPayload {
        schema_version: SCHEMA_VERSION,
        error_code: code.to_string(),
        error: message.to_string(),
    }
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| {
        serde_json::to_string(&ErrorPayload {
            schema_version: SCHEMA_VERSION,
            error_code: "serialization_failed".to_string(),
            error: "serialization failed".to_string(),
        })
        .unwrap_or_else(|_| "{}".to_string())
    })
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

// ─── snippet tests ────────────────────────────────────────────

#[cfg(test)]
mod snippet_tests {
    use super::*;

    #[test]
    fn snippet_has_version_header() {
        let body = snippet_body();
        assert!(
            body.contains(&format!(
                "claudepot-mcp-instructions v{}",
                claudepot_core::mcp_snippet::SNIPPET_VERSION
            )),
            "snippet should embed its version stamp"
        );
    }

    #[test]
    fn snippet_mentions_every_user_facing_tool() {
        let body = snippet_body();
        for tool in [
            "claudepot_search_memory",
            "claudepot_read_conversation",
            "claudepot_remember",
            "claudepot_log_decision",
            "claudepot_submit_evidence",
            "claudepot_list_memories",
            "claudepot_list_decisions",
            "claudepot_list_sessions",
            "claudepot_list_projects",
        ] {
            assert!(
                body.contains(tool),
                "snippet should mention {tool} so the agent knows it exists"
            );
        }
    }

    #[test]
    fn install_snippet_writes_idempotently() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("snippet.md");

        install_snippet(Some(path.clone()), false).unwrap();
        let v1 = std::fs::read_to_string(&path).unwrap();
        assert_eq!(v1, snippet_body());

        // Re-run — should overwrite cleanly with identical content.
        install_snippet(Some(path.clone()), false).unwrap();
        let v2 = std::fs::read_to_string(&path).unwrap();
        assert_eq!(v1, v2, "re-install should produce identical content");
    }
}
