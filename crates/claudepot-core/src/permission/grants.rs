//! Permission-grant schema. Pure data + serde + validation.
//!
//! A [`Grant`] records that, until `expires_at`, Claudepot answers
//! Claude Code's permission prompts for sessions whose working
//! directory is inside `project_path`. It is read by the
//! `PermissionRequest` hook on every prompt (`permission::hook`), so
//! the record on disk **is** the capability: delete the row and the
//! next prompt is drawn at the keyboard. Nothing in CC's settings is
//! written for a grant.
//!
//! **Schema 1 wrote CC settings instead**, and Claude Code stopped
//! honouring that write in 2.1.257 (`settings::PROJECT_SCOPE_IGNORES_SINCE`).
//! A v1 file still loads: its grants land in
//! [`GrantsFile::legacy`], each carrying what the orchestrator needs to
//! put the settings file back (`layer`, `granted_mode`,
//! `previous_mode`), and are promoted to hook grants once that revert
//! has landed. Reading a v1 file as corrupt would have dropped the one
//! record obliging anything to clean that key up.
//!
//! Hand-edit-friendly like `rotation::rules`: `serde(default)` on the
//! schema version, no `deny_unknown_fields` at the top level, but
//! structural defects are rejected on [`GrantsFile::validate`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::permission::mode::PermissionMode;
use crate::settings_writer::SettingsLayer;

/// Bumped on schema-breaking changes. The store moves files with an
/// unrecognized version aside to `.corrupt` — except version 1, which
/// is migrated on read (see the module docs).
///
/// **Downgrade note.** A Claudepot older than this schema reads a v2
/// file as unsupported, moves it aside and starts empty. That fails
/// closed: the older binary's hook verb knows nothing about grants, so
/// every prompt is drawn at the keyboard, and nothing stays granted.
pub const SCHEMA_VERSION: u32 = 2;

/// The schema this replaced. Recognised on read, never written.
const LEGACY_SCHEMA_VERSION: u32 = 1;

/// Top-level on-disk shape of `~/.claudepot/permission-grants.json`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GrantsFile {
    pub schema_version: u32,
    pub grants: Vec<Grant>,
    /// Schema-1 records whose CC settings write has not been reverted
    /// yet. Present only while the orchestrator is still working
    /// through a migration; an empty list is not serialized, so a file
    /// that has finished migrating carries no trace of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy: Vec<LegacySettingsGrant>,
}

impl Default for GrantsFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            grants: Vec::new(),
            legacy: Vec::new(),
        }
    }
}

/// One permission grant. At most one grant exists per `project_path`
/// — the store's validation enforces this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Grant {
    /// Canonicalized project root the grant applies to. Identity key.
    /// A session counts as inside it when its working directory is
    /// this path or a descendant (`hook::path_is_within`).
    pub project_path: String,
    /// When the grant was created.
    pub granted_at: DateTime<Utc>,
    /// When the hook stops answering. `None` means the grant is
    /// **sticky** — never auto-expired; it stays in effect until the
    /// user removes it from the Permissions UI. The record is still
    /// persistent and visible with a one-click revert, which is what
    /// keeps "the elevated state is never left to memory" true of a
    /// grant with no deadline.
    pub expires_at: Option<DateTime<Utc>>,
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
}

/// A schema-1 grant: Claudepot wrote `permissions.defaultMode` into
/// `layer` and owes that file a revert. Everything the revert needs
/// travels with it, plus how many times the revert has failed so a
/// malformed settings file cannot make the orchestrator retry forever.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacySettingsGrant {
    pub project_path: String,
    /// Which settings file the grant wrote to.
    pub layer: SettingsLayer,
    /// The mode Claudepot set when the grant was created.
    pub granted_mode: PermissionMode,
    /// What `permissions.defaultMode` was in `layer` before the grant.
    /// `None` means the key was absent — revert clears it.
    pub previous_mode: Option<PermissionMode>,
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Consecutive failed reverts. The orchestrator gives up after
    /// [`LegacySettingsGrant::MAX_REVERT_ATTEMPTS`] and reports the
    /// file instead of retrying every tick for the app's lifetime.
    #[serde(default)]
    pub revert_attempts: u32,
}

