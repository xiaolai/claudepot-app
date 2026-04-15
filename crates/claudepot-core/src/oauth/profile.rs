use crate::error::OAuthError;
use crate::oauth::http_client;

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
        .send()
        .await?;

    let status = resp.status();
    if status == 401 {
        return Err(OAuthError::AuthFailed(
            "access token rejected by /api/oauth/profile".into(),
        ));
    }
    if status == 429 {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60)
            .min(300); // Cap at 5 minutes to prevent server-controlled DoS
        return Err(OAuthError::RateLimited {
            retry_after_secs: retry_after,
        });
    }
    if !status.is_success() {
        let _ = resp.text().await; // consume body without exposing it
        return Err(OAuthError::ServerError(format!(
            "profile API returned {status}"
        )));
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
