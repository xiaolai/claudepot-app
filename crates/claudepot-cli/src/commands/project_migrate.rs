//! `claudepot project export | import | migrate inspect | migrate undo`
//!
//! Thin CLI adapter over `claudepot_core::migrate`. Per
//! `.claude/rules/commands.md`, this is a verb-group sibling of the
//! existing `project.rs` because the migrate verbs share helpers
//! (substitution-rule parsing, JSON output shape) and would otherwise
//! fragment the project-noun namespace.
//!
//! Per-verb gates (live-session detection, journal nags) reuse the
//! same machinery as `project move` — see spec §9.

use crate::AppContext;
use anyhow::{anyhow, Result};
use claudepot_core::migrate::{
    self, conflicts, ExportOptions, ImportOptions, MigrateError,
};
use claudepot_core::paths;
use claudepot_core::project_helpers::resolve_path;
use std::path::PathBuf;

/// Parse `--remap source=target` repeatedly into pairs. Empty value
/// passes through cleanly so the flag is optional.
pub fn parse_remap(values: &[String]) -> Result<Vec<(String, String)>> {
    values
        .iter()
        .map(|s| {
            s.split_once('=')
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .ok_or_else(|| anyhow!("invalid --remap value (need source=target): {s}"))
        })
        .collect()
}

