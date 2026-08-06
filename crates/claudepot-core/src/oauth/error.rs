//! Boundary error for the `oauth` module. Historically lived in the
//! crate-root `error.rs`; relocated next to its boundary per
//! rust-conventions ("one enum per module boundary").
//! `crate::error::OAuthError` remains a re-export.

use crate::error_code::ErrorCode;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum OAuthError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("token expired and refresh failed: {0}")]
    RefreshFailed(String),

    #[error("rate limited — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    /// Non-401, non-429 non-2xx response from an OAuth endpoint. Separate
    /// from AuthFailed so callers (identity verification in particular)
    /// can distinguish "token is genuinely bad" from "the server had a
    /// bad minute" — the former is a `Rejected` outcome requiring
    /// re-login; the latter is `NetworkError` and should NOT wipe the
    /// verified_email history.
    #[error("OAuth server error: {0}")]
    ServerError(String),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl ErrorCode for OAuthError {
    fn code(&self) -> &'static str {
        match self {
            OAuthError::HttpError(_) => "oauth.http_error",
            OAuthError::RefreshFailed(_) => "oauth.refresh_failed",
            OAuthError::RateLimited { .. } => "oauth.rate_limited",
            OAuthError::AuthFailed(_) => "oauth.auth_failed",
            OAuthError::ServerError(_) => "oauth.server_error",
        }
    }

    fn params(&self) -> Value {
        match self {
            // Every payload here is a status line, an endpoint name, or
            // a transport message — the response bodies are drained
            // without being read precisely so no credential lands in
            // one. Re-check that before widening any of these.
            OAuthError::HttpError(e) => json!({ "detail": e.to_string() }),
            OAuthError::RefreshFailed(detail) => json!({ "detail": detail }),
            // The structured variant: the number a "retry in N seconds"
            // sentence needs, without parsing the English one.
            OAuthError::RateLimited { retry_after_secs } => {
                json!({ "retry_after_secs": retry_after_secs })
            }
            OAuthError::AuthFailed(detail) => json!({ "detail": detail }),
            OAuthError::ServerError(detail) => json!({ "detail": detail }),
        }
    }
}
