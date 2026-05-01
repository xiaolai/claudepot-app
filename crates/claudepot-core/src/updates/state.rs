//! Persisted update settings + cached probe state. Lives at
//! `<claudepot_data_dir>/updates.json`. Single mutex-guarded record
//! mirroring the `preferences.rs` pattern in src-tauri.

use crate::paths;
use crate::updates::errors::{Result, UpdateError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CliSettings {
    /// Show a tray badge when an update is detected.
    pub notify_on_available: bool,
    /// Fire an OS notification (toast / banner) when an update is
    /// detected. Independent from the badge — users who keep the
    /// app in tray-only mode often want one but not the other.
    /// Default off; the badge alone is the less-intrusive default.
    /// Deduped per version: see [`CliCache::last_notified_version`].
    pub notify_os_on_available: bool,
    /// When the background poll detects a new version, also force-run
    /// `claude update` immediately. Off by default — CC's own
    /// background autoupdater handles this for native installs.
    pub force_update_on_check: bool,
}

impl Default for CliSettings {
    fn default() -> Self {
        Self {
            notify_on_available: true,
            notify_os_on_available: false,
            force_update_on_check: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DesktopSettings {
    pub notify_on_available: bool,
    /// Fire an OS notification (toast / banner) when an update is
    /// detected. See [`CliSettings::notify_os_on_available`] for the
    /// rationale — same shape, opt-in. Deduped per version.
    pub notify_os_on_available: bool,
    /// Run the install in the background when Desktop is not running.
    /// Default-on per the user's spec — the asymmetry "Squirrel only
    /// updates while Desktop is open, user keeps Desktop quit" is the
    /// real pain point we're solving.
    pub auto_install_when_quit: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            notify_on_available: true,
            notify_os_on_available: false,
            auto_install_when_quit: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct UpdateSettings {
    pub cli: CliSettings,
    pub desktop: DesktopSettings,
    /// Background poller cadence. Min 30, default 240 (4h), max 1440
    /// (24h). Out-of-range values are clamped via
    /// [`UpdateState::poll_interval_minutes`].
    pub poll_interval_minutes: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CliCache {
    pub last_check_unix: Option<i64>,
    pub last_known_latest: Option<String>,
    pub last_known_stable: Option<String>,
    pub last_error: Option<String>,
    /// Latest version we've already fired an OS notification for.
    /// Used by the watcher to dedupe — the toast fires once per new
    /// version, not on every poll cycle. None at first install.
    pub last_notified_version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DesktopCache {
    pub last_check_unix: Option<i64>,
    pub last_known_latest: Option<String>,
    pub last_known_sha: Option<String>,
    pub last_error: Option<String>,
    pub last_notified_version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UpdateCache {
    pub cli: CliCache,
    pub desktop: DesktopCache,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UpdateState {
    pub settings: UpdateSettings,
    pub cache: UpdateCache,
}

impl UpdateState {
    pub fn path() -> PathBuf {
        paths::claudepot_data_dir().join("updates.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(self)?;
        let parent = p.parent().unwrap_or(std::path::Path::new("."));
        let tmp = tempfile::NamedTempFile::new_in(parent)?;
        std::fs::write(tmp.path(), body)?;
        tmp.persist(&p).map_err(|e| {
            UpdateError::Io(std::io::Error::other(format!(
                "updates state: persist {}: {}",
                p.display(),
                e
            )))
        })?;
        Ok(())
    }

    pub fn poll_interval_minutes(&self) -> u32 {
        self.settings
            .poll_interval_minutes
            .unwrap_or(240)
            .clamp(30, 1440)
    }
}

/// Tauri-managed shared state mirror of `PreferencesState`.
///
/// Two locks:
/// - `.0` (`std::sync::Mutex<UpdateState>`) — fast, in-memory mutations.
///   Held briefly across reads/writes; never across an `await`.
/// - `.1` (`tokio::sync::Mutex<()>`) — save-ordering lock. Acquired
///   by every disk-writing path (the watcher's consolidated cycle
///   save AND `updates_settings_set`'s settings-toggle save) so two
///   `spawn_blocking` writes can't race and overwrite each other's
///   in-memory mutations on out-of-order completion.
///
/// Use `.save_lock()` instead of `.1` directly when accessing the
/// save ordering lock, for readability.
pub struct UpdateStateMutex(pub Mutex<UpdateState>, pub tokio::sync::Mutex<()>);

impl UpdateStateMutex {
    pub fn new(state: UpdateState) -> Self {
        Self(Mutex::new(state), tokio::sync::Mutex::new(()))
    }

    pub fn save_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::lock_data_dir;
    use tempfile::tempdir;

    fn with_temp_data<F: FnOnce()>(f: F) {
        let _lock = lock_data_dir();
        let tmp = tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path());
        f();
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn load_returns_defaults_when_missing() {
        with_temp_data(|| {
            let s = UpdateState::load();
            assert!(s.settings.cli.notify_on_available);
            assert!(!s.settings.cli.force_update_on_check);
            assert!(s.settings.desktop.auto_install_when_quit);
            assert!(s.cache.cli.last_check_unix.is_none());
        });
    }

    #[test]
    fn save_then_load_roundtrips() {
        with_temp_data(|| {
            let mut s = UpdateState::default();
            s.cache.cli.last_known_latest = Some("2.1.126".into());
            s.settings.desktop.auto_install_when_quit = false;
            s.save().unwrap();
            let loaded = UpdateState::load();
            assert_eq!(
                loaded.cache.cli.last_known_latest.as_deref(),
                Some("2.1.126")
            );
            assert!(!loaded.settings.desktop.auto_install_when_quit);
        });
    }

    #[test]
    fn poll_interval_clamps() {
        with_temp_data(|| {
            let mut s = UpdateState::default();
            s.settings.poll_interval_minutes = Some(5);
            assert_eq!(s.poll_interval_minutes(), 30);
            s.settings.poll_interval_minutes = Some(99999);
            assert_eq!(s.poll_interval_minutes(), 1440);
            s.settings.poll_interval_minutes = None;
            assert_eq!(s.poll_interval_minutes(), 240);
        });
    }

    #[test]
    fn missing_fields_get_defaults_via_serde_default() {
        with_temp_data(|| {
            let path = UpdateState::path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, r#"{"settings":{"cli":{}}}"#).unwrap();
            let s = UpdateState::load();
            assert!(s.settings.cli.notify_on_available); // default true
            assert!(!s.settings.cli.force_update_on_check); // default false
            assert!(s.settings.desktop.auto_install_when_quit); // default true
        });
    }

    #[test]
    fn corrupt_json_falls_back_to_default() {
        with_temp_data(|| {
            let path = UpdateState::path();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "not json at all").unwrap();
            let s = UpdateState::load();
            assert_eq!(s, UpdateState::default());
        });
    }
}
