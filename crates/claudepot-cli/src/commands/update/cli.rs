use crate::AppContext;
use anyhow::Result;
use claudepot_core::updates::cli_driver::run_claude_update;

pub async fn run(ctx: &AppContext) -> Result<()> {
    ctx.info("Running `claude update`...");
    let outcome = run_claude_update()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "stdout": outcome.stdout,
                "stderr": outcome.stderr,
                "installed_after": outcome.installed_after,
            })
        );
    } else {
        if !outcome.stdout.trim().is_empty() {
            println!("{}", outcome.stdout.trim_end());
        }
        if !outcome.stderr.trim().is_empty() {
            eprintln!("{}", outcome.stderr.trim_end());
        }
        if let Some(v) = outcome.installed_after {
            println!("✓ Active install: {v}");
        }
    }
    Ok(())
}
