//! Reading and writing `crossSessionInbound` in CC's user settings.
//!
//! Every write goes through [`crate::settings_mutex`], the one
//! sanctioned read-modify-write boundary for CC settings files. The
//! previous value is read *inside* the closure rather than by a
//! separate call beforehand: deciding what to write from a snapshot
//! taken outside the lock is a race by construction, and here the
//! snapshot is exactly what revert depends on.
//!
//! **User layer only.** A project-scoped `accept` cannot loosen the
//! gate (CC: "your own `accept` cannot override a repo tightening"),
//! so a project-scoped writer would report success and change nothing.

use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

use super::InboundMode;
use crate::settings_mutex::{self, Change, SettingsMutexError};
use crate::settings_writer::SettingsLayer;

/// CC's key name. Technical identifier — never localized.
pub const INBOUND_KEY: &str = "crossSessionInbound";

#[derive(Debug, thiserror::Error)]
pub enum InboundSettingsError {
    #[error(transparent)]
    Mutex(#[from] SettingsMutexError),
    /// The key held a value CC does not recognise, so writing over it
    /// would destroy a value we could not put back on expiry.
    ///
    /// Raised from *inside* the mutation closure. See `write_mode`.
    #[error("`{INBOUND_KEY}` holds an unrecognized value ({raw}) that a revert could not restore")]
    UnrecognizedExisting { raw: String },
}

/// Three states, not two.
///
/// Collapsing "absent" into "present but unreadable" is the mistake
/// `settings_writer::read_i64_setting` exists to avoid: it reports a
/// confident value for a file CC is actually treating as broken. Here
/// it would also destroy revert fidelity — we cannot restore a value we
/// declined to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeValue {
    /// The key is not in the file. CC uses its own default.
    Absent,
    Valid(InboundMode),
    /// Present but not one of CC's three words.
    Unrecognized(String),
}

impl ModeValue {
    pub fn valid(&self) -> Option<InboundMode> {
        match self {
            Self::Valid(m) => Some(*m),
            _ => None,
        }
    }
}

/// `~/.claude/settings.json`. The `project_root` argument
/// `SettingsLayer::User` ignores is passed as `.` for clarity.
pub fn user_settings_path() -> PathBuf {
    SettingsLayer::User.settings_file(Path::new("."))
}

pub fn read_mode() -> Result<ModeValue, InboundSettingsError> {
    let object = settings_mutex::read_settings_file(&user_settings_path())?;
    Ok(classify(object.get(INBOUND_KEY)))
}

fn classify(value: Option<&JsonValue>) -> ModeValue {
    match value {
        None | Some(JsonValue::Null) => ModeValue::Absent,
        Some(JsonValue::String(s)) => match InboundMode::from_wire(s) {
            Some(m) => ModeValue::Valid(m),
            None => ModeValue::Unrecognized(s.clone()),
        },
        Some(other) => ModeValue::Unrecognized(other.to_string()),
    }
}

/// Set the key, returning what was there before.
///
/// The returned value is observed under the same lock that performed
/// the write, so it is a safe basis for a later revert.
///
/// **Refuses an unrecognized existing value from inside the closure.**
/// `ops::open` also checks for one before calling, but that check reads
/// the file under a *different* acquisition of the lock — so an edit
/// landing between the two was written over, and `previous` came back
/// as `Unrecognized`, which `.valid()` flattens to `None`. The grant
/// then recorded "there was nothing here before" and expiry *removed*
/// the user's value instead of restoring it. AGENTS.md's settings
/// boundary says it directly: deciding from a snapshot taken outside
/// the closure is a race by construction.
pub fn write_mode(mode: InboundMode) -> Result<ModeValue, InboundSettingsError> {
    let mutation = settings_mutex::mutate_settings_file(
        &user_settings_path(),
        |object, _was| -> Result<Change<ModeValue>, InboundSettingsError> {
            let before = classify(object.get(INBOUND_KEY));
            if let ModeValue::Unrecognized(raw) = &before {
                return Err(InboundSettingsError::UnrecognizedExisting { raw: raw.clone() });
            }
            if before == ModeValue::Valid(mode) {
                // Already what we want. Skipping keeps the file
                // byte-for-byte, so a no-op grant does not reformat a
                // hand-maintained settings file.
                return Ok(Change::Skip(before));
            }
            object.insert(
                INBOUND_KEY.to_string(),
                JsonValue::String(mode.as_wire().to_string()),
            );
            Ok(Change::Write(before))
        },
    )?;
    Ok(mutation.value)
}

/// Remove the key entirely, so CC falls back to its own default.
pub fn clear_mode() -> Result<(), InboundSettingsError> {
    settings_mutex::mutate_settings_file(
        &user_settings_path(),
        |object, _was| -> Result<Change<()>, InboundSettingsError> {
            if object.remove(INBOUND_KEY).is_none() {
                return Ok(Change::Skip(()));
            }
            Ok(Change::Write(()))
        },
    )?;
    Ok(())
}

