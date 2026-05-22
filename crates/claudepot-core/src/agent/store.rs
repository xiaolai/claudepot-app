//! Agent definitions persisted as `~/.claudepot/automations.json`.
//! (On-disk file name kept; renamed by the Phase 1 store migration.)
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

use super::error::AgentError;
use super::slug::validate_name;
use super::types::{Agent, AgentId};

/// On-disk envelope. The `version` field is bumped only when the
/// shape changes incompatibly; serde's `default` handles
/// forward-compat field additions without touching the version.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentsFile {
    #[serde(default = "default_version")]
    version: u32,
    // on-disk JSON key kept as "automations"; renamed by the Phase 1 store migration
    #[serde(default, rename = "automations")]
    agents: Vec<Agent>,
}

/// The schema version this build understands. Bump when the shape
/// changes in a way old binaries cannot read forward.
const CURRENT_VERSION: u32 = 1;

fn default_version() -> u32 {
    CURRENT_VERSION
}

impl Default for AgentsFile {
    fn default() -> Self {
        Self {
            version: default_version(),
            agents: Vec::new(),
        }
    }
}

/// Patch struct for partial updates. `None` means "leave unchanged".
/// Adding fields to `Agent` requires adding them here too —
/// kept verbose deliberately so a missing field is a compile error,
/// not a silent drop.
///
/// String/numeric fields whose underlying type is `Option<T>` on
/// `Agent` collapse here to `Option<T>` (single-level): we
/// can set or leave alone, but cannot explicitly clear via patch.
/// Callers that want to clear send the form's empty value (empty
/// string / NaN / etc.); the patch builder converts that to the
/// underlying `None` before applying.
#[derive(Debug, Default, Clone)]
pub struct AgentPatch {
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
pub struct AgentStore {
    path: PathBuf,
    file: AgentsFile,
}

impl AgentStore {
    /// Open or create the store at `<claudepot_data_dir>/automations.json`.
    pub fn open() -> Result<Self, AgentError> {
        Self::open_at(agents_file_path())
    }

    /// Open or create at an explicit path. Used by tests and any
    /// caller that wants to override the data dir.
    pub fn open_at(path: PathBuf) -> Result<Self, AgentError> {
        let file = if path.exists() {
            let raw = std::fs::read(&path)?;
            if raw.is_empty() {
                AgentsFile::default()
            } else {
                let parsed: AgentsFile = serde_json::from_slice(&raw)?;
                // Refuse to load files newer than this binary
                // understands — saving them back could downgrade
                // their schema and lose data the future format adds.
                if parsed.version > CURRENT_VERSION {
                    return Err(AgentError::InvalidEnv(format!(
                        "automations.json schema version {} is newer than this build (supports up to {}); upgrade Claudepot",
                        parsed.version, CURRENT_VERSION
                    )));
                }
                parsed
            }
        } else {
            AgentsFile::default()
        };
        Ok(Self { path, file })
    }

    pub fn list(&self) -> &[Agent] {
        &self.file.agents
    }

    pub fn get(&self, id: &AgentId) -> Option<&Agent> {
        self.file.agents.iter().find(|a| &a.id == id)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&Agent> {
        self.file.agents.iter().find(|a| a.name == name)
    }

    /// Insert a new agent. The caller is responsible for
    /// having validated every field on `Agent`; the store
    /// only enforces uniqueness of `name` and `id` plus the
    /// cross-field invariant that `bypassPermissions` carries a
    /// non-empty allow-list.
    pub fn add(&mut self, agent: Agent) -> Result<(), AgentError> {
        // Defensive name re-validation — cheap, prevents malformed
        // names from sneaking in via deserialization.
        validate_name(&agent.name)?;
        if matches!(
            agent.permission_mode,
            super::types::PermissionMode::BypassPermissions
        ) && agent.allowed_tools.is_empty()
        {
            return Err(AgentError::InvalidEnv(
                "bypassPermissions requires a non-empty allowed_tools whitelist".into(),
            ));
        }
        if self
            .file
            .agents
            .iter()
            .any(|a| a.name == agent.name)
        {
            return Err(AgentError::DuplicateName(agent.name));
        }
        if self.file.agents.iter().any(|a| a.id == agent.id) {
            return Err(AgentError::DuplicateName(format!(
                "id {}",
                agent.id
            )));
        }
        self.file.agents.push(agent);
        Ok(())
    }

    /// Apply a patch to an existing agent. Bumps `updated_at`.
    pub fn update(
        &mut self,
        id: &AgentId,
        patch: AgentPatch,
    ) -> Result<(), AgentError> {
        let idx = self
            .file
            .agents
            .iter()
            .position(|a| &a.id == id)
            .ok_or_else(|| AgentError::NotFound(id.to_string()))?;
        // Apply the patch to a clone first; only swap into the live
        // store after the cross-field invariant check passes. The
        // previous code mutated in place and only validated at the
        // end, so a rejected patch (e.g. bypassPermissions with an
        // empty allow-list) would leave the live record in a state
        // that disagreed with what `save()` would persist.
        let mut a = self.file.agents[idx].clone();
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
            return Err(AgentError::InvalidEnv(
                "bypassPermissions requires a non-empty allowed_tools whitelist".into(),
            ));
        }
        a.updated_at = chrono::Utc::now();
        // Commit the validated clone back into the live store. Any
        // earlier `Err` return path leaves `self.file.agents[idx]`
        // untouched, preserving the previous valid record.
        self.file.agents[idx] = a;
        Ok(())
    }

