//! `slim` verb — strip noisy events from a session in place.
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::*;

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
        return slim_all_cmd(ctx, older_than, larger_than, project, &opts, execute);
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
        println!(
            "  Bytes saved:          {}",
            format_size(plan.total_bytes_saved)
        );
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
    println!(
        "Bytes saved:         {}",
        format_size(report.total_bytes_saved)
    );
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
        assert_eq!(
            got,
            PathBuf::from("/tmp/aaaaaaaa-1111-2222-3333-444444444444.jsonl")
        );
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
        let rows = vec![row("abc"), row("abcdef-something")];
        let got = resolve_session_path_from_rows("abc", &rows).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/abc.jsonl"));
    }
}
