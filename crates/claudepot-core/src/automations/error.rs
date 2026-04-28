//! Errors raised by `claudepot-core::automations`.
//!
//! One enum at the module boundary. CLI/Tauri callers convert via
//! `Display` (or `?`-into-anyhow at the top level).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AutomationError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("automation not found: {0}")]
    NotFound(String),

    #[error("automation name already taken: {0}")]
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

    #[error("automation file at {0} is not managed by Claudepot — refusing to overwrite")]
    NotManaged(String),
}
