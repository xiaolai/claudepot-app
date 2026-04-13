use crate::error::OAuthError;
use crate::oauth::http_client;
use serde::Deserialize;

const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const DEFAULT_SCOPES: &str =
    "user:file_upload user:inference user:mcp_servers user:profile user:sessions:claude_code";

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
}

/// Exchange a refresh token for a new access token + rotated refresh token.
pub async fn refresh(refresh_token: &str) -> Result<TokenResponse, OAuthError> {
    let client = http_client()?;
    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
            "scope": DEFAULT_SCOPES,
        }))
        .send()
        .await?;

    let status = resp.status();
    if status == 429 {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        return Err(OAuthError::RateLimited {
            retry_after_secs: retry_after,
        });
    }
    if !status.is_success() {
        let _ = resp.text().await; // consume without exposing
        return Err(OAuthError::RefreshFailed(format!(
            "token endpoint returned {status}"
        )));
    }

    let token_resp: TokenResponse = resp.json().await?;
    Ok(token_resp)
}

/// Build a credential blob JSON string from a token response,
/// preserving the original blob's subscription metadata if provided.
pub fn build_blob(resp: &TokenResponse, original: Option<&crate::blob::CredentialBlob>) -> String {
    let expires_at_ms = chrono::Utc::now().timestamp_millis() + (resp.expires_in as i64 * 1000);
    let scopes: Vec<&str> = resp
        .scope
        .as_deref()
        .unwrap_or(DEFAULT_SCOPES)
        .split(' ')
        .collect();

    // Preserve subscription metadata from the original blob if available,
    // otherwise fall back to what the token response implies.
    let sub_type = original
        .and_then(|b| b.claude_ai_oauth.subscription_type.as_deref())
        .unwrap_or("");
    let rate_tier = original
        .and_then(|b| b.claude_ai_oauth.rate_limit_tier.as_deref())
        .unwrap_or("");

    serde_json::json!({
        "claudeAiOauth": {
            "accessToken": resp.access_token,
            "refreshToken": resp.refresh_token,
            "expiresAt": expires_at_ms,
            "scopes": scopes,
            "subscriptionType": sub_type,
            "rateLimitTier": rate_tier
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::CredentialBlob;

    fn make_token_response() -> TokenResponse {
        TokenResponse {
            access_token: "sk-ant-oat01-new".into(),
            refresh_token: "sk-ant-ort01-new".into(),
            expires_in: 3600,
            scope: Some("user:inference".into()),
            token_type: Some("Bearer".into()),
        }
    }

    #[test]
    fn test_build_blob_preserves_subscription_metadata() {
        let original_json = crate::testing::sample_blob_json(0);
        let original = CredentialBlob::from_json(&original_json).unwrap();
        assert_eq!(
            original.claude_ai_oauth.subscription_type.as_deref(),
            Some("pro")
        );
        assert_eq!(
            original.claude_ai_oauth.rate_limit_tier.as_deref(),
            Some("default_claude_pro")
        );

        let resp = make_token_response();
        let result_json = build_blob(&resp, Some(&original));
        let result = CredentialBlob::from_json(&result_json).unwrap();

        assert_eq!(result.claude_ai_oauth.access_token, "sk-ant-oat01-new");
        assert_eq!(
            result.claude_ai_oauth.subscription_type.as_deref(),
            Some("pro")
        );
        assert_eq!(
            result.claude_ai_oauth.rate_limit_tier.as_deref(),
            Some("default_claude_pro")
        );
    }

    #[test]
    fn test_build_blob_without_original_uses_empty_defaults() {
        let resp = make_token_response();
        let result_json = build_blob(&resp, None);
        let result = CredentialBlob::from_json(&result_json).unwrap();

        assert_eq!(result.claude_ai_oauth.access_token, "sk-ant-oat01-new");
        assert_eq!(
            result.claude_ai_oauth.subscription_type.as_deref(),
            Some("")
        );
        assert_eq!(result.claude_ai_oauth.rate_limit_tier.as_deref(), Some(""));
    }

    #[test]
    fn test_build_blob_computes_future_expiry() {
        let resp = make_token_response();
        let result_json = build_blob(&resp, None);
        let result = CredentialBlob::from_json(&result_json).unwrap();

        let now_ms = chrono::Utc::now().timestamp_millis();
        assert!(result.claude_ai_oauth.expires_at > now_ms + 3500_000);
        assert!(result.claude_ai_oauth.expires_at < now_ms + 3700_000);
    }
}
