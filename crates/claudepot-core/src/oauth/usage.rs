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
            "access token rejected by /api/oauth/usage".into(),
        ));
    }
    if status == 429 {
        return Err(OAuthError::RateLimited {
            retry_after_secs: 60,
        });
    }
    if !status.is_success() {
        let _ = resp.text().await; // consume without exposing
        return Err(OAuthError::AuthFailed(format!(
            "usage API returned {status}"
        )));
    }

    // Parse typed fields first, then separately parse unknown fields.
    // Avoids the serde flatten + typed fields duplication issue.
    let body_text = resp.text().await?;
    let usage: UsageResponse = serde_json::from_str(&body_text)
        .map_err(|e| OAuthError::AuthFailed(format!("usage response parse error: {e}")))?;

    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_response_deserialize_full() {
        let json = r#"{
            "five_hour": {"utilization": 42.5, "resets_at": "2026-04-13T10:00:00+00:00"},
            "seven_day": {"utilization": 10.0, "resets_at": "2026-04-19T00:00:00+00:00"}
        }"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(usage.five_hour.unwrap().utilization, 42.5);
        assert_eq!(usage.seven_day.unwrap().utilization, 10.0);
    }

    #[test]
    fn test_usage_response_deserialize_minimal() {
        let json = "{}";
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        assert!(usage.five_hour.is_none());
        assert!(usage.seven_day.is_none());
        assert!(usage.extra_usage.is_none());
    }

    #[test]
    fn test_usage_response_unknown_fields_captured() {
        let json =
            r#"{"new_window": {"utilization": 99.0, "resets_at": "2026-04-13T10:00:00+00:00"}}"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        assert!(usage.unknown.contains_key("new_window"));
    }

    #[test]
    fn test_extra_usage_deserialize() {
        let json = r#"{"extra_usage": {"is_enabled": true, "monthly_limit": 100.0, "used_credits": 25.0, "utilization": 0.25}}"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        let extra = usage.extra_usage.unwrap();
        assert!(extra.is_enabled);
        assert_eq!(extra.monthly_limit, Some(100.0));
        assert_eq!(extra.used_credits, Some(25.0));
    }
}
