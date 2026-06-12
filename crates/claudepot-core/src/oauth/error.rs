//! Boundary error for the `oauth` module. Historically lived in the
//! crate-root `error.rs`; relocated next to its boundary per
//! rust-conventions ("one enum per module boundary").
//! `crate::error::OAuthError` remains a re-export.

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
