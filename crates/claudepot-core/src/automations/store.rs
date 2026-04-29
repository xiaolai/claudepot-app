//! Automation definitions persisted as `~/.claudepot/automations.json`.
//!
//! JSON over SQLite for the same reasons routes use it: different
//! shape from accounts (no migrations, no live state, no
//! transactions across more than one row at a time) and we want
//! zero coupling with the existing `accounts.db` migration story.
//! Atomic writes via `fs_utils::atomic_write` (mode 0600 on unix).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs_utils;
use crate::paths::claudepot_data_dir;

use super::error::AutomationError;
use super::slug::validate_name;
use super::types::{Automation, AutomationId};

/// On-disk envelope. The `version` field is bumped only when the
/// shape changes incompatibly; serde's `default` handles
/// forward-compat field additions without touching the version.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AutomationsFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    automations: Vec<Automation>,
}

/// The schema version this build understands. Bump when the shape
/// changes in a way old binaries cannot read forward.
const CURRENT_VERSION: u32 = 1;

fn default_version() -> u32 {
    CURRENT_VERSION
}

impl Default for AutomationsFile {
    fn default() -> Self {
        Self {
            version: default_version(),
            automations: Vec::new(),
        }
    }
}

/// Patch struct for partial updates. `None` means "leave unchanged".
/// Adding fields to `Automation` requires adding them here too —
/// kept verbose deliberately so a missing field is a compile error,
/// not a silent drop.
///
/// String/numeric fields whose underlying type is `Option<T>` on
/// `Automation` collapse here to `Option<T>` (single-level): we
/// can set or leave alone, but cannot explicitly clear via patch.
/// Callers that want to clear send the form's empty value (empty
/// string / NaN / etc.); the patch builder converts that to the
/// underlying `None` before applying.
#[derive(Debug, Default, Clone)]
pub struct AutomationPatch {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub prompt: Option<String>,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub permission_mode: Option<super::types::PermissionMode>,
    pub allowed_tools: Option<Vec<String>>,
    pub add_dir: Option<Vec<String>>,
    pub max_budget_usd: Option<f64>,
    pub fallback_model: Option<String>,
    pub output_format: Option<super::types::OutputFormat>,
    pub json_schema: Option<String>,
    pub bare: Option<bool>,
    pub extra_env: Option<std::collections::BTreeMap<String, String>>,
    pub trigger: Option<super::types::Trigger>,
    pub platform_options: Option<super::types::PlatformOptions>,
    pub log_retention_runs: Option<u32>,
}

/// In-memory cache + read-modify-write helper around
/// `automations.json`. Construct once per command; not `Clone`.
/// Internally serializes every mutation through atomic writes,
/// so cross-process safety is best-effort (concurrent claudepot
/// CLI + GUI mutations may stomp each other).
pub struct AutomationStore {
    path: PathBuf,
    file: AutomationsFile,
}

impl AutomationStore {
    /// Open or create the store at `<claudepot_data_dir>/automations.json`.
    pub fn open() -> Result<Self, AutomationError> {
        Self::open_at(automations_file_path())
    }

    /// Open or create at an explicit path. Used by tests and any
    /// caller that wants to override the data dir.
    pub fn open_at(path: PathBuf) -> Result<Self, AutomationError> {
        let file = if path.exists() {
            let raw = std::fs::read(&path)?;
            if raw.is_empty() {
                AutomationsFile::default()
            } else {
                let parsed: AutomationsFile = serde_json::from_slice(&raw)?;
                // Refuse to load files newer than this binary
                // understands — saving them back could downgrade
                // their schema and lose data the future format adds.
                if parsed.version > CURRENT_VERSION {
                    return Err(AutomationError::InvalidEnv(format!(
                        "automations.json schema version {} is newer than this build (supports up to {}); upgrade Claudepot",
                        parsed.version, CURRENT_VERSION
                    )));
                }
                parsed
            }
        } else {
            AutomationsFile::default()
        };
        Ok(Self { path, file })
    }

    pub fn list(&self) -> &[Automation] {
        &self.file.automations
    }

