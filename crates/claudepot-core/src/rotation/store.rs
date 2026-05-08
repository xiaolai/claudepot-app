//! Atomic load/save of `~/.claudepot/rotation-rules.json`.
//!
//! Mirrors the corruption-recovery pattern in `notification_log` and
//! `usage_alerts`: missing file → empty rules; corrupt file → rename
//! to `<path>.corrupt`, return empty, log a warn. Never fatal at boot.

use std::path::{Path, PathBuf};

use crate::fs_utils::atomic_write;
use crate::rotation::rules::{RotationRulesFile, ValidationError};

/// Standard filename inside `claudepot_data_dir()`.
pub const RULES_FILENAME: &str = "rotation-rules.json";

/// `~/.claudepot/rotation-rules.json` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn rules_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(RULES_FILENAME)
}

#[derive(Debug, thiserror::Error)]
pub enum RotationStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("validation: {0}")]
    Validation(#[from] ValidationError),
}

/// Load rules from the canonical path. Three outcomes:
///
/// - `Ok(file)` — successfully read + parsed + validated, OR the
///   file didn't exist (returns `RotationRulesFile::default()`), OR
///   the file existed but was corrupt (moved aside; default
///   returned).
/// - `Err(io_error)` — a *real* filesystem failure (permission
///   denied, transient I/O error, disk unmounted). Caller should
///   refuse to act on the assumption "no rules" until the error is
///   resolved — silently treating a permission failure as
///   "no rules" then saving would clobber the user's real config.
///
/// Corruption recovery is intentionally NOT an error case: a
/// missing or unparseable file is a recoverable steady state, and
/// the rename-aside `<path>.corrupt` preserves forensics.
pub fn load() -> std::io::Result<RotationRulesFile> {
    load_from(&rules_path())
}

/// Test-friendly load that takes the path directly. See [`load`].
pub fn load_from(path: &Path) -> std::io::Result<RotationRulesFile> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RotationRulesFile::default());
        }
        Err(e) => return Err(e),
    };
    match serde_json::from_slice::<RotationRulesFile>(&bytes) {
        Ok(file) => match file.validate() {
            Ok(()) => Ok(file),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "rotation_store: parsed but invalid; moving aside and starting empty"
                );
                let corrupt = path.with_extension("json.corrupt");
                let _ = std::fs::rename(path, corrupt);
                Ok(RotationRulesFile::default())
            }
        },
        Err(e) => {
            tracing::warn!(
                error = %e,
                "rotation_store: parse failed; moving aside and starting empty"
            );
            let corrupt = path.with_extension("json.corrupt");
            let _ = std::fs::rename(path, corrupt);
            Ok(RotationRulesFile::default())
        }
    }
}

/// Convenience: log + swallow real I/O errors, always returning a
/// usable file. Use this from sites that can't propagate errors
/// (legacy callers, the dry-run command's snapshot read). New code
/// should prefer [`load`] and surface failures.
pub fn load_or_default() -> RotationRulesFile {
    match load() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "rotation_store: read failed; defaulting to empty (real failures should propagate, but caller asked for default)");
            RotationRulesFile::default()
        }
    }
}

/// Persist `file` to the canonical path. Validates before writing —
/// invalid input is rejected so on-disk files are always loadable.
pub fn save(file: &RotationRulesFile) -> Result<(), RotationStoreError> {
    save_to(&rules_path(), file)
}

/// Test-friendly save that takes the path directly.
pub fn save_to(path: &Path, file: &RotationRulesFile) -> Result<(), RotationStoreError> {
    file.validate()?;
    let json = serde_json::to_vec_pretty(file)?;
    atomic_write(path, &json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::rules::{
        Action, RotationGuards, RotationMode, RotationRule, Selector, Trigger, SCHEMA_VERSION,
    };
    use crate::services::usage_alerts::UsageWindowKind;

    fn sample_rule() -> RotationRule {
        RotationRule {
            id: "5h-near-cap".into(),
            enabled: true,
            trigger: Trigger::UtilizationThreshold {
                window: UsageWindowKind::FiveHour,
                pct: 90,
            },
            action: Action::RotateTo {
                selector: Selector::LeastUsed {
                    window: UsageWindowKind::FiveHour,
                    candidates: vec!["a@x.com".into(), "b@x.com".into()],
                },
            },
            mode: RotationMode::Confirm,
            guards: RotationGuards::default(),
        }
    }

    #[test]
    fn load_missing_file_yields_default() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("nope.json");
        let f = load_from(&p).unwrap();
        assert_eq!(f.schema_version, SCHEMA_VERSION);
        assert!(f.rules.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("rules.json");
        let mut file = RotationRulesFile::default();
        file.rules.push(sample_rule());
        save_to(&p, &file).unwrap();
        let back = load_from(&p).unwrap();
        assert_eq!(back.rules.len(), 1);
        assert_eq!(back.rules[0], sample_rule());
    }

    #[test]
    fn save_rejects_invalid_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("rules.json");
        let mut bad = RotationRulesFile::default();
        let mut r = sample_rule();
        r.id = "".into();
        bad.rules.push(r);
        let err = save_to(&p, &bad);
        assert!(matches!(err, Err(RotationStoreError::Validation(_))));
        // The temp-rejected file is never written.
        assert!(!p.exists());
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("rules.json");
        std::fs::write(&p, b"this is not json").unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.rules.is_empty());
        let corrupt = p.with_extension("json.corrupt");
        assert!(corrupt.exists(), "corrupt file should be moved aside");
    }

    #[test]
    fn invalid_but_parsable_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("rules.json");
        // Parses fine, fails validate (duplicate id).
        let bad = serde_json::json!({
            "schema_version": 1,
            "rules": [
                { "id": "dup", "trigger": { "kind": "utilization_threshold", "window": "five_hour", "pct": 90 },
                  "action": { "kind": "rotate_to", "selector": { "kind": "least_used", "window": "five_hour", "candidates": ["a@x.com"] } } },
                { "id": "dup", "trigger": { "kind": "utilization_threshold", "window": "five_hour", "pct": 95 },
                  "action": { "kind": "rotate_to", "selector": { "kind": "least_used", "window": "five_hour", "candidates": ["b@x.com"] } } }
            ]
        });
        std::fs::write(&p, bad.to_string()).unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.rules.is_empty());
        let corrupt = p.with_extension("json.corrupt");
        assert!(corrupt.exists());
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_returns_err_not_default() {
        // Regression for the silent-clobber bug: a permission error
        // on read used to look like "no rules" — a follow-up save
        // would then write a fresh file and lose the user's real
        // config. Now the error propagates so the caller can refuse
        // to act on the assumption.
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("rules.json");
        std::fs::write(&p, br#"{"schema_version":1,"rules":[]}"#).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = load_from(&p);
        // Restore permissions so the tempdir cleanup can delete it.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(result.is_err(), "permission denied must surface as Err");
    }

    /// Add a malformed-JSON regression test (Codex audit Low finding):
    /// a hand-edited file with a bogus window value parses to a
    /// SerdeError, which the load path treats as corruption.
    #[test]
    fn malformed_window_value_is_treated_as_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("rules.json");
        let bad = r#"{"schema_version":1,"rules":[{"id":"r","enabled":true,
            "trigger":{"kind":"utilization_threshold","window":"not_a_window","pct":90},
            "action":{"kind":"rotate_to","selector":{"kind":"explicit","email":"a@x.com"}}}]}"#;
        std::fs::write(&p, bad).unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.rules.is_empty());
        assert!(p.with_extension("json.corrupt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("rules.json");
        let file = RotationRulesFile::default();
        save_to(&p, &file).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
