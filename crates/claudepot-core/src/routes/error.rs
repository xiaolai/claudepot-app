use thiserror::Error;

#[derive(Debug, Error)]
pub enum RouteError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("route not found: {0}")]
    NotFound(String),

    #[error("route name already taken: {0}")]
    DuplicateName(String),

    #[error("wrapper name already in use by another route: {0}")]
    DuplicateWrapperName(String),

    #[error("wrapper name '{0}' would collide with the first-party `claude` binary")]
    WrapperShadowsClaude(String),

    #[error("wrapper '{0}' already exists on disk and was not written by Claudepot — refusing to overwrite")]
    WrapperFileNotManaged(String),

    #[error("invalid wrapper name '{0}': {1}")]
    InvalidWrapperName(String, String),

    #[error("invalid base URL '{0}': {1}")]
    InvalidBaseUrl(String, String),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("home directory unavailable")]
    NoHomeDir,

    #[error("operation not supported on this platform: {0}")]
    UnsupportedPlatform(&'static str),
}
