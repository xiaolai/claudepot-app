//! Project migrate — cross-machine project transport.
//!
//! See `dev-docs/project-migrate-spec.md` for the full design and
//! `dev-docs/project-migrate-cc-research.md` for verified CC source
//! citations.
//!
//! # Capability matrix
//!
//! What ships:
//!   - Bundle format (`*.claudepot.tar.zst`): writer + reader, outer
//!     sidecar sha256 (REQUIRED at open), inner `integrity.sha256`
//!     (cross-checked against on-disk after extract), manifest with
//!     self-trailer (REQUIRED).
//!   - Path-rewrite engine: NFC normalization on both sides of every
//!     substitution rule AND on lookups, multi-rule table with
//!     longest-prefix-wins, slug recompute, target-native separator
//!     coercion on cross-OS rewrites, file-history dual rewrite
//!     (rename on disk + JSONL `Record` key rewrite).
//!   - Conflict-resolution policy (skip / merge / replace + prefer-
//!     imported|target).
//!   - Trust gates: hooks split out into `proposed-hooks.json`,
//!     MCP absolute-path commands flagged `needs_resolution`.
//!   - Apply pipeline P0-P5 + P8: integrity verify, stage, plan,
//!     rewrite, lock (global import lock via `project_lock`), apply,
//!     release. Per-step `after_sha256` populated for tamper
//!     detection on undo.
//!   - Surgical CreateDir rollback: removes only journaled files,
//!     surfaces user-added survivors as `skipped_tampered`.
//!   - Counter-journal on undo (replays the inverse so undo-of-undo
//!     is well-defined within the 24h window).
//!   - `age` passphrase encryption + `minisign` signing/verification.
//!   - `--include-global` (Bucket C: CLAUDE.md, agents/, skills/,
//!     commands/, plugins registry, scrubbed settings/mcp).
//!   - `--include-worktree` (project-scoped settings + CLAUDE.md;
//!     `settings.local.json` and `managed-settings.*` always
//!     excluded).
//!   - `--include-claudepot-state` (account stubs without secrets,
//!     prefs, lifecycle; account stubs are LOG-ONLY per §16 Q2).
//!
//! What returns `NotImplemented` or `Configuration` (loud refusal,
//! not silent degradation):
//!   - `--include-live` — spec §9.1 retry semantics.
//!   - `--upgrade-schema` — no older schema versions exist yet.
//!   - Encrypted export without a passphrase — `Configuration` (the
//!     feature ships; the user must supply the passphrase).
//!
//! Known partial coverage / spec gaps (documented here so the
//! mismatch is visible — these are tracked in the audit follow-up):
//!   - Slug derivation uses cwd, not the canonical git root. Worktrees
//!     of one repo end up under separate slugs instead of sharing one.
//!     Spec §5.3 calls for `findCanonicalGitRoot`; we still do
//!     `sanitize_path(cwd)`. Auto-memory paths drift accordingly.
//!   - Session enumeration walks one slug, not the worktree set.
//!     Sibling-worktree sessions can fall outside the export.
//!   - Conflict resolution lands `Apply`. `Merge` and `ArchiveThenApply`
//!     are dispatched to the loud refusal path inside `import_bundle`.
//!   - Bucket B (file-history dirs, todos/tasks/plans/session-env,
//!     security_warnings_state) is partially implemented (file-history
//!     repath only).
//!   - `~/.claude.json` projects-map fragment merge (§5.6) and
//!     `history.jsonl` fragment dedupe (§5.7) are not yet wired.
//!   - Plugin re-install on import, `--accept-mcp` write-side, and
//!     statusline-script trust gating live in `global` as TODO.
//!   - Tauri commands return synchronously; the spec §12.3 op-id
//!     progress channel is not wired.
//!   - WyHash long-path slug parity — still djb2 (CC's
//!     `findProjectDir` prefix-scan tolerates it, spec §5.3).
//!   - macOS `com.apple.quarantine` xattr stripping is a no-op stub.

// Public surface exposed across crate boundaries (CLI / Tauri use
// these). The other modules are internal implementation detail.
pub mod apply;
pub mod conflicts;
pub mod manifest;
pub mod state;

// Internal-only — the orchestrator wires these together. External
// callers should use the public surface re-exported below
// (`MigrateError`, `SecretString`, `export_projects`, `import_bundle`,
// `inspect`, `inspect_encrypted`, `import_undo`).
pub(crate) mod bundle;
pub(crate) mod crypto;
pub(crate) mod error;
pub(crate) mod file_history;
pub(crate) mod global;
pub(crate) mod nfc;
pub(crate) mod plan;
pub(crate) mod quarantine;
pub(crate) mod rewrite;
pub(crate) mod trust;
pub(crate) mod worktree;

#[cfg(test)]
mod golden_tests;

pub use error::MigrateError;

