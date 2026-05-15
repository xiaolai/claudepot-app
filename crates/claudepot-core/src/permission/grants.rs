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
    pub expires_at: DateTime<Utc>,
}

impl Grant {
    /// True once `now` has reached or passed `expires_at`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
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
        if self.expires_at <= self.granted_at {
            return Err(ValidationError::NonPositiveDuration(
                self.project_path.clone(),
            ));
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
            expires_at: ts(7200),
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
        g.expires_at = g.granted_at;
        assert_eq!(
            g.validate(),
            Err(ValidationError::NonPositiveDuration("/p/a".into()))
        );
        g.expires_at = ts(-1);
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
