//! `inspect` verb — detailed account view incl. token health + usage.
//!
//! Sub-module of `commands/account.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

pub async fn inspect(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::{account_service, usage_cache::UsageFetchError};

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    let health = account_service::token_health(account.uuid, account.has_cli_credentials).await;
    let usage_result = account_service::fetch_usage(&ctx.usage_cache, account.uuid, false).await;

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
                // Helper closure: the server may return `resets_at: null`
                // for a window with no reset yet — preserve that shape
                // downstream instead of letting serde_json panic on None.
                let win_json = |w: &claudepot_core::oauth::usage::UsageWindow| {
                    serde_json::json!({
                        "utilization": w.utilization,
                        "resets_at": w.resets_at.as_ref().map(|t| t.to_rfc3339()),
                    })
                };
                if let Some(ref w) = u.five_hour {
                    j["five_hour"] = win_json(w);
                }
                if let Some(ref w) = u.seven_day {
                    j["seven_day"] = win_json(w);
                }
                if let Some(ref w) = u.seven_day_opus {
                    j["seven_day_opus"] = win_json(w);
                }
                if let Some(ref w) = u.seven_day_sonnet {
                    j["seven_day_sonnet"] = win_json(w);
                }
                if let Some(ref extra) = u.extra_usage {
                    // monthly_limit / used_credits are passed through
                    // in MINOR units (pence/cents) to match the raw
                    // API. Consumers of --json that render amounts
                    // must divide by 100 and pair with `currency`.
                    j["extra_usage"] = serde_json::json!({
                        "is_enabled": extra.is_enabled,
                        "monthly_limit": extra.monthly_limit,
                        "used_credits": extra.used_credits,
                        "utilization": extra.utilization,
                        "currency": extra.currency,
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
            crate::time_fmt::format_local_datetime(
                &account.created_at.with_timezone(&chrono::Local)
            )
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
                        // API returns amounts in MINOR units (pence/cents).
                        let used = extra.used_credits.unwrap_or(0.0) / 100.0;
                        let limit = extra.monthly_limit.unwrap_or(0.0) / 100.0;
                        let sym = currency_symbol(extra.currency.as_deref());
                        if limit > 0.0 {
                            let pct = extra.utilization.unwrap_or_else(|| (used / limit) * 100.0);
                            let balance = (limit - used).max(0.0);
                            println!(
                                "  Extra:     {pct:.0}% used · {sym}{used:.2} / {sym}{limit:.2} · {sym}{balance:.2} left"
                            );
                        } else {
                            println!("  Extra:     enabled ({sym}{used:.2} used)");
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

fn print_window(label: &str, window: &Option<claudepot_core::oauth::usage::UsageWindow>) {
    if let Some(w) = window {
        match &w.resets_at {
            Some(ts) => {
                let local = ts.with_timezone(&chrono::Local);
                println!(
                    "  {:<9}  {:.0}% (resets {})",
                    format!("{label}:"),
                    w.utilization,
                    crate::time_fmt::format_local_time_of_day(&local)
                );
            }
            None => {
                println!("  {:<9}  {:.0}%", format!("{label}:"), w.utilization);
            }
        }
    }
}

/// Map an ISO 4217 currency code to its common symbol for terminal
/// display. Unknown / missing codes fall through to "$" (matches the
/// Anthropic console default for USD-billed accounts).
fn currency_symbol(code: Option<&str>) -> &'static str {
    match code.map(str::to_ascii_uppercase).as_deref() {
        Some("GBP") => "£",
        Some("EUR") => "€",
        Some("JPY") => "¥",
        Some("CNY") => "¥",
        Some("AUD") | Some("CAD") | Some("USD") | None => "$",
        _ => "$",
    }
}
