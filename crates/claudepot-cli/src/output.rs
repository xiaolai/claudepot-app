use claudepot_core::account::Account;
use std::collections::HashMap;
use uuid::Uuid;

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
