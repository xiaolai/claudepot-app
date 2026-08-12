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
    /// CC 2.1.123 dropped this field from its typed read (we
    /// confirmed via `strings` against the live binary), but we
    /// don't know if the server stopped emitting it. The
    /// frontend's `UsageBlock` and `OAuthUsageModal` still render a
    /// 7-day OAuth-apps row when the field is non-null, and the
    /// render-if-nonzero rule means a `null` from the server hides
    /// the row cleanly. Keep the typed field so live data isn't
    /// dropped on the floor; let the catch-all handle it the day
    /// the wire truly removes it.
    #[serde(default)]
    pub seven_day_oauth_apps: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day_opus: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day_sonnet: Option<UsageWindow>,
    /// Cowork/team-shared budget window. Earlier client versions
    /// emitted `seven_day_cowork`; the 2.1.x line normalised the key
    /// shape to plain `cowork` (still a `UsageWindow`). Both spellings
    /// are accepted: the alias keeps the field populated whichever
    /// shape the server returns.
    #[serde(default, alias = "cowork")]
    pub seven_day_cowork: Option<UsageWindow>,
    /// Experimental codename observed during 0.0.6 — stopped
    /// appearing in CC 2.1.123. May reappear under a new name; the
    /// catch-all picks it up if so. Kept here only because removing
    /// the typed field would silently swallow the value if the
    /// server were to emit it again. Scheduled for removal once we
    /// have two consecutive releases with no live observation.
    #[serde(default)]
    pub iguana_necktie: Option<UsageWindow>,
    #[serde(default)]
    pub extra_usage: Option<ExtraUsage>,
    /// Model- and surface-scoped rate limits. Added 2026-08: Anthropic
    /// moved per-model windows out of the `seven_day_<model>` keys —
    /// which now come back `null` on every observed account — and into
    /// this generic array. Untyped, it landed in `unknown` and never
    /// reached the UI.
    ///
    /// `deserialize_with` swallows a reshaped array: a parse error here
    /// would blank the whole usage panel, and a missing row is a far
    /// better failure than a missing panel.
    #[serde(default, deserialize_with = "lenient_limits")]
    pub limits: Vec<UsageLimit>,
    /// Catch-all for any future field Anthropic adds — keeps the
    /// response parseable so UI doesn't blank when the wire grows.
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

/// One entry of `/api/oauth/usage`'s `limits[]`.
///
/// `kind` is `session` | `weekly_all` | `weekly_scoped` in every
/// observation, but stays a `String`: a new kind must not fail the
/// parse.
#[derive(Debug, Clone, Deserialize)]
pub struct UsageLimit {
    pub kind: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub percent: f64,
    /// `normal` | `warning` observed. Not rendered yet.
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub resets_at: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub scope: Option<LimitScope>,
    /// Whether this limit is the currently binding one.
    #[serde(default)]
    pub is_active: bool,
}

