use crate::error::OAuthError;
use crate::oauth::http_client;
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct UsageResponse {
    #[serde(default)]
    pub five_hour: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day_oauth_apps: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day_opus: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day_sonnet: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day_cowork: Option<UsageWindow>,
    #[serde(default)]
    pub iguana_necktie: Option<UsageWindow>,
    #[serde(default)]
    pub extra_usage: Option<ExtraUsage>,
    /// Catch-all for new fields Anthropic adds.
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UsageWindow {
    pub utilization: f64,
    pub resets_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtraUsage {
    pub is_enabled: bool,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    pub utilization: Option<f64>,
}

pub async fn fetch(access_token: &str) -> Result<UsageResponse, OAuthError> {
    let client = http_client()?;
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(access_token)
        .header("anthropic-beta", crate::oauth::beta_header::get_or_default())
        .header("Content-Type", "application/json")
        .send()
        .await?;

    let status = resp.status();
    if status == 401 {
        return Err(OAuthError::AuthFailed("access token rejected by /api/oauth/usage".into()));
    }
    if status == 429 {
        return Err(OAuthError::RateLimited { retry_after_secs: 60 });
    }
    if !status.is_success() {
        let _ = resp.text().await; // consume without exposing
        return Err(OAuthError::AuthFailed(format!("usage API returned {status}")));
    }

    // Parse typed fields first, then separately parse unknown fields.
    // Avoids the serde flatten + typed fields duplication issue.
    let body_text = resp.text().await?;
    let usage: UsageResponse = serde_json::from_str(&body_text)
        .map_err(|e| OAuthError::AuthFailed(format!("usage response parse error: {e}")))?;

    Ok(usage)
}
