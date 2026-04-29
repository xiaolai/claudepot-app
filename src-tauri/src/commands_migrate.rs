//! Tauri commands for project migrate (export / inspect / import / undo).
//!
//! See `dev-docs/project-migrate-spec.md` §12.3 for the surface contract.
//!
//! Symmetry note: the synchronous shape of these commands mirrors
//! `commands_session_share` and `commands_session_prune` — neither uses
//! `spawn_op_thread` because the user-perceived bundle write is fast
//! enough on real-world data (a few hundred MB, mostly compressible
//! JSONL) that progress streaming is not yet warranted. When that
//! changes, lift these into the `op-progress::<op_id>` pipeline that
//! `commands_repair` and `commands_session_move` use.

use crate::dto_migrate::{
    ExportArgsDto, ExportReceiptDto, ImportArgsDto, ImportPlanDto, ImportReceiptDto,
    InspectArgsDto, UndoReceiptDto,
};
use claudepot_core::account::AccountStore;
use claudepot_core::migrate::{
    self, conflicts as mc, state as migrate_state, ExportOptions, ImportOptions, SecretString,
};
use claudepot_core::paths;
use claudepot_core::project_helpers::resolve_path;
use std::path::PathBuf;
use zeroize::Zeroize;

/// RAII helper: takes ownership of a plain `String` carrying a secret
/// (passphrase) that arrived over IPC, hands the `SecretString` (which
/// itself zeroes on drop) to the caller, and zeroes any remaining
/// owned copies on every exit path.
///
/// We can't change the wire shape — Tauri serde-deserializes JSON
/// `string` values into plain `String` heap allocations before the
/// command body runs. The earliest we can act is here, where we
/// immediately convert the bytes into a zeroize-on-drop wrapper and
/// scrub the original.
fn promote_secret(mut raw: Option<String>) -> Option<SecretString> {
    let s = raw.take()?;
    let secret = SecretString::from(s.clone());
    // Zeroize the leftover plain String before returning. `s.clone()`
    // forced a fresh allocation that SecretString now owns; the
    // original raw allocation goes away here.
    let mut to_clear = s;
    to_clear.zeroize();
    Some(secret)
}

/// Inspect a bundle's manifest. Encrypted bundles need a passphrase.
#[tauri::command]
pub async fn migrate_inspect(args: InspectArgsDto) -> Result<ImportPlanDto, String> {
    let bundle = PathBuf::from(args.bundle_path);
    if bundle.extension().map(|e| e == "age").unwrap_or(false) {
        let pwd = promote_secret(args.passphrase)
            .ok_or_else(|| "encrypted bundle requires passphrase".to_string())?;
        let m = migrate::inspect_encrypted(&bundle, &pwd).map_err(|e| e.to_string())?;
        return Ok(m.into());
    }
    let m = migrate::inspect(&bundle).map_err(|e| e.to_string())?;
    Ok(m.into())
}

/// Bundle one or more projects.
///
/// Resolves project prefixes to absolute cwds via the same rule as the
/// CLI (`project_helpers::resolve_path`). Account stubs are pulled from
/// the local store only when `include_claudepot_state` is true.
#[tauri::command]
pub async fn migrate_export(args: ExportArgsDto) -> Result<ExportReceiptDto, String> {
    let config_dir = paths::claude_config_dir();

    let mut project_cwds = Vec::with_capacity(args.project_prefixes.len());
    for prefix in &args.project_prefixes {
        let resolved = resolve_path(prefix).map_err(|e| e.to_string())?;
        project_cwds.push(resolved);
    }

    let account_stubs = if args.include_claudepot_state {
        let data_dir = paths::claudepot_data_dir();
        let store = AccountStore::open(&data_dir.join("accounts.db")).map_err(|e| e.to_string())?;
        Some(migrate_state::account_stubs_from_store(&store).map_err(|e| e.to_string())?)
    } else {
        None
    };

    // Move plain-String secrets through `promote_secret` so the JSON-
    // deserialized String allocations are zeroed as soon as we have a
    // SecretString copy. `sign_password` lives as a plain String per
    // the minisign API; we still zeroize the local after passing.
    let encrypt_passphrase = promote_secret(args.encrypt_passphrase);
    let mut sign_password_raw = args.sign_password;
    let sign_password = sign_password_raw.clone();
    if let Some(s) = sign_password_raw.as_mut() {
        s.zeroize();
    }
    let opts = ExportOptions {
        output: PathBuf::from(args.output_path),
        project_cwds,
        include_global: args.include_global,
        include_worktree: args.include_worktree,
        include_live: args.include_live,
        include_claudepot_state: args.include_claudepot_state,
        include_file_history: !args.no_file_history,
        encrypt: args.encrypt,
        encrypt_passphrase,
        sign_keyfile: args.sign_keyfile,
        sign_password,
        account_stubs,
    };
    let receipt = migrate::export_projects(&config_dir, opts).map_err(|e| e.to_string())?;
    Ok(receipt.into())
}

/// Import a bundle.
#[tauri::command]
pub async fn migrate_import(args: ImportArgsDto) -> Result<ImportReceiptDto, String> {
    let config_dir = paths::claude_config_dir();
    let mode = match args.mode.as_str() {
        "skip" => mc::ConflictMode::Skip,
        "merge" => mc::ConflictMode::Merge,
        "replace" => mc::ConflictMode::Replace,
        other => return Err(format!("unknown mode: {other}")),
    };
    let prefer = match args.prefer.as_deref() {
        Some("imported") => Some(mc::MergePreference::Imported),
        Some("target") => Some(mc::MergePreference::Target),
        Some(other) => return Err(format!("unknown prefer: {other}")),
        None => None,
    };
    let opts = ImportOptions {
        mode,
        prefer,
        accept_hooks: args.accept_hooks,
        accept_mcp: args.accept_mcp,
        remap_rules: args
            .remap
            .into_iter()
            .map(|p| (p.source, p.target))
            .collect(),
        include_file_history: !args.no_file_history,
        dry_run: args.dry_run,
        decrypt_passphrase: promote_secret(args.passphrase),
        verify_key: args.verify_key_path.map(PathBuf::from),
    };
    let receipt =
        migrate::import_bundle(&config_dir, std::path::Path::new(&args.bundle_path), opts)
            .map_err(|e| e.to_string())?;
    Ok(receipt.into())
}

/// Undo the most recent import within the 24h window.
#[tauri::command]
pub async fn migrate_undo() -> Result<UndoReceiptDto, String> {
    let r = migrate::import_undo().map_err(|e| e.to_string())?;
    Ok(r.into())
}
