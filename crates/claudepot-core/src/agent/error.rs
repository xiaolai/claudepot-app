//! Errors raised by `claudepot-core::agent`.
//!
//! One enum at the module boundary. CLI/Tauri callers convert via
//! `Display` (or `?`-into-anyhow at the top level).

use crate::error_code::ErrorCode;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("agent not found: {0}")]
    NotFound(String),

    #[error("agent name already taken: {0}")]
    DuplicateName(String),

    #[error("invalid name '{0}': {1}")]
    InvalidName(String, &'static str),

    #[error("invalid cron expression '{0}': {1}")]
    InvalidCron(String, String),

    #[error("cron '{0}' expands to {1} launch slots, exceeds limit of {2}")]
    CronTooDense(String, usize, usize),

    #[error("invalid env: {0}")]
    InvalidEnv(String),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid path '{0}': {1}")]
    InvalidPath(String, &'static str),

    #[error("home directory unavailable")]
    NoHomeDir,

    #[error("operation not supported on this platform: {0}")]
    UnsupportedPlatform(&'static str),

    #[error("agent file at {0} is not managed by Claudepot — refusing to overwrite")]
    NotManaged(String),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl ErrorCode for AgentError {
    fn code(&self) -> &'static str {
        match self {
            AgentError::Io(_) => "agent.io",
            AgentError::Json(_) => "agent.json",
            AgentError::NotFound(_) => "agent.not_found",
            AgentError::DuplicateName(_) => "agent.duplicate_name",
            AgentError::InvalidName(_, _) => "agent.invalid_name",
            AgentError::InvalidCron(_, _) => "agent.invalid_cron",
            AgentError::CronTooDense(_, _, _) => "agent.cron_too_dense",
            AgentError::InvalidEnv(_) => "agent.invalid_env",
            AgentError::MissingField(_) => "agent.missing_field",
            AgentError::InvalidPath(_, _) => "agent.invalid_path",
            AgentError::NoHomeDir => "agent.no_home_dir",
            AgentError::UnsupportedPlatform(_) => "agent.unsupported_platform",
            AgentError::NotManaged(_) => "agent.not_managed",
        }
    }

    fn params(&self) -> Value {
        match self {
            AgentError::Io(e) => json!({ "detail": e.to_string() }),
            AgentError::Json(e) => json!({ "detail": e.to_string() }),
            // An agent id (or a `route <uuid>` locator), never a secret
            // — `store.rs` and `install.rs` are the only constructors.
            AgentError::NotFound(id) => json!({ "id": id }),
            AgentError::DuplicateName(name) => json!({ "name": name }),
            AgentError::InvalidName(name, reason) => json!({ "name": name, "reason": reason }),
            AgentError::InvalidCron(cron, reason) => json!({ "cron": cron, "reason": reason }),
            AgentError::CronTooDense(cron, slots, limit) => {
                json!({ "cron": cron, "slots": slots, "limit": limit })
            }
            // The *key* and the rule it broke — `agent::env` builds this
            // text from key names and policy, never from a value. The
            // env allowlist is what keeps a pasted token out of here.
            AgentError::InvalidEnv(detail) => json!({ "detail": detail }),
            AgentError::MissingField(field) => json!({ "field": field }),
            AgentError::InvalidPath(path, reason) => json!({ "path": path, "reason": reason }),
            AgentError::NoHomeDir => json!({}),
            AgentError::UnsupportedPlatform(operation) => json!({ "operation": operation }),
            AgentError::NotManaged(path) => json!({ "path": path }),
        }
    }
}
