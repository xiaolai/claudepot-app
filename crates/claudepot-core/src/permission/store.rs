//! Atomic load/save of `~/.claudepot/permission-grants.json`.
//!
//! Mirrors `rotation::store`: missing file → empty grants; corrupt or
//! invalid file → rename to `<path>.corrupt`, return empty, log a
//! warn. A *real* I/O failure (permission denied, disk gone)
//! propagates as `Err` so callers don't mistake it for "no grants"
//! and clobber the user's real file on the next save.

use std::path::{Path, PathBuf};

use crate::fs_utils::atomic_write;
use crate::permission::grants::{GrantsFile, ValidationError};

/// Standard filename inside `claudepot_data_dir()`.
pub const GRANTS_FILENAME: &str = "permission-grants.json";

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

/// Load grants from the canonical path. See `rotation::store::load`
/// for the three-outcome contract — `Ok` covers success, missing
/// file, and recovered-from-corruption; `Err` is a real I/O failure.
pub fn load() -> std::io::Result<GrantsFile> {
    load_from(&grants_path())
}

/// Test-friendly load that takes the path directly. See [`load`].
pub fn load_from(path: &Path) -> std::io::Result<GrantsFile> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GrantsFile::default());
        }
        Err(e) => return Err(e),
    };
    match serde_json::from_slice::<GrantsFile>(&bytes) {
        Ok(file) => match file.validate() {
            Ok(()) => Ok(file),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "permission_store: parsed but invalid; moving aside and starting empty"
                );
                move_aside(path);
                Ok(GrantsFile::default())
            }
        },
        Err(e) => {
            tracing::warn!(
                error = %e,
                "permission_store: parse failed; moving aside and starting empty"
            );
            move_aside(path);
            Ok(GrantsFile::default())
        }
    }
}

fn move_aside(path: &Path) {
    let corrupt = path.with_extension("json.corrupt");
    let _ = std::fs::rename(path, corrupt);
}

/// Log + swallow real I/O errors, always returning a usable file.
/// Use only where errors cannot be propagated; new code prefers
/// [`load`].
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
    file.validate()?;
    let json = serde_json::to_vec_pretty(file)?;
    atomic_write(path, &json)?;
    Ok(())
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
    fn corrupt_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("grants.json");
        std::fs::write(&p, b"not json at all").unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.grants.is_empty());
        assert!(p.with_extension("json.corrupt").exists());
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
        assert!(p.with_extension("json.corrupt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_returns_err_not_default() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("grants.json");
        std::fs::write(&p, br#"{"schema_version":1,"grants":[]}"#).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = load_from(&p);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(result.is_err(), "permission denied must surface as Err");
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("grants.json");
        save_to(&p, &GrantsFile::default()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