    /// Remove and return the agent with the given id.
    pub fn remove(&mut self, id: &AgentId) -> Result<Agent, AgentError> {
        let idx = self
            .file
            .agents
            .iter()
            .position(|a| &a.id == id)
            .ok_or_else(|| AgentError::NotFound(id.to_string()))?;
        Ok(self.file.agents.remove(idx))
    }

    /// Persist in-memory state to disk. Atomic write, mode 0600.
    /// Creates parent directories on first save.
    pub fn save(&self) -> Result<(), AgentError> {
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
pub fn agents_file_path() -> PathBuf {
    // on-disk name kept; renamed by the Phase 1 store migration
    claudepot_data_dir().join("automations.json")
}

/// Per-agent directory inside the data dir.
pub fn agent_dir(id: &AgentId) -> PathBuf {
    claudepot_data_dir()
        // on-disk name kept; renamed by the Phase 1 store migration
        .join("automations")
        .join(id.to_string())
}

/// Per-agent runs directory.
pub fn agent_runs_dir(id: &AgentId) -> PathBuf {
    agent_dir(id).join("runs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::*;
    use chrono::Utc;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn sample(name: &str) -> Agent {
        let now = Utc::now();
        Agent {
            id: Uuid::new_v4(),
            name: name.into(),
            display_name: None,
            description: None,
            enabled: true,
            binary: AgentBinary::FirstParty,
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
            template_id: None,
        }
    }

    #[test]
    fn open_missing_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let store = AgentStore::open_at(path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn add_save_reopen_preserves_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        store.add(sample("morning-pr")).unwrap();
        store.add(sample("evening-summary")).unwrap();
        store.save().unwrap();

        let reopened = AgentStore::open_at(path).unwrap();
        let names: Vec<&str> = reopened.list().iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["morning-pr", "evening-summary"]);
    }

    #[test]
    fn duplicate_name_rejected() {
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        store.add(sample("morning-pr")).unwrap();
        let err = store.add(sample("morning-pr")).unwrap_err();
        assert!(matches!(err, AgentError::DuplicateName(_)));
    }

    #[test]
    fn update_applies_patch_and_bumps_timestamp() {
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut a = sample("morning-pr");
        let original_updated = a.updated_at;
        // Backdate to make timestamp bump observable on fast machines.
        a.updated_at = original_updated - chrono::Duration::seconds(60);
        let id = a.id;
        store.add(a).unwrap();

        let patch = AgentPatch {
            enabled: Some(false),
            prompt: Some("new prompt".into()),
            ..AgentPatch::default()
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
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
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
        // On-disk JSON key is "automations" (kept by the Phase 1
        // store migration); exercise that wire form here.
        std::fs::write(
            &path,
            r#"{"version":1,"automations":[],"future_field":"ignored"}"#,
        )
        .unwrap();
        let store = AgentStore::open_at(path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("a.json");
        let store = AgentStore::open_at(path.clone()).unwrap();
        store.save().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn add_with_invalid_name_rejected() {
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut bad = sample("x");
        bad.name = "INVALID".into();
        assert!(matches!(
            store.add(bad),
            Err(AgentError::InvalidName(..))
        ));
    }
}
