//! The `session redact` verb — remove content from an indexed
//! transcript.
//!
//! Dry-run by default, like `slim`. `--execute` rewrites.
//!
//! Two things this handler is careful about, because both are ways a
//! redaction tool can quietly fail to redact:
//!
//! 1. **It never prints a match.** The plan reports counts. Echoing
//!    the matched line into a terminal (and therefore into *this*
//!    session's transcript, and the index) would re-leak the thing the
//!    user is trying to remove — which is precisely how the leak that
//!    motivated this verb happened in the first place.
//!
//! 2. **It says where the backup went, every time.** The default keeps
//!    a pre-redaction snapshot in the trash, and that snapshot still
//!    contains the secret. A user who thinks "redacted" means "gone"
//!    and leaves a copy in `~/.claudepot/trash` has been failed by the
//!    tool, not by themselves. `--purge` is the honest removal.

use super::*;

use claudepot_core::session::redact::{execute_redact, plan_redact, RedactOpts};

use super::slim::resolve_session_path;

#[derive(clap::Args, Debug)]
pub struct RedactArgs {
    /// Session UUID or absolute `.jsonl` path.
    pub target: String,
    /// Byte-exact string to remove. Repeatable.
    ///
    /// Literal, not a regex — you must be able to name what you are
    /// removing. Quote it; the shell will otherwise mangle anything
    /// interesting.
    #[arg(long = "pattern", value_name = "STRING")]
    pub patterns: Vec<String>,
    /// Also remove built-in secret shapes: `sk-ant-*` keys, email
    /// addresses, and `FOO=bar` env assignments.
    #[arg(long)]
    pub secrets: bool,
    /// Replace the whole string value containing a match, not just the
    /// matching substring. Use when the match means the entire value is
    /// tainted — e.g. a tool result that dumped a table of records.
    #[arg(long)]
    pub whole_value: bool,
    /// Do not keep a backup. The removal becomes irreversible — and
    /// therefore real. Correct for a leaked credential: the default
    /// backup still contains it.
    #[arg(long)]
    pub purge: bool,
    /// Apply the rewrite. Without this, only a dry-run plan is printed.
    #[arg(long)]
    pub execute: bool,
}

pub fn redact_cmd(ctx: &AppContext, args: RedactArgs) -> Result<()> {
    let RedactArgs {
        target,
        patterns,
        secrets,
        whole_value,
        purge,
        execute,
    } = args;

    if patterns.is_empty() && !secrets {
        bail!("nothing to redact — pass --pattern <string> (repeatable) and/or --secrets");
    }

    let opts = RedactOpts {
        patterns,
        secrets,
        whole_value,
        purge,
    };
    let path = resolve_session_path(&target)?;

    // Refuse to touch a transcript Claude Code is still appending to.
    // The core layer has a (size, mtime) TOCTOU guard that would catch
    // a concurrent write mid-rewrite, but failing here is a much better
    // error than failing there: it names the cause instead of reporting
    // "the file changed under me".
    if is_live_session(&path) {
        bail!(
            "{} looks like a live session (modified in the last 60s).\n\
             Redacting a transcript Claude Code still holds open would fork the file \
             and lose every turn written after this point.\n\
             Exit that session first, then re-run.",
            path.display()
        );
    }

    let plan = plan_redact(&path, &opts).context("plan redact")?;

    if !execute {
        if ctx.json {
            print_json(&plan)?;
            return Ok(());
        }
        if plan.is_noop() {
            println!("No matches in {}. Nothing to redact.", path.display());
            return Ok(());
        }
        println!(
            "Plan (dry-run): {} value(s) across {} line(s) would be rewritten.",
            plan.matched_values, plan.matched_lines
        );
        for hit in &plan.hits {
            // The pattern is the caller's own string — echoing it back
            // discloses nothing they don't already have. The
            // surrounding content is never shown.
            println!("  {:>4} × {}", hit.count, hit.pattern);
        }
        if opts.secrets {
            println!("  (plus any built-in secret shapes matched by --secrets)");
        }
        println!("\nRun with --execute to rewrite.");
        if opts.purge {
            println!("--purge is set: no backup will be kept. This is irreversible.");
        } else {
            println!(
                "A pre-redaction backup goes to the trash — it WILL still contain \
                 the matched content. Pass --purge to skip it."
            );
        }
        return Ok(());
    }

    let data_dir = paths::claudepot_data_dir();
    let sink = claudepot_core::project_progress::NoopSink;
    let report = execute_redact(&data_dir, &path, &opts, &sink).context("execute redact")?;

    // The index caches (size, mtime, inode) per file and re-parses when
    // the triple moves — and a re-parse DELETEs the file's exchanges
    // before re-inserting, so the FTS rows cannot outlive the text they
    // were built from. Refresh now rather than leaving the old content
    // searchable until whenever the app next happens to tick.
    let reindexed = reindex_after_redact();

    if ctx.json {
        print_json(&report)?;
        return Ok(());
    }

    println!(
        "Redacted {} value(s) across {} line(s). {} → {}",
        report.matched_values,
        report.matched_lines,
        format_size(report.original_bytes),
        format_size(report.final_bytes)
    );
    match reindexed {
        Ok(()) => println!("Index refreshed — the removed content is no longer searchable."),
        Err(e) => eprintln!(
            "warning: rewrite succeeded but the index refresh failed: {e}\n\
             The old text is still in sessions.db. Run `claudepot session rebuild-index`."
        ),
    }
    match &report.backup_trash_id {
        Some(id) => {
            println!(
                "\nA pre-redaction backup is in the trash as `{id}`.\n\
                 IT STILL CONTAINS THE REDACTED CONTENT."
            );
            println!("  restore:  claudepot session trash restore {id}");
            println!("  destroy:  claudepot session trash empty");
        }
        None => println!("\nNo backup kept (--purge). The removal is irreversible."),
    }
    Ok(())
}

/// Heuristic: a transcript touched in the last minute is probably being
/// written to right now. Deliberately conservative — a false positive
/// costs the user one `--` retry after quitting their session; a false
/// negative costs them the tail of a live conversation.
fn is_live_session(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    mtime
        .elapsed()
        .map(|age| age.as_secs() < 60)
        .unwrap_or(false)
}

/// Re-parse the rewritten transcript into `sessions.db`.
///
/// Uses the same two-step the app itself runs (`refresh` →
/// `backfill_claude_exchanges`) rather than reaching for a private
/// path. Both are keyed on the `(size, mtime, inode)` staleness triple,
/// so only the file we just rewrote is re-parsed; the other ~130
/// transcripts are skipped. The backfill DELETEs a changed file's
/// exchanges before re-inserting, which is what actually evicts the old
/// text from the FTS index.
fn reindex_after_redact() -> Result<()> {
    let db = paths::claudepot_data_dir().join("sessions.db");
    if !db.exists() {
        return Ok(()); // nothing indexed yet; nothing to evict
    }
    let cfg = paths::claude_config_dir();
    let idx = claudepot_core::session_index::SessionIndex::open(&db)
        .context("open session index for refresh")?;
    idx.refresh(&cfg).context("refresh session index")?;
    claudepot_core::shared_memory::claude_exchanges::backfill_claude_exchanges(&idx, &cfg)
        .context("re-index the redacted transcript")?;
    Ok(())
}
