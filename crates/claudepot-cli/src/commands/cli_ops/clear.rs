//! `clear` verb — clear CC credentials (log out).
//!
//! Sub-module of `commands/cli_ops.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

pub async fn clear(ctx: &AppContext) -> Result<()> {
    use claudepot_core::cli_backend::swap::DefaultProfileFetcher;
    use claudepot_core::services::cli_service;

    let fetcher = DefaultProfileFetcher;
    cli_service::clear_credentials(&ctx.store, &fetcher).await?;

    if ctx.json {
        println!("{}", serde_json::json!({"cleared": true}));
    } else {
        println!("CC credentials cleared.");
    }

    Ok(())
}
