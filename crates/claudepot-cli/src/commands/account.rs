use crate::output;
use crate::AppContext;
use anyhow::Result;

pub async fn list(ctx: &AppContext) -> Result<()> {
    let accounts = ctx.store.list()?;

    // Collect UUIDs for accounts with credentials to batch-fetch usage.
    let uuids: Vec<uuid::Uuid> = accounts
        .iter()
        .filter(|a| a.has_cli_credentials)
        .map(|a| a.uuid)
        .collect();

    let usage_map = if uuids.is_empty() {
        std::collections::HashMap::new()
    } else {
        ctx.usage_cache
            .fetch_batch(&uuids)
            .await
            .into_iter()
            .filter_map(|(id, result)| {
                result
                    .ok()
                    .flatten()
                    .and_then(|r| r.five_hour.map(|fh| (id, fh.utilization)))
            })
            .collect()
    };

    let formatted = output::format_account_list(&accounts, &usage_map, ctx.json);
    println!("{formatted}");
    Ok(())
}

pub async fn add(ctx: &AppContext, from_current: bool, from_token: Option<String>) -> Result<()> {
    use claudepot_core::services::account_service;

    let result = if from_current {
        ctx.info("Reading current CC credentials...");
        ctx.info("Fetching account profile...");
        account_service::register_from_current(&ctx.store).await?
    } else if let Some(token_arg) = from_token {
        // Read token from stdin if "-" is passed (avoids shell history exposure)
        let token = if token_arg == "-" {
            ctx.info("Reading refresh token from stdin...");
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;
            buf.trim().to_string()
        } else {
            token_arg
        };
        ctx.info("Exchanging refresh token...");
        ctx.info("Fetching account profile...");
        account_service::register_from_token(&ctx.store, &token).await?
    } else {
        return add_via_browser(ctx).await;
    };

    print_register_result(&result, ctx.json);
    Ok(())
}

/// Browser-based add delegates to core's register_from_browser.
async fn add_via_browser(ctx: &AppContext) -> Result<()> {
    use claudepot_core::services::account_service;

    ctx.info("Opening browser for OAuth login...");
    ctx.info("(Complete the login in your browser)");

    let result = account_service::register_from_browser(&ctx.store).await?;
    print_register_result(&result, ctx.json);
    Ok(())
}

fn print_register_result(
    result: &claudepot_core::services::account_service::RegisterResult,
    json: bool,
) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "registered": true,
                "email": result.email,
                "org": result.org_name,
                "plan": result.subscription_type,
                "uuid": result.uuid.to_string(),
            })
        );
    } else {
        println!(
            "Registered: {} ({} {})",
            result.email,
            capitalize(&result.subscription_type),
            result
                .rate_limit_tier
                .as_deref()
                .and_then(|t| t.split('_').next_back())
                .unwrap_or("")
        );
    }
}

pub async fn remove(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::account_service;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    if !ctx.yes {
        eprint!("Remove account \"{email}\"? [y/N] ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    let result =
        account_service::remove_account(&ctx.store, account.uuid, Some(&ctx.usage_cache))
            .await?;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "removed": true,
                "email": result.email,
                "was_cli_active": result.was_cli_active,
                "was_desktop_active": result.was_desktop_active,
                "had_desktop_profile": result.had_desktop_profile,
                "warnings": result.warnings,
            })
        );
    } else {
        if result.had_desktop_profile {
            ctx.info("Deleted Desktop profile snapshot.");
        }
        if result.was_cli_active {
            ctx.info("Note: this was the active CLI account. CLI slot is now empty.");
        }
        if result.was_desktop_active {
            ctx.info("Note: this was the active Desktop account. Desktop slot is now empty.");
        }
        for warning in &result.warnings {
            eprintln!("Warning: {warning}");
        }
        ctx.info(&format!("Removed: {}", result.email));
    }
    Ok(())
}

