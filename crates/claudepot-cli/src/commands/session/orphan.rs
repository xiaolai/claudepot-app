//! Orphan-projects verbs: `list-orphans`, `move`, `adopt-orphan`, `rebuild-index`.
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::*;

pub fn list_orphans(ctx: &AppContext) -> Result<()> {
    let config_dir = paths::claude_config_dir();
    let orphans = detect_orphaned_projects(&config_dir)
        .context("failed to scan for orphaned projects")?;

    if ctx.json {
        println!("{}", format_orphans_json(&orphans));
        return Ok(());
    }

    if orphans.is_empty() {
        println!("No orphaned projects found.");
        return Ok(());
    }

    print_orphans_human(&orphans);
    Ok(())
}

/// Move a single session transcript from one project cwd to another.
#[allow(clippy::too_many_arguments)]
pub fn move_cmd(
    ctx: &AppContext,
    session_id: &str,
    from_cwd: &str,
    to_cwd: &str,
    force_live: bool,
    force_conflict: bool,
    cleanup_source: bool,
) -> Result<()> {
    let sid: Uuid = session_id
        .parse()
        .with_context(|| format!("invalid session id: {session_id}"))?;
    let config_dir = paths::claude_config_dir();
    let opts = MoveSessionOpts {
        force_live_session: force_live,
        force_sync_conflict: force_conflict,
        cleanup_source_if_empty: cleanup_source,
        claude_json_path: claude_json_path(),
    };
    let report = move_session(
        &config_dir,
        sid,
        Path::new(from_cwd),
        Path::new(to_cwd),
        opts,
    )
    .with_context(|| format!("failed to move session {sid}"))?;

    if ctx.json {
        println!("{}", format_move_report_json(&report));
    } else {
        print_move_report_human(&report);
    }
    Ok(())
}

/// Adopt every session under an orphan slug into a live target cwd.
pub fn adopt_orphan_cmd(
    ctx: &AppContext,
    orphan_slug: &str,
    target_cwd: &str,
) -> Result<()> {
    let config_dir = paths::claude_config_dir();
    let target = Path::new(target_cwd);
    if !target.is_dir() {
        bail!("target cwd does not exist: {target_cwd}");
    }

    let report = adopt_orphan_project(&config_dir, orphan_slug, target, claude_json_path())
        .with_context(|| format!("failed to adopt {orphan_slug} into {target_cwd}"))?;

    if ctx.json {
        println!("{}", format_adopt_report_json(&report));
    } else {
        print_adopt_report_human(&report);
    }
    Ok(())
}

