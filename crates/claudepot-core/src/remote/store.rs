//! Persistence for paired devices.
//!
//! **Fails loud on corruption**, and for a sharper reason than the
//! other stores that do: this file is the *revocation list*. Recovering
//! it silently to empty would not merely lose settings — it would erase
//! `revoked_at` for every device that had been turned off, and the only
//! reason a revoked token stays refused is that its record is still
//! here. A silent reset is how a revoked phone quietly becomes
//! unknown-again rather than denied.
//!
//! It also loses the device list itself, which fails *closed* (nothing
//! authenticates until re-paired). That half is safe; the revocation
//! half is not, and the recovery marker exists so a caller can say so.

use std::path::{Path, PathBuf};

use super::{DevicesFile, SCHEMA_VERSION};
use crate::json_store::{self, SaveError};

pub use crate::json_store::{CorruptionRecovery, Loaded};

pub const DEVICES_FILENAME: &str = "remote-devices.json";

const STORE: &str = "remote_devices_store";

pub fn devices_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(DEVICES_FILENAME)
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("device record is schema version {found}, expected {expected}")]
    UnknownSchema { expected: u32, found: u32 },
    #[error("two devices share a token hash — one would shadow the other")]
    DuplicateTokenHash,
    #[error("a device has an empty token hash, which would match nothing or everything")]
    EmptyTokenHash,
}

impl DevicesFile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnknownSchema {
                expected: SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        let mut seen = std::collections::HashSet::new();
        for d in &self.devices {
            if d.token_hash.is_empty() {
                return Err(ValidationError::EmptyTokenHash);
            }
            if !seen.insert(d.token_hash.as_str()) {
                return Err(ValidationError::DuplicateTokenHash);
            }
        }
        Ok(())
    }
}

impl json_store::Validate for DevicesFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), ValidationError> {
        DevicesFile::validate(self)
    }
}

pub fn load_at(path: &Path) -> std::io::Result<Loaded<DevicesFile>> {
    let loaded = json_store::load_or_recover::<DevicesFile>(path, STORE)?;
    if loaded.recovery.is_some() {
        tracing::error!(
            store = STORE,
            path = %path.display(),
            "paired-device records were unreadable and have been moved aside; \
             every device must re-pair, and any REVOCATION recorded there is \
             gone — re-check which devices should have access"
        );
    }
    Ok(loaded)
}

pub fn load() -> std::io::Result<Loaded<DevicesFile>> {
    load_at(&devices_path())
}

pub fn save_at(path: &Path, file: &DevicesFile) -> Result<(), SaveError<ValidationError>> {
    json_store::save(path, file)
}

pub fn save(file: &DevicesFile) -> Result<(), SaveError<ValidationError>> {
    save_at(&devices_path(), file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::{new_device_token, Device};
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn device(hash: &str) -> Device {
        Device {
            id: Uuid::new_v4(),
            name: "phone".into(),
            token_hash: hash.into(),
            created_at: Utc.with_ymd_and_hms(2026, 8, 22, 10, 0, 0).unwrap(),
            last_seen: None,
            revoked_at: None,
            expires_at: None,
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICES_FILENAME);
        let tok = new_device_token();
        let file = DevicesFile {
            devices: vec![device(&tok.hash)],
            ..Default::default()
        };
        save_at(&path, &file).unwrap();
        assert_eq!(load_at(&path).unwrap().value, file);
    }

    #[test]
    fn the_file_on_disk_never_contains_a_plaintext_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICES_FILENAME);
        let tok = new_device_token();
        save_at(
            &path,
            &DevicesFile {
                devices: vec![device(&tok.hash)],
                ..Default::default()
            },
        )
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains(&tok.plaintext),
            "bearer token written to disk"
        );
    }

    #[test]
    fn a_missing_file_loads_empty_and_is_not_a_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_at(&dir.path().join(DEVICES_FILENAME)).unwrap();
        assert!(loaded.value.devices.is_empty());
        assert!(loaded.recovery.is_none());
    }

    #[test]
    fn a_corrupt_file_reports_its_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICES_FILENAME);
        std::fs::write(&path, "{ not json").unwrap();
        let loaded = load_at(&path).unwrap();
        assert!(
            loaded.recovery.is_some(),
            "a silent reset would erase the revocation list"
        );
    }

    #[test]
    fn duplicate_token_hashes_are_refused() {
        let tok = new_device_token();
        let file = DevicesFile {
            devices: vec![device(&tok.hash), device(&tok.hash)],
            ..Default::default()
        };
        assert!(matches!(
            file.validate(),
            Err(ValidationError::DuplicateTokenHash)
        ));
    }

    #[test]
    fn an_empty_token_hash_is_refused() {
        let file = DevicesFile {
            devices: vec![device("")],
            ..Default::default()
        };
        assert!(matches!(
            file.validate(),
            Err(ValidationError::EmptyTokenHash)
        ));
    }
}
