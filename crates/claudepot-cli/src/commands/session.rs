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
                println!("{:>5}  CATEGORY", "IDX");
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
                    "{:<12}  {:<40}  {:>9}  STATUS",
                    "TOOL", "ID", "DUR(ms)"
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
                    "{:<18}  {:<12}  {:>6}  {:>9}  DESCRIPTION",
                    "ID", "TYPE", "MSGS", "DUR(ms)"
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
                    "{:>5}  {:>8}  {:>8}  SUMMARY",
                    "PHASE", "START", "END"
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

/// `claudepot session export <target> --format fmt --to dest [flags]`
///
/// Pure dispatcher: format → policy → body → `core::session_export_delivery::deliver`.
/// All file/clipboard/gist mechanics live in `claudepot-core`; the CLI
/// only supplies a [`SubprocessClipboard`] for the `clipboard` arm.
#[allow(clippy::too_many_arguments)]
pub async fn export_cmd(
    ctx: &AppContext,
    target: &str,
    format: &str,
    to: &str,
    output: Option<&str>,
    public: bool,
    redact_paths: &str,
    redact_emails: bool,
    redact_env: bool,
    redact_regex: Vec<String>,
    html_no_js: bool,
) -> Result<()> {
    use claudepot_core::session_export_delivery::{
        deliver, default_export_filename, extension_for, DeliveryReceipt, ExportDestination,
    };
    let _ = ctx;
    let detail = resolve_detail(target)?;
    let fmt = match format {
        "md" | "markdown" => claudepot_core::session_export::ExportFormat::Markdown,
        "markdown-slim" => claudepot_core::session_export::ExportFormat::MarkdownSlim,
        "json" => claudepot_core::session_export::ExportFormat::Json,
        "html" => claudepot_core::session_export::ExportFormat::Html {
            no_js: html_no_js,
        },
        other => bail!("unknown format: {other}"),
    };
    let policy = build_policy(redact_paths, redact_emails, redact_env, redact_regex)?;
    let body = claudepot_core::session_export::export_with(&detail, fmt.clone(), &policy);
    let dest = match to {
        "file" => {
            let path = output.ok_or_else(|| anyhow::anyhow!("--output required for --to file"))?;
            ExportDestination::File {
                path: PathBuf::from(path),
            }
        }
        "clipboard" => ExportDestination::Clipboard,
        "gist" => ExportDestination::Gist {
            filename: default_export_filename(&detail.row.session_id, extension_for(&fmt)),
            description: format!("Claudepot session export: {}", detail.row.session_id),
            public,
        },
        other => bail!("unknown destination: {other}"),
    };
    let clipboard = crate::clipboard::SubprocessClipboard;
    let receipt = deliver(
        &body,
        dest,
        Some(&clipboard),
        &claudepot_core::project_progress::NoopSink,
    )
    .await?;
    match receipt {
        DeliveryReceipt::File { path, bytes } => {
            eprintln!("Wrote {bytes} bytes to {}", path.display());
        }
        DeliveryReceipt::Clipboard { bytes } => {
            eprintln!("Copied {bytes} bytes to clipboard");
        }
        DeliveryReceipt::Gist { result, .. } => {
            eprintln!("Uploaded to {}", result.url);
            println!("{}", result.url);
        }
    }
    Ok(())
}

fn build_policy(
    redact_paths: &str,
    redact_emails: bool,
    redact_env: bool,
    redact_regex: Vec<String>,
) -> Result<claudepot_core::redaction::RedactionPolicy> {
    use claudepot_core::redaction::{PathStrategy, RedactionPolicy};
    let paths = match redact_paths {
        "off" => PathStrategy::Off,
        "relative" => PathStrategy::Relative {
            root: dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")),
        },
        "hash" => PathStrategy::Hash,
        other => bail!("unknown redact-paths strategy: {other}"),
    };
    Ok(RedactionPolicy {
        anthropic_keys: true,
        paths,
        emails: redact_emails,
        env_assignments: redact_env,
        custom_regex: redact_regex,
    })
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
    println!("{:>4}  {:<8}  {:>6}  {:>9}  DETAIL", "ID", "TYPE", "MSGS", "DUR(ms)");
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

// ---------------------------------------------------------------------------
// Prune + trash
// ---------------------------------------------------------------------------

fn parse_duration(s: &str) -> Result<std::time::Duration> {
    let t = s.trim();
    if t.is_empty() {
        bail!("empty duration");
    }
    let (num_part, unit) = t.split_at(
        t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len()),
    );
    let n: u64 = num_part
        .parse()
        .with_context(|| format!("invalid duration: {s}"))?;
    let secs = match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86400,
        _ => bail!("unknown duration unit in {s:?} (use s/m/h/d)"),
    };
    Ok(std::time::Duration::from_secs(secs))
}