/// Truncate the persistent session-index cache. Leaves the DB file
/// and schema intact; only the row data is dropped. The next
/// `session_list_all` (from the GUI or another CLI) re-scans every
/// transcript.
pub fn rebuild_index_cmd(ctx: &AppContext) -> Result<()> {
    let db_path = paths::claudepot_data_dir().join("sessions.db");
    let idx = claudepot_core::session_index::SessionIndex::open(&db_path)
        .context("open session index")?;
    idx.rebuild().context("rebuild session index")?;
    if ctx.json {
        println!(r#"{{"status":"ok","path":{:?}}}"#, db_path.display().to_string());
    } else {
        eprintln!("Session index cleared at {}", db_path.display());
        eprintln!("Next `session` list will re-parse every transcript.");
    }
    Ok(())
}

fn print_orphans_human(orphans: &[OrphanedProject]) {
    println!(
        "{:<48}  {:>8}  {:>12}  Slug",
        "Original cwd (from transcript)", "Sessions", "Size"
    );
    println!(
        "{:<48}  {:>8}  {:>12}  ────",
        "─".repeat(48),
        "────────",
        "────────────"
    );
    for o in orphans {
        let cwd = o
            .cwd_from_transcript
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(unparseable)".to_string());
        let size = format_bytes(o.total_size_bytes);
        let cwd_short = truncate_start(&cwd, 48);
        println!(
            "{:<48}  {:>8}  {:>12}  {}",
            cwd_short, o.session_count, size, o.slug
        );
    }
    println!(
        "\n{} orphan project(s). Run `claudepot session adopt-orphan <slug> --target <cwd>` to rescue.",
        orphans.len()
    );
}

fn format_orphans_json(orphans: &[OrphanedProject]) -> String {
    let arr: Vec<serde_json::Value> = orphans
        .iter()
        .map(|o| {
            serde_json::json!({
                "slug": o.slug,
                "cwd_from_transcript": o
                    .cwd_from_transcript
                    .as_ref()
                    .map(|p| p.display().to_string()),
                "session_count": o.session_count,
                "total_size_bytes": o.total_size_bytes,
                "suggested_adoption_target": o
                    .suggested_adoption_target
                    .as_ref()
                    .map(|p| p.display().to_string()),
            })
        })
        .collect();
    serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".to_string())
}

fn print_move_report_human(r: &MoveSessionReport) {
    let sid = r
        .session_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".to_string());
    println!("Session {sid} moved.");
    println!("  from slug:              {}", r.from_slug);
    println!("  to slug:                {}", r.to_slug);
    println!("  jsonl lines rewritten:  {}", r.jsonl_lines_rewritten);
    if r.subagent_files_moved > 0 {
        println!("  subagent files moved:   {}", r.subagent_files_moved);
    }
    if r.remote_agent_files_moved > 0 {
        println!("  remote-agent files:     {}", r.remote_agent_files_moved);
    }
    if r.history_entries_moved > 0 {
        println!("  history entries moved:  {}", r.history_entries_moved);
    }
    if r.history_entries_unmapped > 0 {
        println!(
            "  history entries stayed: {} (pre-sessionId; cannot be attributed)",
            r.history_entries_unmapped
        );
    }
    if r.claude_json_pointers_cleared > 0 {
        println!(
            "  .claude.json pointers:  {} cleared",
            r.claude_json_pointers_cleared
        );
    }
    if r.source_dir_removed {
        println!("  source slug dir:        removed (was empty)");
    }
}

fn format_move_report_json(r: &MoveSessionReport) -> String {
    let v = serde_json::json!({
        "session_id": r.session_id.map(|s| s.to_string()),
        "from_slug": r.from_slug,
        "to_slug": r.to_slug,
        "jsonl_lines_rewritten": r.jsonl_lines_rewritten,
        "subagent_files_moved": r.subagent_files_moved,
        "remote_agent_files_moved": r.remote_agent_files_moved,
        "history_entries_moved": r.history_entries_moved,
        "history_entries_unmapped": r.history_entries_unmapped,
        "claude_json_pointers_cleared": r.claude_json_pointers_cleared,
        "source_dir_removed": r.source_dir_removed,
    });
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
}

fn print_adopt_report_human(r: &AdoptReport) {
    println!(
        "Adopted {}/{} session(s).",
        r.sessions_moved, r.sessions_attempted
    );
    if !r.sessions_failed.is_empty() {
        eprintln!("Failures:");
        for (sid, reason) in &r.sessions_failed {
            eprintln!("  {sid}: {reason}");
        }
    }
    if r.source_dir_removed {
        println!("Source slug dir removed (was empty after adoption).");
    }
    let rewritten: usize = r.per_session.iter().map(|m| m.jsonl_lines_rewritten).sum();
    let hist: usize = r.per_session.iter().map(|m| m.history_entries_moved).sum();
    let unmapped: usize = r
        .per_session
        .iter()
        .map(|m| m.history_entries_unmapped)
        .sum();
    if rewritten + hist + unmapped > 0 {
        println!(
            "Totals: {rewritten} transcript lines, {hist} history entries moved ({unmapped} stayed)."
        );
    }
}

fn format_adopt_report_json(r: &AdoptReport) -> String {
    let per: Vec<serde_json::Value> = r
        .per_session
        .iter()
        .map(|m| {
            serde_json::json!({
                "session_id": m.session_id.map(|s| s.to_string()),
                "jsonl_lines_rewritten": m.jsonl_lines_rewritten,
                "history_entries_moved": m.history_entries_moved,
                "history_entries_unmapped": m.history_entries_unmapped,
            })
        })
        .collect();
    let failed: Vec<serde_json::Value> = r
        .sessions_failed
        .iter()
        .map(|(sid, msg)| serde_json::json!({"session_id": sid.to_string(), "error": msg}))
        .collect();
    let v = serde_json::json!({
        "sessions_attempted": r.sessions_attempted,
        "sessions_moved": r.sessions_moved,
        "sessions_failed": failed,
        "source_dir_removed": r.source_dir_removed,
        "per_session": per,
    });
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
}
