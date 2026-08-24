//! Persistence for the open grant.
//!
//! **Fails loud on corruption**, for the same reason
//! `permission::store` does: this record is the *only* thing obliging
//! anything to close the window. Recovering it silently to empty would
//! end the revert obligation while leaving `crossSessionInbound:
//! accept` in CC's settings — the machine stays open and nothing
//! remembers to shut it.

use std::path::{Path, PathBuf};

use super::GrantFile;
use crate::json_store::{self, SaveError};

pub use crate::json_store::{CorruptionRecovery, Loaded};

pub const GRANT_FILENAME: &str = "peer-inbound-grant.json";

const STORE: &str = "peer_inbound_store";

pub fn grant_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(GRANT_FILENAME)
}

/// Structural defects that make a grant record untrustworthy.
///
/// A grant whose window runs backwards cannot be evaluated: `decide`
/// would report it expired the moment it was written, silently ending a
/// window the user believes is open. Better to treat the file as
/// corrupt and say so.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("grant record is schema version {found}, expected {expected}")]
    UnknownSchema { expected: u32, found: u32 },
    #[error("grant expires at or before it was granted ({granted_at} -> {expires_at})")]
    BackwardsWindow {
        granted_at: String,
        expires_at: String,
    },
}

impl GrantFile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != super::SCHEMA_VERSION {
            return Err(ValidationError::UnknownSchema {
                expected: super::SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if let Some(g) = &self.grant {
            if g.expires_at <= g.granted_at {
                return Err(ValidationError::BackwardsWindow {
                    granted_at: g.granted_at.to_rfc3339(),
                    expires_at: g.expires_at.to_rfc3339(),
                });
            }
        }
        Ok(())
    }
}

impl json_store::Validate for GrantFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), ValidationError> {
        // Inherent methods win resolution over trait methods, so this
        // is delegation rather than recursion.
        GrantFile::validate(self)
    }
}

/// Load, recovering a corrupt file to empty but logging at `error!` and
/// exposing the recovery so a caller can tell the user the window may
/// not close by itself.
pub fn load_at(path: &Path) -> std::io::Result<Loaded<GrantFile>> {
    let loaded = json_store::load_or_recover::<GrantFile>(path, STORE)?;
    if loaded.recovery.is_some() {
        tracing::error!(
            store = STORE,
            path = %path.display(),
            "peer inbound grant record was unreadable and has been moved aside; \
             an open remote-control window may no longer auto-close — check \
             crossSessionInbound in CC's user settings by hand"
        );
    }
    Ok(loaded)
}

pub fn load() -> std::io::Result<Loaded<GrantFile>> {
    load_at(&grant_path())
}

pub fn save_at(path: &Path, file: &GrantFile) -> Result<(), SaveError<ValidationError>> {
    json_store::save(path, file)
}

pub fn save(file: &GrantFile) -> Result<(), SaveError<ValidationError>> {
    save_at(&grant_path(), file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::inbound::{InboundGrant, InboundMode, SCHEMA_VERSION};
    use chrono::{TimeZone, Utc};

    fn sample() -> GrantFile {
        GrantFile {
            schema_version: SCHEMA_VERSION,
            grant: Some(InboundGrant {
                granted: InboundMode::Accept,
                previous: Some(InboundMode::Hold),
                granted_at: Utc.with_ymd_and_hms(2026, 8, 22, 10, 0, 0).unwrap(),
                expires_at: Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap(),
                reason: Some("phone".into()),
            }),
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(GRANT_FILENAME);
        save_at(&path, &sample()).unwrap();
        assert_eq!(load_at(&path).unwrap().value, sample());
    }

    #[test]
    fn a_missing_file_loads_as_no_grant() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_at(&dir.path().join(GRANT_FILENAME)).unwrap();
        assert!(loaded.value.grant.is_none());
        assert!(loaded.recovery.is_none());
    }

    #[test]
    fn a_corrupt_file_is_recovered_but_the_recovery_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(GRANT_FILENAME);
        std::fs::write(&path, "{ not json").unwrap();
        let loaded = load_at(&path).unwrap();
        assert!(loaded.value.grant.is_none());
        assert!(
            loaded.recovery.is_some(),
            "a silent recovery would end the revert obligation while the \
             machine stays open"
        );
    }

    #[test]
    fn clearing_the_grant_persists_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(GRANT_FILENAME);
        save_at(&path, &sample()).unwrap();
        save_at(&path, &GrantFile::default()).unwrap();
        assert!(load_at(&path).unwrap().value.grant.is_none());
    }
}
