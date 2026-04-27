//! Project migrate — cross-machine project transport.
//!
//! See `dev-docs/project-migrate-spec.md` for the full design and
//! `dev-docs/project-migrate-cc-research.md` for verified CC source
//! citations.
//!
//! # v0 scope
//!
//! What ships in this revision:
//!   - Bundle format (`*.claudepot.tar.zst`) — write/read, integrity
//!     sha256 sidecar, manifest with self-trailer.
//!   - Path-rewrite engine — NFC normalization, multi-rule
//!     substitution table, slug recompute, file-history dual rewrite.
//!   - Conflict-resolution policy (skip / merge / replace).
//!   - Trust gates (hooks split, MCP scrubbing).
//!   - Journal + staging + undo-window primitives.
//!   - Export + import orchestrators for the simple-project cases
//!     (one project, no `--include-global`, no `--include-worktree`).
//!
//! What is deliberately deferred (with explicit `NotImplemented` errors):
//!   - Bundle encryption (`age`) — `crypto::require_plaintext_only`.
//!   - Bundle signing (`minisign`) — `crypto::require_unsigned`.
//!   - `--include-worktree` worktree.tar bundling.
//!   - Plugin re-installation on import.
//!   - macOS `com.apple.quarantine` xattr stripping.
//!   - GUI / Tauri integration.
//!   - WyHash long-path slug parity (still uses djb2 via
//!     `project_sanitize`; CC's `findProjectDir` prefix-scan handles it,
//!     see spec §5.3).
//!
//! Each deferred surface has a stub or `NotImplemented` return so
//! callers see a deliberate refusal rather than silent degradation.

pub mod apply;
pub mod bundle;
pub mod conflicts;
pub mod crypto;
pub mod error;
pub mod file_history;
pub mod manifest;
pub mod nfc;
pub mod plan;
pub mod quarantine;
pub mod rewrite;
pub mod trust;

#[cfg(test)]
mod golden_tests;

pub use error::MigrateError;

use crate::project_sanitize::sanitize_path;
use std::fs;
use std::path::{Path, PathBuf};

/// Options for `export_projects`.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub output: PathBuf,
    /// One absolute cwd per project (resolved + canonicalized at the
    /// caller; see `project_helpers::resolve_path`).
    pub project_cwds: Vec<String>,
    pub include_global: bool,
    pub include_worktree: bool,
    pub include_live: bool,
    pub include_claudepot_state: bool,
    pub include_file_history: bool,
    pub encrypt: bool,
    pub sign_keyfile: Option<String>,
}

/// Outcome of a successful export.
#[derive(Debug, Clone)]
pub struct ExportReceipt {
    pub bundle_path: PathBuf,
    pub bundle_sha256_sidecar: PathBuf,
    pub project_count: usize,
    pub file_count: usize,
}