pub async fn inspect(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::{account_service, usage_cache::UsageFetchError};

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    let health = account_service::token_health(account.uuid, account.has_cli_credentials);
    let usage_result =
        account_service::fetch_usage(&ctx.usage_cache, account.uuid, false).await;

    if ctx.json {
        let mut j = serde_json::json!({
            "uuid": account.uuid.to_string(),
            "email": account.email,
            "org_uuid": account.org_uuid,
            "org_name": account.org_name,
            "subscription_type": account.subscription_type,
            "rate_limit_tier": account.rate_limit_tier,
            "created_at": account.created_at.to_rfc3339(),
            "cli_active": account.is_cli_active,
            "desktop_active": account.is_desktop_active,
            "token_status": health.status,
        });
        match &usage_result {
            Ok(Some(u)) => {
                if let Some(ref w) = u.five_hour {
                    j["five_hour"] =
                        serde_json::json!({"utilization": w.utilization, "resets_at": w.resets_at.to_rfc3339()});
                }
                if let Some(ref w) = u.seven_day {
                    j["seven_day"] =
                        serde_json::json!({"utilization": w.utilization, "resets_at": w.resets_at.to_rfc3339()});
                }
                if let Some(ref w) = u.seven_day_opus {
                    j["seven_day_opus"] =
                        serde_json::json!({"utilization": w.utilization, "resets_at": w.resets_at.to_rfc3339()});
                }
                if let Some(ref w) = u.seven_day_sonnet {
                    j["seven_day_sonnet"] =
                        serde_json::json!({"utilization": w.utilization, "resets_at": w.resets_at.to_rfc3339()});
                }
                if let Some(ref extra) = u.extra_usage {
                    j["extra_usage"] = serde_json::json!({
                        "is_enabled": extra.is_enabled,
                        "monthly_limit": extra.monthly_limit,
                        "used_credits": extra.used_credits,
                        "utilization": extra.utilization,
                    });
                }
            }
            Ok(None) => {}
            Err(e) => {
                j["usage_error"] = serde_json::json!(e.to_string());
            }
        }
        println!("{}", serde_json::to_string_pretty(&j)?);
    } else {
        println!("Account: {}", account.email);
        println!(
            "  Org:       {}",
            account.org_name.as_deref().unwrap_or("?")
        );
        println!(
            "  Org UUID:  {}",
            account.org_uuid.as_deref().unwrap_or("?")
        );
        println!(
            "  Plan:      {} {}",
            capitalize(account.subscription_type.as_deref().unwrap_or("?")),
            account
                .rate_limit_tier
                .as_deref()
                .and_then(|t| t.split('_').next_back())
                .unwrap_or("")
        );
        println!("  Token:     {}", health.status);
        println!(
            "  Added:     {}",
            account.created_at.format("%Y-%m-%d %H:%M")
        );
        println!(
            "  CLI:       {}",
            if account.is_cli_active {
                "active"
            } else {
                "—"
            }
        );
        println!(
            "  Desktop:   {}",
            if account.is_desktop_active {
                "active"
            } else if account.has_desktop_profile {
                "profile stored"
            } else {
                "—"
            }
        );

        match &usage_result {
            Ok(Some(u)) => {
                print_window("5h usage", &u.five_hour);
                print_window("7d usage", &u.seven_day);
                print_window("7d opus", &u.seven_day_opus);
                print_window("7d sonnet", &u.seven_day_sonnet);
                if let Some(ref extra) = u.extra_usage {
                    if extra.is_enabled {
                        let used = extra.used_credits.unwrap_or(0.0);
                        let limit = extra.monthly_limit.unwrap_or(0.0);
                        if limit > 0.0 {
                            println!("  Extra:     ${used:.2} / ${limit:.2} ({:.0}%)", (used / limit) * 100.0);
                        } else {
                            println!("  Extra:     enabled (${used:.2} used)");
                        }
                    } else {
                        println!("  Extra:     disabled");
                    }
                }
            }
            Ok(None) => {
                // No credentials — nothing to show
            }
            Err(UsageFetchError::Cooldown { remaining_secs }) => {
                println!("  Usage:     rate limited ({remaining_secs}s remaining)");
            }
            Err(UsageFetchError::TokenExpired) => {
                println!("  Usage:     token expired");
            }
            Err(e) => {
                println!("  Usage:     {e}");
            }
        }
    }

    Ok(())
}

