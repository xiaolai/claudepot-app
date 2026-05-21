//! Atomic load/save of `~/.claudepot/rotation-breaker.json`.
//!
//! Rotation's auto-mode has no persisted per-rule unit — swaps are
//! re-derived from the snapshot each tick, so there is no pending
//! entry to flag the way a permission `Grant` can be. The
//! consecutive-failure circuit breaker therefore needs its own home:
//! a small JSON map from `rule_id` to a [`FailureLedger`].
//!
//! Deriving the failure count from `rotation-audit.json` was
//! considered and rejected — that log is a 500-entry ring buffer, so
//! a busy account could evict the failure history before the breaker
//! reads it. The breaker state must be authoritative.
//!
//! Mirrors `rotation::store` exactly: missing file → empty;
//! corrupt/invalid file → renamed aside to `<path>.corrupt`, return
//! empty, log a warn; a *real* I/O failure (permission denied, disk
//! gone) propagates as `Err` so the orchestrator skips the tick
//! instead of clobbering the user's real breaker state on the next
//! save.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::breaker::FailureLedger;
use crate::fs_utils::atomic_write;

/// Bumped on schema-breaking changes. A file with an unrecognized
/// version is treated as corrupt (moved aside, empty returned).
pub const SCHEMA_VERSION: u32 = 1;

/// Standard filename inside `claudepot_data_dir()`.
pub const BREAKER_FILENAME: &str = "rotation-breaker.json";

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// `~/.claudepot/rotation-breaker.json` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn breaker_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(BREAKER_FILENAME)
}

/// On-disk shape of one rule's failure ledger. `FailureLedger` itself
/// is not `Serialize` (it is pure runtime logic); this is its
/// serde-friendly mirror so the breaker module stays I/O-free.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LedgerEntry {
    #[serde(default)]
    pub consecutive: u32,
    #[serde(default)]
    pub last_failure: Option<DateTime<Utc>>,
}

impl From<FailureLedger> for LedgerEntry {
    fn from(l: FailureLedger) -> Self {
        Self {
            consecutive: l.consecutive,
            last_failure: l.last_failure,
        }
    }
}

impl From<LedgerEntry> for FailureLedger {
    fn from(e: LedgerEntry) -> Self {
        Self {
            consecutive: e.consecutive,
            last_failure: e.last_failure,
        }
    }
}

/// Top-level on-disk shape of `~/.claudepot/rotation-breaker.json`.
/// `ledgers` maps `rule_id` → that rule's breaker state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BreakerFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub ledgers: BTreeMap<String, LedgerEntry>,
}

impl Default for BreakerFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ledgers: BTreeMap::new(),
        }
    }
}

impl BreakerFile {
    /// Validate the whole file. The store refuses to persist an
    /// invalid file, so on-disk state is always loadable + coherent.
    pub fn validate(&self) -> Result<(), RotationBreakerError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(RotationBreakerError::UnsupportedSchemaVersion {
                found: self.schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    /// The ledger for `rule_id`, or the default clean ledger when the
    /// rule has no recorded failures.
    pub fn ledger_for(&self, rule_id: &str) -> FailureLedger {
        self.ledgers
            .get(rule_id)
            .copied()
            .map(FailureLedger::from)
            .unwrap_or_default()
    }

    /// Store `ledger` for `rule_id`. A clean (default) ledger removes
    /// the entry instead of writing a zero row — the file only ever
    /// holds rules with a live failure run.
    pub fn set_ledger(&mut self, rule_id: &str, ledger: FailureLedger) {
        if ledger == FailureLedger::default() {
            self.ledgers.remove(rule_id);
        } else {
            self.ledgers.insert(rule_id.to_string(), ledger.into());
        }
    }

    /// Drop the breaker state for `rule_id`. Used to prune entries for
    /// rules the user has since deleted, so the file doesn't grow
    /// stale ledgers for rule ids that no longer exist.
    pub fn retain_rules(&mut self, live_rule_ids: &std::collections::HashSet<String>) {
        self.ledgers.retain(|id, _| live_rule_ids.contains(id));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RotationBreakerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("schema version {found} is unsupported (expected {expected})")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },
}

/// Load breaker state from the canonical path. See
/// `rotation::store::load` for the three-outcome contract — `Ok`
/// covers success, missing file, and recovered-from-corruption;
/// `Err` is a real I/O failure.
pub fn load() -> std::io::Result<BreakerFile> {
    load_from(&breaker_path())
}

/// Test-friendly load that takes the path directly. See [`load`].
pub fn load_from(path: &Path) -> std::io::Result<BreakerFile> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BreakerFile::default());
        }
        Err(e) => return Err(e),
    };
    match serde_json::from_slice::<BreakerFile>(&bytes) {
        Ok(file) => match file.validate() {
            Ok(()) => Ok(file),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "rotation_breaker_store: parsed but invalid; moving aside and starting empty"
                );
                move_aside(path);
                Ok(BreakerFile::default())
            }
        },
        Err(e) => {
            tracing::warn!(
                error = %e,
                "rotation_breaker_store: parse failed; moving aside and starting empty"
            );
            move_aside(path);
            Ok(BreakerFile::default())
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
pub fn load_or_default() -> BreakerFile {
    match load() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "rotation_breaker_store: read failed; defaulting to empty");
            BreakerFile::default()
        }
    }
}

