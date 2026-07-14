//! `plugin-bindings` verb — read-only health check over the global
//! plugin registry (`~/.claude/plugins/installed_plugins.json`).
//!
//! Reports project-scoped plugin bindings whose `projectPath` no longer
//! exists on disk — the plugin-side symptom of a project moved outside
//! `claudepot project move` (Finder / `mv` / `git`). It is the plugin
//! dimension that `session list-orphans` (transcripts only) doesn't
//! cover. The fix it points at — `claudepot project move <old> <new>` —
//! runs P10 and repoints the bindings.
//!
//! Sub-module of `commands/project.rs`; see that file's header.

use super::*;

pub fn plugin_bindings(ctx: &AppContext) -> Result<()> {
    let config_dir = paths::claude_config_dir();
    let registry = config_dir.join("plugins").join("installed_plugins.json");
    let stale = project::detect_stale_plugin_bindings(&registry)
        .context("failed to scan installed_plugins.json")?;

    if ctx.json {
        // Emit a stable, scriptable shape.
        let rows: Vec<_> = stale
            .iter()
            .map(|b| {
                serde_json::json!({
                    "plugin": b.plugin,
                    "project_path": b.project_path,
                    "scope": b.scope,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    if stale.is_empty() {
        println!("All project-scoped plugin bindings resolve to existing directories.");
        return Ok(());
    }

    println!(
        "{} stale plugin binding{} \u{2014} projectPath no longer exists on disk:",
        stale.len(),
        if stale.len() == 1 { "" } else { "s" }
    );
    println!();
    for b in &stale {
        println!("  {} [{}]", b.plugin, b.scope);
        println!("    \u{21b3} {}", b.project_path);
    }
    println!();
    println!(
        "These plugins were installed for a project directory that is gone \
         (moved or deleted)."
    );
    println!(
        "If the project MOVED, repoint every binding by running:\n  \
         claudepot project move <old-path> <new-path>"
    );

    Ok(())
}
