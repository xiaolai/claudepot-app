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
        let mut builder = reqwest::Client::builder()
            .user_agent("claudepot/0.1.0")
            .timeout(std::time::Duration::from_secs(15));

        // rustls-tls doesn't read macOS system proxy automatically; read env
        // vars explicitly so Surge / Clash / etc. work when launched from GUI.
        // Filter empty strings — HTTPS_PROXY="" short-circuits or_else chains.
        let proxy_url = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
            .iter()
            .filter_map(|var| std::env::var(var).ok())
            .find(|v| !v.is_empty());
        if let Some(url) = proxy_url {
            if let Ok(proxy) = reqwest::Proxy::all(&url) {
                builder = builder.proxy(proxy.no_proxy(reqwest::NoProxy::from_env()));
            }
        }

        builder.build().expect("failed to build HTTP client")
    }))
}