fn print_window(
    label: &str,
    window: &Option<claudepot_core::oauth::usage::UsageWindow>,
) {
    if let Some(w) = window {
        let local = w.resets_at.with_timezone(&chrono::Local);
        println!(
            "  {:<9}  {:.0}% (resets {})",
            format!("{label}:"),
            w.utilization,
            format_local_time(&local)
        );
    }
}

/// Format a local datetime as "HH:MM" with a compact UTC offset.
/// Examples: "20:59 (+08)", "14:59 (-05)", "12:59 (UTC)"
fn format_local_time(dt: &chrono::DateTime<chrono::Local>) -> String {
    let offset_secs = dt.offset().local_minus_utc();
    if offset_secs == 0 {
        format!("{} (UTC)", dt.format("%H:%M"))
    } else {
        let hours = offset_secs / 3600;
        let mins = (offset_secs.abs() % 3600) / 60;
        if mins == 0 {
            format!("{} ({:+03})", dt.format("%H:%M"), hours)
        } else {
            format!("{} ({:+03}:{:02})", dt.format("%H:%M"), hours, mins)
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

/// Verify per-account blob identity against `/api/oauth/profile`.
///
/// Runs `services::identity::verify_account_identity` for each account
/// (or just `email_input` if given) and prints a table (or JSON) of
/// outcomes. Exit code is 2 on any drift, 3 on any rejection or
/// network_error, 0 only when every account returns Ok — so scripts can
/// distinguish "healthy" from "needs re-login" from "couldn't check".
pub async fn verify(ctx: &AppContext, email_input: Option<&str>) -> Result<()> {
    use claudepot_core::account::VerifyOutcome;
    use claudepot_core::cli_backend::swap::DefaultProfileFetcher;
    use claudepot_core::services::identity;

    let accounts = if let Some(email) = email_input {
        let resolved = claudepot_core::resolve::resolve_email(&ctx.store, email)?;
        vec![ctx
            .store
            .find_by_email(&resolved)?
            .expect("resolved email not in store")]
    } else {
        ctx.store.list()?
    };

    let fetcher = DefaultProfileFetcher;
    let mut drift = false;
    let mut rejected = false;
    let mut net = false;
    let mut rows: Vec<(String, String, String, Option<String>)> = Vec::new();

    for account in &accounts {
        if !account.has_cli_credentials {
            rows.push((
                account.email.clone(),
                account.uuid.to_string(),
                "no_creds".to_string(),
                None,
            ));
            continue;
        }
        let outcome = identity::verify_account_identity(&ctx.store, account.uuid, &fetcher).await;
        let (status, actual) = match outcome {
            Ok(VerifyOutcome::Ok { email }) => ("ok".to_string(), Some(email)),
            Ok(VerifyOutcome::Drift { actual_email, .. }) => {
                drift = true;
                ("drift".to_string(), Some(actual_email))
            }
            Ok(VerifyOutcome::Rejected) => {
                rejected = true;
                ("rejected".to_string(), None)
            }
            Ok(VerifyOutcome::NetworkError) => {
                net = true;
                ("network_error".to_string(), None)
            }
            Err(e) => {
                net = true;
                (format!("error: {e}"), None)
            }
        };
        rows.push((account.email.clone(), account.uuid.to_string(), status, actual));
    }

    if ctx.json {
        let json: Vec<_> = rows
            .iter()
            .map(|(email, uuid, status, actual)| {
                serde_json::json!({
                    "email": email,
                    "uuid": uuid,
                    "status": status,
                    "actual_email": actual,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!("{:<32} {:<8} {}", "ACCOUNT", "STATUS", "DETAIL");
        for (email, _uuid, status, actual) in &rows {
            let detail = match (status.as_str(), actual) {
                ("drift", Some(a)) => format!("authenticates as {a}"),
                ("ok", Some(a)) => format!("verified as {a}"),
                ("rejected", _) => "token revoked — re-login required".to_string(),
                ("network_error", _) => "could not reach /profile".to_string(),
                ("no_creds", _) => "no credentials stored".to_string(),
                _ => String::new(),
            };
            println!("{email:<32} {status:<8} {detail}");
        }
    }

    // Exit code contract:
    //   0 = all ok, 2 = drift, 3 = rejected or network error
    if drift {
        std::process::exit(2);
    }
    if rejected || net {
        std::process::exit(3);
    }
    Ok(())
}
