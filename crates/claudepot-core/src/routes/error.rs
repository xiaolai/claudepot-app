use crate::error_code::ErrorCode;
use serde_json::{json, Value};
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

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("home directory unavailable")]
    NoHomeDir,

    #[error("operation not supported on this platform: {0}")]
    UnsupportedPlatform(&'static str),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// A route's secret never reaches this enum: `lifecycle::commit_secrets`
/// zeroizes the inline field and the keychain/helper writes report
/// failure as `Io`, whose text is a filesystem or `security(1)` message.
impl ErrorCode for RouteError {
    fn code(&self) -> &'static str {
        match self {
            RouteError::Io(_) => "routes.io",
            RouteError::Json(_) => "routes.json",
            RouteError::NotFound(_) => "routes.not_found",
            RouteError::DuplicateName(_) => "routes.duplicate_name",
            RouteError::DuplicateWrapperName(_) => "routes.duplicate_wrapper_name",
            RouteError::WrapperShadowsClaude(_) => "routes.wrapper_shadows_claude",
            RouteError::WrapperFileNotManaged(_) => "routes.wrapper_file_not_managed",
            RouteError::InvalidWrapperName(_, _) => "routes.invalid_wrapper_name",
            RouteError::MissingField(_) => "routes.missing_field",
            RouteError::NoHomeDir => "routes.no_home_dir",
            RouteError::UnsupportedPlatform(_) => "routes.unsupported_platform",
        }
    }

    fn params(&self) -> Value {
        match self {
            RouteError::Io(e) => json!({ "detail": e.to_string() }),
            RouteError::Json(e) => json!({ "detail": e.to_string() }),
            RouteError::NotFound(id) => json!({ "id": id }),
            RouteError::DuplicateName(name) => json!({ "name": name }),
            RouteError::DuplicateWrapperName(wrapper) => json!({ "wrapper": wrapper }),
            RouteError::WrapperShadowsClaude(wrapper) => json!({ "wrapper": wrapper }),
            // `{0}` is the wrapper's path on disk, not its name.
            RouteError::WrapperFileNotManaged(path) => json!({ "path": path }),
            RouteError::InvalidWrapperName(wrapper, reason) => {
                json!({ "wrapper": wrapper, "reason": reason })
            }
            // A base URL is user-entered but not a credential —
            // `url::validate_base_url` rejects embedded auth outright.
            RouteError::MissingField(field) => json!({ "field": field }),
            RouteError::NoHomeDir => json!({}),
            RouteError::UnsupportedPlatform(operation) => json!({ "operation": operation }),
        }
    }
}