/// Re-exported so adapters (CLI, Tauri) can build passphrases without
/// adding `age` to their own dep set.
pub use age::secrecy::SecretString;

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
    /// Passphrase to use when `encrypt: true`. Required for non-
    /// interactive callers; CLI prompts when `None` AND a tty is
    /// available. Held as `SecretString` so `Drop` zeroes it.
    pub encrypt_passphrase: Option<age::secrecy::SecretString>,
    pub sign_keyfile: Option<String>,
    /// Optional secret-key passphrase for `sign_keyfile`. Pass
    /// `Some("")` for unprotected keys; `None` falls back to empty
    /// (no interactive prompt — see `crypto::sign_bundle`).
    pub sign_password: Option<String>,
    /// Optional account stubs to include when
    /// `include_claudepot_state` is true. Caller pre-builds via
    /// `state::account_stubs_from_store` (we don't take a `&AccountStore`
    /// here so the migrate crate stays decoupled from the SQLite
    /// surface — opening the store and pulling rows is the adapter's
    /// job).
    pub account_stubs: Option<Vec<state::AccountStub>>,
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
    if opts.encrypt && opts.encrypt_passphrase.is_none() {
        // Adapter must prompt and pass the SecretString in. Refusing
        // here is loud rather than silent. Configuration (not
        // NotImplemented) — encryption is supported; the user just
        // didn't wire the passphrase.
        return Err(MigrateError::Configuration(
            "encryption requested but no passphrase supplied — adapter must prompt".to_string(),
        ));
    }
    if opts.include_live {
        // Spec §9.1: --include-live requires fstat-based live-session
        // detection plus a 3-retry copy + `live_at_export: true` flag.
        // Neither path is built yet; refuse loudly so the user sees
        // the gap rather than getting a silent normal export.
        return Err(MigrateError::NotImplemented(
            "--include-live (live-session export with retry semantics, spec §9.1)".to_string(),
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

        // Optional: tarball <cwd>/.claude/** + <cwd>/CLAUDE.md when
        // --include-worktree was set. The cwd may not exist on the
        // source (e.g. project was renamed and the on-disk dir
        // removed); in that case worktree_set stays false and apply
        // skips silently.
        let mut worktree_set = false;
        if opts.include_worktree {
            let cwd_path = std::path::PathBuf::from(&nfc_cwd);
            if cwd_path.exists() {
                let n = worktree::append_worktree(&cwd_path, &project_id, &mut writer)?;
                if n > 0 {
                    worktree_set = true;
                    file_count += n;
                }
            }
        }

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
            worktree_set,
        };
        let pm_bytes =
            serde_json::to_vec_pretty(&pm).map_err(|e| MigrateError::Serialize(e.to_string()))?;
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

    if opts.include_global {
        let inv = global::append_global(config_dir, &mut writer)?;
        file_count += inv.len();
    }

    if opts.include_claudepot_state {
        // Account stubs require a connected store; the orchestrator
        // pulls them via the optional `account_stubs` field on
        // ExportOptions. When the caller doesn't supply a store, we
        // ship the prefs+lifecycle files only (account list empty).
        let stubs = opts.account_stubs.clone().unwrap_or_default();
        let data_dir = crate::paths::claudepot_data_dir();
        let protected = state::read_protected_paths_bytes(&data_dir)?;
        let preferences = state::read_preferences_bytes(&data_dir)?;
        let lifecycle = state::read_artifact_lifecycle_bytes(&data_dir)?;
        state::append_claudepot_state(
            &mut writer,
            &stubs,
            protected.as_deref(),
            preferences.as_deref(),
            lifecycle.as_deref(),
        )?;
        file_count += 1; // accounts.export.json always written
        if protected.is_some() {
            file_count += 1;
        }
        if preferences.is_some() {
            file_count += 1;
        }
        if lifecycle.is_some() {
            file_count += 1;
        }
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
    let mut sidecar_path = bundle::sidecar_path_for(&bundle_path);

    // Optional signing — done before encryption so the signature is
    // over the unencrypted (canonical) bundle bytes. Verifiers can
    // then check the signature without needing the passphrase first.
    if let Some(keyfile) = opts.sign_keyfile.as_ref() {
        crypto::sign_bundle(
            &bundle_path,
            std::path::Path::new(keyfile),
            opts.sign_password.clone(),
        )?;
    }

    // Optional encryption.
    let final_path = if opts.encrypt {
        let pwd = opts.encrypt_passphrase.clone().ok_or_else(|| {
            MigrateError::NotImplemented(
                "encryption requested but no passphrase supplied — adapter must prompt".to_string(),
            )
        })?;
        let enc = crypto::encrypt_bundle_with_passphrase(&bundle_path, &pwd)?;
        // Remove the plaintext bundle and its sidecar — only the
        // encrypted artifact ships.
        let _ = fs::remove_file(&bundle_path);
        let plain_sidecar = bundle::sidecar_path_for(&bundle_path);
        let _ = fs::remove_file(&plain_sidecar);
        // Write a fresh sidecar over the encrypted file's bytes so
        // tampering on transport is still detectable without the
        // passphrase.
        let enc_sha = bundle_sha256_sidecar_for(&enc)?;
        let enc_sidecar = bundle::sidecar_path_for(&enc);
        fs::write(&enc_sidecar, enc_sha).map_err(MigrateError::from)?;
        sidecar_path = enc_sidecar;
        enc
    } else {
        bundle_path
    };

    Ok(ExportReceipt {
        bundle_path: final_path,
        bundle_sha256_sidecar: sidecar_path,
        project_count,
        file_count,
    })
}

fn bundle_sha256_sidecar_for(bundle: &Path) -> Result<String, MigrateError> {
    use sha2::Digest;
    let bytes = fs::read(bundle).map_err(MigrateError::from)?;
    let mut h = sha2::Sha256::new();
    h.update(&bytes);
    let digest = hex::encode(h.finalize());
    let name = bundle
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(format!("{digest}  {name}\n"))
}

/// Read a bundle's manifest without extracting. Cheap; used by the
/// `inspect` subcommand. Cannot inspect an encrypted bundle without
/// the passphrase — callers prompt and pass it via
/// `inspect_encrypted`.
pub fn inspect(bundle_path: &Path) -> Result<manifest::BundleManifest, MigrateError> {
    if bundle_path.extension().is_some_and(|e| e == "age") {
        return Err(MigrateError::NotImplemented(
            "this bundle is encrypted; use `migrate inspect --passphrase ...` \
             (the adapter prompts) instead"
                .to_string(),
        ));
    }
    let r = bundle::BundleReader::open(bundle_path)?;
    r.read_manifest()
}

/// Inspect an encrypted bundle by decrypting to a tempfile under our
/// own staging tree and reading the manifest. Never writes plaintext
/// next to the encrypted bundle (see `crypto::decrypt_bundle_with_passphrase`
/// for the rationale).
pub fn inspect_encrypted(
    bundle_path: &Path,
    passphrase: &age::secrecy::SecretString,
) -> Result<manifest::BundleManifest, MigrateError> {
    let stage = apply::imports_root().join(format!("inspect-{}", uuid::Uuid::new_v4()));
    let plaintext = crypto::decrypt_bundle_with_passphrase(bundle_path, passphrase, &stage)?;
    let _cleanup = PlaintextCleanupGuard::new(Some(plaintext.clone()));
    // The decrypted plaintext lives under our staging tree and was
    // never written next to the user's encrypted bundle, so the outer
    // sidecar is by-design absent. Use `open_unverified` — verification
    // is structurally equivalent because age's Poly1305 already
    // authenticated the plaintext.
    let r = bundle::BundleReader::open_unverified(&plaintext)?;
    r.read_manifest()
}

/// RAII cleanup helper: deletes a tempfile path on drop. Used by the
/// import path when the user hands us an encrypted bundle — we
/// decrypt to a sibling plaintext for the duration of import, then
/// remove the plaintext on success/failure either way so it doesn't
/// linger on disk.
struct PlaintextCleanupGuard(Option<PathBuf>);

impl PlaintextCleanupGuard {
    fn new(p: Option<PathBuf>) -> Self {
        Self(p)
    }
}

impl Drop for PlaintextCleanupGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = fs::remove_file(&p);
            let _ = fs::remove_file(bundle::sidecar_path_for(&p));
        }
    }
}

