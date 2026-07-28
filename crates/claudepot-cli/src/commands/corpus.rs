//! `claudepot corpus` — build and inspect the analysis corpus.
//!
//! The corpus is every transcript from every machine, deduped, in
//! `~/.claudepot/corpus.db`. It is deliberately NOT `sessions.db`:
//! that file is a cache of one machine's live `~/.claude` and its
//! refresh deletes any row without a file behind it, which would make
//! importing another host's archive mutually destructive with the
//! normal refresh. See `claudepot_core::corpus`.
//!
//! `index` is incremental and safe to re-run: unchanged files are
//! skipped by `(size, mtime)` without being read, and nothing is ever
//! deleted — an archive of a live host is a snapshot, so a file that is
//! absent now is not evidence the record is stale.

use anyhow::{Context, Result};

use claudepot_core::corpus::{self, CorpusIndex, IndexStats, LOCAL_HOST};
use claudepot_core::paths;

use crate::output::print_json;
use crate::AppContext;

fn open() -> Result<CorpusIndex> {
    CorpusIndex::open(&corpus::default_path()).context("open corpus.db")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn add(a: IndexStats, b: IndexStats) -> IndexStats {
    IndexStats {
        seen: a.seen + b.seen,
        indexed: a.indexed + b.indexed,
        unchanged: a.unchanged + b.unchanged,
        duplicate: a.duplicate + b.duplicate,
        failed: a.failed + b.failed,
    }
}

/// Index the live `~/.claude/projects` plus every host directory under
/// the archive root.
pub fn index_cmd(ctx: &AppContext, archive_root: Option<String>) -> Result<()> {
    let idx = open()?;
    let now = now_ms();
    let mut total = IndexStats::default();

    // Local machine first — it is the one that is live, so it benefits
    // most from being current.
    let live = paths::claude_config_dir().join("projects");
    if live.is_dir() {
        if !ctx.quiet {
            eprintln!("indexing {LOCAL_HOST}: {}", live.display());
        }
        total = add(total, idx.index_root(LOCAL_HOST, &live, now)?);
    }

    let root = archive_root
        .map(std::path::PathBuf::from)
        .or_else(corpus::default_archive_root);
    if let Some(root) = root {
        if let Ok(hosts) = std::fs::read_dir(&root) {
            for host in hosts.flatten() {
                if !host.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let host_id = host.file_name().to_string_lossy().into_owned();
                let projects = host.path().join("projects");
                if !projects.is_dir() {
                    continue;
                }
                if !ctx.quiet {
                    eprintln!("indexing {host_id}: {}", projects.display());
                }
                total = add(total, idx.index_root(&host_id, &projects, now)?);
            }
        }
    }

    let sessions = idx.session_count()?;
    let files = idx.file_count()?;
    if ctx.json {
        return print_json(&serde_json::json!({
            "seen": total.seen,
            "indexed": total.indexed,
            "unchanged": total.unchanged,
            "duplicate": total.duplicate,
            "failed": total.failed,
            "sessions": sessions,
            "files": files,
        }));
    }
    println!(
        "{} file(s) seen — {} indexed, {} unchanged, {} duplicate copies, {} failed.",
        total.seen, total.indexed, total.unchanged, total.duplicate, total.failed
    );
    println!("Corpus now holds {sessions} session(s) across {files} file(s).");
    Ok(())
}

/// Per-host coverage — what each machine contributes, and how stale it is.
pub fn status_cmd(ctx: &AppContext) -> Result<()> {
    let idx = open()?;
    let cov = idx.host_coverage()?;
    if ctx.json {
        return print_json(&cov);
    }
    if cov.is_empty() {
        println!("Corpus is empty. Run `claudepot corpus index`.");
        return Ok(());
    }
    println!("{:<18}{:>8}{:>10}  newest", "host", "files", "sessions");
    for h in &cov {
        let newest = h
            .newest_ts_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "—".into());
        println!(
            "{:<18}{:>8}{:>10}  {}",
            h.host_id, h.files, h.sessions, newest
        );
    }
    println!(
        "\n{} session(s) across {} file(s).",
        idx.session_count()?,
        idx.file_count()?
    );
    Ok(())
}
