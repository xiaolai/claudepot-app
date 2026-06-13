//! Atomic load/save of `~/.claudepot/permission-grants.json`.
//!
//! Thin wrapper over [`crate::json_store`] — see that module for the
//! three-outcome load contract and the corruption-recovery policy
//! (timestamped rename-aside, warn on rename failure, atomic write).
//!
//! **This store fails loud on corruption recovery.** For rotation
//! rules, recovering a corrupt file to empty is safe — the feature
//! just turns off. For grants it is fail-open: the on-disk record is
//! the ONLY thing obliging the orchestrator to revert a time-boxed
//! `bypassPermissions` elevation, and AGENTS.md's contract is "the
//! elevated state is never left to memory". A corrupt or
//! future-schema grants file therefore:
//!
//! - still gets moved aside so boot never wedges, and the store
//!   still returns a usable (empty) file, BUT
//! - the recovery is logged at `error!` (not `warn!`), and
//! - [`load_outcome`] exposes a [`CorruptionRecovery`] marker so the
//!   orchestrator/UI can surface "grants file unreadable; elevated
//!   projects may not auto-revert" instead of silently ending all
//!   revert obligations. [`corrupt_grant_copies`] lets a later boot
//!   detect a recovery that happened in an earlier process.

use std::path::{Path, PathBuf};

use crate::json_store::{self, SaveError};
use crate::permission::grants::{GrantsFile, ValidationError};

pub use crate::json_store::{CorruptionRecovery, Loaded};

/// Standard filename inside `claudepot_data_dir()`.
pub const GRANTS_FILENAME: &str = "permission-grants.json";

/// Store name used in log messages.
const STORE: &str = "permission_store";

/// `~/.claudepot/permission-grants.json` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn grants_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(GRANTS_FILENAME)
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("validation: {0}")]
    Validation(#[from] ValidationError),
}

impl json_store::Validate for GrantsFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), ValidationError> {
        // Delegates to the inherent method (inherent methods win
        // resolution over trait methods, so this is not recursion).
        GrantsFile::validate(self)
    }
}

/// Load grants from the canonical path, reporting corruption
/// recovery explicitly. `Ok(Loaded { recovery: Some(..), .. })`
/// means the on-disk file was corrupt or had an unsupported schema:
/// it was moved aside, the returned grants are empty, and **projects
/// elevated by a Claudepot grant will not auto-revert** until the
/// user re-grants or reverts by hand. Callers that can reach the
/// user (orchestrator, commands) must surface this; see the module
/// docs for why this store cannot fail silently-open.
pub fn load_outcome() -> std::io::Result<Loaded<GrantsFile>> {
    load_outcome_from(&grants_path())
}

/// Test-friendly variant of [`load_outcome`] taking the path
/// directly.
pub fn load_outcome_from(path: &Path) -> std::io::Result<Loaded<GrantsFile>> {
    let loaded = json_store::load_or_recover::<GrantsFile>(path, STORE)?;
    if let Some(rec) = &loaded.recovery {
        tracing::error!(
            error = %rec.error,
            moved_to = ?rec.moved_to,
            "permission_store: grants file was corrupt or unsupported; recovered to \
             empty — projects elevated by a Claudepot grant will NOT auto-revert \
             until re-granted or reverted by hand"
        );
    }
    Ok(loaded)
}

/// Forensic copies of previously-corrupt grants files sitting next
/// to the canonical path (both the timestamped and the legacy fixed
/// `.corrupt` shapes). Lets the orchestrator/UI surface "grants file
/// was unreadable; elevated projects may not auto-revert" even when
/// the recovery happened in an earlier process.
pub fn corrupt_grant_copies() -> Vec<PathBuf> {
    json_store::corrupt_siblings(&grants_path())
}

/// Load grants under the three-outcome contract, discarding the
/// recovery marker (the `error!` log from [`load_outcome_from`]
/// still fires). Prefer [`load_outcome`] in any caller that can
/// surface the recovery to the user.
pub fn load() -> std::io::Result<GrantsFile> {
    load_from(&grants_path())
}

/// Test-friendly load that takes the path directly. See [`load`].
pub fn load_from(path: &Path) -> std::io::Result<GrantsFile> {
    Ok(load_outcome_from(path)?.value)
}

/// Log + swallow real I/O errors, always returning a usable file.
/// Use only where errors cannot be propagated; new code prefers
/// [`load_outcome`].
pub fn load_or_default() -> GrantsFile {
    match load() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "permission_store: read failed; defaulting to empty");
            GrantsFile::default()
        }
    }
}

/// Persist `file` to the canonical path. Validates before writing —
/// invalid input is rejected so on-disk files are always loadable.
pub fn save(file: &GrantsFile) -> Result<(), PermissionStoreError> {
    save_to(&grants_path(), file)
}