/// Bundle one or more projects. The unit is the project; per-session
/// scoping is internal (`session_worktree::worktree_paths` enumerates
/// sibling worktree slugs automatically).
///
/// v0 path: bundles project transcripts (`~/.claude/projects/<slug>/`)
/// only. Bucket-B siblings (file-history, todos, tasks, plans,
/// session-env, security-warnings) and Bucket-C global content are
/// the next layer; spec §4.
pub fn export_projects(
    config_dir: &Path,
    opts: ExportOptions,
) -> Result<ExportReceipt, MigrateError> {
    crypto::require_plaintext_only(opts.encrypt)?;
    crypto::require_unsigned(opts.sign_keyfile.as_deref())?;

    if opts.include_worktree {
        return Err(MigrateError::NotImplemented(
            "--include-worktree (worktree.tar bundling)".to_string(),
        ));
    }
    if opts.include_global {
        return Err(MigrateError::NotImplemented(
            "--include-global (Bucket C: CLAUDE.md, agents, skills, …)".to_string(),
        ));
    }
    if opts.include_claudepot_state {
        return Err(MigrateError::NotImplemented(
            "--include-claudepot-state (account stubs, prefs, lifecycle)".to_string(),
        ));
    }

    let mut writer = bundle::BundleWriter::create(&opts.output)?;
    let mut projects = Vec::new();
    let mut file_count = 0usize;

    for cwd in &opts.project_cwds {
        let nfc_cwd = nfc::nfc(&crate::path_utils::simplify_windows_path(cwd));
        let slug = sanitize_path(&nfc_cwd);
        let slug_dir = config_dir.join("projects").join(&slug);
        if !slug_dir.exists() {
            return Err(MigrateError::ProjectNotInBundle(format!(
                "no on-disk slug for cwd {cwd} (looked for {})",
                slug_dir.display()
            )));
        }
        let project_id = uuid::Uuid::new_v4().to_string();
        let mut session_ids = Vec::new();
        let mut inventory = Vec::new();

        // Append every file under the slug as `claude/projects/<slug>/...`.
        let prefix = format!("projects/{}/claude/projects/{}", project_id, slug);
        walk_and_append(&slug_dir, &slug_dir, &prefix, &mut writer, &mut inventory)?;

        // Collect sessionIds from `*.jsonl` filenames at the slug root.
        for entry in fs::read_dir(&slug_dir).map_err(MigrateError::from)? {
            let entry = entry.map_err(MigrateError::from)?;
            if !entry.file_type().map_err(MigrateError::from)?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if let Some(stem) = s.strip_suffix(".jsonl") {
                session_ids.push(stem.to_string());
            }
        }

        let session_count = session_ids.len() as u32;
        file_count += inventory.len();

        let pm = manifest::ProjectManifest {
            id: project_id.clone(),
            source_cwd: nfc_cwd.clone(),
            // v0: cwd-keyed only. Canonical-git-root recovery via
            // `project_memory::find_canonical_git_root` is the next
            // layer; spec §5.3 documents the worktree orphan case.
            source_canonical_git_root: nfc_cwd.clone(),
            source_slug: slug.clone(),
            session_ids,
            file_inventory: inventory,
            live_at_export: false,
            worktree_set: false,
        };
        let pm_bytes = serde_json::to_vec_pretty(&pm)
            .map_err(|e| MigrateError::Serialize(e.to_string()))?;
        writer.append_bytes(
            &format!("projects/{project_id}/manifest.json"),
            &pm_bytes,
            0o644,
        )?;

        projects.push(manifest::ProjectManifestRef {
            id: project_id,
            source_cwd: nfc_cwd,
            source_slug: slug,
            session_count,
        });
    }

    let m = manifest::BundleManifest {
        schema_version: manifest::SCHEMA_VERSION,
        claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        cc_version: None,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        source_os: source_os(),
        source_arch: source_arch(),
        host_identity: hashed_host_identity(),
        source_home: home_string(),
        source_claude_config_dir: config_dir.to_string_lossy().to_string(),
        projects,
        flags: manifest::ExportFlags {
            include_global: opts.include_global,
            include_worktree: opts.include_worktree,
            include_live: opts.include_live,
            include_claudepot_state: opts.include_claudepot_state,
            include_file_history: opts.include_file_history,
            encrypted: opts.encrypt,
            signed: opts.sign_keyfile.is_some(),
        },
    };

    let project_count = m.projects.len();
    let bundle_path = writer.finalize(&m)?;
    let sidecar_path = bundle::sidecar_path_for(&bundle_path);

    Ok(ExportReceipt {
        bundle_path,
        bundle_sha256_sidecar: sidecar_path,
        project_count,
        file_count,
    })
}

/// Read a bundle's manifest without extracting. Cheap; used by the
/// `inspect` subcommand.
pub fn inspect(bundle_path: &Path) -> Result<manifest::BundleManifest, MigrateError> {
    let r = bundle::BundleReader::open(bundle_path)?;
    r.read_manifest()
}

