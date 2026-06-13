//! `inspect` verb — read a bundle's manifest without extracting.
//!
//! Sub-module of `commands/project_migrate.rs`; see that file's
//! header for the per-verb layout rationale and shared helpers.

use super::*;

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
            .ok_or_else(|| anyhow!("encrypted bundle: set CLAUDEPOT_PASSPHRASE to inspect"))?;
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
        println!(
            "  source_os:       {} / {}",
            manifest.source_os, manifest.source_arch
        );
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
            println!(
                "    - {} ({} sessions) slug={}",
                p.source_cwd, p.session_count, p.source_slug
            );
        }
    }
    Ok(())
}
