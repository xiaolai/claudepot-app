use crate::AppContext;
use anyhow::Result;

pub async fn status(ctx: &AppContext) -> Result<()> {
    use claudepot_core::desktop_backend;

    let platform = desktop_backend::create_platform();
    let platform = match platform {
        Some(p) => p,
        None => {
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({"error": "Desktop not supported on this platform"})
                );
            } else {
                println!("Claude Desktop is not supported on this platform.");
            }
            return Ok(());
        }
    };

    let data_dir = platform.data_dir();
    let installed = data_dir.as_ref().is_some_and(|d| d.exists());

    let active_uuid = ctx.store.active_desktop_uuid()?;
    let active_account = active_uuid
        .and_then(|u| u.parse::<uuid::Uuid>().ok())
        .and_then(|u| ctx.store.find_by_uuid(u).ok().flatten());

    let is_running = platform.is_running().await;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "installed": installed,
                "running": is_running,
                "active": active_account.as_ref().map(|a| &a.email),
            })
        );
    } else {
        if !installed {
            println!("Claude Desktop is not installed.");
            return Ok(());
        }
        match &active_account {
            Some(a) => println!("Active Desktop account: {}", a.email),
            None => println!("No active Desktop account."),
        }
        println!(
            "  Desktop: {}",
            if is_running { "running" } else { "not running" }
        );
    }

    Ok(())
}

pub async fn use_account(ctx: &AppContext, email_input: &str, no_launch: bool) -> Result<()> {
    use claudepot_core::desktop_backend;
    use claudepot_core::resolve::resolve_email;

    let platform = desktop_backend::create_platform()
        .ok_or_else(|| anyhow::anyhow!("Claude Desktop is not supported on this platform"))?;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let target = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    // Check if target has a Desktop profile
    let profile_dir = claudepot_core::paths::desktop_profile_dir(target.uuid);
    if !profile_dir.exists() {
        anyhow::bail!(
            "no Desktop profile stored for {email}. \
             Sign in to Claude Desktop as this account first, then use \
             `claudepot desktop use` to switch."
        );
    }

    let current_uuid = ctx
        .store
        .active_desktop_uuid()?
        .and_then(|s| s.parse::<uuid::Uuid>().ok());

    if current_uuid == Some(target.uuid) {
        // Audit M2: in --json mode, emit a structured payload so
        // scripted callers can distinguish "already active" from an
        // empty/failed command. Previously `ctx.info` printed to
        // stderr and the command returned no stdout at all under
        // --json, which is indistinguishable from a crash to anything
        // parsing stdout as JSON.
        if ctx.json {
            println!(
                "{}",
                serde_json::json!({
                    "already_active": true,
                    "email": email,
                })
            );
        } else {
            ctx.info(&format!("Already active: {email}"));
        }
        return Ok(());
    }

    let from_email = current_uuid
        .and_then(|u| ctx.store.find_by_uuid(u).ok().flatten())
        .map(|a| a.email)
        .unwrap_or_else(|| "(none)".to_string());

    ctx.info(&format!("Switching Desktop: {from_email} → {email}"));

    desktop_backend::swap::switch(
        platform.as_ref(),
        &ctx.store,
        current_uuid,
        target.uuid,
        no_launch,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "from": from_email,
                "to": email,
                "launched": !no_launch,
            })
        );
    } else {
        println!("Desktop: {from_email} → {email}");
        if no_launch {
            println!("Desktop was not relaunched (--no-launch).");
        }
    }

    Ok(())
}

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

/// Reconcile `has_desktop_profile` flags with on-disk truth and
/// clear orphan `state.active_desktop` pointers.
pub async fn reconcile(ctx: &AppContext) -> Result<()> {
    use claudepot_core::services::desktop_service;

    let outcome = desktop_service::reconcile_flags(&ctx.store)
        .map_err(|e| anyhow::anyhow!("reconcile failed: {e}"))?;

    if ctx.json {
        let flips: Vec<_> = outcome
            .flag_flips
            .iter()
            .map(|f| {
                serde_json::json!({
                    "email": f.email,
                    "uuid": f.uuid.to_string(),
                    "new_value": f.new_value,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "flag_flips": flips,
                "orphan_pointer_cleared": outcome.orphan_pointer_cleared,
            })
        );
    } else if outcome.flag_flips.is_empty() && !outcome.orphan_pointer_cleared {
        println!("Desktop reconcile: nothing to do.");
    } else {
        if !outcome.flag_flips.is_empty() {
            println!(
                "Reconciled {} Desktop profile flag(s):",
                outcome.flag_flips.len()
            );
            for f in &outcome.flag_flips {
                let arrow = if f.new_value {
                    "set to true (profile dir found)"
                } else {
                    "set to false (profile dir missing)"
                };
                println!("  {} — {arrow}", f.email);
            }
        }
        if outcome.orphan_pointer_cleared {
            println!("Cleared orphan `active_desktop` pointer.");
        }
    }

    Ok(())
}
