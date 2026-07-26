pub mod beta_header;
pub mod error;
pub mod profile;
pub mod refresh;
pub mod usage;
pub mod wake;

pub use error::OAuthError;

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
/// Seconds to wait after a 429, read from `Retry-After`.
///
/// Defaults to 60 when the header is absent or unparseable, and is
/// capped at 300 so a hostile or misconfigured server cannot dictate a
/// long sleep. Shared because every OAuth endpoint needs the identical
/// parse/default/clamp, and it had already been copy-pasted once.
pub(crate) fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> u64 {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(60)
        .min(300)
}

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

#[cfg(test)]
mod retry_after_tests {
    use super::retry_after_secs;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn headers(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("retry-after", HeaderValue::from_str(v).unwrap());
        h
    }

    #[test]
    fn reads_the_header_when_present() {
        assert_eq!(retry_after_secs(&headers("42")), 42);
    }

    #[test]
    fn defaults_to_sixty_when_absent_or_unparseable() {
        assert_eq!(retry_after_secs(&HeaderMap::new()), 60);
        assert_eq!(retry_after_secs(&headers("soon")), 60);
        // HTTP-date form is legal per RFC 9110 but we don't parse it;
        // the default is the safe read.
        assert_eq!(
            retry_after_secs(&headers("Wed, 21 Oct 2026 07:28:00 GMT")),
            60
        );
    }

    /// The clamp `refresh.rs` was missing before these call sites were
    /// consolidated: without it a server could dictate an unbounded wait.
    #[test]
    fn clamps_a_hostile_value_to_five_minutes() {
        assert_eq!(retry_after_secs(&headers("999999")), 300);
        assert_eq!(retry_after_secs(&headers("301")), 300);
        assert_eq!(retry_after_secs(&headers("300")), 300);
    }

    #[test]
    fn zero_is_honored_rather_than_defaulted() {
        // 0 parses fine and means "retry immediately" — it must not be
        // confused with the absent case.
        assert_eq!(retry_after_secs(&headers("0")), 0);
    }
}
