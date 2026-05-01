use crate::AppContext;
use anyhow::Result;
use claudepot_core::updates::desktop_driver::install_desktop_latest;

pub async fn run(ctx: &AppContext) -> Result<()> {
    ctx.info("Installing Claude Desktop update...");
    let outcome = install_desktop_latest()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "method": outcome.method,
                "version_after": outcome.version_after,
                "stdout": outcome.stdout,
                "stderr": outcome.stderr,
            })
        );
    } else {
        if !outcome.stdout.trim().is_empty() {
            println!("{}", outcome.stdout.trim_end());
        }
        if !outcome.stderr.trim().is_empty() {
            eprintln!("{}", outcome.stderr.trim_end());
        }
        if let Some(v) = outcome.version_after {
            println!("✓ Installed: Claude.app {v} (via {})", outcome.method);
        }
    }
    Ok(())
}
