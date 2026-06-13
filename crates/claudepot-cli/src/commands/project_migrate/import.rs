//! `import` verb — apply a bundle to this machine, plus the
//! conflict-mode / merge-preference value enums its flags use.
//!
//! Sub-module of `commands/project_migrate.rs`; see that file's
//! header for the per-verb layout rationale and shared helpers.

use super::*;

/// Flag bundle for `claudepot import`, flattened into the
/// `Commands::Import` variant in `main.rs`.
#[derive(Debug, clap::Args)]
pub struct ImportArgs {
    /// Path to the bundle file.
    pub bundle: PathBuf,
    /// Conflict-resolution mode. Default is `skip` (refuse on any
    /// pre-existing slug); `merge` unions sessions; `replace`
    /// archives the target slug to claudepot trash first
    /// (requires `--yes`).
    #[arg(long, value_enum, default_value_t = ConflictModeArg::Skip)]
    pub mode: ConflictModeArg,
    /// In `--mode=merge`, pick a side for any session-id collision.
    #[arg(long, value_enum)]
    pub prefer: Option<MergePreferenceArg>,
    /// Opt in to all bundled hooks (still printed to stderr).
    #[arg(long)]
    pub accept_hooks: bool,
    /// Opt in to all needs-resolution MCP entries.
    #[arg(long)]
    pub accept_mcp: bool,
    /// Override path substitution rule. Repeatable.
    #[arg(long, value_name = "SOURCE=TARGET")]
    pub remap: Vec<String>,
    /// Import without repathing file-history (records ride along
    /// but visual diffs for old turns are degraded).
    #[arg(long)]
    pub no_file_history: bool,
    /// Plan only — don't apply.
    #[arg(long)]
    pub dry_run: bool,
}

/// `project import <bundle> [opts]`
pub fn import(ctx: &AppContext, args: ImportArgs) -> Result<()> {
    let ImportArgs {
        bundle,
        mode,
        prefer,
        accept_hooks,
        accept_mcp,
        remap,
        no_file_history,
        dry_run,
    } = args;
    if matches!(mode, ConflictModeArg::Replace) && !ctx.yes {
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
        println!(
            "Dry run: {} project(s) would import",
            receipt.projects_imported.len()
        );
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
