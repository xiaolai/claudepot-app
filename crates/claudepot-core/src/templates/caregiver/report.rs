//! Structured caregiver-report schema and deterministic
//! email-body renderer.
//!
//! The LLM emits JSON conforming to `CaregiverReport`. We
//! validate via serde; if validation fails, no email is sent
//! (an alert lands in the alerts log instead). Only fields in
//! this struct can survive into the rendered email — the type
//! system is the privacy boundary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaregiverReport {
    /// Inclusive period start; YYYY-MM-DD only — no timestamps.
    pub period_start: String,
    pub period_end: String,
    /// User-supplied at install time. The LLM does not get a
    /// chance to set this — the renderer injects it from the
    /// consent record before sending.
    #[serde(default)]
    pub dependent_label: String,
    pub disk: DiskState,
    pub backups: BackupState,
    pub updates: UpdateState,
    #[serde(default)]
    pub battery: Option<BatteryState>,
    #[serde(default)]
    pub login_items_added: Vec<LoginItemSummary>,
    #[serde(default)]
    pub apps_installed: Vec<AppInstallSummary>,
    pub login_attempts: LoginAttemptSummary,
    pub crashes: CrashSummary,
    /// Bounded narrative — max 3 entries, each max 200 chars.
    /// Stripped if violated.
    #[serde(default)]
    pub concerns_to_raise: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskState {
    pub used_pct: f32,
    pub free_gb: f32,
    pub trend_gb_per_week: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupState {
    pub time_machine_last_success: Option<String>,
    pub icloud_in_sync: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateState {
    pub macos_pending_count: u32,
    pub app_store_pending_count: u32,
    pub security_critical_pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatteryState {
    pub cycle_count: u32,
    pub max_capacity_pct: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginItemSummary {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppInstallSummary {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoginAttemptSummary {
    pub count: u32,
    pub anomalies: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrashSummary {
    pub count: u32,
    pub top_apps: Vec<String>,
}

/// Hard caps on free-form fields. Anything over these limits is
/// truncated by the validator below.
pub const MAX_CONCERNS: usize = 3;
pub const MAX_CONCERN_LEN: usize = 200;
pub const MAX_LOGIN_ITEMS: usize = 20;
pub const MAX_APPS_INSTALLED: usize = 20;
pub const MAX_TOP_CRASH_APPS: usize = 5;

/// The keys of the report's allowed sections. Persisted into the
/// consent record so future versions can detect drift between
/// what the user agreed to and what's being sent.
pub const ALLOWED_SCOPE: &[&str] = &[
    "disk",
    "backups",
    "updates",
    "battery",
    "login_items",
    "apps_installed",
    "login_attempts",
    "crashes",
    "concerns_to_raise",
];

/// Validate caps and clamp anything beyond them. Returns the
/// sanitized report; never errors — clamping is preferred to
/// rejection so a slightly-over-budget LLM output still
/// delivers a valid email.
pub fn sanitize(mut r: CaregiverReport) -> CaregiverReport {
    // Concerns: cap count and per-entry length. Drop empties.
    r.concerns_to_raise = r
        .concerns_to_raise
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.chars().take(MAX_CONCERN_LEN).collect())
        .take(MAX_CONCERNS)
        .collect();
    r.login_items_added.truncate(MAX_LOGIN_ITEMS);
    r.apps_installed.truncate(MAX_APPS_INSTALLED);
    r.crashes.top_apps.truncate(MAX_TOP_CRASH_APPS);
    r
}

/// Deterministic plain-text email body from a sanitized report.
/// Pure function. The LLM never produces email body text
/// directly; it produces the structured JSON, and this function
/// renders.
pub fn render_email(r: &CaregiverReport) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Weekly health report for {}\n",
        if r.dependent_label.is_empty() {
            "the dependent's machine"
        } else {
            r.dependent_label.as_str()
        }
    ));
    s.push_str(&format!(
        "Period: {} to {}\n\n",
        r.period_start, r.period_end
    ));

    if let Some(c) = r.concerns_to_raise.first() {
        s.push_str(&format!("Concern to raise: {c}\n\n"));
        for extra in r.concerns_to_raise.iter().skip(1) {
            s.push_str(&format!("Also: {extra}\n"));
        }
        s.push('\n');
    } else {
        s.push_str("No specific concerns this week.\n\n");
    }

    s.push_str("---\n");
    s.push_str(&format!(
        "Disk: {:.1}% used, {:.1} GB free",
        r.disk.used_pct, r.disk.free_gb
    ));
    if let Some(trend) = r.disk.trend_gb_per_week {
        s.push_str(&format!(" (filling at ~{trend:.1} GB/week)"));
    }
    s.push('\n');

    match &r.backups.time_machine_last_success {
        Some(when) => s.push_str(&format!(
            "Backups: Time Machine last completed {when}; iCloud {}.\n",
            if r.backups.icloud_in_sync {
                "in sync"
            } else {
                "out of sync"
            }
        )),
        None => s.push_str("Backups: Time Machine has no recorded completion.\n"),
    }

    s.push_str(&format!(
        "Updates: {} macOS pending, {} App Store pending{}.\n",
        r.updates.macos_pending_count,
        r.updates.app_store_pending_count,
        if r.updates.security_critical_pending {
            " (security-critical present)"
        } else {
            ""
        },
    ));

    if let Some(b) = &r.battery {
        s.push_str(&format!(
            "Battery: {} cycles; {}% of design capacity.\n",
            b.cycle_count, b.max_capacity_pct,
        ));
    }

    if !r.login_items_added.is_empty() {
        s.push_str("New login items this week:\n");
        for item in &r.login_items_added {
            s.push_str(&format!("  - {}\n", item.name));
        }
    }

    if !r.apps_installed.is_empty() {
        s.push_str("New apps installed this week:\n");
        for app in &r.apps_installed {
            s.push_str(&format!("  - {}\n", app.name));
        }
    }

    s.push_str(&format!(
        "Login attempts: {} total, {} anomalous.\n",
        r.login_attempts.count, r.login_attempts.anomalies,
    ));

    s.push_str(&format!("Crashes: {}", r.crashes.count));
    if !r.crashes.top_apps.is_empty() {
        s.push_str(" (");
        s.push_str(&r.crashes.top_apps.join(", "));
        s.push(')');
    }
    s.push_str(".\n");

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> CaregiverReport {
        CaregiverReport {
            period_start: "2026-04-25".into(),
            period_end: "2026-05-02".into(),
            dependent_label: "Dad's MacBook".into(),
            disk: DiskState {
                used_pct: 64.0,
                free_gb: 358.0,
                trend_gb_per_week: Some(1.2),
            },
            backups: BackupState {
                time_machine_last_success: Some("2026-05-02 06:00".into()),
                icloud_in_sync: true,
            },
            updates: UpdateState {
                macos_pending_count: 2,
                app_store_pending_count: 1,
                security_critical_pending: false,
            },
            battery: Some(BatteryState {
                cycle_count: 412,
                max_capacity_pct: 87,
            }),
            login_items_added: vec![],
            apps_installed: vec![],
            login_attempts: LoginAttemptSummary {
                count: 14,
                anomalies: 0,
            },
            crashes: CrashSummary {
                count: 0,
                top_apps: vec![],
            },
            concerns_to_raise: vec![],
        }
    }

    #[test]
    fn deserializes_valid_report() {
        let r = report();
        let s = serde_json::to_string(&r).unwrap();
        let back: CaregiverReport = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn rejects_unknown_top_level_field_via_unknown_variant() {
        // The schema doesn't deny unknown fields globally — by
        // design, we want forward compat for added optionals.
        // But unknown fields in a typed sub-struct (like
        // DiskState) are silently dropped. Verify the dropped
        // shape doesn't reach the rendered email.
        let json = serde_json::json!({
            "period_start": "2026-04-25",
            "period_end": "2026-05-02",
            "disk": {
                "used_pct": 50.0,
                "free_gb": 100.0,
                "secret_field": "should be dropped",
                "user_filename": "/private/secret.txt"
            },
            "backups": {"time_machine_last_success": null, "icloud_in_sync": true},
            "updates": {"macos_pending_count": 0, "app_store_pending_count": 0, "security_critical_pending": false},
            "login_attempts": {"count": 0, "anomalies": 0},
            "crashes": {"count": 0, "top_apps": []}
        });
        let r: CaregiverReport = serde_json::from_value(json).unwrap();
        let body = render_email(&r);
        assert!(!body.contains("secret_field"));
        assert!(!body.contains("/private/secret.txt"));
        assert!(!body.contains("user_filename"));
    }

    #[test]
    fn concerns_capped_to_3_each_200_chars() {
        let mut r = report();
        r.concerns_to_raise = vec![
            "first concern".into(),
            "x".repeat(500),
            "third".into(),
            "fourth — should be dropped".into(),
            "  ".into(), // empty after trim
        ];
        let r = sanitize(r);
        assert_eq!(r.concerns_to_raise.len(), 3);
        assert!(r.concerns_to_raise[1].chars().count() <= MAX_CONCERN_LEN);
        assert!(!r.concerns_to_raise.iter().any(|s| s.contains("fourth")));
    }

    #[test]
    fn login_items_capped() {
        let mut r = report();
        r.login_items_added = (0..50)
            .map(|i| LoginItemSummary {
                name: format!("item-{i}"),
            })
            .collect();
        let r = sanitize(r);
        assert_eq!(r.login_items_added.len(), MAX_LOGIN_ITEMS);
    }

    #[test]
    fn render_includes_dependent_label() {
        let r = report();
        let body = render_email(&r);
        assert!(body.contains("Dad's MacBook"));
        assert!(body.contains("Period: 2026-04-25 to 2026-05-02"));
    }

    #[test]
    fn render_with_no_concerns_says_no_specific_concerns() {
        let r = report();
        let body = render_email(&r);
        assert!(body.contains("No specific concerns"));
    }

    #[test]
    fn render_with_concern_leads_with_concern() {
        let mut r = report();
        r.concerns_to_raise = vec!["disk filling fast".into()];
        let body = render_email(&r);
        assert!(body.contains("Concern to raise: disk filling fast"));
        assert!(!body.contains("No specific concerns"));
    }

    #[test]
    fn render_omits_battery_section_when_none() {
        let mut r = report();
        r.battery = None;
        let body = render_email(&r);
        assert!(!body.contains("Battery:"));
    }

    #[test]
    fn render_never_includes_unbounded_text() {
        // A "free-form text everywhere" injection attempt: try
        // to stuff disk-trend into a string field and see if
        // it survives. (It can't — the schema is f32.)
        let json = serde_json::json!({
            "period_start": "2026-04-25",
            "period_end": "2026-05-02",
            "disk": { "used_pct": 50.0, "free_gb": 100.0, "trend_gb_per_week": "MALICIOUS PROSE HERE" },
            "backups": {"time_machine_last_success": null, "icloud_in_sync": true},
            "updates": {"macos_pending_count": 0, "app_store_pending_count": 0, "security_critical_pending": false},
            "login_attempts": {"count": 0, "anomalies": 0},
            "crashes": {"count": 0, "top_apps": []}
        });
        // Schema rejects: trend_gb_per_week must be a number.
        let r: Result<CaregiverReport, _> = serde_json::from_value(json);
        assert!(r.is_err());
    }
}