/// Options for `import_bundle`.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub mode: conflicts::ConflictMode,
    pub prefer: Option<conflicts::MergePreference>,
    pub accept_hooks: bool,
    pub accept_mcp: bool,
    pub remap_rules: Vec<(String, String)>,
    pub include_file_history: bool,
    pub dry_run: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            mode: conflicts::ConflictMode::Skip,
            prefer: None,
            accept_hooks: false,
            accept_mcp: false,
            remap_rules: Vec::new(),
            include_file_history: true,
            dry_run: false,
        }
    }
}

/// Outcome of a successful import.
#[derive(Debug, Clone)]
pub struct ImportReceipt {
    pub bundle_id: String,
    pub projects_imported: Vec<String>,
    pub projects_refused: Vec<(String, String)>,
    pub journal_path: PathBuf,
    pub dry_run: bool,
}

/// Import a bundle. v0 implements the dry-run path end-to-end and the
/// real apply for the simplest case (no overlap, no global content).
///
/// Returns a receipt the caller can print or surface in the GUI.
pub fn import_bundle(
    config_dir: &Path,
    bundle_path: &Path,
    opts: ImportOptions,
) -> Result<ImportReceipt, MigrateError> {
    let reader = bundle::BundleReader::open(bundle_path)?;
    let manifest = reader.read_manifest()?;
    if manifest.schema_version != manifest::SCHEMA_VERSION {
        return Err(MigrateError::UnsupportedSchemaVersion {
            found: manifest.schema_version,
            expected: manifest::SCHEMA_VERSION,
        });
    }

    let bundle_id = uuid::Uuid::new_v4().to_string();
    let staging = apply::staging_dir(&bundle_id);
    let journal_path = apply::journal_path(&bundle_id);
    let mut journal = apply::ImportJournal::new(bundle_id.clone());

    let mut projects_imported = Vec::new();
    let mut projects_refused = Vec::new();

    if opts.dry_run {
        // P0+P2 only — no extraction. Caller gets the project plan
        // via `inspect`; this returns the resolution decisions.
        for pref in &manifest.projects {
            let target_cwd = pref.source_cwd.clone(); // same-machine fallback
            let target_slug = plan::target_slug(&target_cwd);
            let target_slug_dir = config_dir.join("projects").join(&target_slug);
            let conflict = if target_slug_dir.exists() {
                conflicts::ProjectConflict::PresentNoOverlap {
                    target_slug: target_slug.clone(),
                    target_session_count: 0,
                }
            } else {
                conflicts::ProjectConflict::None
            };
            match conflicts::resolve(&conflict, opts.mode, opts.prefer) {
                conflicts::Resolution::Apply
                | conflicts::Resolution::ArchiveThenApply
                | conflicts::Resolution::Merge { .. } => {
                    projects_imported.push(pref.source_cwd.clone());
                }
                conflicts::Resolution::Refuse(reason) => {
                    projects_refused.push((pref.source_cwd.clone(), reason));
                }
            }
        }
        return Ok(ImportReceipt {
            bundle_id,
            projects_imported,
            projects_refused,
            journal_path,
            dry_run: true,
        });
    }

    // P1 stage — extract.
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(MigrateError::from)?;
    }
    fs::create_dir_all(&staging).map_err(MigrateError::from)?;
    let _digests = reader.extract_all(&staging)?;

    for pref in &manifest.projects {
        // P2 plan — single-project scope; HOME and config dir rules
        // come from the manifest. Cwd rules from --remap.
        let mut table = plan::SubstitutionTable::new();
        let target_cwd = opts
            .remap_rules
            .iter()
            .find(|(s, _)| s == &pref.source_cwd)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| pref.source_cwd.clone());
        table.push(&pref.source_cwd, &target_cwd, plan::RuleOrigin::ProjectCwd);
        // HOME / config-dir rules for embedded paths.
        table.push(&manifest.source_home, &home_string(), plan::RuleOrigin::Home);
        table.push(
            &manifest.source_claude_config_dir,
            &config_dir.to_string_lossy(),
            plan::RuleOrigin::ClaudeConfigDir,
        );
        for (s, t) in &opts.remap_rules {
            if s != &pref.source_cwd {
                table.push(s, t, plan::RuleOrigin::UserRemap);
            }
        }
        table.finalize();

        // Conflict detection.
        let target_slug = plan::target_slug(&target_cwd);
        let target_slug_dir = config_dir.join("projects").join(&target_slug);
        let conflict = if target_slug_dir.exists() {
            conflicts::ProjectConflict::PresentNoOverlap {
                target_slug: target_slug.clone(),
                target_session_count: 0,
            }
        } else {
            conflicts::ProjectConflict::None
        };
        match conflicts::resolve(&conflict, opts.mode, opts.prefer) {
            conflicts::Resolution::Apply => {}
            conflicts::Resolution::Refuse(reason) => {
                projects_refused.push((pref.source_cwd.clone(), reason));
                continue;
            }
            other => {
                // Merge / archive paths land in the next slice; for v0
                // we refuse loudly so callers know to wait.
                projects_refused.push((
                    pref.source_cwd.clone(),
                    format!("v0 only supports apply (no-conflict). got: {other:?}"),
                ));
                continue;
            }
        }

        // P3 rewrite + P5 apply for the slug tree.
        let staged_slug_root = staging
            .join("projects")
            .join(&pref.id)
            .join("claude")
            .join("projects")
            .join(&pref.source_slug);
        rewrite_slug_tree(&staged_slug_root, &table)?;

        if !staged_slug_root.exists() {
            projects_refused.push((
                pref.source_cwd.clone(),
                "bundle missing expected slug tree".to_string(),
            ));
            continue;
        }
        // P5: rename staged → final. Both paths are in the same volume
        // (`~/.claudepot/imports/...` and `~/.claude/projects/...`)
        // are typically on the same FS; if not, fall back to copy.
        if let Some(parent) = target_slug_dir.parent() {
            fs::create_dir_all(parent).map_err(MigrateError::from)?;
        }
        match fs::rename(&staged_slug_root, &target_slug_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices
                || e.raw_os_error() == Some(libc::EXDEV) =>
            {
                copy_dir_recursive(&staged_slug_root, &target_slug_dir)?;
                fs::remove_dir_all(&staged_slug_root).map_err(MigrateError::from)?;
            }
            Err(e) => return Err(MigrateError::from(e)),
        }

        journal.record(apply::JournalStep {
            kind: apply::JournalStepKind::CreateFile,
            before: None,
            after: Some(target_slug_dir.to_string_lossy().to_string()),
            before_sha256: None,
            timestamp_unix_secs: now_secs(),
        });

        projects_imported.push(pref.source_cwd.clone());
    }

    journal.mark_committed();
    journal.persist(&journal_path)?;
    apply::discard_staging(&bundle_id)?;

    Ok(ImportReceipt {
        bundle_id,
        projects_imported,
        projects_refused,
        journal_path,
        dry_run: false,
    })
}

