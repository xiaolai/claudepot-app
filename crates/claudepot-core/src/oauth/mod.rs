pub mod beta_header;
pub mod profile;
pub mod refresh;
pub mod usage;

use crate::error::OAuthError;
use std::sync::OnceLock;

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Shared HTTP client for all OAuth API calls. Connection-pooled, TLS-reused.
pub fn http_client() -> Result<&'static reqwest::Client, OAuthError> {
    Ok(HTTP_CLIENT.get_or_init(|| {
        let config = crate::proxy::detect();
        let builder = reqwest::Client::builder()
            .user_agent("claudepot/0.1.0")
            .timeout(std::time::Duration::from_secs(15));
        crate::proxy::apply(builder, &config)
            .build()
            .expect("failed to build HTTP client")
    }))
}
