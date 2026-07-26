//! `wake` verb — start an account's rate-limit windows so their reset
//! times become reportable.
//!
//! Sub-module of `commands/account.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

/// Send the minimal billable request for one account.
///
/// The whole point is spending quota, so this verb is explicit by
/// design: it names the cost before acting, and `--yes` is the only way
/// to skip the prompt. There is no "wake all" — waking every idle
/// account at once is the automatic behavior we deliberately did not
/// build (see `claudepot_core::oauth::wake`'s module docs).
pub async fn wake(ctx: &AppContext, email_input: &str) -> Result<()> {
    use claudepot_core::oauth::wake::ESTIMATED_TOKENS;
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::wake_service;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;
    let account = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    // Check eligibility *before* prompting: asking the user to approve a
    // spend we would then refuse wastes their decision, and the refusal
    // reason ("run verify") is the actionable message.
    wake_service::eligibility(
        account.has_cli_credentials,
        &account.verify_status,
        &account.email,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    if !ctx.yes {
        eprint!(
            "Send a minimal billable request as \"{email}\"? \
             Spends ~{ESTIMATED_TOKENS} tokens of that account's quota. [y/N] "
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Cancelled.");
            return Ok(());
        }
    }

    let receipt = wake_service::wake_account(&ctx.store, account.uuid)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if ctx.json {
        return output::print_json(&serde_json::json!({
            "email": receipt.email,
            "input_tokens": receipt.cost.input_tokens,
            "output_tokens": receipt.cost.output_tokens,
            "model": receipt.model,
        }));
    }

    println!(
        "Woke {} — spent {} input + {} output token(s) on {}.",
        receipt.email, receipt.cost.input_tokens, receipt.cost.output_tokens, receipt.model
    );
    // Not a hedge — measured. Reset times were absent immediately after
    // the call and present by t+20s, so a user who re-runs `account
    // list` right now would think nothing happened.
    ctx.info("Reset times appear within ~30s, once the usage endpoint catches up.");
    Ok(())
}