fn parse_size(s: &str) -> Result<u64> {
    let t = s.trim().to_ascii_uppercase();
    if t.is_empty() {
        bail!("empty size");
    }
    let (num_part, unit) = t.split_at(
        t.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(t.len()),
    );
    let n: u64 = num_part
        .parse()
        .with_context(|| format!("invalid size: {s}"))?;
    let mult: u64 = match unit {
        "" | "B" => 1,
        "KB" => 1_000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "KIB" => 1024,
        "MIB" => 1024 * 1024,
        "GIB" => 1024 * 1024 * 1024,
        _ => bail!("unknown size unit in {s:?} (use B/KB/MB/GB/KiB/MiB/GiB)"),
    };
    Ok(n.saturating_mul(mult))
}

#[allow(clippy::too_many_arguments)]
pub fn prune_cmd(
    ctx: &AppContext,
    older_than: Option<&str>,
    larger_than: Option<&str>,
    project: Vec<String>,
    has_error: bool,
    sidechain: bool,
    execute: bool,
) -> Result<()> {
    use claudepot_core::session_prune::{execute_prune, plan_prune, PruneFilter};
    let mut filter = PruneFilter::default();
    if let Some(s) = older_than {
        filter.older_than = Some(parse_duration(s)?);
    }
    if let Some(s) = larger_than {
        filter.larger_than = Some(parse_size(s)?);
    }
    filter.project = project.iter().map(PathBuf::from).collect();
    filter.has_error = if has_error { Some(true) } else { None };
    filter.is_sidechain = if sidechain { Some(true) } else { None };

    let cfg = paths::claude_config_dir();
    let plan = plan_prune(&cfg, &filter).context("plan prune")?;

    if plan.entries.is_empty() {
        if ctx.json {
            print_json(&plan);
        } else {
            println!("No sessions match the filter.");
        }
        return Ok(());
    }

    if !execute {
        if ctx.json {
            print_json(&plan);
            return Ok(());
        }
        println!("Plan (dry-run):");
        for e in &plan.entries {
            println!(
                "  - {}    {}    {}",
                e.file_path.display(),
                format_size(e.size_bytes),
                e.last_ts_ms
                    .map(format_ts_ms)
                    .unwrap_or_else(|| "—".to_string())
            );
        }
        println!(
            "Total: {} file(s), {} → trash",
            plan.entries.len(),
            format_size(plan.total_bytes)
        );
        println!("Run with --execute to apply. Trash retained for 7 days.");
        return Ok(());
    }

    let data_dir = paths::claudepot_data_dir();
    let sink = claudepot_core::project_progress::NoopSink;
    let report = execute_prune(&data_dir, &plan, &sink).context("execute prune")?;
    if ctx.json {
        print_json(&report);
        return Ok(());
    }
    println!(
        "Moved {} file(s) to trash, {} freed.",
        report.moved.len(),
        format_size(report.freed_bytes)
    );
    for (p, reason) in &report.failed {
        eprintln!("  ✗ {}: {}", p.display(), reason);
    }
    Ok(())
}

pub fn trash_list_cmd(ctx: &AppContext, older_than: Option<&str>) -> Result<()> {
    use claudepot_core::trash::{self, TrashFilter};
    let filter = TrashFilter {
        older_than: older_than.map(parse_duration).transpose()?,
        kind: None,
    };
    let data_dir = paths::claudepot_data_dir();
    let listing = trash::list(&data_dir, filter).context("list trash")?;
    if ctx.json {
        print_json(&listing);
        return Ok(());
    }
    if listing.entries.is_empty() {
        println!("Trash is empty.");
        return Ok(());
    }
    for e in &listing.entries {
        println!(
            "{}  {:?}  {}  {}",
            e.id,
            e.kind,
            format_size(e.size),
            e.orig_path.display()
        );
    }
    println!(
        "Total: {} entry(ies), {}",
        listing.entries.len(),
        format_size(listing.total_bytes)
    );
    Ok(())
}