/// Walk a slug tree under `staging` and rewrite every `*.jsonl` and
/// `*.meta.json` per the substitution table. Quiet on success.
fn rewrite_slug_tree(
    slug_dir: &Path,
    table: &plan::SubstitutionTable,
) -> Result<(), MigrateError> {
    if !slug_dir.exists() {
        return Ok(());
    }
    let mut stack = vec![slug_dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).map_err(MigrateError::from)? {
            let entry = entry.map_err(MigrateError::from)?;
            let ft = entry.file_type().map_err(MigrateError::from)?;
            let p = entry.path();
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if name.ends_with(".jsonl") {
                    rewrite::rewrite_jsonl_multi(&p, table)?;
                } else if name.ends_with(".meta.json") {
                    let _ = rewrite::rewrite_json_file(&p, table)?;
                }
            }
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), MigrateError> {
    fs::create_dir_all(dst).map_err(MigrateError::from)?;
    for entry in fs::read_dir(src).map_err(MigrateError::from)? {
        let entry = entry.map_err(MigrateError::from)?;
        let ft = entry.file_type().map_err(MigrateError::from)?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ft.is_file() {
            fs::copy(&from, &to).map_err(MigrateError::from)?;
        }
    }
    Ok(())
}

fn walk_and_append(
    root: &Path,
    base: &Path,
    bundle_prefix: &str,
    writer: &mut bundle::BundleWriter,
    inventory: &mut Vec<manifest::FileInventoryEntry>,
) -> Result<(), MigrateError> {
    for entry in fs::read_dir(root).map_err(MigrateError::from)? {
        let entry = entry.map_err(MigrateError::from)?;
        let ft = entry.file_type().map_err(MigrateError::from)?;
        let path = entry.path();
        if ft.is_dir() {
            walk_and_append(&path, base, bundle_prefix, writer, inventory)?;
        } else if ft.is_file() {
            // Refuse symlinks at the source side too — match the
            // bundle-side §3.1 rule.
            if ft.is_symlink() {
                return Err(MigrateError::IntegrityViolation(format!(
                    "source contains symlink: {}",
                    path.display()
                )));
            }
            let rel = path
                .strip_prefix(base)
                .map_err(|e| {
                    MigrateError::Io(std::io::Error::other(format!("strip_prefix: {e}")))
                })?
                .to_string_lossy()
                .replace('\\', "/");
            let bundle_path = format!("{bundle_prefix}/{rel}");
            let len_before = writer.inventory().len();
            writer.append_file(&bundle_path, &path, None)?;
            // Pull the just-added entry into our local inventory copy.
            if let Some(last) = writer.inventory().get(len_before) {
                inventory.push(last.clone());
            }
        }
    }
    Ok(())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn source_os() -> String {
    if cfg!(target_os = "macos") {
        "macos".to_string()
    } else if cfg!(target_os = "linux") {
        "linux".to_string()
    } else if cfg!(target_os = "windows") {
        "windows".to_string()
    } else {
        std::env::consts::OS.to_string()
    }
}

fn source_arch() -> String {
    std::env::consts::ARCH.to_string()
}

fn home_string() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn hashed_host_identity() -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(whoami::fallible::hostname().unwrap_or_default().as_bytes());
    h.update(b"\0");
    h.update(whoami::username().as_bytes());
    h.update(b"\0");
    h.update(home_string().as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::lock_data_dir;
    use std::fs;

    fn seed_project(config_dir: &Path, cwd: &str) -> PathBuf {
        let slug = sanitize_path(&nfc::nfc(cwd));
        let slug_dir = config_dir.join("projects").join(&slug);
        fs::create_dir_all(&slug_dir).unwrap();
        let session_id = uuid::Uuid::new_v4().to_string();
        fs::write(
            slug_dir.join(format!("{session_id}.jsonl")),
            format!("{{\"cwd\":\"{cwd}\",\"slug\":\"{slug}\"}}\n"),
        )
        .unwrap();
        slug_dir
    }

    #[test]
    fn export_then_inspect_round_trips() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude");
        fs::create_dir_all(cfg.join("projects")).unwrap();
        let cwd = "/tmp/test-export-project".to_string();
        seed_project(&cfg, &cwd);

        let bundle_path = tmp.path().join("out.claudepot.tar.zst");
        let opts = ExportOptions {
            output: bundle_path.clone(),
            project_cwds: vec![cwd.clone()],
            include_global: false,
            include_worktree: false,
            include_live: false,
            include_claudepot_state: false,
            include_file_history: true,
            encrypt: false,
            sign_keyfile: None,
        };
        let receipt = export_projects(&cfg, opts).unwrap();
        assert_eq!(receipt.project_count, 1);
        assert!(receipt.file_count >= 1);
        assert!(receipt.bundle_path.exists());

        let m = inspect(&bundle_path).unwrap();
        assert_eq!(m.schema_version, manifest::SCHEMA_VERSION);
        assert_eq!(m.projects.len(), 1);
        assert_eq!(m.projects[0].source_cwd, cwd);
    }

    #[test]
    fn export_refuses_encrypt() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude");
        fs::create_dir_all(cfg.join("projects")).unwrap();
        let opts = ExportOptions {
            output: tmp.path().join("x.tar.zst"),
            project_cwds: vec!["/tmp/missing".to_string()],
            include_global: false,
            include_worktree: false,
            include_live: false,
            include_claudepot_state: false,
            include_file_history: true,
            encrypt: true,
            sign_keyfile: None,
        };
        let err = export_projects(&cfg, opts).unwrap_err();
        assert!(matches!(err, MigrateError::NotImplemented(_)));
    }

    #[test]
    fn export_refuses_unknown_project() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude");
        fs::create_dir_all(cfg.join("projects")).unwrap();
        let opts = ExportOptions {
            output: tmp.path().join("x.tar.zst"),
            project_cwds: vec!["/tmp/never-existed".to_string()],
            include_global: false,
            include_worktree: false,
            include_live: false,
            include_claudepot_state: false,
            include_file_history: true,
            encrypt: false,
            sign_keyfile: None,
        };
        let err = export_projects(&cfg, opts).unwrap_err();
        assert!(matches!(err, MigrateError::ProjectNotInBundle(_)));
    }

    #[test]
    fn import_dry_run_classifies_no_conflict() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        let cfg_src = tmp.path().join("src/.claude");
        fs::create_dir_all(cfg_src.join("projects")).unwrap();
        let cwd = "/tmp/test-import-dry".to_string();
        seed_project(&cfg_src, &cwd);
        let bundle = tmp.path().join("dry.tar.zst");
        export_projects(
            &cfg_src,
            ExportOptions {
                output: bundle.clone(),
                project_cwds: vec![cwd.clone()],
                include_global: false,
                include_worktree: false,
                include_live: false,
                include_claudepot_state: false,
                include_file_history: true,
                encrypt: false,
                sign_keyfile: None,
            },
        )
        .unwrap();

        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();
        let receipt = import_bundle(
            &cfg_target,
            &bundle,
            ImportOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(receipt.dry_run);
        assert_eq!(receipt.projects_imported.len(), 1);
        assert!(receipt.projects_refused.is_empty());
    }

    #[test]
    fn import_real_landing_for_simple_project() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("claudepot"));
        let cfg_src = tmp.path().join("src/.claude");
        fs::create_dir_all(cfg_src.join("projects")).unwrap();
        let cwd = "/tmp/test-import-real".to_string();
        seed_project(&cfg_src, &cwd);
        let bundle = tmp.path().join("real.tar.zst");
        export_projects(
            &cfg_src,
            ExportOptions {
                output: bundle.clone(),
                project_cwds: vec![cwd.clone()],
                include_global: false,
                include_worktree: false,
                include_live: false,
                include_claudepot_state: false,
                include_file_history: true,
                encrypt: false,
                sign_keyfile: None,
            },
        )
        .unwrap();

        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();
        let receipt = import_bundle(
            &cfg_target,
            &bundle,
            ImportOptions::default(),
        )
        .unwrap();
        assert!(!receipt.dry_run);
        assert_eq!(receipt.projects_imported.len(), 1);
        assert!(receipt.journal_path.exists());

        // Slug landed at target.
        let target_slug = plan::target_slug(&cwd);
        let target_dir = cfg_target.join("projects").join(&target_slug);
        assert!(target_dir.exists());
        // Sessions present.
        let mut found_jsonl = false;
        for entry in fs::read_dir(&target_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().map(|e| e == "jsonl").unwrap_or(false) {
                found_jsonl = true;
                let contents = fs::read_to_string(entry.path()).unwrap();
                // cwd preserved (no remap, same machine fallback).
                assert!(contents.contains(&cwd));
            }
        }
        assert!(found_jsonl, "expected at least one rewritten jsonl");

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }
}
