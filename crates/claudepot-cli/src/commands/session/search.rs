//! `search` and `worktrees` verbs.
//!
//! Sub-module of `commands/session.rs`; see that file's header for
//! the verb-group rationale and the shared formatting helpers.

use super::*;

pub fn search_cmd(ctx: &AppContext, query: &str, limit: usize) -> Result<()> {
    let cfg = paths::claude_config_dir();
    // Warm path: refresh the persistent index and read rows from it —
    // the fold cost is paid only on changed transcripts, unlike the
    // full-reparse `session::list_all_sessions`.
    let db_path = paths::claudepot_data_dir().join("sessions.db");
    let idx = claudepot_core::session_index::SessionIndex::open(&db_path)
        .context("open session index")?;
    let rows = idx.list_all(&cfg).context("list sessions for search")?;
    let hits = claudepot_core::session_search::search_rows(&rows, query, limit)
        .context("search sessions")?;
    if ctx.json {
        print_json(&hits)?;
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
    // Same warm path as `search_cmd` above.
    let db_path = paths::claudepot_data_dir().join("sessions.db");
    let idx = claudepot_core::session_index::SessionIndex::open(&db_path)
        .context("open session index")?;
    let rows = idx
        .list_all(&cfg)
        .context("list sessions for worktree grouping")?;
    let groups = claudepot_core::session_worktree::group_by_repo(rows);
    if ctx.json {
        print_json(&groups)?;
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
