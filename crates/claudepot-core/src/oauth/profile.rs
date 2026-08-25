use crate::error::OAuthError;
use crate::oauth::http_client;
use std::time::Duration;

/// Per-request timeout for `/api/oauth/profile`. The shared
/// `http_client` sets a 15 s ceiling, which is appropriate for token
/// refresh (network-heavy, user-waiting). Profile is a best-effort
/// health probe fired on every window focus — 5 s is the latency floor
/// at which the UI starts to feel stuck. If Anthropic is slower than
/// that, the caller (sync_from_current_cc) swallows the error and the
/// UI keeps rendering stored state.
const PROFILE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Profile {
    pub email: String,
    pub org_uuid: String,
    pub org_name: String,
    pub subscription_type: String,
    pub rate_limit_tier: Option<String>,
    pub account_uuid: String,
    pub display_name: Option<String>,
}

/// What an HTTP status from `/profile` means. Split out of [`fetch`]
/// so the judgement is unit-testable without a live server — the I/O
/// needs a network, the classification never does.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StatusVerdict {
    Ok,
    /// The server refused *this token*. Actionable: re-authenticate.
    Refused,
    RateLimited,
    /// Transient — callers retry rather than invalidating credentials.
    ServerTrouble,
}

/// 401 **and 403** are refusals of the token itself.
///
/// 403 used to fall into `ServerTrouble`, which made a permanently
/// refused credential look transient: `services::identity` maps
/// `ServerError` to `NetworkError`, so the account was retried
/// forever and never marked `Rejected`. Surfaced by github#93, where
/// a stale Claude Desktop token 403'd and the only thing the user was
/// shown was the bare status line.
pub(crate) fn classify_status(status: u16) -> StatusVerdict {
    match status {
        401 | 403 => StatusVerdict::Refused,
        429 => StatusVerdict::RateLimited,
        s if (200..300).contains(&s) => StatusVerdict::Ok,
        _ => StatusVerdict::ServerTrouble,
    }
}

pub async fn fetch(access_token: &str) -> Result<Profile, OAuthError> {
    let client = http_client()?;
    let resp = client
        .get("https://api.anthropic.com/api/oauth/profile")
        .bearer_auth(access_token)
        .header(
            "anthropic-beta",
            crate::oauth::beta_header::get_or_default(),
        )
        .header("Content-Type", "application/json")
        .timeout(PROFILE_TIMEOUT)
        .send()
        .await?;

    let status = resp.status();
    match classify_status(status.as_u16()) {
        StatusVerdict::Ok => {}
        StatusVerdict::Refused => {
            return Err(OAuthError::AuthFailed(format!(
                "access token rejected by /api/oauth/profile ({status})"
            )));
        }
        StatusVerdict::RateLimited => {
            return Err(OAuthError::RateLimited {
                retry_after_secs: crate::oauth::retry_after_secs(resp.headers()),
            });
        }
        StatusVerdict::ServerTrouble => {
            let _ = resp.text().await; // consume body without exposing it
            return Err(OAuthError::ServerError(format!(
                "profile API returned {status}"
            )));
        }
    }

    let body: serde_json::Value = resp.json().await?;

    let account = &body["account"];
    let org = &body["organization"];

    let email = account["email"].as_str().unwrap_or("");
    if email.is_empty() {
        // A malformed 2xx body (no `email` field) is a server-side
        // glitch, not a credential problem. Mapping this to AuthFailed
        // would cause `services::identity` to classify it as Rejected
        // and prompt re-login over what is really a transient issue.
        return Err(OAuthError::ServerError(
            "profile response missing email field".into(),
        ));
    }

    Ok(Profile {
        email: email.to_string(),
        account_uuid: account["uuid"].as_str().unwrap_or("").to_string(),
        display_name: account["display_name"].as_str().map(String::from),
        org_uuid: org["uuid"].as_str().unwrap_or("").to_string(),
        org_name: org["name"].as_str().unwrap_or("").to_string(),
        subscription_type: org["organization_type"]
            .as_str()
            .unwrap_or("")
            .replace("claude_", ""),
        rate_limit_tier: org["rate_limit_tier"].as_str().map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::{classify_status, StatusVerdict};

    #[test]
    fn refusal_statuses_are_not_transient() {
        // 403 is the github#93 case. Classifying it as ServerTrouble
        // makes a dead credential look like a bad minute.
        assert_eq!(classify_status(401), StatusVerdict::Refused);
        assert_eq!(classify_status(403), StatusVerdict::Refused);
    }

    #[test]
    fn rate_limit_is_its_own_verdict() {
        assert_eq!(classify_status(429), StatusVerdict::RateLimited);
    }

    #[test]
    fn success_range_is_ok() {
        for s in [200, 201, 204, 299] {
            assert_eq!(classify_status(s), StatusVerdict::Ok, "status {s}");
        }
    }

    #[test]
    fn other_failures_stay_transient() {
        // 5xx and the odd 4xx are genuinely "try again" — they must
        // NOT invalidate a credential.
        for s in [400, 404, 418, 500, 502, 503] {
            assert_eq!(
                classify_status(s),
                StatusVerdict::ServerTrouble,
                "status {s}"
            );
        }
    }
}
