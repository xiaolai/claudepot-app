//! `verify` verb — per-account blob identity check against /profile.
//!
//! Sub-module of `commands/account.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

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
        // Audit Low: previously `.expect("resolved email not in store")`.
        // A concurrent `account remove` between resolve and lookup
        // turns a normal user error into a process panic. Convert to
        // a regular error like every other lookup path.
        let acct = ctx.store.find_by_email(&resolved)?.ok_or_else(|| {
            anyhow::anyhow!("resolved email '{resolved}' not found (removed concurrently?)")
        })?;
        vec![acct]
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
        rows.push((
            account.email.clone(),
            account.uuid.to_string(),
            status,
            actual,
        ));
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
        println!("{:<32} {:<8} DETAIL", "ACCOUNT", "STATUS");
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

    // Exit-code contract (documented in `.claude/rules/commands.md`):
    //   0 = every account returned Ok
    //   2 = at least one drift (slot misfiled)
    //   3 = at least one rejected OR network_error OR un-checkable slot
    //
    // 3 dominates 2: the "we couldn't confirm" condition is strictly
    // worse than "we confirmed something is wrong", because scripts
    // that branch on 2 to auto-remediate drift need to know they got
    // a complete picture first. `no_creds` rows also count toward 3
    // — they weren't checked, so the command cannot honestly report
    // "all ok".
    let no_creds = rows.iter().any(|(_, _, status, _)| status == "no_creds");
    if rejected || net || no_creds {
        std::process::exit(3);
    }
    if drift {
        std::process::exit(2);
    }
    Ok(())
}
