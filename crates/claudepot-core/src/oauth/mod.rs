pub mod beta_header;
pub mod profile;
pub mod refresh;
pub mod usage;

use crate::error::OAuthError;
use parking_lot::Mutex;
use std::sync::Arc;

/// Cache key — two `Option<String>`s that uniquely determine the
/// `reqwest::Client` we'd build. If the user changes their system or env
/// proxy mid-session, the next `http_client()` call sees a different key
/// and rebuilds. The `ProxySource` itself is intentionally NOT part of
/// the key — going from "env-var: HTTPS_PROXY=foo" to "system HTTPS=foo"
/// produces the same client, no rebuild needed.
#[derive(Clone, PartialEq, Eq)]
struct CacheKey {
    url: Option<String>,
    no_proxy: Option<String>,
}

static HTTP_CLIENT: Mutex<Option<(CacheKey, Arc<reqwest::Client>)>> = Mutex::new(None);

/// Shared HTTP client for all OAuth API calls. Connection-pooled, TLS-reused.
///
/// The client is cached but the proxy configuration is re-detected on every
/// call. If the user toggles their system proxy or sets `HTTPS_PROXY` mid
/// session, the next call rebuilds the client; otherwise the cached `Arc`
/// is cloned (cheap). Returning `Arc<Client>` rather than `&'static Client`
/// is what makes invalidation possible.
pub fn http_client() -> Result<Arc<reqwest::Client>, OAuthError> {
    let config = crate::proxy::detect();
    let key = CacheKey {
        url: config.url.clone(),
        no_proxy: config.no_proxy.clone(),
    };

    let mut guard = HTTP_CLIENT.lock();
    if let Some((cached_key, client)) = guard.as_ref() {
        if cached_key == &key {
            return Ok(Arc::clone(client));
        }
    }

    let builder = reqwest::Client::builder()
        .user_agent("claudepot/0.1.0")
        .timeout(std::time::Duration::from_secs(15));
    let client = Arc::new(crate::proxy::apply(builder, &config).build()?);
    *guard = Some((key, Arc::clone(&client)));
    Ok(client)
}
