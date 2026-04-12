use anyhow::Result;
use crate::AppContext;

pub async fn run(ctx: &AppContext) -> Result<()> {
    use claudepot_core::services::doctor_service;

    let report = doctor_service::check_health(&ctx.store).await;

    if ctx.json {
        println!("{}", serde_json::json!({
            "platform": report.platform,
            "arch": report.arch,
            "data_dir": report.data_dir.display().to_string(),
            "data_dir_exists": report.data_dir_exists,
            "account_count": report.account_count,
            "cli_path": report.cli_path.map(|p| p.display().to_string()),
            "cli_version": report.cli_version,
            "desktop_installed": report.desktop_installed,
            "desktop_version": report.desktop_version,
            "beta_header": report.beta_header,
            "api_reachable": matches!(report.api_status, doctor_service::ApiStatus::Reachable),
            "accounts": report.account_health.iter().map(|a| {
                serde_json::json!({
                    "email": a.email,
                    "token_status": a.token_status,
                    "remaining_mins": a.remaining_mins,
                })
            }).collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    println!("Claudepot v{} — Health Check\n", env!("CARGO_PKG_VERSION"));

    // Platform
    println!("  Platform:     {} ({})", report.platform, report.arch);

    // Data dir
    if report.data_dir_exists {
        ok("Data dir", &report.data_dir.display().to_string());
    } else {
        warn("Data dir", &format!("{} (does not exist)", report.data_dir.display()));
    }

    // Accounts
    ok("Accounts", &format!("{} registered", report.account_count));

    // CLI
    match (&report.cli_path, &report.cli_version) {
        (Some(p), Some(v)) => ok("Claude CLI", &format!("{} ({})", p.display(), v)),
        (Some(p), None) => ok("Claude CLI", &format!("{}", p.display())),
        _ => warn("Claude CLI", "not found"),
    }

    // Desktop
    if report.desktop_installed {
        ok("Claude Desktop", &format!("v{}", report.desktop_version.as_deref().unwrap_or("?")));
    } else {
        warn("Claude Desktop", "not installed");
    }

    // Keychain
    if let Some(ref status) = report.keychain_status {
        match status {
            Ok(true) => ok("Keychain", "Claude Code-credentials readable"),
            Ok(false) => warn("Keychain", "Claude Code-credentials empty"),
            Err(e) => err("Keychain", e),
        }
    }

    // Beta header
    ok("Beta header", &report.beta_header);

    // API
    match &report.api_status {
        doctor_service::ApiStatus::Reachable => {
            ok("API reachable", "api.anthropic.com");
        }
        doctor_service::ApiStatus::GeoBlocked => {
            err("API blocked", "403 Forbidden — use HTTPS_PROXY");
        }
        doctor_service::ApiStatus::Unreachable(e) => {
            err("API", &format!("unreachable: {e}"));
        }
        doctor_service::ApiStatus::Unknown => {
            warn("API", "status unknown");
        }
    }

    // DB error
    if let Some(ref db_err) = report.db_error {
        err("Database", db_err);
    }

    // Account health
    let mut expired_accounts = 0;
    if !report.account_health.is_empty() {
        println!("\n  Account health:");
        for a in &report.account_health {
            if a.remaining_mins.map_or(false, |m| m > 0) {
                println!("    {}  ✓ {}", a.email, a.token_status);
            } else {
                println!("    {}  ✗ {}", a.email, a.token_status);
                expired_accounts += 1;
            }
        }
    }

    // Desktop profiles
    if report.desktop_profiles.iter().any(|p| p.item_count.is_some()) {
        println!("\n  Desktop profiles:");
        for p in &report.desktop_profiles {
            match p.item_count {
                Some(c) => println!("    {}  ✓ {} items", p.email, c),
                None => println!("    {}  — no profile", p.email),
            }
        }
    }

    println!();
    let mut errors = 0;
    if matches!(report.api_status, doctor_service::ApiStatus::GeoBlocked | doctor_service::ApiStatus::Unreachable(_)) {
        errors += 1;
    }
    if report.db_error.is_some() {
        errors += 1;
    }

    let mut warnings = 0;
    if !report.data_dir_exists { warnings += 1; }
    if report.cli_path.is_none() { warnings += 1; }
    if !report.desktop_installed { warnings += 1; }
    if expired_accounts > 0 { warnings += expired_accounts; }

    if errors > 0 {
        println!("{} error(s), {} warning(s).", errors, warnings);
    } else if warnings > 0 {
        println!("All checks passed ({} warning(s)).", warnings);
    } else {
        println!("All checks passed.");
    }

    Ok(())
}

fn ok(label: &str, detail: &str) {
    println!("  {:<16}✓ {}", label, detail);
}

fn warn(label: &str, detail: &str) {
    println!("  {:<16}⚠ {}", label, detail);
}

fn err(label: &str, detail: &str) {
    println!("  {:<16}✗ {}", label, detail);
}
