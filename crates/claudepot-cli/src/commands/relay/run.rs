use std::ffi::OsString;

use anyhow::Result;
use claudepot_core::relay::client_ops::{self, ClientCtx, ClientOpError};

use crate::AppContext;

/// Pick a server, then hand the terminal to Claude Code and exit with
/// its code.
pub async fn run_cmd(
    ctx: &AppContext,
    server: Option<&str>,
    upstream: Option<&str>,
    args: Vec<OsString>,
) -> Result<()> {
    let ops = ClientCtx::production();
    let model = client_ops::model_hint(&args);
    let plan =
        match client_ops::plan_run(&ops, server, upstream, model.as_deref(), chrono::Utc::now())
            .await
        {
            Ok(p) => p,
            Err(e @ ClientOpError::NoEligible { .. }) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
            Err(e) => return Err(e.into()),
        };
    for (a, b) in &plan.selection.duplicate_accounts {
        ctx.info(&format!(
            "note: {a} and {b} serve the same account — they share its rate limits"
        ));
    }
    ctx.info(&format!(
        "relay: {} via {} ({})",
        plan.upstream.as_str(),
        plan.pick.name,
        plan.pick.tier
    ));
    let code = client_ops::run(&plan, &args).await?;
    // Dropping the plan deletes the CA bundle; exit() would skip that.
    drop(plan);
    std::process::exit(code);
}
