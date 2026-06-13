//! Atomic load/save of `~/.claudepot/rotation-rules.json`.
//!
//! Thin wrapper over [`crate::json_store`] — see that module for the
//! three-outcome load contract and the corruption-recovery policy
//! (timestamped rename-aside, warn on rename failure, atomic write).
//! This file keeps only the filename const, the error enum, the
//! `Validate` wiring, and domain tests.

use std::path::{Path, PathBuf};

use crate::json_store::{self, SaveError};
use crate::rotation::rules::{RotationRulesFile, ValidationError};

/// Standard filename inside `claudepot_data_dir()`.
pub const RULES_FILENAME: &str = "rotation-rules.json";

/// Store name used in log messages.
const STORE: &str = "rotation_store";

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

impl json_store::Validate for RotationRulesFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), ValidationError> {
        // Delegates to the inherent method (inherent methods win
        // resolution over trait methods, so this is not recursion).
        RotationRulesFile::validate(self)
    }
}

/// Load rules from the canonical path under the three-outcome
/// contract (see [`crate::json_store`]): `Ok` covers success,
/// missing file, and recovered-from-corruption; `Err` is a real I/O
/// failure the caller must not mistake for "no rules".
pub fn load() -> std::io::Result<RotationRulesFile> {
    load_from(&rules_path())
}

/// Test-friendly load that takes the path directly. See [`load`].
pub fn load_from(path: &Path) -> std::io::Result<RotationRulesFile> {
    json_store::load(path, STORE)
}

/// Convenience: log + swallow real I/O errors, always returning a
/// usable file. Use this from sites that can't propagate errors
/// (legacy callers, the dry-run command's snapshot read). New code
/// should prefer [`load`] and surface failures.
pub fn load_or_default() -> RotationRulesFile {
    json_store::load_or_default(&rules_path(), STORE)
}

/// Persist `file` to the canonical path. Validates before writing —
/// invalid input is rejected so on-disk files are always loadable.
pub fn save(file: &RotationRulesFile) -> Result<(), RotationStoreError> {
    save_to(&rules_path(), file)
}

/// Test-friendly save that takes the path directly.
pub fn save_to(path: &Path, file: &RotationRulesFile) -> Result<(), RotationStoreError> {
    json_store::save(path, file).map_err(|e| match e {
        SaveError::Validation(v) => RotationStoreError::Validation(v),
        SaveError::Serde(s) => RotationStoreError::Serde(s),
        SaveError::Io(io) => RotationStoreError::Io(io),
    })
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
        assert_eq!(
            crate::json_store::corrupt_siblings(&p).len(),
            1,
            "corrupt file should be moved aside (timestamped)"
        );
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
        assert_eq!(crate::json_store::corrupt_siblings(&p).len(), 1);
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
        assert_eq!(crate::json_store::corrupt_siblings(&p).len(), 1);
    }
}