/// What a limit is scoped to. `surface` is opaque: null in every
/// observation, and inventing a shape for it would be guessing.
#[derive(Debug, Clone, Deserialize)]
pub struct LimitScope {
    #[serde(default)]
    pub model: Option<LimitModel>,
    #[serde(default)]
    pub surface: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LimitModel {
    #[serde(default)]
    pub id: Option<String>,
    /// Server-supplied, already human-readable ("Fable"). Rendered
    /// verbatim — it is a proper noun in every locale.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Tolerate any `limits` value that is not a well-formed array.
fn lenient_limits<'de, D>(deserializer: D) -> Result<Vec<UsageLimit>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(raw).unwrap_or_default())
}

impl UsageResponse {
    /// Limits carrying a usable model name — the ones with no
    /// plan-level equivalent already on screen.
    ///
    /// `session` and `weekly_all` restate `five_hour` and `seven_day`
    /// to the percent, so yielding them would double-render two rows
    /// the UI already draws. Yields `(display_name, limit)`.
    pub fn scoped_limits(&self) -> impl Iterator<Item = (&str, &UsageLimit)> {
        self.limits.iter().filter_map(|l| {
            let name = l.scope.as_ref()?.model.as_ref()?.display_name.as_deref()?;
            (!name.is_empty()).then_some((name, l))
        })
    }
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
        return Err(OAuthError::RateLimited {
            retry_after_secs: crate::oauth::retry_after_secs(resp.headers()),
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
    /// CC 2.1.x uses the bare key `cowork` for the cowork/team window;
    /// older payloads emit `seven_day_cowork`. Both must populate the
    /// same `seven_day_cowork` field so the UI doesn't drift when the
    /// server flips between key shapes. Regression guard for the
    /// rename observed against the 2.1.123 binary.
    #[test]
    fn test_cowork_alias_accepts_both_keys() {
        // New shape: bare `cowork`.
        let new_shape =
            r#"{"cowork":{"utilization":33.0,"resets_at":"2026-04-19T00:00:00+00:00"}}"#;
        let new: UsageResponse = serde_json::from_str(new_shape).unwrap();
        let w = new.seven_day_cowork.expect("cowork alias should populate");
        assert_eq!(w.utilization, 33.0);
        assert!(w.resets_at.is_some());

        // Old shape: `seven_day_cowork`.
        let old_shape =
            r#"{"seven_day_cowork":{"utilization":44.0,"resets_at":"2026-04-19T00:00:00+00:00"}}"#;
        let old: UsageResponse = serde_json::from_str(old_shape).unwrap();
        assert_eq!(
            old.seven_day_cowork
                .expect("plain key still parses")
                .utilization,
            44.0
        );
    }

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

    /// The 2026-08 wire shape: model-scoped limits moved out of the
    /// `seven_day_<model>` keys into a generic `limits[]` array. Both
    /// observed accounts return exactly three entries, and `scope` is
    /// non-null only on `weekly_scoped`.
    #[test]
    fn limits_array_parses_with_model_scope() {
        let json = r#"{
            "five_hour": {"utilization": 31.0, "resets_at": "2026-08-12T04:40:00+00:00"},
            "seven_day": {"utilization": 13.0, "resets_at": "2026-08-16T16:00:00+00:00"},
            "limits": [
                {"kind": "session", "group": "session", "percent": 31,
                 "severity": "normal", "resets_at": "2026-08-12T04:40:00+00:00",
                 "scope": null, "is_active": true},
                {"kind": "weekly_all", "group": "weekly", "percent": 13,
                 "severity": "normal", "resets_at": "2026-08-16T16:00:00+00:00",
                 "scope": null, "is_active": false},
                {"kind": "weekly_scoped", "group": "weekly", "percent": 23,
                 "severity": "normal", "resets_at": "2026-08-16T15:59:59+00:00",
                 "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null},
                 "is_active": false}
            ]
        }"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(usage.limits.len(), 3);
        assert_eq!(usage.limits[2].kind, "weekly_scoped");
        assert_eq!(usage.limits[2].percent, 23.0);
        assert_eq!(usage.limits[2].severity.as_deref(), Some("normal"));
        assert!(usage.limits[2].resets_at.is_some());
    }

    /// `session` restates `five_hour` and `weekly_all` restates
    /// `seven_day`, to the percent. Appending them would double-render
    /// rows already on screen, so only model-scoped entries survive.
    #[test]
    fn scoped_limits_keeps_only_model_scoped_entries() {
        let json = r#"{
            "limits": [
                {"kind": "session", "percent": 31, "scope": null, "is_active": true},
                {"kind": "weekly_all", "percent": 13, "scope": null, "is_active": false},
                {"kind": "weekly_scoped", "percent": 80,
                 "severity": "warning",
                 "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null},
                 "is_active": true}
            ]
        }"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        let scoped: Vec<_> = usage.scoped_limits().collect();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].0, "Fable");
        assert_eq!(scoped[0].1.percent, 80.0);
        assert_eq!(scoped[0].1.severity.as_deref(), Some("warning"));
    }

    /// A scope with no usable model name is not a row we can label, so
    /// it must not reach the UI as a blank-titled entry.
    #[test]
    fn scoped_limits_skips_scope_without_display_name() {
        let json = r#"{
            "limits": [
                {"kind": "weekly_scoped", "percent": 5,
                 "scope": {"model": {"id": "abc", "display_name": null}, "surface": null},
                 "is_active": false},
                {"kind": "weekly_scoped", "percent": 6,
                 "scope": {"model": null, "surface": null}, "is_active": false}
            ]
        }"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(usage.scoped_limits().count(), 0);
    }

    /// Back-compat: servers that never send `limits` must still parse,
    /// and must not report phantom rows.
    #[test]
    fn response_without_limits_yields_no_scoped_rows() {
        let json = r#"{"five_hour": {"utilization": 1.0}}"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        assert!(usage.limits.is_empty());
        assert_eq!(usage.scoped_limits().count(), 0);
    }

    /// Forward-compat, and the reason every field is defaulted: a
    /// reshaped entry must degrade to an empty list rather than fail
    /// the whole parse and blank the usage panel.
    #[test]
    fn malformed_limits_does_not_blank_the_rest_of_the_response() {
        let json = r#"{
            "five_hour": {"utilization": 7.0},
            "limits": "not-an-array"
        }"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        assert_eq!(usage.five_hour.unwrap().utilization, 7.0);
        assert!(usage.limits.is_empty());
    }

    /// Entries omitting optional keys entirely (not just null) parse.
    #[test]
    fn limit_entry_accepts_missing_optional_fields() {
        let json = r#"{"limits": [{"kind": "session", "percent": 0}]}"#;
        let usage: UsageResponse = serde_json::from_str(json).unwrap();
        let l = &usage.limits[0];
        assert_eq!(l.kind, "session");
        assert!(l.group.is_none());
        assert!(l.severity.is_none());
        assert!(l.resets_at.is_none());
        assert!(l.scope.is_none());
        assert!(!l.is_active);
    }
}