    pub fn get(&self, id: &AutomationId) -> Option<&Automation> {
        self.file.automations.iter().find(|a| &a.id == id)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&Automation> {
        self.file.automations.iter().find(|a| a.name == name)
    }

    /// Insert a new automation. The caller is responsible for
    /// having validated every field on `Automation`; the store
    /// only enforces uniqueness of `name` and `id` plus the
    /// cross-field invariant that `bypassPermissions` carries a
    /// non-empty allow-list.
    pub fn add(&mut self, automation: Automation) -> Result<(), AutomationError> {
        // Defensive name re-validation — cheap, prevents malformed
        // names from sneaking in via deserialization.
        validate_name(&automation.name)?;
        if matches!(
            automation.permission_mode,
            super::types::PermissionMode::BypassPermissions
        ) && automation.allowed_tools.is_empty()
        {
            return Err(AutomationError::InvalidEnv(
                "bypassPermissions requires a non-empty allowed_tools whitelist".into(),
            ));
        }
        if self
            .file
            .automations
            .iter()
            .any(|a| a.name == automation.name)
        {
            return Err(AutomationError::DuplicateName(automation.name));
        }
        if self.file.automations.iter().any(|a| a.id == automation.id) {
            return Err(AutomationError::DuplicateName(format!(
                "id {}",
                automation.id
            )));
        }
        self.file.automations.push(automation);
        Ok(())
    }

    /// Apply a patch to an existing automation. Bumps `updated_at`.
    pub fn update(
        &mut self,
        id: &AutomationId,
        patch: AutomationPatch,
    ) -> Result<(), AutomationError> {
        let idx = self
            .file
            .automations
            .iter()
            .position(|a| &a.id == id)
            .ok_or_else(|| AutomationError::NotFound(id.to_string()))?;
        let a = &mut self.file.automations[idx];
        // Helper: empty string in a single-level patch means "clear to None".
        fn nz(v: String) -> Option<String> {
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        }
        if let Some(v) = patch.display_name {
            a.display_name = nz(v);
        }
        if let Some(v) = patch.description {
            a.description = nz(v);
        }
        if let Some(v) = patch.enabled {
            a.enabled = v;
        }
        if let Some(v) = patch.model {
            a.model = nz(v);
        }
        if let Some(v) = patch.cwd {
            a.cwd = v;
        }
        if let Some(v) = patch.prompt {
            a.prompt = v;
        }
        if let Some(v) = patch.system_prompt {
            a.system_prompt = nz(v);
        }
        if let Some(v) = patch.append_system_prompt {
            a.append_system_prompt = nz(v);
        }
        if let Some(v) = patch.permission_mode {
            a.permission_mode = v;
        }
        if let Some(v) = patch.allowed_tools {
            a.allowed_tools = v;
        }
        if let Some(v) = patch.add_dir {
            a.add_dir = v;
        }
        if let Some(v) = patch.max_budget_usd {
            // NaN means "clear to None"; finite negative also clears.
            a.max_budget_usd = if v.is_finite() && v >= 0.0 {
                Some(v)
            } else {
                None
            };
        }
        if let Some(v) = patch.fallback_model {
            a.fallback_model = nz(v);
        }
        if let Some(v) = patch.output_format {
            a.output_format = v;
        }
        if let Some(v) = patch.json_schema {
            a.json_schema = nz(v);
        }
        if let Some(v) = patch.bare {
            a.bare = v;
        }
        if let Some(v) = patch.extra_env {
            a.extra_env = v;
        }
        if let Some(v) = patch.trigger {
            a.trigger = v;
        }
        if let Some(v) = patch.platform_options {
            a.platform_options = v;
        }
        if let Some(v) = patch.log_retention_runs {
            a.log_retention_runs = v;
        }
        // Cross-field invariant: bypassPermissions requires a non-empty
        // allow-list. The Tauri layer also enforces this on add/update,
        // but the store is the last gate before persistence.
        if matches!(
            a.permission_mode,
            super::types::PermissionMode::BypassPermissions
        ) && a.allowed_tools.is_empty()
        {
            return Err(AutomationError::InvalidEnv(
                "bypassPermissions requires a non-empty allowed_tools whitelist".into(),
            ));
        }
        a.updated_at = chrono::Utc::now();
        Ok(())
    }

    /// Remove and return the automation with the given id.
    pub fn remove(&mut self, id: &AutomationId) -> Result<Automation, AutomationError> {
        let idx = self
            .file
            .automations
            .iter()
            .position(|a| &a.id == id)
            .ok_or_else(|| AutomationError::NotFound(id.to_string()))?;
        Ok(self.file.automations.remove(idx))
    }

    /// Persist in-memory state to disk. Atomic write, mode 0600.
    /// Creates parent directories on first save.
    pub fn save(&self) -> Result<(), AutomationError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&self.file)?;
        fs_utils::atomic_write(&self.path, &bytes)?;
        Ok(())
    }

    /// Path the store reads/writes. Useful for tests.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Canonical path: `<claudepot_data_dir>/automations.json`.