/// Put the setting back exactly as it was before a grant.
///
/// One call, not "write or clear" at the call site: restoring is a
/// single logical transition and splitting it across two entry points
/// invites a caller to handle only the branch it happened to test.
pub fn restore(previous: Option<InboundMode>) -> Result<(), InboundSettingsError> {
    match previous {
        Some(mode) => {
            write_mode(mode)?;
            Ok(())
        }
        None => clear_mode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn isolated() -> (TempDir, std::sync::MutexGuard<'static, ()>) {
        let lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let config = tmp.path().join("config-dir");
        fs::create_dir_all(&config).unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", &config);
        (tmp, lock)
    }

    fn write_settings(body: &str) {
        fs::write(user_settings_path(), body).unwrap();
    }

    #[test]
    fn classify_covers_all_three_states() {
        assert_eq!(classify(None), ModeValue::Absent);
        assert_eq!(classify(Some(&JsonValue::Null)), ModeValue::Absent);
        assert_eq!(
            classify(Some(&JsonValue::String("accept".into()))),
            ModeValue::Valid(InboundMode::Accept)
        );
        assert_eq!(
            classify(Some(&JsonValue::String("yes".into()))),
            ModeValue::Unrecognized("yes".into())
        );
    }

    #[test]
    fn a_non_string_value_is_unrecognized_not_absent() {
        // CC rejects it; reporting "absent" would tell the user the gate
        // is at its default when the file is in fact broken.
        assert!(matches!(
            classify(Some(&JsonValue::Bool(true))),
            ModeValue::Unrecognized(_)
        ));
    }

    #[test]
    fn write_then_read_round_trips() {
        let (_tmp, _lock) = isolated();
        write_settings(r#"{"model":"opus"}"#);
        let before = write_mode(InboundMode::Accept).unwrap();
        assert_eq!(before, ModeValue::Absent);
        assert_eq!(read_mode().unwrap(), ModeValue::Valid(InboundMode::Accept));
    }

    #[test]
    fn write_preserves_neighbouring_keys() {
        let (_tmp, _lock) = isolated();
        write_settings(r#"{"model":"opus","cleanupPeriodDays":30}"#);
        write_mode(InboundMode::Accept).unwrap();
        let object = settings_mutex::read_settings_file(&user_settings_path()).unwrap();
        assert_eq!(object.get("model").unwrap(), "opus");
        assert_eq!(object.get("cleanupPeriodDays").unwrap(), 30);
    }

    #[test]
    fn write_reports_the_previous_value() {
        let (_tmp, _lock) = isolated();
        write_settings(r#"{"crossSessionInbound":"refuse"}"#);
        let before = write_mode(InboundMode::Accept).unwrap();
        assert_eq!(before, ModeValue::Valid(InboundMode::Refuse));
    }

    #[test]
    fn clear_removes_only_our_key() {
        let (_tmp, _lock) = isolated();
        write_settings(r#"{"model":"opus","crossSessionInbound":"accept"}"#);
        clear_mode().unwrap();
        let object = settings_mutex::read_settings_file(&user_settings_path()).unwrap();
        assert!(!object.contains_key(INBOUND_KEY));
        assert_eq!(object.get("model").unwrap(), "opus");
    }

    #[test]
    fn restore_to_absent_removes_the_key() {
        let (_tmp, _lock) = isolated();
        write_settings(r#"{"crossSessionInbound":"accept"}"#);
        restore(None).unwrap();
        assert_eq!(read_mode().unwrap(), ModeValue::Absent);
    }

    #[test]
    fn restore_to_a_prior_value_writes_it_back() {
        let (_tmp, _lock) = isolated();
        write_settings(r#"{"crossSessionInbound":"accept"}"#);
        restore(Some(InboundMode::Hold)).unwrap();
        assert_eq!(read_mode().unwrap(), ModeValue::Valid(InboundMode::Hold));
    }

    #[test]
    fn a_full_grant_and_revert_cycle_leaves_the_file_as_found() {
        let (_tmp, _lock) = isolated();
        write_settings("{\n  \"model\": \"opus\"\n}\n");
        let before = write_mode(InboundMode::Accept).unwrap();
        restore(before.valid()).unwrap();
        let object = settings_mutex::read_settings_file(&user_settings_path()).unwrap();
        assert!(
            !object.contains_key(INBOUND_KEY),
            "revert must not leave the key behind"
        );
        assert_eq!(object.get("model").unwrap(), "opus");
    }

    #[test]
    fn writing_the_value_already_present_is_a_no_op() {
        let (_tmp, _lock) = isolated();
        write_settings("{\"crossSessionInbound\":\"accept\",\"model\":\"opus\"}");
        let raw_before = fs::read_to_string(user_settings_path()).unwrap();
        write_mode(InboundMode::Accept).unwrap();
        assert_eq!(
            fs::read_to_string(user_settings_path()).unwrap(),
            raw_before,
            "a redundant write must not reformat the user's file"
        );
    }

    #[test]
    fn write_mode_refuses_an_unrecognized_value_from_inside_the_lock() {
        // The race this closes: `ops::open` preflights with its own
        // `read_mode`, which takes the settings lock separately. An
        // edit landing between the two used to be written over, and the
        // `previous` it returned was `Unrecognized`, which `.valid()`
        // flattens to `None` — so the grant recorded "nothing was here"
        // and expiry DELETED the user's value instead of restoring it.
        let (_tmp, _lock) = isolated();
        write_settings(r#"{"crossSessionInbound":"sometimes"}"#);

        let err = write_mode(InboundMode::Accept).unwrap_err();
        assert!(
            matches!(err, InboundSettingsError::UnrecognizedExisting { .. }),
            "got {err:?}"
        );
        // And the file is untouched, which is the whole point — the
        // value has to survive for a revert to be able to restore it.
        let after = fs::read_to_string(user_settings_path()).unwrap();
        assert!(after.contains("sometimes"), "{after}");
    }

    #[test]
    fn write_mode_still_writes_over_a_recognized_value() {
        let (_tmp, _lock) = isolated();
        write_settings(r#"{"crossSessionInbound":"hold"}"#);
        let before = write_mode(InboundMode::Accept).unwrap();
        assert_eq!(before, ModeValue::Valid(InboundMode::Hold));
        assert_eq!(read_mode().unwrap(), ModeValue::Valid(InboundMode::Accept));
    }
}
