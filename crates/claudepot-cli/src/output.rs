//! Shared output helpers for human vs `--json` formatting, per
//! `.claude/rules/commands.md` rule 5. Generic helpers (JSON
//! emission, byte sizes, timestamps, truncation) live here so the
//! per-noun handler files don't grow drifting private copies;
//! account-list formatting sits below because `account` is the only
//! consumer.

use claudepot_core::account::Account;
use std::collections::HashMap;
use uuid::Uuid;

/// Print any serializable value as a single pretty JSON document on
/// stdout — the `--json` contract from `rules/commands.md`.
pub fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Human-readable byte size, 1024-based with the matching binary
/// unit labels (KiB/MiB/GiB). Re-exported from core so the CLI and
/// core's dry-run formatter can't drift (the CLI used to carry its
/// own copy).
pub use claudepot_core::project::core::format_size;

/// Render an epoch-milliseconds timestamp as a `YYYY-MM-DD` date,
/// `—` when out of range.
pub fn format_ts_ms(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string())
}

/// Truncate a string to `max` chars by keeping the tail (path-friendly,
/// since the basename usually carries the load-bearing token) and
/// prefixing the elision with `…`. Returns the input untouched when
/// it's already short enough.
pub fn truncate_start(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    // Keep the tail (more informative for paths). Prefix with "…".
    let skip = s.chars().count() - (max - 1);
    let kept: String = s.chars().skip(skip).collect();
    format!("…{kept}")
}

#[derive(Default, Clone, Copy)]
pub struct AccountUsageRow {
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
}

pub fn format_account_list(
    accounts: &[Account],
    usage: &HashMap<Uuid, AccountUsageRow>,
    json: bool,
) -> String {
    if json {
        return format_account_list_json(accounts, usage);
    }

    if accounts.is_empty() {
        return "No accounts registered.\n\nRun `claudepot account add --from-current` to import your current CC account.".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "  {:<30}  {:<6}  {:>4}  {:>4}  {:<8}  {:<8}\n",
        "Email", "Plan", "5h", "7d", "CLI", "Desktop"
    ));
    out.push_str(&format!(
        "  {:<30}  {:<6}  {:>4}  {:>4}  {:<8}  {:<8}\n",
        "─────", "────", "──", "──", "───", "───────"
    ));

    for a in accounts {
        let plan = a.subscription_type.as_deref().unwrap_or("?");
        let cli_mark = if a.is_cli_active { "active" } else { "—" };
        let desk_mark = if a.is_desktop_active { "active" } else { "—" };
        let row = usage.get(&a.uuid).copied().unwrap_or_default();
        let fh_str = row
            .five_hour
            .map(|pct| format!("{:.0}%", pct))
            .unwrap_or_else(|| "—".to_string());
        let sd_str = row
            .seven_day
            .map(|pct| format!("{:.0}%", pct))
            .unwrap_or_else(|| "—".to_string());
        out.push_str(&format!(
            "  {:<30}  {:<6}  {:>4}  {:>4}  {:<8}  {:<8}\n",
            a.email, plan, fh_str, sd_str, cli_mark, desk_mark
        ));
    }
    out.push_str(&format!("\n{} account(s) registered.", accounts.len()));
    out
}

fn format_account_list_json(
    accounts: &[Account],
    usage: &HashMap<Uuid, AccountUsageRow>,
) -> String {
    let entries: Vec<serde_json::Value> = accounts
        .iter()
        .map(|a| {
            let mut obj = serde_json::json!({
                "uuid": a.uuid.to_string(),
                "email": a.email,
                "org_uuid": a.org_uuid,
                "org_name": a.org_name,
                "subscription_type": a.subscription_type,
                "rate_limit_tier": a.rate_limit_tier,
                "cli_active": a.is_cli_active,
                "desktop_active": a.is_desktop_active,
                "has_cli_credentials": a.has_cli_credentials,
                "has_desktop_profile": a.has_desktop_profile,
            });
            if let Some(row) = usage.get(&a.uuid) {
                if let Some(pct) = row.five_hour {
                    obj["five_hour_pct"] = serde_json::json!(pct);
                }
                if let Some(pct) = row.seven_day {
                    obj["seven_day_pct"] = serde_json::json!(pct);
                }
            }
            obj
        })
        .collect();
    serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_uses_binary_units_with_binary_labels() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1024 * 1024), "1.0 MiB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024 / 2), "1.5 GiB");
    }

    #[test]
    fn test_format_ts_ms_renders_date_or_dash() {
        assert_eq!(format_ts_ms(0), "1970-01-01");
        // Far out of chrono's representable range → dash.
        assert_eq!(format_ts_ms(i64::MAX), "—");
    }

    #[test]
    fn test_truncate_start_keeps_tail() {
        assert_eq!(truncate_start("short", 10), "short");
        let got = truncate_start("/Users/joker/projects/claudepot-app", 12);
        assert_eq!(got, "…audepot-app");
        assert_eq!(got.chars().count(), 12);
    }
}