impl LegacySettingsGrant {
    /// How many failed reverts a legacy record survives. Three, like
    /// `crate::breaker::THRESHOLD`: one failure is a transient lock,
    /// three in a row is a settings file that needs a person.
    pub const MAX_REVERT_ATTEMPTS: u32 = 3;

    /// The hook grant this record becomes once its settings write is
    /// undone: same project, same deadline. `None` when the deadline
    /// has already passed — there is nothing left to grant.
    pub fn promoted(&self, now: DateTime<Utc>) -> Option<Grant> {
        if let Some(deadline) = self.expires_at {
            if now >= deadline {
                return None;
            }
        }
        Some(Grant {
            project_path: self.project_path.clone(),
            granted_at: self.granted_at,
            expires_at: self.expires_at,
        })
    }
}

/// What serde reads before the schema is known. `schema_version` is
/// optional here so a file that omits it can be classified by shape.
#[derive(Deserialize)]
struct RawGrantsFile {
    schema_version: Option<u32>,
    #[serde(default)]
    grants: Vec<serde_json::Value>,
    #[serde(default)]
    legacy: Vec<LegacySettingsGrant>,
}

impl<'de> Deserialize<'de> for GrantsFile {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = RawGrantsFile::deserialize(de)?;
        // A schema-1 grant is recognisable by the settings layer it
        // wrote to; a v2 grant has no such field. That is the tie-break
        // for a file with no version key at all.
        let looks_legacy = |g: &serde_json::Value| g.get("layer").is_some();
        let version = raw.schema_version.unwrap_or_else(|| {
            if raw.grants.iter().any(looks_legacy) {
                LEGACY_SCHEMA_VERSION
            } else {
                SCHEMA_VERSION
            }
        });
        let mut legacy = raw.legacy;
        let grants = if version == LEGACY_SCHEMA_VERSION {
            for g in raw.grants {
                legacy.push(serde_json::from_value(g).map_err(D::Error::custom)?);
            }
            Vec::new()
        } else {
            raw.grants
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<Vec<Grant>, _>>()
                .map_err(D::Error::custom)?
        };
        Ok(Self {
            // A migrated file is a v2 file from here on; `validate`
            // still refuses anything newer than it knows.
            schema_version: if version == LEGACY_SCHEMA_VERSION {
                SCHEMA_VERSION
            } else {
                version
            },
            grants,
            legacy,
        })
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

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// The module segment is `permission_grants`, not `permission` — two
/// sibling modules (`rotation::rules`, `pricing::history`) also name an
/// enum `ValidationError`, and a shared `validation.*` namespace would
/// make the three indistinguishable to a translator.
impl crate::error_code::ErrorCode for ValidationError {
    fn code(&self) -> &'static str {
        match self {
            ValidationError::UnsupportedSchemaVersion { .. } => {
                "permission_grants.unsupported_schema_version"
            }
            ValidationError::EmptyProjectPath => "permission_grants.empty_project_path",
            ValidationError::NonPositiveDuration(_) => "permission_grants.non_positive_duration",
            ValidationError::DuplicateProject(_) => "permission_grants.duplicate_project",
            ValidationError::ProjectLayerNotAllowed(_) => {
                "permission_grants.project_layer_not_allowed"
            }
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            ValidationError::UnsupportedSchemaVersion { found, expected } => {
                serde_json::json!({ "found": found, "expected": expected })
            }
            ValidationError::EmptyProjectPath => serde_json::json!({}),
            // A grant is keyed by project_path, so the `{0}` in all
            // three messages is that path — never a secret.
            ValidationError::NonPositiveDuration(path)
            | ValidationError::DuplicateProject(path)
            | ValidationError::ProjectLayerNotAllowed(path) => {
                serde_json::json!({ "project_path": path })
            }
        }
    }
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
        let mut seen_legacy = std::collections::HashSet::new();
        for l in &self.legacy {
            l.validate()?;
            if !seen_legacy.insert(l.project_path.clone()) {
                return Err(ValidationError::DuplicateProject(l.project_path.clone()));
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

    /// Drop the legacy record for `project_path` and, unless it had
    /// already expired, carry its deadline over as a hook grant.
    /// Returns the promoted grant. An existing hook grant for the same
    /// project wins — the user re-granted after upgrading, and their
    /// newer deadline is the one that counts.
    pub fn promote_legacy(&mut self, project_path: &str, now: DateTime<Utc>) -> Option<Grant> {
        let i = self
            .legacy
            .iter()
            .position(|l| l.project_path == project_path)?;
        let legacy = self.legacy.remove(i);
        let promoted = legacy.promoted(now)?;
        if self.find(project_path).is_none() {
            self.grants.push(promoted.clone());
        }
        Some(promoted)
    }
}

impl Grant {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_window(&self.project_path, self.granted_at, self.expires_at)
    }
}

impl LegacySettingsGrant {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_window(&self.project_path, self.granted_at, self.expires_at)?;
        if matches!(self.layer, SettingsLayer::Project) {
            return Err(ValidationError::ProjectLayerNotAllowed(
                self.project_path.clone(),
            ));
        }
        Ok(())
    }
}

