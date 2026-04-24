//! Session debugger DTOs (Tier 1-3 claude-devtools port).
//!
//! Purpose: pin the webview-facing JSON contract so that changes to
//! core serde shapes (new fields, renamed enums, etc.) cannot implicitly
//! flip the JS bindings. Each DTO is a structural clone of the core
//! type, with an explicit `From<&CoreType>` conversion. Reuses
//! `TokenUsageDto` / `SessionRowDto` from `dto_session` for consistency
//! with the rest of the session surface.

use crate::dto_session::{SessionRowDto, TokenUsageDto};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize)]
pub struct ChunkMetricsDto {
    pub duration_ms: i64,
    pub tokens: TokenUsageDto,
    pub message_count: usize,
    pub tool_call_count: usize,
    pub thinking_count: usize,
}

impl From<&claudepot_core::session_chunks::ChunkMetrics> for ChunkMetricsDto {
    fn from(m: &claudepot_core::session_chunks::ChunkMetrics) -> Self {
        Self {
            duration_ms: m.duration_ms,
            tokens: (&m.tokens).into(),
            message_count: m.message_count,
            tool_call_count: m.tool_call_count,
            thinking_count: m.thinking_count,
        }
    }
}

#[derive(Serialize)]
pub struct LinkedToolDto {
    pub tool_use_id: String,
    pub tool_name: String,
    pub model: Option<String>,
    pub call_ts: Option<DateTime<Utc>>,
    pub input_preview: String,
    pub input_full: String,
    pub result_ts: Option<DateTime<Utc>>,
    pub result_content: Option<String>,
    pub is_error: bool,
    pub duration_ms: Option<i64>,
    pub call_index: usize,
    pub result_index: Option<usize>,
}

impl From<&claudepot_core::session_tool_link::LinkedTool> for LinkedToolDto {
    fn from(t: &claudepot_core::session_tool_link::LinkedTool) -> Self {
        Self {
            tool_use_id: t.tool_use_id.clone(),
            tool_name: t.tool_name.clone(),
            model: t.model.clone(),
            call_ts: t.call_ts,
            input_preview: t.input_preview.clone(),
            input_full: t.input_full.clone(),
            result_ts: t.result_ts,
            result_content: t.result_content.clone(),
            is_error: t.is_error,
            duration_ms: t.duration_ms,
            call_index: t.call_index,
            result_index: t.result_index,
        }
    }
}

#[derive(Serialize)]
pub struct ChunkHeaderDto {
    pub id: usize,
    pub start_ts: Option<DateTime<Utc>>,
    pub end_ts: Option<DateTime<Utc>>,
    pub metrics: ChunkMetricsDto,
}

impl From<&claudepot_core::session_chunks::ChunkHeader> for ChunkHeaderDto {
    fn from(h: &claudepot_core::session_chunks::ChunkHeader) -> Self {
        Self {
            id: h.id,
            start_ts: h.start_ts,
            end_ts: h.end_ts,
            metrics: (&h.metrics).into(),
        }
    }
}

/// Matches the shape the JS side already consumes: `chunkType` tag
/// flattened onto the header fields. Each variant carries the data it
/// needs to render in the transcript pane.
#[derive(Serialize)]
#[serde(tag = "chunkType", rename_all = "camelCase")]
pub enum SessionChunkDto {
    #[serde(rename = "user")]
    User {
        #[serde(flatten)]
        header: ChunkHeaderDto,
        event_index: usize,
    },
    #[serde(rename = "ai")]
    Ai {
        #[serde(flatten)]
        header: ChunkHeaderDto,
        event_indices: Vec<usize>,
        tool_executions: Vec<LinkedToolDto>,
    },
    #[serde(rename = "system")]
    System {
        #[serde(flatten)]
        header: ChunkHeaderDto,
        event_index: usize,
    },
    #[serde(rename = "compact")]
    Compact {
        #[serde(flatten)]
        header: ChunkHeaderDto,
        event_index: usize,
    },
}

impl From<&claudepot_core::session_chunks::SessionChunk> for SessionChunkDto {
    fn from(c: &claudepot_core::session_chunks::SessionChunk) -> Self {
        use claudepot_core::session_chunks::SessionChunk;
        match c {
            SessionChunk::User {
                header,
                event_index,
            } => SessionChunkDto::User {
                header: header.into(),
                event_index: *event_index,
            },
            SessionChunk::Ai {
                header,
                event_indices,
                tool_executions,
            } => SessionChunkDto::Ai {
                header: header.into(),
                event_indices: event_indices.clone(),
                tool_executions: tool_executions.iter().map(LinkedToolDto::from).collect(),
            },
            SessionChunk::System {
                header,
                event_index,
            } => SessionChunkDto::System {
                header: header.into(),
                event_index: *event_index,
            },
            SessionChunk::Compact {
                header,
                event_index,
            } => SessionChunkDto::Compact {
                header: header.into(),
                event_index: *event_index,
            },
        }
    }
}

#[derive(Serialize)]
pub struct ContextPhaseDto {
    pub phase_number: usize,
    pub start_index: usize,
    pub end_index: usize,
    pub start_ts: Option<DateTime<Utc>>,
    pub end_ts: Option<DateTime<Utc>>,
    pub summary: Option<String>,
}

