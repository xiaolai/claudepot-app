//! `undo` verb — reverse the most recent import within the 24h
//! undo window.
//!
//! Sub-module of `commands/project_migrate.rs`; see that file's
//! header for the per-verb layout rationale and shared helpers.

use super::*;

/// `project migrate undo` — reverse the most recent import within
/// the 24h undo window. LIFO journal replay; per-step tamper detection.
pub fn undo(ctx: &AppContext) -> Result<()> {
    let receipt = migrate::import_undo().map_err(map_migrate_err)?;
    if ctx.json {
        let v = serde_json::json!({
            "bundle_id": receipt.bundle_id,
            "journal_path": receipt.journal_path.to_string_lossy(),
            "counter_journal_path": receipt.counter_journal_path.to_string_lossy(),
            "steps_reversed": receipt.steps_reversed,
            "steps_tampered": receipt.steps_tampered,
            "steps_errored": receipt.steps_errored,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("Undo of import {}:", receipt.bundle_id);
        println!("  steps reversed:  {}", receipt.steps_reversed);
        if !receipt.steps_tampered.is_empty() {
            println!("  skipped (post-apply tamper):");
            for s in &receipt.steps_tampered {
                println!("    - {s}");
            }
        }
        if !receipt.steps_errored.is_empty() {
            println!("  errors:");
            for s in &receipt.steps_errored {
                println!("    - {s}");
            }
        }
        println!(
            "  counter-journal: {}",
            receipt.counter_journal_path.display()
        );
    }
    Ok(())
}
