//! `identity` verb — probe the live Desktop session identity.
//!
//! Sub-module of `commands/desktop_ops.rs`; see that file's header
//! for the per-verb layout rationale.

use super::*;

/// Probe the live Desktop session identity.
///
/// Fast path (default): org-UUID candidate match against the live
/// `config.json`. Slow path (`--strict`): decrypts `oauth:tokenCache`
/// via the OS keychain + calls `/api/oauth/profile` for an
/// authoritative identity.
pub async fn identity(ctx: &AppContext, strict: bool) -> Result<()> {
    use claudepot_core::desktop_backend;
    use claudepot_core::desktop_identity::{
        probe_live_identity, probe_live_identity_async, DefaultProfileFetcher, ProbeMethod,
        ProbeOptions,
    };

    let Some(platform) = desktop_backend::create_platform() else {
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({
                    "email": null,
                    "org_uuid": null,
                    "probe_method": "none",
                    "error": "Desktop not supported on this platform",
                })
            );
        } else {
            println!("Claude Desktop is not supported on this platform.");
        }
        return Ok(());
    };

    let opts = ProbeOptions { strict };
    let result = if strict {
        let fetcher = DefaultProfileFetcher;
        probe_live_identity_async(&*platform, &ctx.store, opts, &fetcher).await
    } else {
        probe_live_identity(&*platform, &ctx.store, opts)
    };
    match result {
        Ok(None) => {
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "email": null,
                        "org_uuid": null,
                        "probe_method": "none",
                        "error": null,
                    })
                );
            } else {
                println!("No identifiable Desktop identity.");
                println!("  Desktop appears signed in, but no registered account matches the live org UUID (or matching is ambiguous). Sign in as a registered account, or add the current account via `claudepot account add`.");
            }
        }
        Ok(Some(id)) => {
            let method = match id.probe_method {
                ProbeMethod::OrgUuidCandidate => "org_uuid_candidate",
                ProbeMethod::Decrypted => "decrypted",
            };
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "email": id.email,
                        "org_uuid": id.org_uuid,
                        "probe_method": method,
                        "error": null,
                    })
                );
            } else {
                println!("Desktop identity: {}", id.email);
                println!("  org_uuid:     {}", id.org_uuid);
                println!("  probe_method: {method}");
                if matches!(id.probe_method, ProbeMethod::OrgUuidCandidate) {
                    println!(
                        "  note:         candidate match (org UUID only) — NOT verified. \
                         Phase 2 will add a `--strict` decrypted probe."
                    );
                }
            }
        }
        Err(e) => {
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "email": null,
                        "org_uuid": null,
                        "probe_method": "none",
                        "error": e.to_string(),
                    })
                );
            } else {
                println!("Desktop identity probe: {e}");
            }
        }
    }

    Ok(())
}
