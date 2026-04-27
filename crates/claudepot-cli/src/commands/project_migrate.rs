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
use claudepot_core::account::AccountStore;
use claudepot_core::migrate::{
    self, conflicts, state as migrate_state, ExportOptions, ImportOptions, MigrateError,
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

    // When --include-claudepot-state is set, pull account stubs from
    // the local store. Per spec §16 Q2 we only carry the
    // (uuid, email, org, verification shape); never tokens.
    let account_stubs = if include_claudepot_state {
        let data_dir = paths::claudepot_data_dir();
        let store = AccountStore::open(&data_dir.join("accounts.db"))?;
        Some(migrate_state::account_stubs_from_store(&store).map_err(map_migrate_err)?)
    } else {
        None
    };

    // Passphrase resolution. The CLI honors `CLAUDEPOT_PASSPHRASE`
    // (env var) for non-interactive flows; otherwise tty prompt would
    // be ideal but we don't ship `rpassword` in this slice — the CLI
    // refuses cleanly so the user can `--no-encrypt` or set the env.
    let encrypt_passphrase = if encrypt {
        match std::env::var("CLAUDEPOT_PASSPHRASE") {
            Ok(s) if !s.is_empty() => Some(migrate::SecretString::from(s)),
            _ => {
                return Err(anyhow!(
                    "encryption requested; set CLAUDEPOT_PASSPHRASE or pass --no-encrypt"
                ));
            }
        }
    } else {
        None
    };
    let sign_password = std::env::var("CLAUDEPOT_SIGN_PASSWORD").ok();

    let opts = ExportOptions {
        output,
        project_cwds: project_cwds.clone(),
        include_global,
        include_worktree,
        include_live,
        include_claudepot_state,
        include_file_history: !no_file_history,
        encrypt,
        encrypt_passphrase,
        sign_keyfile,
        sign_password,
        account_stubs,
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
    let decrypt_passphrase = std::env::var("CLAUDEPOT_PASSPHRASE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(migrate::SecretString::from);
    let opts = ImportOptions {
        mode: mode.into(),
        prefer: prefer.map(Into::into),
        accept_hooks,
        accept_mcp,
        remap_rules: parse_remap(&remap)?,
        include_file_history: !no_file_history,
        dry_run,
        decrypt_passphrase,
        verify_key: None,
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
        if !receipt.accounts_listed.is_empty() {
            println!(
                "Source machine had {} account(s) — re-login here to use them \
                 (no credentials traveled):",
                receipt.accounts_listed.len()
            );
            for stub in &receipt.accounts_listed {
                println!("  - {} (verify: {})", stub.email, stub.verify_status);
            }
        }
        println!("Undo within 24h: claudepot migrate undo");
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
    // Encrypted bundles route through the inspect_encrypted helper.
    // Passphrase resolution mirrors `import` — env first, no
    // interactive prompt (the CLI hasn't pulled in `rpassword` yet).
    let is_encrypted = bundle.extension().is_some_and(|e| e == "age");
    let manifest = if is_encrypted {
        let pwd = std::env::var("CLAUDEPOT_PASSPHRASE")
            .ok()
            .filter(|s| !s.is_empty())
            .map(migrate::SecretString::from)
            .ok_or_else(|| {
                anyhow!(
                    "encrypted bundle: set CLAUDEPOT_PASSPHRASE to inspect"
                )
            })?;
        migrate::inspect_encrypted(&bundle, &pwd).map_err(map_migrate_err)?
    } else {
        migrate::inspect(&bundle).map_err(map_migrate_err)?
    };
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

/// `project migrate undo` — reverse the most recent import within
/// the 24h undo window. LIFO journal replay; per-step tamper detection.
pub fn undo(ctx: &AppContext) -> Result<()> {
    let receipt = migrate::import_undo().map_err(map_migrate_err)?;
    if ctx.json {
        let v = serde_json::json!({
            "bundle_id": receipt.bundle_id,
            "journal_path": receipt.journal_path.to_string_lossy(),
            "counter_journal_path": receipt.counter_journal_path.to_string_lossy(),
            "steps_reversed": receipt.steps_reversed,
            "steps_tampered": receipt.steps_tampered,
            "steps_errored": receipt.steps_errored,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("Undo of import {}:", receipt.bundle_id);
        println!("  steps reversed:  {}", receipt.steps_reversed);
        if !receipt.steps_tampered.is_empty() {
            println!("  skipped (post-apply tamper):");
            for s in &receipt.steps_tampered {
                println!("    - {s}");
            }
        }
        if !receipt.steps_errored.is_empty() {
            println!("  errors:");
            for s in &receipt.steps_errored {
                println!("    - {s}");
            }
        }
        println!("  counter-journal: {}", receipt.counter_journal_path.display());
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
