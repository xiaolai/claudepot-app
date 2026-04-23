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

const VALIDATION_ENDPOINT: &str = "https://api.anthropic.com/v1/models";
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
    let client = http_client()?;
    let resp = client
        .get(VALIDATION_ENDPOINT)
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
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60)
            .min(300);
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
