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
        .header("anthropic-beta", crate::oauth::beta_header::get_or_default())
        .header("Content-Type", "application/json")
        .send()
        .await?;

    let status = resp.status();
    if status == 401 {
        return Err(OAuthError::AuthFailed("access token rejected by /api/oauth/profile".into()));
    }
    if status == 429 {
        return Err(OAuthError::RateLimited { retry_after_secs: 60 });
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(OAuthError::AuthFailed(format!("profile API returned {status}: {body}")));
    }

    let body: serde_json::Value = resp.json().await?;

    let account = &body["account"];
    let org = &body["organization"];

    Ok(Profile {
        email: account["email"].as_str().unwrap_or("").to_string(),
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
