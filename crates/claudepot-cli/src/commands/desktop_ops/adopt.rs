//! `adopt` verb — capture the live Desktop session into an account's
//! snapshot dir (identity-verified).
//!
//! Sub-module of `commands/desktop_ops.rs`; see that file's header
//! for the per-verb layout rationale.

use super::*;

pub async fn adopt(ctx: &AppContext, email_input: Option<&str>, overwrite: bool) -> Result<()> {
    use claudepot_core::desktop_backend;
    use claudepot_core::desktop_identity::verify_live_identity;
    use claudepot_core::resolve::resolve_email;
    use claudepot_core::services::desktop_service;

    let platform = desktop_backend::create_platform()
        .ok_or_else(|| anyhow::anyhow!("Desktop not supported on this platform"))?;

    // Resolve the target account. If --email wasn't given, use the
    // live /profile email as the target — the common case is "adopt
    // whoever Desktop is signed in as into the matching Claudepot
    // account."
    let verified = verify_live_identity(&*platform, &ctx.store)
        .await
        .map_err(|e| anyhow::anyhow!("identity probe failed: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("no live Desktop identity — sign in first"))?;

    let target_email = match email_input {
        Some(e) => resolve_email(&ctx.store, e).map_err(|e| anyhow::anyhow!("{e}"))?,
        None => verified.email().to_string(),
    };
    let target = ctx
        .store
        .find_by_email(&target_email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {target_email}"))?;

    let outcome =
        desktop_service::adopt_current(&*platform, &ctx.store, target.uuid, &verified, overwrite)
            .await
            .map_err(|e| anyhow::anyhow!("adopt failed: {e}"))?;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "email": outcome.account_email,
                "captured_items": outcome.captured_items,
                "size_bytes": outcome.size_bytes,
            })
        );
    } else {
        println!(
            "Adopted live Desktop session for {}: {} item(s), {} bytes.",
            outcome.account_email, outcome.captured_items, outcome.size_bytes
        );
    }
    Ok(())
}