impl From<&claudepot_core::session_phases::ContextPhase> for ContextPhaseDto {
    fn from(p: &claudepot_core::session_phases::ContextPhase) -> Self {
        Self {
            phase_number: p.phase_number,
            start_index: p.start_index,
            end_index: p.end_index,
            start_ts: p.start_ts,
            end_ts: p.end_ts,
            summary: p.summary.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct TokensByCategoryDto {
    pub claude_md: u64,
    pub mentioned_file: u64,
    pub tool_output: u64,
    pub thinking_text: u64,
    pub team_coordination: u64,
    pub user_message: u64,
}

impl From<&claudepot_core::session_context::TokensByCategory> for TokensByCategoryDto {
    fn from(t: &claudepot_core::session_context::TokensByCategory) -> Self {
        Self {
            claude_md: t.claude_md,
            mentioned_file: t.mentioned_file,
            tool_output: t.tool_output,
            thinking_text: t.thinking_text,
            team_coordination: t.team_coordination,
            user_message: t.user_message,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextCategoryDto {
    ClaudeMd,
    MentionedFile,
    ToolOutput,
    ThinkingText,
    TeamCoordination,
    UserMessage,
}

impl From<claudepot_core::session_context::ContextCategory> for ContextCategoryDto {
    fn from(c: claudepot_core::session_context::ContextCategory) -> Self {
        use claudepot_core::session_context::ContextCategory;
        match c {
            ContextCategory::ClaudeMd => ContextCategoryDto::ClaudeMd,
            ContextCategory::MentionedFile => ContextCategoryDto::MentionedFile,
            ContextCategory::ToolOutput => ContextCategoryDto::ToolOutput,
            ContextCategory::ThinkingText => ContextCategoryDto::ThinkingText,
            ContextCategory::TeamCoordination => ContextCategoryDto::TeamCoordination,
            ContextCategory::UserMessage => ContextCategoryDto::UserMessage,
        }
    }
}

#[derive(Serialize)]
pub struct ContextInjectionDto {
    pub event_index: usize,
    pub category: ContextCategoryDto,
    pub label: String,
    pub tokens: u64,
    pub ts: Option<DateTime<Utc>>,
    pub phase: usize,
}

impl From<&claudepot_core::session_context::ContextInjection> for ContextInjectionDto {
    fn from(i: &claudepot_core::session_context::ContextInjection) -> Self {
        Self {
            event_index: i.event_index,
            category: i.category.into(),
            label: i.label.clone(),
            tokens: i.tokens,
            ts: i.ts,
            phase: i.phase,
        }
    }
}

#[derive(Serialize)]
pub struct ContextStatsDto {
    pub totals: TokensByCategoryDto,
    pub injections: Vec<ContextInjectionDto>,
    pub phases: Vec<ContextPhaseDto>,
    pub reported_total_tokens: u64,
}

impl From<&claudepot_core::session_context::ContextStats> for ContextStatsDto {
    fn from(s: &claudepot_core::session_context::ContextStats) -> Self {
        Self {
            totals: (&s.totals).into(),
            injections: s.injections.iter().map(ContextInjectionDto::from).collect(),
            phases: s.phases.iter().map(ContextPhaseDto::from).collect(),
            reported_total_tokens: s.reported_total_tokens,
        }
    }
}

#[derive(Serialize)]
pub struct SearchHitDto {
    pub session_id: String,
    pub slug: String,
    pub file_path: String,
    pub project_path: String,
    pub role: String,
    pub snippet: String,
    pub match_offset: usize,
    pub last_ts: Option<DateTime<Utc>>,
    pub score: f32,
}

impl From<&claudepot_core::session_search::SearchHit> for SearchHitDto {
    fn from(h: &claudepot_core::session_search::SearchHit) -> Self {
        Self {
            session_id: h.session_id.clone(),
            slug: h.slug.clone(),
            file_path: h.file_path.display().to_string(),
            project_path: h.project_path.clone(),
            role: h.role.clone(),
            snippet: h.snippet.clone(),
            match_offset: h.match_offset,
            last_ts: h.last_ts,
            score: h.score,
        }
    }
}

#[cfg(test)]
mod search_hit_dto_tests {
    use super::*;

    fn sample_core_hit() -> claudepot_core::session_search::SearchHit {
        claudepot_core::session_search::SearchHit {
            session_id: "abc".into(),
            slug: "-r".into(),
            file_path: std::path::PathBuf::from("/tmp/x.jsonl"),
            project_path: "/repo".into(),
            role: "user".into(),
            snippet: "hello world".into(),
            match_offset: 3,
            last_ts: None,
            score: 0.7,
        }
    }

    #[test]
    fn search_hit_dto_roundtrips_score_field() {
        let dto: SearchHitDto = (&sample_core_hit()).into();
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("\"score\":0.7"), "missing score: {json}");
    }

    #[test]
    fn search_hit_dto_converts_all_fields_from_core() {
        let dto: SearchHitDto = (&sample_core_hit()).into();
        assert_eq!(dto.session_id, "abc");
        assert_eq!(dto.slug, "-r");
        assert_eq!(dto.file_path, "/tmp/x.jsonl");
        assert_eq!(dto.project_path, "/repo");
        assert_eq!(dto.role, "user");
        assert_eq!(dto.snippet, "hello world");
        assert_eq!(dto.match_offset, 3);
        assert!(dto.last_ts.is_none());
        assert!((dto.score - 0.7).abs() < f32::EPSILON);
    }
}

#[derive(Serialize)]
pub struct RepositoryGroupDto {
    pub repo_root: Option<String>,
    pub label: String,
    pub sessions: Vec<SessionRowDto>,
    pub branches: Vec<String>,
    pub worktree_paths: Vec<String>,
}

impl From<&claudepot_core::session_worktree::RepositoryGroup> for RepositoryGroupDto {
    fn from(g: &claudepot_core::session_worktree::RepositoryGroup) -> Self {
        Self {
            repo_root: g.repo_root.as_ref().map(|p| p.display().to_string()),
            label: g.label.clone(),
            sessions: g.sessions.iter().map(SessionRowDto::from).collect(),
            branches: g.branches.clone(),
            worktree_paths: g
                .worktree_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        }
    }
}
