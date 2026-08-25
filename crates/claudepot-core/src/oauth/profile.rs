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

/// **401 only.** 403 is deliberately NOT a refusal — see below.
///
/// `Refused` is not a message, it is an action: `services::identity`
/// maps it to `ProfileCheck::Rejected`, which enters the refresh path
/// and **spends the refresh token**. That token is single-use and a
/// running Claude Code may still hold it in memory, so spending it on
/// a false positive retires the token CC holds and forces the user to
/// re-login.
///
/// 401 earns that risk and 403 does not. 401 means "you are not
/// authenticated" — unambiguously the credential. 403 means "you are
/// authenticated and still refused", which in front of a
/// Cloudflare-fronted endpoint is as easily a WAF rule, a geo block,
/// or a VPN exit node as it is a dead token. The reporter of
/// github#93 chased exactly that reading first, and was right to:
/// their 403 *did* come from a stale token, but nothing in the status
/// said so.
///
/// So the asymmetry decides it. Classifying 403 transient costs a
/// retry loop against a credential that is genuinely dead.
/// Classifying it a refusal burns a single-use token because someone
/// was on a VPN. A surface that wants to *say* something about a 403
/// should improve its message, not reach for this verdict.
pub(crate) fn classify_status(status: u16) -> StatusVerdict {
    match status {
        401 => StatusVerdict::Refused,
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
    fn only_401_is_a_refusal() {
        assert_eq!(classify_status(401), StatusVerdict::Refused);
    }

    /// Regression lock. `Refused` spends a single-use refresh token,
    /// and a 403 from a Cloudflare-fronted endpoint is as likely to be
    /// a WAF or geo block as a dead credential — so promoting it here
    /// burns the token CC may still hold, over someone's VPN. Shipped
    /// that way briefly in 0.5.3; this test is why it cannot return.
    #[test]
    fn forbidden_is_transient_because_refusal_spends_a_refresh_token() {
        assert_eq!(classify_status(403), StatusVerdict::ServerTrouble);
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
        for s in [400, 403, 404, 418, 500, 502, 503] {
            assert_eq!(
                classify_status(s),
                StatusVerdict::ServerTrouble,
                "status {s}"
            );
        }
    }
}