/// Outcome of `import_undo`.
#[derive(Debug, Clone)]
pub struct UndoReceipt {
    pub journal_path: PathBuf,
    pub bundle_id: String,
    pub steps_reversed: usize,
    pub steps_tampered: Vec<String>,
    pub steps_errored: Vec<String>,
    /// Counter-journal recording the undo (so undo-of-undo is well-
    /// defined). Path is `~/.claudepot/repair/journals/undo-<id>.json`.
    pub counter_journal_path: PathBuf,
}

/// Reverse the most recent committed import within the 24h undo
/// window. Looks up the newest `import-*.json` journal under the
/// repair tree, verifies it's within window, runs the LIFO replay,
/// and writes a counter-journal alongside.
pub fn import_undo() -> Result<UndoReceipt, MigrateError> {
    // Take the same lock as import — mutually exclusive with any
    // other in-flight migrate / rename / repair op (spec §8.3).
    let (journals_dir, locks_dir, _) = crate::paths::claudepot_repair_dirs();
    let (_lock_guard, _broken) =
        crate::project_lock::acquire(&locks_dir, "import").map_err(|e| {
            MigrateError::Configuration(format!(
                "could not acquire import lock for undo — another claudepot \
                 operation may be running: {e}"
            ))
        })?;
    if !journals_dir.exists() {
        return Err(MigrateError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no journal directory at {}", journals_dir.display()),
        )));
    }
    let mut newest: Option<(PathBuf, u64)> = None;
    for entry in fs::read_dir(&journals_dir).map_err(MigrateError::from)? {
        let entry = entry.map_err(MigrateError::from)?;
        let p = entry.path();
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !name.starts_with("import-") || !name.ends_with(".json") {
            continue;
        }
        let m = entry.metadata().map_err(MigrateError::from)?;
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if newest.as_ref().is_none_or(|(_, t)| mtime > *t) {
            newest = Some((p, mtime));
        }
    }
    let Some((journal_path, _)) = newest else {
        return Err(MigrateError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no import-*.json journals found",
        )));
    };
    let journal = apply::ImportJournal::load(&journal_path)?;
    if !apply::within_undo_window(&journal) {
        return Err(MigrateError::Conflict(format!(
            "journal {} is older than 24h — outside undo window",
            journal_path.display()
        )));
    }

    let report = apply::rollback(&journal)?;

    // Write counter-journal so undo-of-undo is well-defined (§8.3).
    // Record the original journal steps (in apply order) — replaying
    // them is the inverse of an undo. Crucially we keep the original
    // `dir_inventory` and `snapshot_path` references so a redo would
    // restore exactly what the undo removed.
    let counter = apply::ImportJournal {
        bundle_id: format!("undo-{}", journal.bundle_id),
        started_unix_secs: now_secs(),
        finished_unix_secs: Some(now_secs()),
        claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        steps: journal.steps.clone(),
        committed: true,
    };
    let counter_path = journals_dir.join(format!("undo-{}.json", journal.bundle_id));
    counter.persist(&counter_path)?;

    // Discard snapshots only if rollback completed cleanly (no errors,
    // no tampered).
    if report.errors.is_empty() && report.skipped_tampered.is_empty() {
        let _ = apply::discard_snapshots(&journal.bundle_id);
        // Mark the original journal as undone by removing it.
        let _ = fs::remove_file(&journal_path);
    }

    Ok(UndoReceipt {
        journal_path,
        bundle_id: journal.bundle_id,
        steps_reversed: report.reversed,
        steps_tampered: report.skipped_tampered,
        steps_errored: report.errors,
        counter_journal_path: counter_path,
    })
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
    /// Passphrase for decrypting an `.age` bundle. The adapter is
    /// responsible for prompting and clearing the secret afterward.
    pub decrypt_passphrase: Option<age::secrecy::SecretString>,
    /// Optional minisign public-key file. When set, the importer
    /// verifies `<bundle>.minisig` against this public key before
    /// extraction. Required for `--unattended-import`.
    pub verify_key: Option<PathBuf>,
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
            decrypt_passphrase: None,
            verify_key: None,
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
    /// Populated when `--include-claudepot-state` was set on the bundle.
    /// Account stubs surface to the user as "the source had these;
    /// re-login here." Never auto-inserted (spec §16 Q2).
    pub accounts_listed: Vec<state::AccountStub>,
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
    // Optional minisign verification — done against whatever the user
    // pointed us at. Verification is byte-level over the bundle file,
    // so encrypted bundles get verified before any decryption. The
    // signature was made over the *plaintext* during export, so for
    // encrypted bundles the user must verify against the encrypted
    // bytes — by writing a signature over the encrypted artifact
    // separately. v0 keeps verification simple: signature is over
    // whatever file the user hands us. Spec §3.3 documents this
    // (signature over `manifest.json sha256` is the future contract).
    if let Some(verify_key) = opts.verify_key.as_ref() {
        crypto::verify_bundle_signature(bundle_path, verify_key)?;
    }

    // If the path ends in `.age`, decrypt into a tempfile under our
    // staging tree (NOT next to the user's encrypted bundle — that
    // could clobber an unrelated plaintext sibling). We delete the
    // tempfile after success or failure via `PlaintextCleanupGuard`.
    let bundle_id = uuid::Uuid::new_v4().to_string();
    let (bundle_path_resolved, plaintext_to_cleanup, encrypted) =
        if bundle_path.extension().is_some_and(|e| e == "age") {
            let pwd = opts.decrypt_passphrase.clone().ok_or_else(|| {
                MigrateError::Configuration(
                    "encrypted bundle but no passphrase supplied — adapter must prompt".to_string(),
                )
            })?;
            let stage = apply::imports_root().join(format!("decrypt-{bundle_id}"));
            let plaintext = crypto::decrypt_bundle_with_passphrase(bundle_path, &pwd, &stage)?;
            (plaintext.clone(), Some(plaintext), true)
        } else {
            (bundle_path.to_path_buf(), None, false)
        };
    let bundle_path = &bundle_path_resolved;

    let reader = if encrypted {
        // age's Poly1305 already authenticated the plaintext bytes, so
        // the outer sidecar is unnecessary on this temp file. Skip it.
        bundle::BundleReader::open_unverified(bundle_path)?
    } else {
        bundle::BundleReader::open(bundle_path)?
    };
    let manifest = reader.read_manifest()?;
    let _cleanup_guard = PlaintextCleanupGuard::new(plaintext_to_cleanup);
    if manifest.schema_version != manifest::SCHEMA_VERSION {
        return Err(MigrateError::UnsupportedSchemaVersion {
            found: manifest.schema_version,
            expected: manifest::SCHEMA_VERSION,
        });
    }

    let staging = apply::staging_dir(&bundle_id);
    let journal_path = apply::journal_path(&bundle_id);
    let mut journal = apply::ImportJournal::new(bundle_id.clone());

    // Acquire the global import lock per spec §8 P4 — mutually
    // exclusive with rename / repair / clean_orphans / move so a
    // concurrent claudepot run can't trample our staging or journal.
    // Lifetime: held for P1..P8 via the RAII `LockGuard`.
    let (_, locks_dir, _) = crate::paths::claudepot_repair_dirs();
    let (_lock_guard, _broken) =
        crate::project_lock::acquire(&locks_dir, "import").map_err(|e| {
            MigrateError::Configuration(format!(
                "could not acquire import lock — another claudepot operation \
                 may be running: {e}"
            ))
        })?;

    let mut projects_imported = Vec::new();
    let mut projects_refused = Vec::new();
    let mut accounts_listed: Vec<state::AccountStub> = Vec::new();

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
            accounts_listed: Vec::new(),
        });
    }

    // P1 stage — extract.
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(MigrateError::from)?;
    }
    fs::create_dir_all(&staging).map_err(MigrateError::from)?;
    if let Err(e) = reader.extract_all(&staging) {
        // Stage failure: nothing landed yet. Just clean staging.
        let _ = apply::discard_staging(&bundle_id);
        return Err(e);
    }
    // §3.3 inner integrity gate: the outer sidecar covers the
    // compressed bytes; this gate covers each unpacked file. Without
    // it, a bundle whose tar contents were swapped after a re-pack
    // (with a fresh outer sidecar) would still pass open(). With it,
    // every payload digest must match `integrity.sha256` or we abort.
    if let Err(e) = bundle::verify_extracted_dir(&staging) {
        let _ = apply::discard_staging(&bundle_id);
        return Err(e);
    }

    // P3..P5 — apply per project. On any hard error mid-apply, the
    // outer `?` would skip the journal persist. Wrap the loop so we
    // can run rollback against whatever we've already recorded.
    let apply_outcome: Result<(), MigrateError> = (|| {
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
            table.push(
                &manifest.source_home,
                &home_string(),
                plan::RuleOrigin::Home,
            );
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
                Err(e)
                    if e.kind() == std::io::ErrorKind::CrossesDevices
                        || e.raw_os_error() == Some(libc::EXDEV) =>
                {
                    copy_dir_recursive(&staged_slug_root, &target_slug_dir)?;
                    fs::remove_dir_all(&staged_slug_root).map_err(MigrateError::from)?;
                }
                Err(e) => return Err(MigrateError::from(e)),
            }

            // Populate dir_inventory so surgical rollback knows exactly
            // which files we wrote — preserves user work added post-
            // import (audit Robustness finding).
            let dir_inventory = apply::collect_dir_inventory(&target_slug_dir);
            journal.record(apply::JournalStep {
                kind: apply::JournalStepKind::CreateDir,
                before: None,
                after: Some(target_slug_dir.to_string_lossy().to_string()),
                snapshot_path: None,
                after_sha256: None,
                fragment_key: None,
                dir_inventory,
                timestamp_unix_secs: now_secs(),
            });

            // Worktree apply (when bundle carries it).
            if manifest.flags.include_worktree {
                let staged_project_root = staging.join("projects").join(&pref.id);
                if staged_project_root.join("project-scoped").exists() {
                    let target_cwd = std::path::PathBuf::from(&target_cwd);
                    if target_cwd.exists() {
                        let steps = worktree::apply_worktree(
                            &staged_project_root,
                            &target_cwd,
                            &bundle_id,
                        )?;
                        for s in steps {
                            let kind = match s.kind {
                                worktree::WorktreeApplyKind::Created => {
                                    apply::JournalStepKind::CreateFile
                                }
                                worktree::WorktreeApplyKind::SideBySide => {
                                    apply::JournalStepKind::CreateFile
                                }
                                worktree::WorktreeApplyKind::SkippedIdentical => continue,
                            };
                            let after_path = std::path::Path::new(&s.after);
                            let after_sha256 = apply::sha256_of_file_optional(after_path);
                            journal.record(apply::JournalStep {
                                kind,
                                before: None,
                                after: Some(s.after),
                                snapshot_path: None,
                                after_sha256,
                                fragment_key: None,
                                dir_inventory: Vec::new(),
                                timestamp_unix_secs: now_secs(),
                            });
                        }
                    }
                    // Target cwd missing: skip silently. The slug landed
                    // either way; the user can re-apply worktree later.
                }
            }

            projects_imported.push(pref.source_cwd.clone());
        }

        // Claudepot state (Bucket C-adjacent). Surface account stubs to
        // the receipt; write protected-paths/preferences/artifact-lifecycle
        // to the target's claudepot data dir.
        if manifest.flags.include_claudepot_state {
            let outcome = state::apply_claudepot_state(
                &staging,
                &crate::paths::claudepot_data_dir(),
                &bundle_id,
            )?;
            for created in outcome.created {
                // Tamper baseline so undo can detect post-apply edits to
                // protected-paths.json / preferences.json / artifact-
                // lifecycle.json (audit Robustness #12 follow-up).
                let after_sha256 = apply::sha256_of_file_optional(std::path::Path::new(&created));
                journal.record(apply::JournalStep {
                    kind: apply::JournalStepKind::CreateFile,
                    before: None,
                    after: Some(created),
                    snapshot_path: None,
                    after_sha256,
                    fragment_key: None,
                    dir_inventory: Vec::new(),
                    timestamp_unix_secs: now_secs(),
                });
            }
            for sbs in outcome.side_by_side {
                let after_sha256 = apply::sha256_of_file_optional(std::path::Path::new(&sbs));
                journal.record(apply::JournalStep {
                    kind: apply::JournalStepKind::CreateFile,
                    before: None,
                    after: Some(sbs),
                    snapshot_path: None,
                    after_sha256,
                    fragment_key: None,
                    dir_inventory: Vec::new(),
                    timestamp_unix_secs: now_secs(),
                });
            }
            // Account stubs are intentionally NOT recorded in the journal
            // — they don't mutate any target file (log-only per §16 Q2).
            // The receipt picks them up via `bundle_id` correlation in
            // `import_undo`, but the orchestrator returns them in
            // `ImportReceipt::accounts_listed` for the CLI to display.
            accounts_listed = outcome.accounts_listed;
        }

        // Bucket C — global content. Only when the bundle's flags say so
        // AND staging actually carries it.
        if manifest.flags.include_global {
            let global_steps =
                global::apply_global(&staging, config_dir, opts.accept_hooks, &bundle_id)?;
            for step in global_steps {
                let kind = match step.kind {
                    global::GlobalApplyKind::Created => apply::JournalStepKind::CreateFile,
                    global::GlobalApplyKind::HooksAccepted => apply::JournalStepKind::ReplaceFile,
                    global::GlobalApplyKind::SideBySide
                    | global::GlobalApplyKind::HooksProposed
                    | global::GlobalApplyKind::McpProposed => apply::JournalStepKind::CreateFile,
                    global::GlobalApplyKind::SkippedIdentical => continue,
                };
                // Compute sha256 of the post-apply file so rollback's
                // tamper detection has a real baseline. Without this the
                // after_sha256 field stays None and post-import edits go
                // undetected during undo.
                let after_path = std::path::Path::new(&step.after);
                let after_sha256 = apply::sha256_of_file_optional(after_path);
                journal.record(apply::JournalStep {
                    kind,
                    before: None,
                    after: Some(step.after),
                    snapshot_path: step.snapshot,
                    after_sha256,
                    fragment_key: None,
                    dir_inventory: Vec::new(),
                    timestamp_unix_secs: now_secs(),
                });
            }
        }

        Ok(())
    })();

    if let Err(e) = apply_outcome {
        // Best-effort: persist a partial journal so the user can see
        // what landed before reverse-LIFO rollback.
        let _ = journal.persist(&journal_path);
        let report = apply::rollback(&journal)?;
        let _ = apply::discard_staging(&bundle_id);
        let _ = apply::discard_snapshots(&bundle_id);
        // If rollback was clean, also remove the journal — the failed
        // import never officially happened.
        if report.errors.is_empty() && report.skipped_tampered.is_empty() {
            let _ = fs::remove_file(&journal_path);
        }
        return Err(e);
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
        accounts_listed,
    })
}

