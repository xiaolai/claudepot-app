//! `claudepot codex index` — populate `sessions.db` with Codex
//! rollouts so the cross-harness MCP memory tools can surface them
//! (H4 of the grill fixing plan).
//!
//! Without this command, `backfill_codex` had no production caller
//! and `claudepot_search_memory` would silently return empty
//! results in production.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::indexer::{backfill_codex, CodexIndexerStats};

/// Default sessions.db path: `~/.claudepot/sessions.db`.
fn default_db_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claudepot")
        .join("sessions.db")
}

/// Default Codex sessions root. Honors `$CODEX_HOME` if set; falls
/// back to `~/.codex/sessions/`. Matches Codex CLI's own resolution
/// rules.
fn default_codex_sessions_root() -> PathBuf {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return PathBuf::from(home).join("sessions");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("sessions")
}

#[derive(Debug, Serialize)]
struct JsonReport {
    discovered: usize,
    indexed: usize,
    skipped_unchanged: usize,
    deleted: usize,
    failed_count: usize,
    failed: Vec<JsonFailure>,
    codex_sessions_root: PathBuf,
    db: PathBuf,
}

#[derive(Debug, Serialize)]
struct JsonFailure {
    path: PathBuf,
    error: String,
}

/// Set the `_pending_rescan` marker on sessions.db so the next
/// `SessionIndex::open` clears the transcript-derived cache
/// atomically inside the migration transaction. Durable rows
/// survive. The user typically follows with `claudepot codex
/// index` which drives the rescan deliberately.
pub async fn rebuild(db: Option<PathBuf>) -> Result<()> {
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
    println!(
        "Marker set on {}. Next `claudepot codex index` (or any open) \
         will rebuild transcript-derived rows.",
        db_path.display()
    );
    Ok(())
}

/// Wipe all Shared Memory rows (transcript-derived + durable).
/// Requires `--yes`. Without confirmation, prints what would be
/// removed and exits non-zero so scripts can distinguish "needs
/// confirmation" from "did the work."
pub async fn forget(db: Option<PathBuf>, confirm: bool) -> Result<()> {
    use claudepot_core::shared_memory::indexer::{
        count_shared_memory_rows, forget_shared_memory,
    };

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
        println!("If you proceed, the following will be removed from {}:", db_path.display());
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

pub async fn index(
    codex_home: Option<PathBuf>,
    db: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    // The CLI flag is named `--codex-home` and the docstring on
    // main.rs::CodexAction::Index also describes it that way.
    // Honor that contract: append `sessions/` ourselves so users
    // can pass their literal `$CODEX_HOME` value. If they're
    // setting up a custom layout (rare), the env-var path of
    // `default_codex_sessions_root` still appends `sessions`
    // either way.
    let codex_sessions_root = match codex_home {
        Some(home) => home.join("sessions"),
        None => default_codex_sessions_root(),
    };
    let db_path = db.unwrap_or_else(default_db_path);

    let idx = SessionIndex::open(&db_path)
        .with_context(|| format!("open sessions.db at {}", db_path.display()))?;

    // Run inside a blocking task — `backfill_codex` is synchronous
    // and may walk thousands of files.
    let codex_root_clone = codex_sessions_root.clone();
    let stats: CodexIndexerStats = tokio::task::spawn_blocking(move || {
        backfill_codex(&idx, &codex_root_clone)
    })
    .await
    .with_context(|| "join indexer task")?
    .with_context(|| "backfill_codex")?;

    if json {
        let report = JsonReport {
            discovered: stats.discovered,
            indexed: stats.indexed,
            skipped_unchanged: stats.skipped_unchanged,
            deleted: stats.deleted,
            failed_count: stats.failed.len(),
            failed: stats
                .failed
                .iter()
                .map(|(p, e)| JsonFailure {
                    path: p.clone(),
                    error: e.clone(),
                })
                .collect(),
            codex_sessions_root,
            db: db_path,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Codex sessions root: {}", codex_sessions_root.display());
        println!("sessions.db:         {}", db_path.display());
        println!("discovered:          {}", stats.discovered);
        println!("indexed:             {}", stats.indexed);
        println!("skipped (unchanged): {}", stats.skipped_unchanged);
        println!("deleted (vanished):  {}", stats.deleted);
        println!("failed:              {}", stats.failed.len());
        if !stats.failed.is_empty() {
            println!();
            println!("Failed files:");
            for (p, e) in &stats.failed {
                println!("  {} — {}", p.display(), e);
            }
        }
    }
    Ok(())
}