/// Test-friendly save that takes the path directly.
pub fn save_to(path: &Path, file: &GrantsFile) -> Result<(), PermissionStoreError> {
    json_store::save(path, file).map_err(|e| match e {
        SaveError::Validation(v) => PermissionStoreError::Validation(v),
        SaveError::Serde(s) => PermissionStoreError::Serde(s),
        SaveError::Io(io) => PermissionStoreError::Io(io),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::grants::Grant;
    use crate::permission::mode::PermissionMode;
    use crate::settings_writer::SettingsLayer;
    use chrono::{TimeZone, Utc};

    fn sample_grant(path: &str) -> Grant {
        Grant {
            project_path: path.to_string(),
            layer: SettingsLayer::LocalProject,
            granted_mode: PermissionMode::BypassPermissions,
            previous_mode: Some(PermissionMode::Default),
            granted_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            expires_at: Some(Utc.timestamp_opt(1_700_007_200, 0).unwrap()),
            consecutive_failures: 0,
            last_failure_at: None,
        }
    }

    #[test]
    fn load_missing_file_yields_default() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("nope.json");
        let f = load_from(&p).unwrap();
        assert!(f.grants.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("grants.json");
        let mut file = GrantsFile::default();
        file.grants.push(sample_grant("/p/a"));
        save_to(&p, &file).unwrap();
        let back = load_from(&p).unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn save_rejects_invalid_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("grants.json");
        let bad = GrantsFile {
            schema_version: 1,
            grants: vec![sample_grant("/p/a"), sample_grant("/p/a")],
        };
        let err = save_to(&p, &bad);
        assert!(matches!(err, Err(PermissionStoreError::Validation(_))));
        assert!(!p.exists(), "rejected file must never be written");
    }

    #[test]
    fn corrupt_file_is_moved_aside_and_reported() {
        // Fail-open hardening: corruption must still recover (boot
        // never wedges) but must be REPORTED — the grants file is
        // the only record obliging the orchestrator to revert an
        // elevated project.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("grants.json");
        std::fs::write(&p, b"not json at all").unwrap();
        let loaded = load_outcome_from(&p).unwrap();
        assert!(loaded.value.grants.is_empty());
        let rec = loaded
            .recovery
            .expect("corruption must surface a recovery marker");
        let copies = crate::json_store::corrupt_siblings(&p);
        assert_eq!(copies.len(), 1);
        assert_eq!(rec.moved_to.as_deref(), Some(copies[0].as_path()));
    }

    #[test]
    fn unsupported_schema_version_is_moved_aside_and_reported() {
        // App-downgrade path: a future schema_version fails validate
        // and must be treated exactly like corruption — recovered,
        // moved aside, and loudly reported.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("grants.json");
        std::fs::write(&p, br#"{"schema_version":99,"grants":[]}"#).unwrap();
        let loaded = load_outcome_from(&p).unwrap();
        assert!(loaded.value.grants.is_empty());
        let rec = loaded
            .recovery
            .expect("future schema must surface a recovery marker");
        assert!(rec.error.contains("unsupported"), "got: {}", rec.error);
        assert_eq!(crate::json_store::corrupt_siblings(&p).len(), 1);
    }

    #[test]
    fn clean_and_missing_files_carry_no_recovery_marker() {
        let tmp = tempfile::tempdir().unwrap();
        // Missing file.
        let missing = tmp.path().join("nope.json");
        assert!(load_outcome_from(&missing).unwrap().recovery.is_none());
        // Clean file.
        let p = tmp.path().join("grants.json");
        let mut file = GrantsFile::default();
        file.grants.push(sample_grant("/p/a"));
        save_to(&p, &file).unwrap();
        let loaded = load_outcome_from(&p).unwrap();
        assert!(loaded.recovery.is_none());
        assert_eq!(loaded.value, file);
    }

    #[test]
    fn invalid_but_parsable_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("grants.json");
        // Parses, fails validate (duplicate project).
        let bad = serde_json::json!({
            "schema_version": 1,
            "grants": [
                { "project_path": "/p/a", "layer": "local_project",
                  "granted_mode": "bypassPermissions", "previous_mode": "default",
                  "granted_at": "2023-11-14T22:13:20Z", "expires_at": "2023-11-15T00:13:20Z" },
                { "project_path": "/p/a", "layer": "local_project",
                  "granted_mode": "bypassPermissions", "previous_mode": "default",
                  "granted_at": "2023-11-14T22:13:20Z", "expires_at": "2023-11-15T00:13:20Z" }
            ]
        });
        std::fs::write(&p, bad.to_string()).unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.grants.is_empty());
        assert_eq!(crate::json_store::corrupt_siblings(&p).len(), 1);
    }
}
