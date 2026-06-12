//! Validity probes for stored key secrets.
//!
//! Both probes are "is this credential still accepted?" checks only —
//! they do not consume tokens, do not read usage data, and do not
//! persist their outcome. The caller is expected to surface the
//! result as a transient toast, not a long-lived row flag, so that a
//! stale cached status never lies about the current state.
//!
//! * OAuth tokens are probed via `oauth::profile::fetch` — call that
//!   directly; it already returns the right `OAuthError` shape.
//! * API keys are probed here via `GET /v1/models` with `x-api-key:`.

use crate::error::OAuthError;
use crate::oauth::http_client;

const VALIDATION_BASE: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Validate an `ANTHROPIC_API_KEY` by calling `GET /v1/models`. The
/// endpoint does not consume input tokens and is the standard "is
/// this key valid?" ping. Returns `()` on 2xx; on failure, maps
/// responses to the reusable `OAuthError` shape so callers can share
/// toast formatting with the OAuth probe path:
///
/// * 401 / 403 → `AuthFailed` (key rejected)
/// * 429       → `RateLimited` (`Retry-After` parsed, capped at 300s)
/// * other     → `ServerError`
/// * transport → `HttpError` (from reqwest)
pub async fn probe_api_key(key: &str) -> Result<(), OAuthError> {
    probe_api_key_with_base(key, VALIDATION_BASE).await
}

/// Base-URL injection point for tests (same seam pattern as
/// `session_share::share_gist_with_base`). Production callers use
/// [`probe_api_key`].
pub async fn probe_api_key_with_base(key: &str, base_url: &str) -> Result<(), OAuthError> {
    let client = http_client()?;
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .header("x-api-key", key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .send()
        .await?;

    let status = resp.status();
    if status == 401 || status == 403 {
        return Err(OAuthError::AuthFailed(
            "api key rejected by /v1/models".into(),
        ));
    }
    if status == 429 {
        let retry_after = retry_after_secs(
            resp.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
        );
        return Err(OAuthError::RateLimited {
            retry_after_secs: retry_after,
        });
    }
    if !status.is_success() {
        let _ = resp.text().await; // drain body without exposing it
        return Err(OAuthError::ServerError(format!(
            "/v1/models returned {status}"
        )));
    }
    Ok(())
}

/// Parse a `Retry-After` header value into seconds: missing or
/// unparsable → 60, anything above 300 clamped to 300. Pure — the
/// arithmetic the audit flagged as silently driftable.
fn retry_after_secs(header: Option<&str>) -> u64 {
    header
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(60)
        .min(300)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── retry-after parse/clamp (pure) ─────────────────────────────

    #[test]
    fn test_retry_after_missing_defaults_to_60() {
        assert_eq!(retry_after_secs(None), 60);
    }

    #[test]
    fn test_retry_after_unparsable_defaults_to_60() {
        assert_eq!(retry_after_secs(Some("soon")), 60);
        assert_eq!(retry_after_secs(Some("-5")), 60);
        assert_eq!(retry_after_secs(Some("")), 60);
    }

    #[test]
    fn test_retry_after_in_range_passes_through() {
        assert_eq!(retry_after_secs(Some("42")), 42);
        assert_eq!(retry_after_secs(Some("300")), 300);
    }

    #[test]
    fn test_retry_after_oversized_clamps_to_300() {
        assert_eq!(retry_after_secs(Some("9999")), 300);
    }

    // ── status→error mapping via local one-shot mock server ────────
    // Mirrors the `session_share` TcpListener pattern: bind on an
    // ephemeral port, serve exactly one canned response, assert the
    // mapped error.

    async fn one_shot_server(response: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        });
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test]
    async fn test_probe_200_returns_ok() {
        let base =
            one_shot_server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await;
        probe_api_key_with_base("sk-test", &base).await.unwrap();
    }

    #[tokio::test]
    async fn test_probe_401_maps_to_auth_failed() {
        let base = one_shot_server(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        let err = probe_api_key_with_base("sk-test", &base).await.unwrap_err();
        assert!(matches!(err, OAuthError::AuthFailed(_)), "err={err:?}");
    }

    #[tokio::test]
    async fn test_probe_403_maps_to_auth_failed() {
        let base = one_shot_server(
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        let err = probe_api_key_with_base("sk-test", &base).await.unwrap_err();
        assert!(matches!(err, OAuthError::AuthFailed(_)), "err={err:?}");
    }

    #[tokio::test]
    async fn test_probe_429_with_oversized_retry_after_clamps_to_300() {
        let base = one_shot_server(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 9999\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        let err = probe_api_key_with_base("sk-test", &base).await.unwrap_err();
        assert!(
            matches!(
                err,
                OAuthError::RateLimited {
                    retry_after_secs: 300
                }
            ),
            "err={err:?}"
        );
    }

    #[tokio::test]
    async fn test_probe_429_without_retry_after_defaults_to_60() {
        let base = one_shot_server(
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        let err = probe_api_key_with_base("sk-test", &base).await.unwrap_err();
        assert!(
            matches!(
                err,
                OAuthError::RateLimited {
                    retry_after_secs: 60
                }
            ),
            "err={err:?}"
        );
    }

    #[tokio::test]
    async fn test_probe_500_maps_to_server_error_without_leaking_body() {
        let base = one_shot_server(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 19\r\nConnection: close\r\n\r\n{\"secret\":\"sk-ant\"}",
        )
        .await;
        let err = probe_api_key_with_base("sk-test", &base).await.unwrap_err();
        match &err {
            OAuthError::ServerError(msg) => {
                assert!(msg.contains("500"), "msg={msg}");
                // The body is deliberately drained, never surfaced.
                assert!(!msg.contains("sk-ant"), "msg={msg}");
            }
            other => panic!("expected ServerError, got {other:?}"),
        }
    }
}