pub fn automations_file_path() -> PathBuf {
    claudepot_data_dir().join("automations.json")
}

/// Per-automation directory inside the data dir.
pub fn automation_dir(id: &AutomationId) -> PathBuf {
    claudepot_data_dir()
        .join("automations")
        .join(id.to_string())
}

/// Per-automation runs directory.
pub fn automation_runs_dir(id: &AutomationId) -> PathBuf {
    automation_dir(id).join("runs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations::types::*;
    use chrono::Utc;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn sample(name: &str) -> Automation {
        let now = Utc::now();
        Automation {
            id: Uuid::new_v4(),
            name: name.into(),
            display_name: None,
            description: None,
            enabled: true,
            binary: AutomationBinary::FirstParty,
            model: Some("sonnet".into()),
            cwd: "/tmp".into(),
            prompt: "say hi".into(),
            system_prompt: None,
            append_system_prompt: None,
            permission_mode: PermissionMode::DontAsk,
            allowed_tools: vec!["Read".into()],
            add_dir: vec![],
            max_budget_usd: Some(0.5),
            fallback_model: None,
            output_format: OutputFormat::Json,
            json_schema: None,
            bare: false,
            extra_env: Default::default(),
            trigger: Trigger::Cron {
                cron: "0 9 * * *".into(),
                timezone: None,
            },
            platform_options: PlatformOptions::default(),
            log_retention_runs: 50,
            created_at: now,
            updated_at: now,
            claudepot_managed: true,
        }
    }

    #[test]
    fn open_missing_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("automations.json");
        let store = AutomationStore::open_at(path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn add_save_reopen_preserves_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("automations.json");
        let mut store = AutomationStore::open_at(path.clone()).unwrap();
        store.add(sample("morning-pr")).unwrap();
        store.add(sample("evening-summary")).unwrap();
        store.save().unwrap();

        let reopened = AutomationStore::open_at(path).unwrap();
        let names: Vec<&str> = reopened.list().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["morning-pr", "evening-summary"]);
    }

    #[test]
    fn duplicate_name_rejected() {
        let dir = tempdir().unwrap();
        let mut store = AutomationStore::open_at(dir.path().join("a.json")).unwrap();
        store.add(sample("morning-pr")).unwrap();
        let err = store.add(sample("morning-pr")).unwrap_err();
        assert!(matches!(err, AutomationError::DuplicateName(_)));
    }

    #[test]
    fn update_applies_patch_and_bumps_timestamp() {
        let dir = tempdir().unwrap();
        let mut store = AutomationStore::open_at(dir.path().join("a.json")).unwrap();
        let mut a = sample("morning-pr");
        let original_updated = a.updated_at;
        // Backdate to make timestamp bump observable on fast machines.
        a.updated_at = original_updated - chrono::Duration::seconds(60);
        let id = a.id;
        store.add(a).unwrap();

        let patch = AutomationPatch {
            enabled: Some(false),
            prompt: Some("new prompt".into()),
            ..AutomationPatch::default()
        };
        store.update(&id, patch).unwrap();

        let updated = store.get(&id).unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.prompt, "new prompt");
        assert!(updated.updated_at > original_updated - chrono::Duration::seconds(1));
    }

    #[test]
    fn remove_deletes_record() {
        let dir = tempdir().unwrap();
        let mut store = AutomationStore::open_at(dir.path().join("a.json")).unwrap();
        let a = sample("morning-pr");
        let id = a.id;
        store.add(a).unwrap();
        let _ = store.remove(&id).unwrap();
        assert!(store.get(&id).is_none());
        assert!(store.list().is_empty());
    }

    #[test]
    fn forward_compat_extra_field_preserved() {
        // Hand-author a JSON file with an unknown field; verify it
        // round-trips harmlessly. (Behavior: serde drops unknown
        // fields by default, but the version envelope itself
        // tolerates them.)
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.json");
        std::fs::write(
            &path,
            r#"{"version":1,"automations":[],"future_field":"ignored"}"#,
        )
        .unwrap();
        let store = AutomationStore::open_at(path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("a.json");
        let store = AutomationStore::open_at(path.clone()).unwrap();
        store.save().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn add_with_invalid_name_rejected() {
        let dir = tempdir().unwrap();
        let mut store = AutomationStore::open_at(dir.path().join("a.json")).unwrap();
        let mut bad = sample("x");
        bad.name = "INVALID".into();
        assert!(matches!(
            store.add(bad),
            Err(AutomationError::InvalidName(..))
        ));
    }
}
