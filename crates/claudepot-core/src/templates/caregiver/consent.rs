//! Consent records for the caregiver bundle.
//!
//! Each install creates one record at
//! `~/.claudepot/consent-records/<id>.json` (mode 0600). The
//! revoke flow sets `revoked_at` and emails the caregiver. The
//! record is append-only by design — revoked records stay
//! present so the user can audit history.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fs_utils;
use crate::paths;

/// One consent record. Persisted at
/// `~/.claudepot/consent-records/<id>.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsentRecord {
    pub id: String,
    pub automation_id: String,
    pub blueprint_id: String,
    pub blueprint_version: u32,
    /// Free-form label the user typed at install time.
    /// Renders into reports as the dependent's machine name.
    pub dependent_label: String,
    /// The dependent's typed full name. Captured to make the
    /// consent step concrete; never emailed.
    pub dependent_typed_name: String,
    pub caregiver_email: String,
    pub smtp_provider: SmtpProvider,
    /// Allowed report-section keys. Hard-coded by the schema in
    /// `report.rs`; persisted here for audit / future-version
    /// drift detection.
    pub report_scope_shown: Vec<String>,
    pub consented_at: String,
    #[serde(default)]
    pub revoked_at: Option<String>,
    #[serde(default)]
    pub revoke_reason: Option<RevokeReason>,
    /// Append-only history of any privacy-class downgrades.
    #[serde(default)]
    pub schedule_changes: Vec<ScheduleChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmtpProvider {
    GmailAppPassword,
    IcloudAppPassword,
    Generic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RevokeReason {
    UserRequest,
    PrivacyClassChanged,
    BlueprintRemoved,
    UninstallCascade,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleChange {
    pub at: String,
    pub note: String,
}

impl ConsentRecord {
    pub fn new_id() -> String {
        format!("cr-{}", Uuid::new_v4())
    }

    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// Storage handle. CRUD over individual JSON files in the
/// consent-records directory.
#[derive(Debug, Clone)]
pub struct ConsentStore {
    dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ConsentError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed consent record: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("consent record {0} not found")]
    NotFound(String),
}

impl ConsentStore {
    pub fn open() -> Result<Self, ConsentError> {
        let dir = paths::claudepot_data_dir().join("consent-records");
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn at(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn create(&self, record: ConsentRecord) -> Result<ConsentRecord, ConsentError> {
        let path = self.dir.join(format!("{}.json", record.id));
        let bytes = serde_json::to_vec_pretty(&record)?;
        fs_utils::atomic_write(&path, &bytes)?;
        // Tighten permissions to 0600 on Unix. Best-effort on
        // other platforms.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(record)
    }

    pub fn get(&self, id: &str) -> Result<ConsentRecord, ConsentError> {
        let path = self.dir.join(format!("{id}.json"));
        if !path.exists() {
            return Err(ConsentError::NotFound(id.to_string()));
        }
        let bytes = std::fs::read(&path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn list(&self) -> Result<Vec<ConsentRecord>, ConsentError> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            match serde_json::from_slice::<ConsentRecord>(&bytes) {
                Ok(r) => out.push(r),
                Err(_) => continue, // tolerate unrelated junk
            }
        }
        out.sort_by(|a, b| a.consented_at.cmp(&b.consented_at));
        Ok(out)
    }

    pub fn revoke(&self, id: &str, reason: RevokeReason) -> Result<ConsentRecord, ConsentError> {
        let mut record = self.get(id)?;
        if record.revoked_at.is_none() {
            record.revoked_at = Some(chrono::Utc::now().to_rfc3339());
            record.revoke_reason = Some(reason);
            let path = self.dir.join(format!("{id}.json"));
            let bytes = serde_json::to_vec_pretty(&record)?;
            fs_utils::atomic_write(&path, &bytes)?;
        }
        Ok(record)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> ConsentRecord {
        ConsentRecord {
            id: ConsentRecord::new_id(),
            automation_id: "auto-1".into(),
            blueprint_id: "caregiver.weekly-report".into(),
            blueprint_version: 1,
            dependent_label: "Dad's MacBook".into(),
            dependent_typed_name: "Robert Smith".into(),
            caregiver_email: "daughter@example.com".into(),
            smtp_provider: SmtpProvider::GmailAppPassword,
            report_scope_shown: vec!["disk".into(), "backups".into()],
            consented_at: chrono::Utc::now().to_rfc3339(),
            revoked_at: None,
            revoke_reason: None,
            schedule_changes: vec![],
        }
    }

    #[test]
    fn round_trip_create_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConsentStore::at(dir.path().to_path_buf());
        let r = record();
        let id = r.id.clone();
        store.create(r.clone()).unwrap();
        let back = store.get(&id).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn revoke_sets_timestamp_and_reason() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConsentStore::at(dir.path().to_path_buf());
        let r = record();
        let id = r.id.clone();
        store.create(r).unwrap();
        let revoked = store.revoke(&id, RevokeReason::UserRequest).unwrap();
        assert!(revoked.revoked_at.is_some());
        assert_eq!(revoked.revoke_reason, Some(RevokeReason::UserRequest));
        assert!(!revoked.is_active());
    }

    #[test]
    fn revoke_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConsentStore::at(dir.path().to_path_buf());
        let r = record();
        let id = r.id.clone();
        store.create(r).unwrap();
        let r1 = store.revoke(&id, RevokeReason::UserRequest).unwrap();
        let r2 = store.revoke(&id, RevokeReason::PrivacyClassChanged).unwrap();
        // Second revoke does not change the timestamp/reason.
        assert_eq!(r1.revoked_at, r2.revoked_at);
        assert_eq!(r1.revoke_reason, r2.revoke_reason);
    }

    #[test]
    fn list_returns_all_records_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConsentStore::at(dir.path().to_path_buf());
        let mut r1 = record();
        r1.consented_at = "2026-01-01T00:00:00Z".into();
        let mut r2 = record();
        r2.consented_at = "2026-02-01T00:00:00Z".into();
        store.create(r1.clone()).unwrap();
        store.create(r2.clone()).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].consented_at, r1.consented_at);
        assert_eq!(listed[1].consented_at, r2.consented_at);
    }

    #[test]
    fn missing_record_returns_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConsentStore::at(dir.path().to_path_buf());
        let err = store.get("never-existed").unwrap_err();
        assert!(matches!(err, ConsentError::NotFound(_)));
    }

    #[cfg(unix)]
    #[test]
    fn record_files_are_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = ConsentStore::at(dir.path().to_path_buf());
        let r = record();
        let id = r.id.clone();
        store.create(r).unwrap();
        let path = dir.path().join(format!("{id}.json"));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "expected mode 0600, got {mode:o}");
    }
}
