//! `reconcile` verb — align `has_desktop_profile` flags with on-disk
//! truth and clear orphan `state.active_desktop` pointers.
//!
//! Sub-module of `commands/desktop_ops.rs`; see that file's header
//! for the per-verb layout rationale.

use super::*;

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