pub fn trash_restore_cmd(ctx: &AppContext, id: &str, to: Option<&str>) -> Result<()> {
    use claudepot_core::trash;
    let data_dir = paths::claudepot_data_dir();
    let cwd = to.map(Path::new);
    let restored = trash::restore(&data_dir, id, cwd).context("restore trash")?;
    if ctx.json {
        print_json(&serde_json::json!({ "restored": restored }));
    } else {
        println!("Restored to {}", restored.display());
    }
    Ok(())
}

pub fn trash_empty_cmd(ctx: &AppContext, older_than: Option<&str>) -> Result<()> {
    use claudepot_core::trash::{self, TrashFilter};
    // Refuse on a TTY without --yes.
    if !ctx.yes && atty_like() {
        bail!("`trash empty` requires --yes on a TTY. Pass -y to confirm.");
    }
    let filter = TrashFilter {
        older_than: older_than.map(parse_duration).transpose()?,
        kind: None,
    };
    let data_dir = paths::claudepot_data_dir();
    let freed = trash::empty(&data_dir, filter).context("empty trash")?;
    if ctx.json {
        print_json(&serde_json::json!({ "freed_bytes": freed }));
    } else {
        println!("Emptied. Freed {}.", format_size(freed));
    }
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GiB", b / GB)
    } else if b >= MB {
        format!("{:.1} MiB", b / MB)
    } else if b >= KB {
        format!("{:.1} KiB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn format_ts_ms(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string())
}

#[allow(clippy::too_many_arguments)]
pub fn slim_cmd(
    ctx: &AppContext,
    target: Option<&str>,
    all: bool,
    older_than: Option<&str>,
    larger_than: Option<&str>,
    project: Vec<String>,
    drop_over: Option<&str>,
    exclude_tool: Vec<String>,
    strip_images: bool,
    strip_documents: bool,
    execute: bool,
) -> Result<()> {
    use claudepot_core::session_slim::SlimOpts;
    let mut opts = SlimOpts {
        exclude_tools: exclude_tool,
        strip_images,
        strip_documents,
        ..SlimOpts::default()
    };
    if let Some(s) = drop_over {
        opts.drop_tool_results_over_bytes = parse_size(s)?;
    }
    if all {
        return slim_all_cmd(
            ctx,
            older_than,
            larger_than,
            project,
            &opts,
            execute,
        );
    }
    // Bulk-only filter flags are meaningless without --all. If the
    // user passed one, reject rather than silently ignore it — a
    // single-target slim that quietly drops your filter is a
    // footgun.
    let stray_filters: Vec<&str> = [
        ("--older-than", older_than.is_some()),
        ("--larger-than", larger_than.is_some()),
        ("--project", !project.is_empty()),
    ]
    .iter()
    .filter_map(|(name, set)| if *set { Some(*name) } else { None })
    .collect();
    if !stray_filters.is_empty() {
        bail!(
            "{} requires --all (filter flags are bulk-only)",
            stray_filters.join(", ")
        );
    }
    let Some(t) = target else {
        bail!("session slim requires either <target> or --all")
    };
    slim_single_cmd(ctx, t, &opts, execute)
}

fn slim_single_cmd(
    ctx: &AppContext,
    target: &str,
    opts: &claudepot_core::session_slim::SlimOpts,
    execute: bool,
) -> Result<()> {
    use claudepot_core::session_slim::{execute_slim, plan_slim};
    let path = resolve_session_path(target)?;
    let plan = plan_slim(&path, opts).context("plan slim")?;
    if !execute {
        if ctx.json {
            print_json(&plan);
            return Ok(());
        }
        println!(
            "Plan (dry-run): {} → {} ({} saved, {} tool_result redactions)",
            format_size(plan.original_bytes),
            format_size(plan.projected_bytes),
            format_size(plan.bytes_saved()),
            plan.redact_count
        );
        if opts.strip_images {
            println!("Images redacted:     {}", plan.image_redact_count);
        }
        if opts.strip_documents {
            println!("Documents redacted:  {}", plan.document_redact_count);
        }
        if !plan.tools_affected.is_empty() {
            println!("Tools affected: {}", plan.tools_affected.join(", "));
        }
        println!("Run with --execute to rewrite. Original kept in trash for 7 days.");
        return Ok(());
    }
    let data_dir = paths::claudepot_data_dir();
    let sink = claudepot_core::project_progress::NoopSink;
    let report = execute_slim(&data_dir, &path, opts, &sink).context("execute slim")?;
    if ctx.json {
        print_json(&report);
        return Ok(());
    }
    println!(
        "Slimmed: {} → {} ({} saved, {} redactions). Trash id: {}",
        format_size(report.original_bytes),
        format_size(report.final_bytes),
        format_size(report.bytes_saved()),
        report.redact_count,
        report.trashed_original.display(),
    );
    if opts.strip_images {
        println!("Images redacted:     {}", report.image_redact_count);
    }
    if opts.strip_documents {
        println!("Documents redacted:  {}", report.document_redact_count);
    }
    Ok(())
}

fn slim_all_cmd(
    ctx: &AppContext,
    older_than: Option<&str>,
    larger_than: Option<&str>,
    project: Vec<String>,
    opts: &claudepot_core::session_slim::SlimOpts,
    execute: bool,
) -> Result<()> {
    use claudepot_core::session_prune::PruneFilter;
    use claudepot_core::session_slim::{execute_slim_all, plan_slim_all};
    let filter = PruneFilter {
        older_than: older_than.map(parse_duration).transpose()?,
        larger_than: larger_than.map(parse_size).transpose()?,
        project: project.into_iter().map(std::path::PathBuf::from).collect(),
        has_error: None,
        is_sidechain: None,
    };
    let config_dir = paths::claude_config_dir();
    let plan = plan_slim_all(&config_dir, &filter, opts).context("plan slim --all")?;

    if !execute {
        if ctx.json {
            print_json(&plan);
            return Ok(());
        }
        println!("Plan (dry-run): {} session(s)", plan.entries.len());
        if opts.strip_images {
            println!("  Images to redact:     {}", plan.total_image_redacts);
        }
        if opts.strip_documents {
            println!("  Documents to redact:  {}", plan.total_document_redacts);
        }
        if opts.drop_tool_results_over_bytes < u64::MAX {
            println!("  Tool-result redacts:  {}", plan.total_tool_result_redacts);
        }
        println!("  Bytes saved:          {}", format_size(plan.total_bytes_saved));
        // Show top 10 by bytes saved.
        if !plan.entries.is_empty() {
            println!("\nTop {}:", plan.entries.len().min(10));
            for e in plan.entries.iter().take(10) {
                println!(
                    "  {:>10}  imgs={:<3} docs={:<3}  {}",
                    format_size(e.plan.bytes_saved()),
                    e.plan.image_redact_count,
                    e.plan.document_redact_count,
                    e.file_path.display()
                );
            }
        }
        // Surface matched rows that couldn't be scanned so the user
        // sees them instead of silently dropping them from the preview.
        if !plan.failed_to_plan.is_empty() {
            eprintln!(
                "\nCould not plan {} session(s) (unreadable / parse error):",
                plan.failed_to_plan.len()
            );
            for (p, err) in &plan.failed_to_plan {
                eprintln!("  {}: {err}", p.display());
            }
        }
        println!("\nRun with --execute to apply. Originals kept in trash for 7 days.");
        return Ok(());
    }
    let data_dir = paths::claudepot_data_dir();
    let sink = claudepot_core::project_progress::NoopSink;
    let report = execute_slim_all(&data_dir, &plan, opts, &sink);
    if ctx.json {
        print_json(&report);
        return Ok(());
    }
    println!(
        "Bulk slim: {} succeeded, {} skipped (live), {} failed",
        report.succeeded.len(),
        report.skipped_live.len(),
        report.failed.len()
    );
    if opts.strip_images {
        println!("Images redacted:     {}", report.total_image_redacts);
    }
    if opts.strip_documents {
        println!("Documents redacted:  {}", report.total_document_redacts);
    }
    println!("Bytes saved:         {}", format_size(report.total_bytes_saved));
    if !report.skipped_live.is_empty() {
        eprintln!("\nSkipped (still being written to):");
        for p in &report.skipped_live {
            eprintln!("  {}", p.display());
        }
    }
    if !report.failed.is_empty() {
        eprintln!("\nFailed:");
        for (p, err) in &report.failed {
            eprintln!("  {}: {err}", p.display());
        }
    }
    Ok(())
}

/// Accept either a bare UUID (looked up against the index) or an
/// absolute `.jsonl` path.
///
/// Prefix matching mirrors the email-prefix-matching contract in
/// `.claude/rules/architecture.md`: zero matches → error, exactly one
/// match → use it, more than one → error and list the ambiguous
/// candidates so the user can disambiguate.
fn resolve_session_path(target: &str) -> Result<PathBuf> {
    if target.ends_with(".jsonl") {
        let p = PathBuf::from(target);
        if !p.exists() {
            bail!("not found: {}", p.display());
        }
        return Ok(p);
    }
    // Treat as UUID — search the index.
    let cfg = paths::claude_config_dir();
    let rows = claudepot_core::session::list_all_sessions(&cfg)?;
    resolve_session_path_from_rows(target, &rows)
}

/// Pure helper for prefix resolution. Split out so it can be unit-tested
/// without touching the on-disk session index.
fn resolve_session_path_from_rows(
    target: &str,
    rows: &[claudepot_core::session::SessionRow],
) -> Result<PathBuf> {
    // Exact match short-circuits ambiguity: a full UUID is always
    // unambiguous.
    if let Some(exact) = rows.iter().find(|r| r.session_id == target) {
        return Ok(exact.file_path.clone());
    }
    let matches: Vec<&claudepot_core::session::SessionRow> = rows
        .iter()
        .filter(|r| r.session_id.starts_with(target))
        .collect();
    match matches.len() {
        0 => bail!("no session found for {target}"),
        1 => Ok(matches[0].file_path.clone()),
        n => {
            // Surface up to a handful of candidates so the user can
            // disambiguate. Avoid spamming for huge prefix matches.
            const PREVIEW: usize = 8;
            let mut msg = format!("ambiguous session id `{target}` — {n} matches:\n");
            for r in matches.iter().take(PREVIEW) {
                msg.push_str(&format!("  {}\n", r.session_id));
            }
            if n > PREVIEW {
                msg.push_str(&format!("  … and {} more\n", n - PREVIEW));
            }
            msg.push_str("Use a longer prefix or the full UUID.");
            bail!("{msg}")
        }
    }
}

fn atty_like() -> bool {
    // Used by `trash empty` to refuse without `--yes`. On a non-TTY
    // (pipe, CI, test harness) we don't demand the confirmation.
    std::io::IsTerminal::is_terminal(&std::io::stdin())
}

#[cfg(test)]
mod tests {
    use super::resolve_session_path_from_rows;
    use claudepot_core::session::{SessionRow, TokenUsage};
    use std::path::PathBuf;

    fn row(id: &str) -> SessionRow {
        SessionRow {
            session_id: id.to_string(),
            slug: "-test".to_string(),
            file_path: PathBuf::from(format!("/tmp/{id}.jsonl")),
            file_size_bytes: 0,
            last_modified: None,
            project_path: "/test".to_string(),
            project_from_transcript: false,
            first_ts: None,
            last_ts: None,
            event_count: 0,
            message_count: 0,
            user_message_count: 0,
            assistant_message_count: 0,
            first_user_prompt: None,
            models: vec![],
            tokens: TokenUsage::default(),
            git_branch: None,
            cc_version: None,
            display_slug: None,
            has_error: false,
            is_sidechain: false,
        }
    }

    #[test]
    fn test_resolve_session_path_unique_prefix_resolves() {
        let rows = vec![
            row("aaaaaaaa-1111-2222-3333-444444444444"),
            row("bbbbbbbb-1111-2222-3333-444444444444"),
        ];
        let got = resolve_session_path_from_rows("aaa", &rows).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/aaaaaaaa-1111-2222-3333-444444444444.jsonl"));
    }

    #[test]
    fn test_resolve_session_path_no_match_errors() {
        let rows = vec![row("aaaaaaaa-1111-2222-3333-444444444444")];
        let err = resolve_session_path_from_rows("zzz", &rows).unwrap_err();
        assert!(err.to_string().contains("no session found"));
    }

    #[test]
    fn test_resolve_session_path_ambiguous_prefix_errors_and_lists() {
        // Two ids share the prefix "abc". Old code returned the first
        // match (and silently slimmed the wrong transcript). New code
        // must reject the ambiguous prefix and list the candidates.
        let rows = vec![
            row("abc11111-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
            row("abc22222-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            row("dead0000-cccc-cccc-cccc-cccccccccccc"),
        ];
        let err = resolve_session_path_from_rows("abc", &rows).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ambiguous"), "msg: {msg}");
        assert!(msg.contains("abc11111"), "msg: {msg}");
        assert!(msg.contains("abc22222"), "msg: {msg}");
        // Non-matching id must NOT appear in the candidate list.
        assert!(!msg.contains("dead0000"), "msg: {msg}");
    }

    #[test]
    fn test_resolve_session_path_exact_match_wins_over_prefix() {
        // If the target is exactly equal to one id but is also a prefix
        // of another, the exact match should win unambiguously.
        let rows = vec![
            row("abc"),
            row("abcdef-something"),
        ];
        let got = resolve_session_path_from_rows("abc", &rows).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/abc.jsonl"));
    }
}