/// Persist `file` to the canonical path. Validates before writing —
/// invalid input is rejected so on-disk files are always loadable.
pub fn save(file: &BreakerFile) -> Result<(), RotationBreakerError> {
    save_to(&breaker_path(), file)
}

/// Test-friendly save that takes the path directly.
pub fn save_to(path: &Path, file: &BreakerFile) -> Result<(), RotationBreakerError> {
    file.validate()?;
    let json = serde_json::to_vec_pretty(file)?;
    atomic_write(path, &json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 21, 12, 0, 0).unwrap()
    }

    fn sample_ledger() -> FailureLedger {
        FailureLedger {
            consecutive: 2,
            last_failure: Some(ts()),
        }
    }

    #[test]
    fn test_breaker_store_load_missing_file_yields_default() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("nope.json");
        let f = load_from(&p).unwrap();
        assert_eq!(f.schema_version, SCHEMA_VERSION);
        assert!(f.ledgers.is_empty());
    }

    #[test]
    fn test_breaker_store_save_then_load_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("breaker.json");
        let mut file = BreakerFile::default();
        file.set_ledger("rule-a", sample_ledger());
        save_to(&p, &file).unwrap();
        let back = load_from(&p).unwrap();
        assert_eq!(back, file);
        assert_eq!(back.ledger_for("rule-a"), sample_ledger());
    }

    #[test]
    fn test_breaker_store_ledger_for_unknown_rule_is_default() {
        let f = BreakerFile::default();
        assert_eq!(f.ledger_for("never-seen"), FailureLedger::default());
    }

    #[test]
    fn test_breaker_store_set_ledger_clean_removes_entry() {
        let mut f = BreakerFile::default();
        f.set_ledger("rule-a", sample_ledger());
        assert_eq!(f.ledgers.len(), 1);
        // Storing a default (clean) ledger prunes the row.
        f.set_ledger("rule-a", FailureLedger::default());
        assert!(f.ledgers.is_empty());
    }

    #[test]
    fn test_breaker_store_retain_rules_prunes_deleted_rules() {
        let mut f = BreakerFile::default();
        f.set_ledger("rule-a", sample_ledger());
        f.set_ledger("rule-b", sample_ledger());
        f.set_ledger("rule-c", sample_ledger());
        let mut live = std::collections::HashSet::new();
        live.insert("rule-a".to_string());
        live.insert("rule-c".to_string());
        f.retain_rules(&live);
        assert!(f.ledgers.contains_key("rule-a"));
        assert!(!f.ledgers.contains_key("rule-b"));
        assert!(f.ledgers.contains_key("rule-c"));
    }

    #[test]
    fn test_breaker_store_corrupt_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("breaker.json");
        std::fs::write(&p, b"this is not json").unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.ledgers.is_empty());
        assert!(
            p.with_extension("json.corrupt").exists(),
            "corrupt file should be moved aside"
        );
    }

    #[test]
    fn test_breaker_store_unsupported_schema_version_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("breaker.json");
        // Parses fine, fails validate (bad schema version).
        std::fs::write(&p, br#"{"schema_version":99,"ledgers":{}}"#).unwrap();
        let f = load_from(&p).unwrap();
        assert!(f.ledgers.is_empty());
        assert!(p.with_extension("json.corrupt").exists());
    }

    #[test]
    fn test_breaker_store_schema_version_defaults_when_omitted() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("breaker.json");
        std::fs::write(&p, br#"{"ledgers":{}}"#).unwrap();
        let f = load_from(&p).unwrap();
        assert_eq!(f.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn test_breaker_store_save_rejects_invalid_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("breaker.json");
        let bad = BreakerFile {
            schema_version: 99,
            ledgers: BTreeMap::new(),
        };
        let err = save_to(&p, &bad);
        assert!(matches!(
            err,
            Err(RotationBreakerError::UnsupportedSchemaVersion { .. })
        ));
        assert!(!p.exists(), "rejected file must never be written");
    }

    #[cfg(unix)]
    #[test]
    fn test_breaker_store_permission_denied_returns_err() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("breaker.json");
        std::fs::write(&p, br#"{"schema_version":1,"ledgers":{}}"#).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = load_from(&p);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(result.is_err(), "permission denied must surface as Err");
    }

    #[cfg(unix)]
    #[test]
    fn test_breaker_store_save_writes_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("breaker.json");
        save_to(&p, &BreakerFile::default()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
