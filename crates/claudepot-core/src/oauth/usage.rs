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
    #[serde(default)]
    pub utilization: f64,
    /// The server returns `null` for windows that have had no activity
    /// yet (no reset timestamp to report). Older code made this a
    /// required `DateTime` and the whole usage response failed to
    /// parse whenever any window had a null reset — rendering usage
    /// entirely blank for accounts whose 5h/7d windows haven't yet
    /// ticked. Now optional; the DTO renders "—" when absent.
    #[serde(default)]
    pub resets_at: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtraUsage {
    pub is_enabled: bool,
    /// Monthly cap in MINOR units (e.g. pence for GBP, cents for USD).
    /// Divide by 100 for display. Confirmed against live /api/oauth/usage:
    /// `monthly_limit: 15000` corresponds to the £150 limit shown in the
    /// Anthropic billing UI for a GBP-billed account.
    pub monthly_limit: Option<f64>,
    /// Amount spent this period in MINOR units (same basis as
    /// `monthly_limit`). Divide by 100 for display.
    pub used_credits: Option<f64>,
    pub utilization: Option<f64>,
    /// ISO 4217 currency code — "USD", "GBP", "EUR", etc. Missing on
    /// older responses; frontend falls back to USD when absent.
    #[serde(default)]
    pub currency: Option<String>,
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

    /// Regression: the /usage endpoint returns `resets_at: null` for a
    /// window that has had no activity yet. An earlier non-optional
    /// type made the WHOLE response fail to parse — causing the
    /// sidebar to show blank usage even though the server responded
    /// 200 OK. Optional resets_at keeps the window parseable; callers
    /// render "—" in place of the reset time.
    #[test]
    fn test_usage_window_accepts_null_resets_at() {
        let json = r#"{
            "five_hour": {"utilization": 0.0, "resets_at": null},
            "seven_day": {"utilization": 12.5, "resets_at": "2026-04-19T00:00:00+00:00"}
        }"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        let fh = usage.five_hour.unwrap();
        assert_eq!(fh.utilization, 0.0);
        assert!(fh.resets_at.is_none());
        let sd = usage.seven_day.unwrap();
        assert!(sd.resets_at.is_some());
    }

    /// Regression: resets_at omitted entirely (vs. explicit null) — the
    /// serde(default) attribute should yield None either way.
    #[test]
    fn test_usage_window_accepts_missing_resets_at() {
        let json = r#"{"five_hour": {"utilization": 5.0}}"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        let fh = usage.five_hour.unwrap();
        assert_eq!(fh.utilization, 5.0);
        assert!(fh.resets_at.is_none());
    }

    #[test]
    fn test_extra_usage_deserialize() {
        let json = r#"{"extra_usage": {"is_enabled": true, "monthly_limit": 100.0, "used_credits": 25.0, "utilization": 0.25}}"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        let extra = usage.extra_usage.unwrap();
        assert!(extra.is_enabled);
        assert_eq!(extra.monthly_limit, Some(100.0));
        assert_eq!(extra.used_credits, Some(25.0));
        // No `currency` on this older-shape payload — Option stays None.
        assert!(extra.currency.is_none());
    }

    /// Captures the real /api/oauth/usage shape for a GBP-billed
    /// account: amounts in MINOR units (pence), `currency` field
    /// present. `monthly_limit: 15000` maps to £150; `used_credits:
    /// 1911` maps to £19.11. Display-layer divides by 100 and uses
    /// the ISO code for the symbol. Regression guard for the bug
    /// where the GUI showed $15000 / $1911 because the currency
    /// field was dropped and values were treated as major units.
    #[test]
    fn test_extra_usage_deserialize_gbp_minor_units() {
        let json = r#"{
            "extra_usage": {
                "is_enabled": true,
                "monthly_limit": 15000,
                "used_credits": 1911.0,
                "utilization": 12.74,
                "currency": "GBP"
            }
        }"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        let extra = usage.extra_usage.unwrap();
        assert!(extra.is_enabled);
        assert_eq!(extra.monthly_limit, Some(15000.0));
        assert_eq!(extra.used_credits, Some(1911.0));
        assert_eq!(extra.currency.as_deref(), Some("GBP"));
    }
}
