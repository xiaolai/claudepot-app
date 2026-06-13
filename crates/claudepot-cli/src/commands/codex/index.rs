//! `index` verb — walk the Codex sessions tree and populate
//! `sessions.db`.
//!
//! Sub-module of `commands/codex.rs`; see that file's header for the
//! per-verb layout rationale.

use super::*;

use serde::Serialize;

use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::indexer::{backfill_codex, CodexIndexerStats};

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

pub async fn index(codex_home: Option<PathBuf>, db: Option<PathBuf>, json: bool) -> Result<()> {
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
    let stats: CodexIndexerStats =
        tokio::task::spawn_blocking(move || backfill_codex(&idx, &codex_root_clone))
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