fn validate_window(
    project_path: &str,
    granted_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(), ValidationError> {
    if project_path.trim().is_empty() {
        return Err(ValidationError::EmptyProjectPath);
    }
    // Time-boxed grants must have a positive duration. Sticky grants
    // (`expires_at = None`) skip this check by design — they don't
    // carry a deadline to validate against.
    if let Some(deadline) = expires_at {
        if deadline <= granted_at {
            return Err(ValidationError::NonPositiveDuration(
                project_path.to_string(),
            ));
        }
    }
    Ok(())
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
            granted_at: ts(0),
            expires_at: Some(ts(7200)),
        }
    }

    fn sticky_grant(path: &str) -> Grant {
        Grant {
            expires_at: None,
            ..sample_grant(path)
        }
    }

    /// Byte-for-byte what a schema-1 Claudepot wrote.
    const V1_FILE: &str = r#"{
        "schema_version": 1,
        "grants": [{
            "project_path": "/p/a",
            "layer": "local_project",
            "granted_mode": "bypassPermissions",
            "previous_mode": "default",
            "granted_at": "2023-11-14T22:13:20Z",
            "expires_at": "2023-11-15T00:13:20Z",
            "consecutive_failures": 2,
            "last_failure_at": "2023-11-14T23:00:00Z"
        }]
    }"#;

    #[test]
    fn round_trips_through_json() {
        let mut file = GrantsFile::default();
        file.grants.push(sample_grant("/p/a"));
        let s = serde_json::to_string(&file).unwrap();
        let back: GrantsFile = serde_json::from_str(&s).unwrap();
        assert_eq!(back, file);
        assert!(
            !s.contains("legacy"),
            "a file with nothing to migrate carries no legacy key: {s}"
        );
    }

    #[test]
    fn schema_version_defaults_when_omitted() {
        let json = r#"{"grants":[]}"#;
        let f: GrantsFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn a_v1_file_loads_into_legacy_rather_than_as_corrupt() {
        // The whole reason for the custom deserializer: the v1 record
        // is the only thing obliging anyone to take the settings key
        // back out. Losing it to a `.corrupt` rename would leave the
        // key behind forever.
        let f: GrantsFile = serde_json::from_str(V1_FILE).unwrap();
        assert!(f.grants.is_empty(), "v1 grants are not hook grants yet");
        assert_eq!(f.legacy.len(), 1);
        let l = &f.legacy[0];
        assert_eq!(l.project_path, "/p/a");
        assert_eq!(l.layer, SettingsLayer::LocalProject);
        assert_eq!(l.granted_mode, PermissionMode::BypassPermissions);
        assert_eq!(l.previous_mode, Some(PermissionMode::Default));
        assert_eq!(
            l.revert_attempts, 0,
            "v1 breaker counters are not carried over"
        );
        assert_eq!(f.schema_version, SCHEMA_VERSION, "migrated in memory");
        assert!(f.validate().is_ok());
    }

    #[test]
    fn a_versionless_file_with_v1_shaped_grants_is_read_as_v1() {
        let json = r#"{"grants":[{"project_path":"/p/a","layer":"local_project",
            "granted_mode":"bypassPermissions","previous_mode":null,
            "granted_at":"2023-11-14T22:13:20Z","expires_at":null}]}"#;
        let f: GrantsFile = serde_json::from_str(json).unwrap();
        assert!(f.grants.is_empty());
        assert_eq!(f.legacy.len(), 1);
        assert_eq!(f.legacy[0].previous_mode, None);
    }

    #[test]
    fn a_migrated_file_saves_as_v2_with_its_legacy_records_intact() {
        let f: GrantsFile = serde_json::from_str(V1_FILE).unwrap();
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"schema_version\":2"), "{s}");
        assert!(s.contains("\"legacy\""), "{s}");
        let back: GrantsFile = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn promote_legacy_carries_the_deadline_over_and_drops_the_record() {
        let mut f: GrantsFile = serde_json::from_str(V1_FILE).unwrap();
        let before_deadline = Utc.with_ymd_and_hms(2023, 11, 14, 23, 0, 0).unwrap();
        let g = f.promote_legacy("/p/a", before_deadline).unwrap();
        assert_eq!(g.project_path, "/p/a");
        assert_eq!(
            g.expires_at,
            Some(Utc.with_ymd_and_hms(2023, 11, 15, 0, 13, 20).unwrap())
        );
        assert!(f.legacy.is_empty());
        assert_eq!(f.grants, vec![g]);
    }

    #[test]
    fn promote_legacy_of_an_expired_record_grants_nothing() {
        let mut f: GrantsFile = serde_json::from_str(V1_FILE).unwrap();
        let after_deadline = Utc.with_ymd_and_hms(2023, 11, 16, 0, 0, 0).unwrap();
        assert!(f.promote_legacy("/p/a", after_deadline).is_none());
        assert!(f.legacy.is_empty(), "the record is still consumed");
        assert!(f.grants.is_empty());
    }

    #[test]
    fn promote_legacy_never_overwrites_a_newer_hook_grant() {
        let mut f: GrantsFile = serde_json::from_str(V1_FILE).unwrap();
        let regranted = Grant {
            project_path: "/p/a".into(),
            granted_at: ts(0),
            expires_at: None,
        };
        f.upsert(regranted.clone());
        let before_deadline = Utc.with_ymd_and_hms(2023, 11, 14, 23, 0, 0).unwrap();
        assert!(f.promote_legacy("/p/a", before_deadline).is_some());
        assert_eq!(f.grants, vec![regranted]);
    }

    #[test]
    fn promote_legacy_of_an_unknown_path_is_none() {
        let mut f = GrantsFile::default();
        assert!(f.promote_legacy("/p/none", ts(0)).is_none());
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
        assert!(g.expires_at.is_none());
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
    }

    #[test]
    fn validate_rejects_unknown_schema_version() {
        let f = GrantsFile {
            schema_version: 99,
            ..GrantsFile::default()
        };
        assert_eq!(
            f.validate(),
            Err(ValidationError::UnsupportedSchemaVersion {
                found: 99,
                expected: SCHEMA_VERSION
            })
        );
        // And the deserializer does not silently rewrite a newer
        // version down to ours — validate must still see 99.
        let parsed: GrantsFile = serde_json::from_str(r#"{"schema_version":99}"#).unwrap();
        assert_eq!(parsed.schema_version, 99);
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
    fn validate_rejects_a_legacy_record_on_the_project_layer() {
        let mut f: GrantsFile = serde_json::from_str(V1_FILE).unwrap();
        f.legacy[0].layer = SettingsLayer::Project;
        assert_eq!(
            f.validate(),
            Err(ValidationError::ProjectLayerNotAllowed("/p/a".into()))
        );
    }

    #[test]
    fn validate_file_rejects_duplicate_project() {
        let file = GrantsFile {
            grants: vec![sample_grant("/p/a"), sample_grant("/p/a")],
            ..GrantsFile::default()
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
        let updated = sticky_grant("/p/a");
        let prev = file.upsert(updated.clone()).unwrap();
        assert_eq!(prev.expires_at, Some(ts(7200)));
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
