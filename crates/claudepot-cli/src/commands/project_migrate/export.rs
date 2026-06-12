//! `export` verb — write the current project's CC state to a
//! portable bundle.
//!
//! Sub-module of `commands/project_migrate.rs`; see that file's
//! header for the per-verb layout rationale and shared helpers.

use super::*;

/// Flag bundle for `claudepot export`, flattened into the
/// `Commands::Export` variant in `main.rs` (the in-tree `DraftArgs`
/// pattern). Field docs are the user-visible `--help` text.
#[derive(Debug, clap::Args)]
pub struct ExportArgs {
    /// One or more project prefixes (resolved against the user's
    /// CC project tree via the same exactly-one-prefix-match rule
    /// as `account` resolution).
    pub project_prefixes: Vec<String>,
    /// Output path. Defaults to `<host>-<YYYYMMDD>.claudepot.tar.zst`
    /// in the current working directory.
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// Include `~/.claude/CLAUDE.md`, `agents/`, `skills/`,
    /// `commands/`, scrubbed `settings.json`, plugin registry.
    /// Carries trust-gate items the importer must accept.
    #[arg(long)]
    pub include_global: bool,
    /// Tarball the project's `<cwd>/.claude/**` and `<cwd>/CLAUDE.md`
    /// alongside the session state. Use only when the project is
    /// not in git (escape hatch — git is the right transport for
    /// source).
    #[arg(long)]
    pub include_worktree: bool,
    /// Best-effort snapshot of in-flight sessions. Marks them
    /// `live_at_export: true` in the per-project manifest; the
    /// importer surfaces a banner before applying.
    #[arg(long)]
    pub include_live: bool,
    /// Include claudepot's own state (protected paths,
    /// preferences, artifact-lifecycle). NOT credentials.
    #[arg(long)]
    pub include_claudepot_state: bool,
    /// Skip the `file-history/<sid>/` dirs entirely. JSONL records
    /// still ride along; only the on-disk backups are dropped.
    #[arg(long)]
    pub no_file_history: bool,
    /// Encrypt the bundle with `age` (passphrase prompt). Default
    /// for v1; v0 ships plaintext only.
    #[arg(long)]
    pub encrypt: bool,
    /// Optional minisign signature over the manifest sha256.
    #[arg(long, value_name = "KEYFILE")]
    pub sign: Option<String>,
}

/// `project export <project-prefix>... [opts]`
pub fn export(ctx: &AppContext, args: ExportArgs) -> Result<()> {
    let ExportArgs {
        project_prefixes,
        out: output,
        include_global,
        include_worktree,
        include_live,
        include_claudepot_state,
        no_file_history,
        encrypt,
        sign: sign_keyfile,
    } = args;
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
        println!(
            "Exported {} project(s), {} file(s)",
            receipt.project_count, receipt.file_count
        );
        println!("  Bundle:  {}", receipt.bundle_path.display());
        println!("  Sidecar: {}", receipt.bundle_sha256_sidecar.display());
    }
    Ok(())
}

/// Best-effort hostname for the default bundle name. Falls back to
/// `"host"`. Reads `HOSTNAME` (set by most shells) or `COMPUTERNAME`
/// (Windows) — neither requires extra deps.
fn hostname_or_unknown() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "host".to_string())
}
