//! `forget` verb — wipe Shared Memory rows (destructive; `--yes`).
//!
//! Sub-module of `commands/codex.rs`; see that file's header for the
//! per-verb layout rationale.

use super::*;

/// Wipe all Shared Memory rows (transcript-derived + durable).
/// Requires `--yes`. Without confirmation, prints what would be
/// removed and exits non-zero so scripts can distinguish "needs
/// confirmation" from "did the work."
pub async fn forget(db: Option<PathBuf>, confirm: bool) -> Result<()> {
    use claudepot_core::shared_memory::indexer::{count_shared_memory_rows, forget_shared_memory};

    let db_path = db.unwrap_or_else(default_db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "sessions.db not found at {} — nothing to forget",
            db_path.display()
        );
    }

    if !confirm {
        let dp = db_path.clone();
        let counts = tokio::task::spawn_blocking(move || count_shared_memory_rows(&dp))
            .await
            .with_context(|| "join count task")?
            .with_context(|| "count shared memory rows")?;
        println!("Refusing to forget without --yes.");
        println!(
            "If you proceed, the following will be removed from {}:",
            db_path.display()
        );
        println!("  exchanges:           {}", counts.exchanges);
        println!("  tool_calls:          {}", counts.tool_calls);
        println!("  exchange_fts rows:   {}", counts.exchange_fts);
        println!("  memories:            {}", counts.memories);
        println!("  decisions:           {}", counts.decisions);
        println!("  evidence_records:    {}", counts.evidence_records);
        println!("  memory_links:        {}", counts.memory_links);
        println!();
        println!("The v4 schema and `sessions` rows are preserved.");
        println!("Re-run with `--yes` to confirm.");
        anyhow::bail!("confirmation required");
    }

    let dp = db_path.clone();
    let counts = tokio::task::spawn_blocking(move || forget_shared_memory(&dp))
        .await
        .with_context(|| "join forget task")?
        .with_context(|| "forget shared memory")?;

    println!("Wiped Shared Memory rows from {}.", db_path.display());
    println!("  exchanges:          {} removed", counts.exchanges);
    println!("  tool_calls:         {} removed", counts.tool_calls);
    println!("  memories:           {} removed", counts.memories);
    println!("  decisions:          {} removed", counts.decisions);
    println!("  evidence_records:   {} removed", counts.evidence_records);
    println!("  memory_links:       {} removed", counts.memory_links);
    println!();
    println!("Next `claudepot codex index` will repopulate transcript-derived");
    println!("rows from disk. Durable rows must be re-created manually.");
    Ok(())
}
