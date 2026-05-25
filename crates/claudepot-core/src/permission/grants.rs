//! Time-boxed permission-grant schema. Pure data + serde + validation.
//!
//! A [`Grant`] records that Claudepot set `permissions.defaultMode`
//! on a project to a (usually elevated) mode, and must revert it at
//! `expires_at`. The grant carries `previous_mode` so revert restores
//! the exact prior state — including "the key was absent" (revert =
//! clear the key).
//!
//! Hand-edit-friendly like `rotation::rules`: `serde(default)` on the
//! schema version, no `deny_unknown_fields` at the top level, but
//! structural defects are rejected on [`GrantsFile::validate`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::permission::mode::PermissionMode;
use crate::settings_writer::SettingsLayer;

/// Bumped on schema-breaking changes. The store moves files with an
/// unrecognized version aside to `.corrupt`.
pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// Top-level on-disk shape of `~/.claudepot/permission-grants.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantsFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub grants: Vec<Grant>,
}

impl Default for GrantsFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            grants: Vec::new(),
        }
    }
}

/// One time-boxed permission grant. At most one grant exists per
/// `project_path` — the store's validation enforces this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Grant {
    /// Canonicalized project root the grant applies to. Identity key.
    pub project_path: String,
    /// Which settings file the grant wrote to (always `LocalProject`
    /// in practice; recorded so revert targets the same file).
    pub layer: SettingsLayer,
    /// The mode Claudepot set when the grant was created.
    pub granted_mode: PermissionMode,
    /// What `permissions.defaultMode` was in `layer` before the grant.
    /// `None` means the key was absent — revert clears it rather than
    /// writing a guessed default.
    pub previous_mode: Option<PermissionMode>,
    /// When the grant was created.
    pub granted_at: DateTime<Utc>,
    /// When the orchestrator must revert to `previous_mode`.
    /// `None` means the grant is **sticky** — never auto-reverted; it
    /// stays in effect until the user removes it explicitly via the
    /// Permissions UI (or by hand-editing settings, which un-manages
    /// it). Used for "auto mode" workflows where the elevated state
    /// should outlive any time bound. The architecture-rule guidance
    /// that "the elevated state is never left to memory" is honored
    /// because the grant record is still persistent: revert is
    /// available with one click and Claudepot still notices when the
    /// user un-elevates by hand.
    pub expires_at: Option<DateTime<Utc>>,
    /// Consecutive-failure circuit breaker state — number of revert
    /// attempts that have failed in an unbroken run. `#[serde(default)]`
    /// so `permission-grants.json` files written before the breaker
    /// shipped still deserialize (they read back as `0`). Reset to `0`
    /// implicitly — a grant whose revert succeeds is removed from the
    /// file entirely. See `claudepot_core::breaker`.
    #[serde(default)]
    pub consecutive_failures: u32,
    /// When the most recent revert failure happened — the breaker's
    /// cooldown clock. `None` iff `consecutive_failures == 0`.
    /// `#[serde(default)]` for the same backward-compat reason as
    /// `consecutive_failures`.
    #[serde(default)]
    pub last_failure_at: Option<DateTime<Utc>>,
}

impl Grant {
    /// True once `now` has reached or passed `expires_at`. Sticky
    /// grants (`expires_at = None`) are never expired.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        match self.expires_at {
            Some(deadline) => now >= deadline,
            None => false,
        }
    }

    /// True when the grant has no deadline (the "auto mode" / sticky
    /// shape). Provided so UI + audit code don't pattern-match on the
    /// raw Option at every call site.
    pub fn is_sticky(&self) -> bool {
        self.expires_at.is_none()
    }

    /// This grant's circuit-breaker ledger, read off the two breaker
    /// fields. Pass to `claudepot_core::breaker::evaluate` to decide
    /// whether the orchestrator should attempt the revert.
    pub fn breaker_ledger(&self) -> crate::breaker::FailureLedger {
        crate::breaker::FailureLedger {
            consecutive: self.consecutive_failures,
            last_failure: self.last_failure_at,
        }
    }

    /// Write `ledger` back onto the grant's two breaker fields. The
    /// orchestrator calls this after a failed revert (advanced
    /// ledger) before persisting the file.
    pub fn set_breaker_ledger(&mut self, ledger: crate::breaker::FailureLedger) {
        self.consecutive_failures = ledger.consecutive;
        self.last_failure_at = ledger.last_failure;
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("schema version {found} is unsupported (expected {expected})")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },
    #[error("grant project_path must not be empty")]
    EmptyProjectPath,
    #[error("grant for `{0}` has expires_at <= granted_at")]
    NonPositiveDuration(String),
    #[error("more than one grant targets project `{0}`")]
    DuplicateProject(String),
    #[error("write to the committed Project settings layer is not allowed for grant `{0}`")]
    ProjectLayerNotAllowed(String),
}

