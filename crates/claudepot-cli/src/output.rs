use claudepot_core::account::Account;

pub fn format_account_list(accounts: &[Account], json: bool) -> String {
    if json {
        return format_account_list_json(accounts);
    }

    if accounts.is_empty() {
        return "No accounts registered.\n\nRun `claudepot account add --from-current` to import your current CC account.".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "  {:<30}  {:<6}  {:<8}  {:<8}\n",
        "Email", "Plan", "CLI", "Desktop"
    ));
    out.push_str(&format!(
        "  {:<30}  {:<6}  {:<8}  {:<8}\n",
        "─────", "────", "───", "───────"
    ));

    for a in accounts {
        let plan = a.subscription_type.as_deref().unwrap_or("?");
        let cli_mark = if a.is_cli_active { "active" } else { "—" };
        let desk_mark = if a.is_desktop_active { "active" } else { "—" };
        out.push_str(&format!(
            "  {:<30}  {:<6}  {:<8}  {:<8}\n",
            a.email, plan, cli_mark, desk_mark
        ));
    }
    out.push_str(&format!("\n{} account(s) registered.", accounts.len()));
    out
}

fn format_account_list_json(accounts: &[Account]) -> String {
    let entries: Vec<serde_json::Value> = accounts
        .iter()
        .map(|a| {
            serde_json::json!({
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
            })
        })
        .collect();
    serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
}
