pub mod beta_header;
pub mod profile;
pub mod refresh;
pub mod usage;

use crate::error::OAuthError;

/// Shared HTTP client for OAuth API calls.
pub fn http_client() -> Result<reqwest::Client, OAuthError> {
    reqwest::Client::builder()
        .user_agent("claudepot/0.1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| OAuthError::HttpError(e))
}
