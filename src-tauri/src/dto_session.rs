//! Session index DTOs — list + transcript + protected-paths surface.
//! Mirrors `claudepot_core::session::*`.

use crate::dto::system_time_to_ms;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize)]
pub struct TokenUsageDto {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub total: u64,
}

impl From<&claudepot_core::session::TokenUsage> for TokenUsageDto {
    fn from(t: &claudepot_core::session::TokenUsage) -> Self {
        Self {
            input: t.input,
            output: t.output,
            cache_creation: t.cache_creation,
            cache_read: t.cache_read,
            total: t.total(),
        }
    }
}

#[derive(Serialize)]
pub struct SessionRowDto {
    pub session_id: String,
    pub slug: String,
    pub file_path: String,
    pub file_size_bytes: u64,
    pub last_modified_ms: Option<i64>,
    pub project_path: String,
    pub project_from_transcript: bool,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    pub event_count: usize,
    pub message_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    pub first_user_prompt: Option<String>,
    pub models: Vec<String>,
    pub tokens: TokenUsageDto,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub display_slug: Option<String>,
    pub has_error: bool,
    pub is_sidechain: bool,
}

impl From<&claudepot_core::session::SessionRow> for SessionRowDto {
    fn from(r: &claudepot_core::session::SessionRow) -> Self {
        Self {
            session_id: r.session_id.clone(),
            slug: r.slug.clone(),
            file_path: r.file_path.to_string_lossy().to_string(),
            file_size_bytes: r.file_size_bytes,
            last_modified_ms: system_time_to_ms(r.last_modified),
            project_path: r.project_path.clone(),
            project_from_transcript: r.project_from_transcript,
            first_ts: r.first_ts,
            last_ts: r.last_ts,
            event_count: r.event_count,
            message_count: r.message_count,
            user_message_count: r.user_message_count,
            assistant_message_count: r.assistant_message_count,
            first_user_prompt: r.first_user_prompt.clone(),
            models: r.models.clone(),
            tokens: TokenUsageDto::from(&r.tokens),
            git_branch: r.git_branch.clone(),
            cc_version: r.cc_version.clone(),
            display_slug: r.display_slug.clone(),
            has_error: r.has_error,
            is_sidechain: r.is_sidechain,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum SessionEventDto {
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
        usage: Option<TokenUsageDto>,
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
        /// Raw JSON of the tool input, untruncated. Only the detail
        /// search reads this; list views and display paths stay on
        /// `input_preview`.
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
    #[serde(rename = "fileSnapshot")]
    FileHistorySnapshot {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        file_count: usize,
    },
    #[serde(rename = "taskSummary")]
    TaskSummary {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        summary: String,
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

impl From<&claudepot_core::session::SessionEvent> for SessionEventDto {
    fn from(e: &claudepot_core::session::SessionEvent) -> Self {
        use claudepot_core::session::SessionEvent as E;
        match e {
            E::UserText { ts, uuid, text } => Self::UserText {
                ts: *ts,
                uuid: uuid.clone(),
                text: text.clone(),
            },
            E::UserToolResult {
                ts,
                uuid,
                tool_use_id,
                content,
                is_error,
            } => Self::UserToolResult {
                ts: *ts,
                uuid: uuid.clone(),
                tool_use_id: tool_use_id.clone(),
                content: content.clone(),
                is_error: *is_error,
            },
            E::AssistantText {
                ts,
                uuid,
                model,
                text,
                usage,
                stop_reason,
            } => Self::AssistantText {
                ts: *ts,
                uuid: uuid.clone(),
                model: model.clone(),
                text: text.clone(),
                usage: usage.as_ref().map(TokenUsageDto::from),
                stop_reason: stop_reason.clone(),
            },
            E::AssistantToolUse {
                ts,
                uuid,
                model,
                tool_name,
                tool_use_id,
                input_preview,
                input_full,
            } => Self::AssistantToolUse {
                ts: *ts,
                uuid: uuid.clone(),
                model: model.clone(),
                tool_name: tool_name.clone(),
                tool_use_id: tool_use_id.clone(),
                input_preview: input_preview.clone(),
                input_full: input_full.clone(),
            },
            E::AssistantThinking { ts, uuid, text } => Self::AssistantThinking {
                ts: *ts,
                uuid: uuid.clone(),
                text: text.clone(),
            },
            E::Summary { ts, uuid, text } => Self::Summary {
                ts: *ts,
                uuid: uuid.clone(),
                text: text.clone(),
            },
            E::System {
                ts,
                uuid,
                subtype,
                detail,
            } => Self::System {
                ts: *ts,
                uuid: uuid.clone(),
                subtype: subtype.clone(),
                detail: detail.clone(),
            },
            E::Attachment {
                ts,
                uuid,
                name,
                mime,
            } => Self::Attachment {
                ts: *ts,
                uuid: uuid.clone(),
                name: name.clone(),
                mime: mime.clone(),
            },
            E::FileHistorySnapshot {
                ts,
                uuid,
                file_count,
            } => Self::FileHistorySnapshot {
                ts: *ts,
                uuid: uuid.clone(),
                file_count: *file_count,
            },
            E::TaskSummary { ts, uuid, summary } => Self::TaskSummary {
                ts: *ts,
                uuid: uuid.clone(),
                summary: summary.clone(),
            },
            E::Other { ts, uuid, raw_type } => Self::Other {
                ts: *ts,
                uuid: uuid.clone(),
                raw_type: raw_type.clone(),
            },
            E::Malformed {
                line_number,
                error,
                preview,
            } => Self::Malformed {
                line_number: *line_number,
                error: error.clone(),
                preview: preview.clone(),
            },
        }
    }
}

#[derive(Serialize)]
pub struct SessionDetailDto {
    pub row: SessionRowDto,
    pub events: Vec<SessionEventDto>,
}

impl From<&claudepot_core::session::SessionDetail> for SessionDetailDto {
    fn from(d: &claudepot_core::session::SessionDetail) -> Self {
        Self {
            row: SessionRowDto::from(&d.row),
            events: d.events.iter().map(SessionEventDto::from).collect(),
        }
    }
}

/// One row in the protected-paths Settings list. `source` tells the
/// UI which badge to render (`default` | `user`).
#[derive(Serialize)]
pub struct ProtectedPathDto {
    pub path: String,
    /// Lowercase string: `"default"` or `"user"`. We don't expose the
    /// Rust enum variant names directly so the JS side doesn't need to
    /// keep its discriminant in lockstep with the core enum.
    pub source: String,
}

impl From<&claudepot_core::protected_paths::ProtectedPath> for ProtectedPathDto {
    fn from(p: &claudepot_core::protected_paths::ProtectedPath) -> Self {
        let source = match p.source {
            claudepot_core::protected_paths::PathSource::Default => "default",
            claudepot_core::protected_paths::PathSource::User => "user",
        };
        Self {
            path: p.path.clone(),
            source: source.to_string(),
        }
    }
}
