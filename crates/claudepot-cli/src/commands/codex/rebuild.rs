//! `rebuild` verb — set the `_pending_rescan` marker on sessions.db.
//!
//! Sub-module of `commands/codex.rs`; see that file's header for the
//! per-verb layout rationale.

use super::*;

/// Set the `_pending_rescan` marker on sessions.db so the next
/// `SessionIndex::open` clears the transcript-derived cache
/// atomically inside the migration transaction. Durable rows
/// survive. The user typically follows with `claudepot codex
/// index` which drives the rescan deliberately.
pub async fn rebuild(db: Option<PathBuf>, json: bool) -> Result<()> {
    let db_path = db.unwrap_or_else(default_db_path);
    if !db_path.exists() {
        anyhow::bail!(
            "sessions.db not found at {} — nothing to rebuild",
            db_path.display()
        );
    }
    let db_for_task = db_path.clone();
    tokio::task::spawn_blocking(move || {
        claudepot_core::shared_memory::indexer::mark_pending_rescan(&db_for_task)
    })
    .await
    .with_context(|| "join rebuild task")?
    .with_context(|| format!("mark _pending_rescan on {}", db_path.display()))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "ok",
                "db": db_path.display().to_string(),
            }))?
        );
    } else {
        println!(
            "Marker set on {}. Next `claudepot codex index` (or any open) \
             will rebuild transcript-derived rows.",
            db_path.display()
        );
    }
    Ok(())
}
