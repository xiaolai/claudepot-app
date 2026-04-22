//! CC session transcript management — move, list-orphans, adopt-orphan,
//! inspect (view/chunks/tools/classify/subagents/phases/context),
//! export, search, worktree grouping.
//!
//! All handlers are thin wrappers around `claudepot_core`. No business
//! logic lives here (per `.claude/rules/architecture.md`).

use crate::AppContext;
use anyhow::{bail, Context, Result};
use claudepot_core::paths;
use claudepot_core::session::{read_session_detail, read_session_detail_at_path, SessionDetail};
use claudepot_core::session_move::{
    adopt_orphan_project, detect_orphaned_projects, move_session, AdoptReport, MoveSessionOpts,
    MoveSessionReport, OrphanedProject,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// CC stores `.claude.json` at `$HOME/.claude.json` — a sibling of
/// `~/.claude/`, not inside. Central accessor so CLI and Tauri agree.
fn claude_json_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

/// List projects whose internal `cwd` no longer exists on disk. These
/// are the adoption candidates — typically sessions orphaned by
/// `git worktree remove`.
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

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

fn print_orphans_human(orphans: &[OrphanedProject]) {
    println!(
        "{:<48}  {:>8}  {:>12}  {}",
        "Original cwd (from transcript)", "Sessions", "Size", "Slug"
    );
    println!(
        "{:<48}  {:>8}  {:>12}  {}",
        "─".repeat(48),
        "────────",
        "────────────",
        "────"
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

// ---------------------------------------------------------------------------
// Small presentation helpers
// ---------------------------------------------------------------------------

fn truncate_start(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    // Keep the tail (more informative for paths). Prefix with "…".
    let skip = s.chars().count() - (max - 1);
    let kept: String = s.chars().skip(skip).collect();
    format!("…{kept}")
}

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB {
        format!("{:.2} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.2} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.2} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

// ---------------------------------------------------------------------------
// Session debugger commands — Tier 1/2/3 ports from claude-devtools
// ---------------------------------------------------------------------------

/// Resolve `target` to a parsed `SessionDetail`. Accepts either a
/// session UUID (will be located under `<config>/projects/*/`) or an
/// absolute path to a `.jsonl` file under the projects tree.
fn resolve_detail(target: &str) -> Result<SessionDetail> {
    let cfg = paths::claude_config_dir();
    // Heuristic: anything containing `/` or ending in `.jsonl` is a path.
    let looks_like_path = target.contains('/')
        || target.contains('\\')
        || target.ends_with(".jsonl");
    if looks_like_path {
        read_session_detail_at_path(&cfg, Path::new(target))
            .with_context(|| format!("read transcript at {target}"))
    } else {
        read_session_detail(&cfg, target)
            .with_context(|| format!("locate session {target}"))
    }
}

/// `claudepot session view <target> --show ...`
pub fn view_cmd(ctx: &AppContext, target: &str, show: &str) -> Result<()> {
    let detail = resolve_detail(target)?;
    let events = &detail.events;
    match show {
        "classify" => {
            let cats = claudepot_core::session_classify::classify_all(events);
            if ctx.json {
                let payload: Vec<serde_json::Value> = cats
                    .iter()
                    .map(|(c, i)| {
                        serde_json::json!({
                            "index": i,
                            "category": c,
                        })
                    })
                    .collect();
                print_json(&payload);
            } else {
                println!("{:>5}  {}", "IDX", "CATEGORY");
                for (cat, idx) in &cats {
                    println!("{idx:>5}  {cat:?}");
                }
            }
        }
        "chunks" => {
            let chunks = claudepot_core::session_chunks::build_chunks(events);
            if ctx.json {
                print_json(&chunks);
            } else {
                print_chunks_human(&chunks);
            }
        }
        "tools" => {
            let linked = claudepot_core::session_tool_link::link_tools(events);
            if ctx.json {
                print_json(&linked);
            } else {
                println!(
                    "{:<12}  {:<40}  {:>9}  {}",
                    "TOOL", "ID", "DUR(ms)", "STATUS"
                );
                for t in &linked {
                    let id = truncate_start(&t.tool_use_id, 40);
                    let dur = t
                        .duration_ms
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| "-".into());
                    let status = if t.result_content.is_none() {
                        "orphaned"
                    } else if t.is_error {
                        "error"
                    } else {
                        "ok"
                    };
                    println!("{:<12}  {:<40}  {:>9}  {status}", t.tool_name, id, dur);
                }
            }
        }
        "subagents" => {
            let mut agents = claudepot_core::session_subagents::resolve_subagents(
                &paths::claude_config_dir(),
                &detail.row.slug,
                &detail.row.session_id,
            )
            .context("resolve subagents")?;
            claudepot_core::session_subagents::link_parent_tasks(events, &mut agents);
            if ctx.json {
                print_json(&agents);
            } else if agents.is_empty() {
                println!("(no subagents)");
            } else {
                println!(
                    "{:<18}  {:<12}  {:>6}  {:>9}  {}",
                    "ID", "TYPE", "MSGS", "DUR(ms)", "DESCRIPTION"
                );
                for a in &agents {
                    println!(
                        "{:<18}  {:<12}  {:>6}  {:>9}  {}",
                        truncate_start(&a.id, 18),
                        a.agent_type.as_deref().unwrap_or("-"),
                        a.metrics.message_count,
                        a.metrics.duration_ms,
                        a.description.as_deref().unwrap_or("")
                    );
                }
            }
        }
        "phases" => {
            let info = claudepot_core::session_phases::compute_phases(events);
            if ctx.json {
                print_json(&info);
            } else {
                println!(
                    "{:>5}  {:>8}  {:>8}  {}",
                    "PHASE", "START", "END", "SUMMARY"
                );
                for p in &info.phases {
                    println!(
                        "{:>5}  {:>8}  {:>8}  {}",
                        p.phase_number,
                        p.start_index,
                        p.end_index,
                        p.summary.as_deref().unwrap_or("—").chars().take(80).collect::<String>()
                    );
                }
            }
        }
        "context" => {
            let stats = claudepot_core::session_context::attribute_context(events);
            if ctx.json {
                print_json(&stats);
            } else {
                let t = &stats.totals;
                let total = t.total().max(1);
                let pct = |n: u64| (n as f64 / total as f64) * 100.0;
                println!("Visible context totals ({} tokens):", t.total());
                println!(
                    "  CLAUDE.md         {:>8}  {:>5.1}%",
                    t.claude_md,
                    pct(t.claude_md)
                );
                println!(
                    "  Mentioned files   {:>8}  {:>5.1}%",
                    t.mentioned_file,
                    pct(t.mentioned_file)
                );
                println!(
                    "  Tool output       {:>8}  {:>5.1}%",
                    t.tool_output,
                    pct(t.tool_output)
                );
                println!(
                    "  Thinking/text     {:>8}  {:>5.1}%",
                    t.thinking_text,
                    pct(t.thinking_text)
                );
                println!(
                    "  Team coord.       {:>8}  {:>5.1}%",
                    t.team_coordination,
                    pct(t.team_coordination)
                );
                println!(
                    "  User messages     {:>8}  {:>5.1}%",
                    t.user_message,
                    pct(t.user_message)
                );
                println!("Reported by model: {} tokens", stats.reported_total_tokens);
            }
        }
        _ /* summary */ => {
            if ctx.json {
                let cats = claudepot_core::session_classify::classify_all(events);
                let chunks = claudepot_core::session_chunks::build_chunks(events);
                let linked = claudepot_core::session_tool_link::link_tools(events);
                let info = claudepot_core::session_phases::compute_phases(events);
                let context = claudepot_core::session_context::attribute_context(events);
                let payload = serde_json::json!({
                    "row": &detail.row,
                    "chunks": chunks,
                    "linked_tools": linked,
                    "phases": info,
                    "context": context,
                    "classification_counts": count_by_category(&cats),
                });
                print_json(&payload);
            } else {
                print_summary_human(&detail);
            }
        }
    }
    Ok(())
}

/// `claudepot session export <target> --format md|json [--output FILE]`
pub fn export_cmd(
    ctx: &AppContext,
    target: &str,
    format: &str,
    output: Option<&str>,
) -> Result<()> {
    let _ = ctx; // --json flag doesn't apply here
    let detail = resolve_detail(target)?;
    let fmt = match format {
        "md" | "markdown" => claudepot_core::session_export::ExportFormat::Markdown,
        "json" => claudepot_core::session_export::ExportFormat::Json,
        other => bail!("unknown format: {other}"),
    };
    let body = claudepot_core::session_export::export(&detail, fmt);
    match output {
        Some(path) => {
            write_export_file(path, body.as_bytes())
                .with_context(|| format!("write {path}"))?;
            eprintln!("Wrote {} bytes to {path}", body.len());
        }
        None => {
            print!("{body}");
        }
    }
    Ok(())
}

/// Create the export file with 0600 mode on Unix — the file may carry
/// secrets from the transcript even after redaction (user-supplied
/// content that never passes through the redactor), and the rule is
/// "minimum permissions on credential-adjacent data".
fn write_export_file(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    use std::io::Write as _;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// `claudepot session search <query> [--limit N]`
pub fn search_cmd(ctx: &AppContext, query: &str, limit: usize) -> Result<()> {
    let cfg = paths::claude_config_dir();
    let rows = claudepot_core::session::list_all_sessions(&cfg)
        .context("list sessions for search")?;
    let hits = claudepot_core::session_search::search_rows(&rows, query, limit)
        .context("search sessions")?;
    if ctx.json {
        print_json(&hits);
        return Ok(());
    }
    if hits.is_empty() {
        println!("No matches for {query:?}.");
        return Ok(());
    }
    println!(
        "{} hit(s) for {query:?} (showing {} of {}):",
        hits.len(),
        hits.len(),
        rows.len()
    );
    println!();
    for h in &hits {
        println!("{}  [{}]  score={:.2}", h.session_id, h.role, h.score);
        println!("  path:   {}", h.file_path.display());
        println!("  match:  {}", h.snippet);
        println!();
    }
    Ok(())
}

/// `claudepot session worktrees`
pub fn worktrees_cmd(ctx: &AppContext) -> Result<()> {
    let cfg = paths::claude_config_dir();
    let rows = claudepot_core::session::list_all_sessions(&cfg)
        .context("list sessions for worktree grouping")?;
    let groups = claudepot_core::session_worktree::group_by_repo(rows);
    if ctx.json {
        print_json(&groups);
        return Ok(());
    }
    for g in &groups {
        println!(
            "{} — {} session(s), {} worktree(s), branches: {}",
            g.label,
            g.sessions.len(),
            g.worktree_paths.len(),
            if g.branches.is_empty() {
                "—".into()
            } else {
                g.branches.join(", ")
            }
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pretty-print helpers for the new commands
// ---------------------------------------------------------------------------

fn print_chunks_human(chunks: &[claudepot_core::session_chunks::SessionChunk]) {
    use claudepot_core::session_chunks::SessionChunk;
    println!("{:>4}  {:<8}  {:>6}  {:>9}  {}", "ID", "TYPE", "MSGS", "DUR(ms)", "DETAIL");
    for c in chunks {
        let h = c.header();
        let (kind, detail) = match c {
            SessionChunk::User { event_index, .. } => ("user", format!("event #{event_index}")),
            SessionChunk::System { event_index, .. } => {
                ("system", format!("event #{event_index}"))
            }
            SessionChunk::Compact { event_index, .. } => {
                ("compact", format!("event #{event_index}"))
            }
            SessionChunk::Ai {
                event_indices,
                tool_executions,
                ..
            } => (
                "ai",
                format!(
                    "{} events, {} tool(s)",
                    event_indices.len(),
                    tool_executions.len()
                ),
            ),
        };
        println!(
            "{:>4}  {:<8}  {:>6}  {:>9}  {detail}",
            h.id, kind, h.metrics.message_count, h.metrics.duration_ms,
        );
    }
}

fn print_summary_human(d: &SessionDetail) {
    let r = &d.row;
    let chunks = claudepot_core::session_chunks::build_chunks(&d.events);
    let linked = claudepot_core::session_tool_link::link_tools(&d.events);
    let phases = claudepot_core::session_phases::compute_phases(&d.events);

    let mut user_chunks = 0usize;
    let mut ai_chunks = 0usize;
    let mut system_chunks = 0usize;
    let mut compact_chunks = 0usize;
    for c in &chunks {
        match c {
            claudepot_core::session_chunks::SessionChunk::User { .. } => user_chunks += 1,
            claudepot_core::session_chunks::SessionChunk::Ai { .. } => ai_chunks += 1,
            claudepot_core::session_chunks::SessionChunk::System { .. } => system_chunks += 1,
            claudepot_core::session_chunks::SessionChunk::Compact { .. } => compact_chunks += 1,
        }
    }
    let orphaned = linked.iter().filter(|t| t.result_content.is_none()).count();
    let errored = linked.iter().filter(|t| t.is_error).count();

    println!("Session:     {}", r.session_id);
    println!("Project:     {}", r.project_path);
    if let Some(b) = &r.git_branch {
        println!("Branch:      {b}");
    }
    println!(
        "Tokens:      input {}, output {}, cache r/w {}/{} ({} total)",
        r.tokens.input,
        r.tokens.output,
        r.tokens.cache_read,
        r.tokens.cache_creation,
        r.tokens.total()
    );
    println!(
        "Messages:    {} ({} user, {} assistant)",
        r.message_count, r.user_message_count, r.assistant_message_count
    );
    println!(
        "Chunks:      {} total — {user_chunks} user, {ai_chunks} ai, {system_chunks} system, {compact_chunks} compact",
        chunks.len()
    );
    println!(
        "Tools:       {} linked, {orphaned} orphaned, {errored} errored",
        linked.len()
    );
    println!(
        "Compactions: {} ({} phase{})",
        phases.compaction_count,
        phases.phases.len(),
        if phases.phases.len() == 1 { "" } else { "s" }
    );
}

fn count_by_category(
    cats: &[(claudepot_core::session_classify::MessageCategory, usize)],
) -> serde_json::Value {
    use claudepot_core::session_classify::MessageCategory;
    let mut user = 0;
    let mut ai = 0;
    let mut system = 0;
    let mut compact = 0;
    let mut hard_noise = 0;
    for (c, _) in cats {
        match c {
            MessageCategory::User => user += 1,
            MessageCategory::Ai => ai += 1,
            MessageCategory::System => system += 1,
            MessageCategory::Compact => compact += 1,
            MessageCategory::HardNoise => hard_noise += 1,
        }
    }
    serde_json::json!({
        "user": user,
        "ai": ai,
        "system": system,
        "compact": compact,
        "hard_noise": hard_noise,
    })
}

fn print_json<T: serde::Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("json serialization failed: {e}"),
    }
}