impl GrantsFile {
    /// Validate the whole file. The store refuses to persist an
    /// invalid file, so on-disk grants are always loadable + coherent.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnsupportedSchemaVersion {
                found: self.schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        let mut seen = std::collections::HashSet::new();
        for g in &self.grants {
            g.validate()?;
            if !seen.insert(g.project_path.clone()) {
                return Err(ValidationError::DuplicateProject(g.project_path.clone()));
            }
        }
        Ok(())
    }

    /// The grant for `project_path`, if any.
    pub fn find(&self, project_path: &str) -> Option<&Grant> {
        self.grants.iter().find(|g| g.project_path == project_path)
    }

    /// Mutable handle to the grant for `project_path`, if any. Used by
    /// the orchestrator to advance a grant's circuit-breaker fields
    /// in place after a failed revert.
    pub fn find_mut(&mut self, project_path: &str) -> Option<&mut Grant> {
        self.grants
            .iter_mut()
            .find(|g| g.project_path == project_path)
    }

    /// Insert or replace the grant for its `project_path`. Returns the
    /// previous grant for that path, if one existed.
    pub fn upsert(&mut self, grant: Grant) -> Option<Grant> {
        match self
            .grants
            .iter()
            .position(|g| g.project_path == grant.project_path)
        {
            Some(i) => Some(std::mem::replace(&mut self.grants[i], grant)),
            None => {
                self.grants.push(grant);
                None
            }
        }
    }

    /// Remove the grant for `project_path`. Returns it if it existed.
    pub fn remove(&mut self, project_path: &str) -> Option<Grant> {
        match self
            .grants
            .iter()
            .position(|g| g.project_path == project_path)
        {
            Some(i) => Some(self.grants.remove(i)),
            None => None,
        }
    }
}

