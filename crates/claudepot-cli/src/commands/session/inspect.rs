//! `view` verb (multi-flavor) — chunks, tools, classify, subagents, phases, context.
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::*;

pub(super) fn resolve_detail(target: &str) -> Result<SessionDetail> {
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

