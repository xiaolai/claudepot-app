//! `verify` verb — per-account blob identity check against /profile.
//!
//! Sub-module of `commands/account.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

/// Verify per-account blob identity against `/api/oauth/profile`.
///
/// Runs `services::identity::verify_account_identity` for each account
/// (or just `email_input` if given) and prints a table (or JSON) of
/// outcomes. Exit code is 2 on any drift, 3 on any rejection,
/// signed-out slot or network_error, 0 only when every account returns
/// Ok — so scripts can distinguish "healthy" from "needs re-login" from
/// "couldn't check".
use claudepot_core::account::VerifyOutcome;

pub async fn verify(ctx: &AppContext, email_input: Option<&str>) -> Result<()> {
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
    // `status_of` / `detail_for` / `exit_code_for` below are the pure
    // seam this loop drives. They exist because everything a script
    // consumes here — the status token, the human detail, the exit code
    // — used to be inline in an `async fn` that ends in
    // `std::process::exit`, which no test can call.

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
            Ok(o) => {
                match &o {
                    VerifyOutcome::Drift { .. } => drift = true,
                    // Both terminal, so both feed the same exit-code
                    // flag — a script branching on "needs re-login"
                    // must see either. Only the STATUS token stays
                    // distinct, because that is what a human reads and
                    // "rejected" would misstate a self-inflicted
                    // sign-out.
                    VerifyOutcome::Rejected | VerifyOutcome::SignedOut => rejected = true,
                    VerifyOutcome::NetworkError => net = true,
                    VerifyOutcome::Ok { .. } => {}
                }
                let actual = match &o {
                    VerifyOutcome::Ok { email } => Some(email.clone()),
                    VerifyOutcome::Drift { actual_email, .. } => Some(actual_email.clone()),
                    _ => None,
                };
                (status_of(&o).to_string(), actual)
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
        // Width 13 fits the longest status ("network_error"); at the
        // old 8 both that and "signed_out" overflowed the column and
        // pushed DETAIL out of alignment on exactly the rows a user is
        // reading closely.
        println!("{:<32} {:<13} DETAIL", "ACCOUNT", "STATUS");
        for (email, _uuid, status, actual) in &rows {
            let detail = detail_for(status, actual.as_deref());
            println!("{email:<32} {status:<13} {detail}");
        }
    }

    // Exit-code contract (documented in `.claude/rules/commands.md`):
    //   0 = every account returned Ok
    //   2 = at least one drift (slot misfiled)
    //   3 = at least one rejected OR signed_out OR network_error OR
    //       un-checkable slot
    //
    // 3 dominates 2: the "we couldn't confirm" condition is strictly
    // worse than "we confirmed something is wrong", because scripts
    // that branch on 2 to auto-remediate drift need to know they got
    // a complete picture first. `no_creds` rows also count toward 3
    // — they weren't checked, so the command cannot honestly report
    // "all ok".
    let no_creds = rows.iter().any(|(_, _, status, _)| status == "no_creds");
    match exit_code_for(drift, rejected, net, no_creds) {
        0 => Ok(()),
        code => std::process::exit(code),
    }
}

/// The status token a `VerifyOutcome` prints, and the one `--json`
/// emits. Deliberately mirrors `VerifyOutcome::as_str` rather than
/// wrapping it, because these strings are a **script-facing contract**:
/// a rename in core must not silently retitle a column that users grep.
/// The test below asserts the two agree, so a divergence is loud.
fn status_of(outcome: &VerifyOutcome) -> &'static str {
    match outcome {
        VerifyOutcome::Ok { .. } => "ok",
        VerifyOutcome::Drift { .. } => "drift",
        VerifyOutcome::Rejected => "rejected",
        VerifyOutcome::SignedOut => "signed_out",
        VerifyOutcome::NetworkError => "network_error",
    }
}

/// Human-readable DETAIL column for a status row.
fn detail_for(status: &str, actual: Option<&str>) -> String {
    match (status, actual) {
        ("drift", Some(a)) => format!("authenticates as {a}"),
        ("ok", Some(a)) => format!("verified as {a}"),
        ("rejected", _) => "token revoked — re-login required".to_string(),
        ("signed_out", _) => "Claude Code cleared its credentials — re-login required".to_string(),
        ("network_error", _) => "could not reach /profile".to_string(),
        ("no_creds", _) => "no credentials stored".to_string(),
        _ => String::new(),
    }
}

/// The documented exit-code contract, as a pure function.
///
/// 3 dominates 2 on purpose: "we could not confirm" is strictly worse
/// than "we confirmed something is wrong", because a script that
/// branches on 2 to auto-remediate drift needs to know it got a
/// complete picture first.
fn exit_code_for(drift: bool, rejected: bool, net: bool, no_creds: bool) -> i32 {
    if rejected || net || no_creds {
        3
    } else if drift {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status tokens are what `--json` consumers key on. Locking
    /// them to core's `as_str` here means a rename in core breaks this
    /// test instead of silently retitling a script's input.
    #[test]
    fn status_tokens_match_core() {
        for outcome in [
            VerifyOutcome::Ok {
                email: "a@b.c".into(),
            },
            VerifyOutcome::Drift {
                stored_email: "a@b.c".into(),
                actual_email: "d@e.f".into(),
            },
            VerifyOutcome::Rejected,
            VerifyOutcome::SignedOut,
            VerifyOutcome::NetworkError,
        ] {
            assert_eq!(status_of(&outcome), outcome.as_str());
        }
    }

    #[test]
    fn signed_out_prints_its_own_cause_not_a_rejection() {
        let detail = detail_for("signed_out", None);
        assert!(detail.contains("re-login"), "must name the remedy");
        assert!(
            !detail.contains("revoked"),
            "nothing was revoked — that is the `rejected` story: {detail}"
        );
        assert_ne!(detail, detail_for("rejected", None));
    }

    /// Both terminal statuses must reach exit 3. A signed-out account
    /// that exited 0 would tell every CI script the fleet is healthy.
    #[test]
    fn terminal_statuses_exit_three() {
        assert_eq!(exit_code_for(false, true, false, false), 3);
        assert_eq!(exit_code_for(false, false, true, false), 3);
        assert_eq!(exit_code_for(false, false, false, true), 3);
    }

    #[test]
    fn drift_alone_exits_two_and_a_clean_run_exits_zero() {
        assert_eq!(exit_code_for(true, false, false, false), 2);
        assert_eq!(exit_code_for(false, false, false, false), 0);
    }

    /// 3 dominates 2 — asserted rather than assumed, since the ordering
    /// of the `if`s is the only thing that encodes it.
    #[test]
    fn incomplete_information_outranks_confirmed_drift() {
        assert_eq!(exit_code_for(true, true, false, false), 3);
        assert_eq!(exit_code_for(true, false, false, true), 3);
    }
}