impl Grant {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.project_path.trim().is_empty() {
            return Err(ValidationError::EmptyProjectPath);
        }
        // Time-boxed grants must have a positive duration. Sticky
        // grants (`expires_at = None`) skip this check by design —
        // they don't carry a deadline to validate against.
        if let Some(deadline) = self.expires_at {
            if deadline <= self.granted_at {
                return Err(ValidationError::NonPositiveDuration(
                    self.project_path.clone(),
                ));
            }
        }
        if matches!(self.layer, SettingsLayer::Project) {
            return Err(ValidationError::ProjectLayerNotAllowed(
                self.project_path.clone(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn sample_grant(path: &str) -> Grant {
        Grant {
            project_path: path.to_string(),
            layer: SettingsLayer::LocalProject,
            granted_mode: PermissionMode::BypassPermissions,
            previous_mode: Some(PermissionMode::Default),
            granted_at: ts(0),
            expires_at: Some(ts(7200)),
            consecutive_failures: 0,
            last_failure_at: None,
        }
    }

    fn sticky_grant(path: &str) -> Grant {
        Grant {
            expires_at: None,
            ..sample_grant(path)
        }
    }

    #[test]
    fn round_trips_through_json() {
        let mut file = GrantsFile::default();
        file.grants.push(sample_grant("/p/a"));
        let s = serde_json::to_string(&file).unwrap();
        let back: GrantsFile = serde_json::from_str(&s).unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn previous_mode_none_round_trips() {
        let mut g = sample_grant("/p/a");
        g.previous_mode = None;
        let s = serde_json::to_string(&g).unwrap();
        assert_eq!(serde_json::from_str::<Grant>(&s).unwrap(), g);
    }

    #[test]
    fn schema_version_defaults_when_omitted() {
        let json = r#"{"grants":[]}"#;
        let f: GrantsFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn is_expired_is_inclusive_of_the_deadline() {
        let g = sample_grant("/p/a");
        assert!(!g.is_expired(ts(7199)));
        assert!(g.is_expired(ts(7200)));
        assert!(g.is_expired(ts(7201)));
    }

    #[test]
    fn sticky_grant_is_never_expired() {
        let g = sticky_grant("/p/sticky");
        assert!(!g.is_expired(ts(0)));
        assert!(!g.is_expired(ts(86_400 * 365)));
        assert!(!g.is_expired(ts(i32::MAX as i64)));
        assert!(g.is_sticky());
    }

    #[test]
    fn sticky_grant_passes_validate() {
        let g = sticky_grant("/p/sticky");
        assert!(g.validate().is_ok());
    }

    #[test]
    fn sticky_grant_round_trips_through_json() {
        let g = sticky_grant("/p/sticky");
        let s = serde_json::to_string(&g).unwrap();
        // expires_at must serialize as JSON null for the sticky shape.
        assert!(s.contains("\"expires_at\":null"), "got: {s}");
        let back: Grant = serde_json::from_str(&s).unwrap();
        assert_eq!(back, g);
        assert!(back.is_sticky());
    }

    #[test]
    fn time_boxed_grant_round_trips_through_json() {
        // Lock the wire shape so old on-disk files still load.
        let g = sample_grant("/p/timed");
        let s = serde_json::to_string(&g).unwrap();
        assert!(s.contains("\"expires_at\":\""), "got: {s}");
        let back: Grant = serde_json::from_str(&s).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn validate_rejects_unknown_schema_version() {
        let f = GrantsFile {
            schema_version: 99,
            grants: vec![],
        };
        assert_eq!(
            f.validate(),
            Err(ValidationError::UnsupportedSchemaVersion {
                found: 99,
                expected: 1
            })
        );
    }

    #[test]
    fn validate_rejects_empty_project_path() {
        let mut g = sample_grant("");
        g.project_path = "   ".into();
        assert_eq!(g.validate(), Err(ValidationError::EmptyProjectPath));
    }

    #[test]
    fn validate_rejects_non_positive_duration() {
        let mut g = sample_grant("/p/a");
        g.expires_at = Some(g.granted_at);
        assert_eq!(
            g.validate(),
            Err(ValidationError::NonPositiveDuration("/p/a".into()))
        );
        g.expires_at = Some(ts(-1));
        assert_eq!(
            g.validate(),
            Err(ValidationError::NonPositiveDuration("/p/a".into()))
        );
    }

    #[test]
    fn validate_rejects_project_layer() {
        let mut g = sample_grant("/p/a");
        g.layer = SettingsLayer::Project;
        assert_eq!(
            g.validate(),
            Err(ValidationError::ProjectLayerNotAllowed("/p/a".into()))
        );
    }

    #[test]
    fn validate_file_rejects_duplicate_project() {
        let file = GrantsFile {
            schema_version: SCHEMA_VERSION,
            grants: vec![sample_grant("/p/a"), sample_grant("/p/a")],
        };
        assert_eq!(
            file.validate(),
            Err(ValidationError::DuplicateProject("/p/a".into()))
        );
    }

    #[test]
    fn empty_file_validates() {
        assert!(GrantsFile::default().validate().is_ok());
    }

    #[test]
    fn upsert_replaces_existing_grant_for_same_path() {
        let mut file = GrantsFile::default();
        assert!(file.upsert(sample_grant("/p/a")).is_none());
        let mut updated = sample_grant("/p/a");
        updated.granted_mode = PermissionMode::Plan;
        let prev = file.upsert(updated.clone()).unwrap();
        assert_eq!(prev.granted_mode, PermissionMode::BypassPermissions);
        assert_eq!(file.grants.len(), 1);
        assert_eq!(file.grants[0], updated);
        assert!(file.validate().is_ok());
    }

    #[test]
    fn test_grant_breaker_fields_default_when_absent_in_json() {
        // A `permission-grants.json` written before the circuit
        // breaker shipped has no `consecutive_failures` /
        // `last_failure_at` keys — it must still deserialize, with
        // the breaker fields defaulting to the clean state.
        let json = r#"{
            "project_path": "/p/a",
            "layer": "local_project",
            "granted_mode": "bypassPermissions",
            "previous_mode": "default",
            "granted_at": "2023-11-14T22:13:20Z",
            "expires_at": "2023-11-15T00:13:20Z"
        }"#;
        let g: Grant = serde_json::from_str(json).unwrap();
        assert_eq!(g.consecutive_failures, 0);
        assert_eq!(g.last_failure_at, None);
        assert_eq!(g.breaker_ledger(), crate::breaker::FailureLedger::default());
    }

    #[test]
    fn test_grant_breaker_fields_round_trip_when_present() {
        let mut g = sample_grant("/p/a");
        g.consecutive_failures = 4;
        g.last_failure_at = Some(ts(123));
        let s = serde_json::to_string(&g).unwrap();
        let back: Grant = serde_json::from_str(&s).unwrap();
        assert_eq!(back, g);
        assert_eq!(back.consecutive_failures, 4);
        assert_eq!(back.last_failure_at, Some(ts(123)));
    }

    #[test]
    fn test_grant_breaker_ledger_round_trips_through_setter() {
        let mut g = sample_grant("/p/a");
        let ledger = crate::breaker::FailureLedger {
            consecutive: 2,
            last_failure: Some(ts(50)),
        };
        g.set_breaker_ledger(ledger);
        assert_eq!(g.consecutive_failures, 2);
        assert_eq!(g.last_failure_at, Some(ts(50)));
        assert_eq!(g.breaker_ledger(), ledger);
    }

    #[test]
    fn find_and_remove_work_by_path() {
        let mut file = GrantsFile::default();
        file.upsert(sample_grant("/p/a"));
        file.upsert(sample_grant("/p/b"));
        assert!(file.find("/p/a").is_some());
        assert!(file.find("/p/missing").is_none());
        let removed = file.remove("/p/a").unwrap();
        assert_eq!(removed.project_path, "/p/a");
        assert!(file.find("/p/a").is_none());
        assert!(file.remove("/p/a").is_none());
    }
}