/// Walk a slug tree under `staging` and rewrite every `*.jsonl` and
/// `*.meta.json` per the substitution table. Quiet on success.
fn rewrite_slug_tree(slug_dir: &Path, table: &plan::SubstitutionTable) -> Result<(), MigrateError> {
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
        // Symlink check FIRST. The legacy version put this inside the
        // `is_file` branch, where it was unreachable because a symlink
        // satisfies neither `is_file` nor `is_dir` — `FileType` reports
        // them as their own variant. Result: symlinks were silently
        // skipped, contradicting the §3.1 zero-symlink policy.
        if ft.is_symlink() {
            return Err(MigrateError::IntegrityViolation(format!(
                "source contains symlink: {}",
                path.display()
            )));
        }
        if ft.is_dir() {
            walk_and_append(&path, base, bundle_prefix, writer, inventory)?;
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(base)
                .map_err(|e| MigrateError::Io(std::io::Error::other(format!("strip_prefix: {e}"))))?
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
            account_stubs: None,
            encrypt_passphrase: None,
            sign_password: None,
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
    fn export_refuses_encrypt_without_passphrase() {
        // Encryption is supported, but adapters must supply a
        // passphrase. Refusing without one keeps the failure loud
        // rather than silent (matches the spec §3.3 contract).
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
            account_stubs: None,
            encrypt_passphrase: None,
            sign_password: None,
        };
        let err = export_projects(&cfg, opts).unwrap_err();
        // Configuration (not NotImplemented) — encryption ships;
        // missing passphrase is a usage error.
        assert!(matches!(err, MigrateError::Configuration(_)));
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
            account_stubs: None,
            encrypt_passphrase: None,
            sign_password: None,
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
                account_stubs: None,
                encrypt_passphrase: None,
                sign_password: None,
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
    fn export_with_include_global_round_trips_settings_and_hooks() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("claudepot"));
        let cfg_src = tmp.path().join("src/.claude");
        fs::create_dir_all(cfg_src.join("projects")).unwrap();
        fs::create_dir_all(cfg_src.join("agents")).unwrap();
        let cwd = "/tmp/test-global".to_string();
        seed_project(&cfg_src, &cwd);
        // Bucket C content.
        fs::write(cfg_src.join("CLAUDE.md"), "# user prefs\n").unwrap();
        fs::write(cfg_src.join("agents/foo.md"), "# foo\n").unwrap();
        fs::write(
            cfg_src.join("settings.json"),
            r#"{"theme":"dark","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[]}]}}"#,
        )
        .unwrap();

        let bundle = tmp.path().join("g.tar.zst");
        export_projects(
            &cfg_src,
            ExportOptions {
                output: bundle.clone(),
                project_cwds: vec![cwd.clone()],
                include_global: true,
                include_worktree: false,
                include_live: false,
                include_claudepot_state: false,
                include_file_history: true,
                encrypt: false,
                sign_keyfile: None,
                account_stubs: None,
                encrypt_passphrase: None,
                sign_password: None,
            },
        )
        .unwrap();

        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();
        let receipt = import_bundle(&cfg_target, &bundle, ImportOptions::default()).unwrap();
        assert_eq!(receipt.projects_imported.len(), 1);

        // Bucket C content landed.
        assert!(cfg_target.join("CLAUDE.md").exists());
        assert!(cfg_target.join("agents/foo.md").exists());
        // Settings present (from scrubbed) without hooks (since we
        // didn't pass --accept-hooks).
        let settings_v: serde_json::Value =
            serde_json::from_slice(&fs::read(cfg_target.join("settings.json")).unwrap()).unwrap();
        assert!(settings_v.get("hooks").is_none());
        // proposed-hooks.json placed next to settings for review.
        assert!(cfg_target.join("proposed-hooks.json").exists());

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn export_with_include_worktree_round_trips_dot_claude_tree() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("claudepot"));
        let cfg_src = tmp.path().join("src/.claude");
        fs::create_dir_all(cfg_src.join("projects")).unwrap();

        // Real project tree on disk.
        let project_cwd = tmp.path().join("proj");
        fs::create_dir_all(project_cwd.join(".claude")).unwrap();
        fs::write(project_cwd.join("CLAUDE.md"), "# project prefs\n").unwrap();
        fs::write(
            project_cwd.join(".claude/settings.json"),
            r#"{"theme":"dark"}"#,
        )
        .unwrap();
        fs::write(
            project_cwd.join(".claude/settings.local.json"),
            r#"{"secret":1}"#,
        )
        .unwrap();
        let cwd = project_cwd.to_string_lossy().to_string();
        seed_project(&cfg_src, &cwd);

        let bundle = tmp.path().join("wt.tar.zst");
        export_projects(
            &cfg_src,
            ExportOptions {
                output: bundle.clone(),
                project_cwds: vec![cwd.clone()],
                include_global: false,
                include_worktree: true,
                include_live: false,
                include_claudepot_state: false,
                include_file_history: true,
                encrypt: false,
                sign_keyfile: None,
                account_stubs: None,
                encrypt_passphrase: None,
                sign_password: None,
            },
        )
        .unwrap();

        // Target tree must exist for worktree apply (mirroring the
        // contract: user clones their project's git repo at the
        // target before importing). We re-use the same path here
        // since this is a same-machine round trip.
        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();

        let receipt = import_bundle(&cfg_target, &bundle, ImportOptions::default()).unwrap();
        assert_eq!(receipt.projects_imported.len(), 1);

        // settings.json from the bundle is identical to what's on disk
        // (we never overwrote the source) → SkippedIdentical, so no
        // .imported sibling. CLAUDE.md likewise.
        // Local settings absolutely must not have traveled.
        // The target == source path, so both files coexist.
        assert!(project_cwd.join(".claude/settings.local.json").exists());
        // The bundle's content must NOT have written settings.local.imported.
        assert!(!project_cwd
            .join(".claude/settings.local.imported.json")
            .exists());

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn import_with_accept_hooks_merges_into_settings() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("claudepot"));
        let cfg_src = tmp.path().join("src/.claude");
        fs::create_dir_all(cfg_src.join("projects")).unwrap();
        let cwd = "/tmp/test-hooks-accept".to_string();
        seed_project(&cfg_src, &cwd);
        fs::write(
            cfg_src.join("settings.json"),
            r#"{"theme":"light","hooks":{"PreToolUse":[{"matcher":"Bash"}]}}"#,
        )
        .unwrap();

        let bundle = tmp.path().join("ah.tar.zst");
        export_projects(
            &cfg_src,
            ExportOptions {
                output: bundle.clone(),
                project_cwds: vec![cwd.clone()],
                include_global: true,
                include_worktree: false,
                include_live: false,
                include_claudepot_state: false,
                include_file_history: true,
                encrypt: false,
                sign_keyfile: None,
                account_stubs: None,
                encrypt_passphrase: None,
                sign_password: None,
            },
        )
        .unwrap();

        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();
        let opts = ImportOptions {
            accept_hooks: true,
            ..ImportOptions::default()
        };
        import_bundle(&cfg_target, &bundle, opts).unwrap();

        let settings_v: serde_json::Value =
            serde_json::from_slice(&fs::read(cfg_target.join("settings.json")).unwrap()).unwrap();
        assert!(settings_v.get("hooks").is_some());
        // No side-by-side proposed-hooks.json (it merged into
        // settings).
        assert!(!cfg_target.join("proposed-hooks.json").exists());

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn export_with_encryption_then_import_round_trip() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("claudepot"));
        let cfg_src = tmp.path().join("src/.claude");
        fs::create_dir_all(cfg_src.join("projects")).unwrap();
        let cwd = "/tmp/test-encrypt".to_string();
        seed_project(&cfg_src, &cwd);

        let plain_bundle = tmp.path().join("e.tar.zst");
        let pwd = age::secrecy::SecretString::from("test-pass-1234".to_string());
        let receipt = export_projects(
            &cfg_src,
            ExportOptions {
                output: plain_bundle.clone(),
                project_cwds: vec![cwd.clone()],
                include_global: false,
                include_worktree: false,
                include_live: false,
                include_claudepot_state: false,
                include_file_history: true,
                encrypt: true,
                encrypt_passphrase: Some(pwd.clone()),
                sign_keyfile: None,
                sign_password: None,
                account_stubs: None,
            },
        )
        .unwrap();
        // Plaintext bundle is gone; encrypted artifact is what shipped.
        assert!(!plain_bundle.exists());
        assert!(receipt.bundle_path.to_string_lossy().ends_with(".age"));
        assert!(receipt.bundle_sha256_sidecar.exists());

        // Inspect must refuse without a passphrase.
        let err = inspect(&receipt.bundle_path).unwrap_err();
        assert!(matches!(err, MigrateError::NotImplemented(_)));
        // inspect_encrypted works.
        let m = inspect_encrypted(&receipt.bundle_path, &pwd).unwrap();
        assert_eq!(m.projects.len(), 1);

        // Import works with the passphrase.
        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();
        let imp = ImportOptions {
            decrypt_passphrase: Some(pwd.clone()),
            ..ImportOptions::default()
        };
        let r = import_bundle(&cfg_target, &receipt.bundle_path, imp).unwrap();
        assert_eq!(r.projects_imported.len(), 1);

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn undo_writes_counter_journal_replayable_for_redo() {
        // Audit Robustness fix: counter-journal must record the
        // inverse steps so undo-of-undo (redo) is well-defined within
        // the 24h window. The earlier shape persisted an EMPTY
        // counter, making redo impossible.
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("claudepot"));
        let cfg_src = tmp.path().join("src/.claude");
        fs::create_dir_all(cfg_src.join("projects")).unwrap();
        let cwd = "/tmp/test-undo-redo".to_string();
        seed_project(&cfg_src, &cwd);
        let bundle = tmp.path().join("ur.tar.zst");
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
                account_stubs: None,
                encrypt_passphrase: None,
                sign_password: None,
            },
        )
        .unwrap();

        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();
        let receipt = import_bundle(&cfg_target, &bundle, ImportOptions::default()).unwrap();
        let undo = import_undo().unwrap();
        // Counter-journal must exist AND be non-empty so a future
        // redo can replay it.
        assert!(undo.counter_journal_path.exists());
        let counter = apply::ImportJournal::load(&undo.counter_journal_path).unwrap();
        assert!(
            !counter.steps.is_empty(),
            "counter-journal must record the inverse steps"
        );
        assert_eq!(counter.bundle_id, format!("undo-{}", receipt.bundle_id));

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn import_then_undo_round_trip_removes_slug() {
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("claudepot"));
        let cfg_src = tmp.path().join("src/.claude");
        fs::create_dir_all(cfg_src.join("projects")).unwrap();
        let cwd = "/tmp/test-import-undo".to_string();
        seed_project(&cfg_src, &cwd);
        let bundle = tmp.path().join("u.tar.zst");
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
                account_stubs: None,
                encrypt_passphrase: None,
                sign_password: None,
            },
        )
        .unwrap();

        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();
        let receipt = import_bundle(&cfg_target, &bundle, ImportOptions::default()).unwrap();
        assert!(!receipt.dry_run);
        assert_eq!(receipt.projects_imported.len(), 1);

        let target_slug = plan::target_slug(&cwd);
        let target_dir = cfg_target.join("projects").join(&target_slug);
        assert!(target_dir.exists(), "slug must exist after import");

        // Undo. Should remove the slug because CreateDir rolls back.
        let undo_receipt = import_undo().unwrap();
        assert!(undo_receipt.steps_reversed >= 1);
        assert!(undo_receipt.steps_errored.is_empty());
        assert!(undo_receipt.steps_tampered.is_empty());
        assert!(!target_dir.exists(), "slug must be removed after undo");
        assert!(undo_receipt.counter_journal_path.exists());

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
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
                account_stubs: None,
                encrypt_passphrase: None,
                sign_password: None,
            },
        )
        .unwrap();

        let cfg_target = tmp.path().join("dst/.claude");
        fs::create_dir_all(cfg_target.join("projects")).unwrap();
        let receipt = import_bundle(&cfg_target, &bundle, ImportOptions::default()).unwrap();
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
            if entry
                .path()
                .extension()
                .map(|e| e == "jsonl")
                .unwrap_or(false)
            {
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
