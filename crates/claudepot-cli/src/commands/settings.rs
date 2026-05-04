//! `claudepot settings` — read / write CC settings that Claudepot
//! exposes as first-class toggles.
//!
//! v1 covers only `auto-memory`. Adding more keys here should follow
//! the same shape: one core `settings_writer` helper, one CLI verb,
//! one parallel Tauri command + GUI surface.

use crate::AppContext;
use anyhow::{anyhow, Context, Result};
use claudepot_core::project_helpers::resolve_path;
use claudepot_core::settings_writer::{
    clear_auto_memory_enabled, local_settings_is_gitignored, resolve_auto_memory_enabled,
    write_auto_memory_enabled, AutoMemoryDecisionSource, SettingsLayer,
};
use serde_json::json;
use std::path::PathBuf;

fn resolve_project(project: Option<&str>) -> Result<PathBuf> {
    let raw = match project {
        Some(p) => p.to_string(),
        None => std::env::current_dir()?.to_string_lossy().into_owned(),
    };
    let resolved = resolve_path(&raw)
        .map_err(|e| anyhow!("resolve project path {}: {}", raw, e))?;
    Ok(PathBuf::from(resolved))
}

fn pick_layer(project_scope: bool) -> SettingsLayer {
    if project_scope {
        SettingsLayer::LocalProject
    } else {
        SettingsLayer::User
    }
}

fn decided_label(d: AutoMemoryDecisionSource) -> &'static str {
    match d {
        AutoMemoryDecisionSource::EnvDisable => "env: CLAUDE_CODE_DISABLE_AUTO_MEMORY",
        AutoMemoryDecisionSource::EnvSimple => "env: CLAUDE_CODE_SIMPLE",
        AutoMemoryDecisionSource::LocalProjectSettings => ".claude/settings.local.json",
        AutoMemoryDecisionSource::ProjectSettings => ".claude/settings.json",
        AutoMemoryDecisionSource::UserSettings => "~/.claude/settings.json",
        AutoMemoryDecisionSource::Default => "default (CC enables by default)",
    }
}

fn print_state(
    ctx: &AppContext,
    project_root: &std::path::Path,
    state: &claudepot_core::settings_writer::AutoMemoryState,
    gitignored: Option<bool>,
) -> Result<()> {
    if ctx.json {
        let body = json!({
            "project_root": project_root,
            "effective": state.effective,
            "decided_by": state.decided_by,
            "user_writable": state.user_writable,
            "user_settings_value": state.user_settings_value,
            "project_settings_value": state.project_settings_value,
            "local_project_settings_value": state.local_project_settings_value,
            "env_disable_set": state.env_disable_set,
            "env_simple_set": state.env_simple_set,
            "local_settings_gitignored": gitignored,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    println!(
        "auto-memory: {} ({})",
        if state.effective { "ENABLED" } else { "DISABLED" },
        decided_label(state.decided_by)
    );
    println!("project:     {}", project_root.display());
    println!();
    println!("Sources:");
    let row = |label: &str, val: Option<bool>| {
        let v = match val {
            Some(true) => "true",
            Some(false) => "false",
            None => "—",
        };
        println!("  {:<32}  {}", label, v);
    };
    row("~/.claude/settings.json", state.user_settings_value);
    row(
        ".claude/settings.json (committed)",
        state.project_settings_value,
    );
    row(
        ".claude/settings.local.json",
        state.local_project_settings_value,
    );
    if state.env_disable_set {
        println!("  CLAUDE_CODE_DISABLE_AUTO_MEMORY   set (overrides settings)");
    }
    if state.env_simple_set {
        println!("  CLAUDE_CODE_SIMPLE                set (overrides settings)");
    }
    if let Some(false) = gitignored {
        println!();
        println!(
            "Note: project's .gitignore does NOT cover settings.local.json.\n      Add `.claude/settings.local.json` (or `*.local.json`) to keep\n      this override out of commits."
        );
    }
    Ok(())
}

/// `claudepot settings auto-memory status [--project]` — print state.
pub async fn auto_memory_status(ctx: &AppContext, project: Option<&str>) -> Result<()> {
    let project_root = resolve_project(project)?;
    let state = resolve_auto_memory_enabled(&project_root);
    let gitignored = local_settings_is_gitignored(&project_root).ok();
    print_state(ctx, &project_root, &state, gitignored)
}

/// `claudepot settings auto-memory enable [--project]`.
pub async fn auto_memory_enable(
    ctx: &AppContext,
    project: Option<&str>,
    project_scope: bool,
) -> Result<()> {
    set_value(ctx, project, project_scope, true).await
}

/// `claudepot settings auto-memory disable [--project]`.
pub async fn auto_memory_disable(
    ctx: &AppContext,
    project: Option<&str>,
    project_scope: bool,
) -> Result<()> {
    set_value(ctx, project, project_scope, false).await
}

async fn set_value(
    ctx: &AppContext,
    project: Option<&str>,
    project_scope: bool,
    value: bool,
) -> Result<()> {
    let project_root = resolve_project(project)?;
    let layer = pick_layer(project_scope);
    write_auto_memory_enabled(layer, &project_root, value)
        .with_context(|| format!("write auto-memory={value} to {layer:?}"))?;
    if !ctx.quiet {
        eprintln!(
            "set autoMemoryEnabled={value} in {} ({})",
            layer.settings_file(&project_root).display(),
            match layer {
                SettingsLayer::User => "global",
                SettingsLayer::LocalProject => "per-project, local",
                SettingsLayer::Project => unreachable!(),
            }
        );
    }
    let state = resolve_auto_memory_enabled(&project_root);
    let gitignored = if matches!(layer, SettingsLayer::LocalProject) {
        local_settings_is_gitignored(&project_root).ok()
    } else {
        None
    };
    print_state(ctx, &project_root, &state, gitignored)
}

/// `claudepot settings auto-memory clear [--project]` — remove the
/// `autoMemoryEnabled` key from the relevant settings file. Lets the
/// next-higher layer take over.
pub async fn auto_memory_clear(
    ctx: &AppContext,
    project: Option<&str>,
    project_scope: bool,
) -> Result<()> {
    let project_root = resolve_project(project)?;
    let layer = pick_layer(project_scope);
    clear_auto_memory_enabled(layer, &project_root)
        .with_context(|| format!("clear auto-memory in {layer:?}"))?;
    if !ctx.quiet {
        eprintln!(
            "cleared autoMemoryEnabled from {}",
            layer.settings_file(&project_root).display()
        );
    }
    let state = resolve_auto_memory_enabled(&project_root);
    print_state(ctx, &project_root, &state, None)
}