/// `project export <project-prefix>... [opts]`
#[allow(clippy::too_many_arguments)]
pub fn export(
    ctx: &AppContext,
    project_prefixes: Vec<String>,
    output: Option<PathBuf>,
    include_global: bool,
    include_worktree: bool,
    include_live: bool,
    include_claudepot_state: bool,
    no_file_history: bool,
    encrypt: bool,
    sign_keyfile: Option<String>,
) -> Result<()> {
    if project_prefixes.is_empty() {
        return Err(anyhow!("at least one project-prefix is required"));
    }
    let config_dir = paths::claude_config_dir();

    // Resolve each prefix to an absolute cwd. Cross-OS shape handled
    // by `resolve_path` (which calls `simplify_windows_path` first).
    let mut project_cwds = Vec::new();
    for prefix in &project_prefixes {
        let resolved = resolve_path(prefix)?;
        project_cwds.push(resolved);
    }

    let output = output.unwrap_or_else(|| {
        // Default filename: `<host>-<YYYYMMDD>.claudepot.tar.zst`. We
        // pull hostname from libc rather than adding `whoami` to the
        // CLI's dep set — the CLI already pulls libc transitively.
        let host = hostname_or_unknown();
        let date = chrono::Utc::now().format("%Y%m%d");
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(format!("{host}-{date}.claudepot.tar.zst"))
    });

    let opts = ExportOptions {
        output,
        project_cwds: project_cwds.clone(),
        include_global,
        include_worktree,
        include_live,
        include_claudepot_state,
        include_file_history: !no_file_history,
        encrypt,
        sign_keyfile,
    };

    let receipt = migrate::export_projects(&config_dir, opts).map_err(map_migrate_err)?;

    if ctx.json {
        let v = serde_json::json!({
            "bundle_path": receipt.bundle_path.to_string_lossy(),
            "sidecar_sha256": receipt.bundle_sha256_sidecar.to_string_lossy(),
            "project_count": receipt.project_count,
            "file_count": receipt.file_count,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("Exported {} project(s), {} file(s)", receipt.project_count, receipt.file_count);
        println!("  Bundle:  {}", receipt.bundle_path.display());
        println!("  Sidecar: {}", receipt.bundle_sha256_sidecar.display());
    }
    Ok(())
}

/// `project import <bundle> [opts]`
#[allow(clippy::too_many_arguments)]
pub fn import(
    ctx: &AppContext,
    bundle: PathBuf,
    mode: ConflictModeArg,
    prefer: Option<MergePreferenceArg>,
    accept_hooks: bool,
    accept_mcp: bool,
    remap: Vec<String>,
    no_file_history: bool,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    if matches!(mode, ConflictModeArg::Replace) && !yes {
        return Err(anyhow!(
            "--mode=replace requires --yes (this will archive target slugs to claudepot trash)"
        ));
    }

    let config_dir = paths::claude_config_dir();
    let opts = ImportOptions {
        mode: mode.into(),
        prefer: prefer.map(Into::into),
        accept_hooks,
        accept_mcp,
        remap_rules: parse_remap(&remap)?,
        include_file_history: !no_file_history,
        dry_run,
    };

    let receipt = migrate::import_bundle(&config_dir, &bundle, opts).map_err(map_migrate_err)?;

    if ctx.json {
        let v = serde_json::json!({
            "bundle_id": receipt.bundle_id,
            "dry_run": receipt.dry_run,
            "projects_imported": receipt.projects_imported,
            "projects_refused": receipt.projects_refused,
            "journal_path": receipt.journal_path.to_string_lossy(),
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else if receipt.dry_run {
        println!("Dry run: {} project(s) would import", receipt.projects_imported.len());
        for cwd in &receipt.projects_imported {
            println!("  ✓ {cwd}");
        }
        for (cwd, reason) in &receipt.projects_refused {
            println!("  ✗ {cwd}: {reason}");
        }
    } else {
        println!(
            "Imported {} project(s); journal {}",
            receipt.projects_imported.len(),
            receipt.journal_path.display()
        );
        for cwd in &receipt.projects_imported {
            println!("  ✓ {cwd}");
        }
        if !receipt.projects_refused.is_empty() {
            println!("Refused {} project(s):", receipt.projects_refused.len());
            for (cwd, reason) in &receipt.projects_refused {
                println!("  ✗ {cwd}: {reason}");
            }
        }
        println!("Undo within 24h: claudepot project migrate undo");
    }
    Ok(())
}

/// `project migrate inspect <bundle>`
pub fn inspect(ctx: &AppContext, bundle: PathBuf, upgrade_schema: bool) -> Result<()> {
    if upgrade_schema {
        return Err(anyhow!(
            "--upgrade-schema not yet implemented (no older schema versions exist)"
        ));
    }
    let manifest = migrate::inspect(&bundle).map_err(map_migrate_err)?;
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
    } else {
        println!("Bundle: {}", bundle.display());
        println!("  schema_version:  {}", manifest.schema_version);
        println!("  claudepot:       {}", manifest.claudepot_version);
        println!("  source_os:       {} / {}", manifest.source_os, manifest.source_arch);
        println!("  created_at:      {}", manifest.created_at);
        println!(
            "  flags:           global={} worktree={} live={} state={} fhist={} enc={} sig={}",
            manifest.flags.include_global,
            manifest.flags.include_worktree,
            manifest.flags.include_live,
            manifest.flags.include_claudepot_state,
            manifest.flags.include_file_history,
            manifest.flags.encrypted,
            manifest.flags.signed,
        );
        println!("  projects ({})", manifest.projects.len());
        for p in &manifest.projects {
            println!("    - {} ({} sessions) slug={}", p.source_cwd, p.session_count, p.source_slug);
        }
    }
    Ok(())
}

/// `project migrate undo` — v0 stub. Reads the most recent journal
/// inside the 24h undo window and reports it; the actual reverse-
/// LIFO replay lands with the rollback layer.
pub fn undo(ctx: &AppContext) -> Result<()> {
    let (journals_dir, _, _) = paths::claudepot_repair_dirs();
    if !journals_dir.exists() {
        if ctx.json {
            println!("{}", serde_json::json!({"undone": false, "reason": "no journals"}));
        } else {
            println!("No import journals to undo.");
        }
        return Ok(());
    }
    let mut newest: Option<(PathBuf, u64)> = None;
    for entry in std::fs::read_dir(&journals_dir)? {
        let entry = entry?;
        let p = entry.path();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if !name.starts_with("import-") {
            continue;
        }
        let m = entry.metadata()?;
        let mtime = m.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if newest.as_ref().is_none_or(|(_, t)| mtime > *t) {
            newest = Some((p, mtime));
        }
    }
    let Some((path, _)) = newest else {
        if ctx.json {
            println!("{}", serde_json::json!({"undone": false, "reason": "no recent imports"}));
        } else {
            println!("No recent import journals.");
        }
        return Ok(());
    };
    let journal = migrate::apply::ImportJournal::load(&path).map_err(map_migrate_err)?;
    if !migrate::apply::within_undo_window(&journal) {
        return Err(anyhow!(
            "most recent import journal is older than 24h — outside undo window: {}",
            path.display()
        ));
    }
    // v0: the rollback machinery is the next layer. Surface the
    // journal so the user can see what would have been reversed.
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "undone": false,
                "reason": "rollback engine deferred to next slice",
                "journal": &journal,
            })
        );
    } else {
        println!("Most recent import journal: {}", path.display());
        println!("  bundle_id:  {}", journal.bundle_id);
        println!("  steps:      {}", journal.steps.len());
        println!("  committed:  {}", journal.committed);
        println!();
        println!(
            "Note: the rollback engine is deferred — this command currently \
             reports the journal so the user can audit it. Reverse-LIFO \
             replay lands with apply v1."
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------
// CLI enums — kept here so main.rs can derive `clap::ValueEnum` cleanly.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ConflictModeArg {
    Skip,
    Merge,
    Replace,
}

impl From<ConflictModeArg> for conflicts::ConflictMode {
    fn from(a: ConflictModeArg) -> Self {
        match a {
            ConflictModeArg::Skip => Self::Skip,
            ConflictModeArg::Merge => Self::Merge,
            ConflictModeArg::Replace => Self::Replace,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum MergePreferenceArg {
    Imported,
    Target,
}

impl From<MergePreferenceArg> for conflicts::MergePreference {
    fn from(a: MergePreferenceArg) -> Self {
        match a {
            MergePreferenceArg::Imported => Self::Imported,
            MergePreferenceArg::Target => Self::Target,
        }
    }
}

fn map_migrate_err(e: MigrateError) -> anyhow::Error {
    anyhow!("{e}")
}

/// Best-effort hostname for the default bundle name. Falls back to
/// `"host"`. Reads `HOSTNAME` (set by most shells) or `COMPUTERNAME`
/// (Windows) — neither requires extra deps.
fn hostname_or_unknown() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "host".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remap_well_formed() {
        let v = parse_remap(&["/a=/b".to_string(), "C:\\x=/y".to_string()]).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], ("/a".to_string(), "/b".to_string()));
        assert_eq!(v[1], ("C:\\x".to_string(), "/y".to_string()));
    }

    #[test]
    fn parse_remap_rejects_missing_separator() {
        let r = parse_remap(&["bad".to_string()]);
        assert!(r.is_err());
    }
}
