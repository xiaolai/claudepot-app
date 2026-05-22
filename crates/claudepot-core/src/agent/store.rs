//! Agent definitions persisted as `~/.claudepot/agents.json`.
//!
//! JSON over SQLite for the same reasons routes use it: different
//! shape from accounts (no migrations, no live state, no
//! transactions across more than one row at a time) and we want
//! zero coupling with the existing `accounts.db` migration story.
//! Atomic writes via `fs_utils::atomic_write` (mode 0600 on unix).
//!
//! ## v1 -> v2 migration
//!
//! Phase 0 renamed the code identifiers `automation*` -> `agent*`
//! but kept the on-disk strings (`automations.json`, the
//! `~/.claudepot/automations/` runs dir, the `"automations"` JSON
//! key) behind a `#[serde(rename)]` shim. Phase 1 retires those
//! strings.
//!
//! The v2 format is `~/.claudepot/agents.json` with
//! `{"version":2,"agents":[...]}` — a native field name, no rename
//! shim. On store open, [`AgentStore::open_at`] runs an idempotent
//! migration:
//!
//! 1. If `agents.json` exists -> load it as v2.
//! 2. Else if `automations.json` exists -> parse it as v1
//!    (`{"version":1,"automations":[...]}`), upgrade each record
//!    (new fields default; `lifecycle` forced to `Installed`
//!    because pre-existing automations are already armed), write
//!    `agents.json`, rename the runs dir, and back up the old file
//!    as `automations.json.pre-v2-backup`.
//! 3. Else -> a fresh empty v2 store.
//!
//! The migration never deletes data: the old JSON is preserved as
//! a backup, and a failed runs-dir rename (cross-device, etc.) is
//! logged and tolerated.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs_utils;
use crate::paths::claudepot_data_dir;

use super::error::AgentError;
use super::slug::validate_name;
use super::types::{Agent, AgentId, Lifecycle};

/// On-disk envelope (v2). The `version` field is bumped only when
/// the shape changes incompatibly; serde's `default` handles
/// forward-compat field additions without touching the version.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentsFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    agents: Vec<Agent>,
}

/// The v1 on-disk envelope — `{"version":1,"automations":[...]}`.
/// Read-only: used solely by the migration path to parse a legacy
/// `automations.json`. New writes always go through [`AgentsFile`].
#[derive(Debug, Clone, Deserialize)]
struct AgentsFileV1 {
    #[serde(default = "default_v1_version")]
    version: u32,
    #[serde(default, rename = "automations")]
    agents: Vec<Agent>,
}

/// The schema version this build understands. Bump when the shape
/// changes in a way old binaries cannot read forward.
const CURRENT_VERSION: u32 = 2;

/// The legacy schema version retired by the Phase 1 migration.
const LEGACY_VERSION: u32 = 1;

fn default_version() -> u32 {
    CURRENT_VERSION
}

fn default_v1_version() -> u32 {
    LEGACY_VERSION
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
    // ---- Agent-spec fields (Phase 1) ----
    pub disallowed_tools: Option<Vec<String>>,
    pub mcp_servers: Option<Vec<super::types::McpServerRef>>,
    pub run_as: Option<String>,
    /// `Some(0)` means "clear to None"; any positive value sets the
    /// per-run token ceiling. The patch builder maps the form's
    /// empty-input to `0` so a cleared budget round-trips.
    pub task_budget: Option<u64>,
    /// `Some(RateLimit::default())` (all-None inner fields) clears
    /// the rate limit to `None`; any populated value sets it.
    pub rate_limit: Option<super::types::RateLimit>,
}

/// In-memory cache + read-modify-write helper around
/// `agents.json`. Construct once per command; not `Clone`.
/// Internally serializes every mutation through atomic writes,
/// so cross-process safety is best-effort (concurrent claudepot
/// CLI + GUI mutations may stomp each other).
pub struct AgentStore {
    path: PathBuf,
    file: AgentsFile,
}

impl AgentStore {
    /// Open or create the store at `<claudepot_data_dir>/agents.json`,
    /// running the v1 -> v2 migration if a legacy `automations.json`
    /// is found and no `agents.json` exists yet.
    pub fn open() -> Result<Self, AgentError> {
        Self::open_at(agents_file_path())
    }

    /// Open or create at an explicit path. Used by tests and any
    /// caller that wants to override the data dir.
    ///
    /// When `path` does not exist, this looks for a legacy
    /// `automations.json` sibling and migrates it in place — see
    /// the module docs for the exact, idempotent order.
    pub fn open_at(path: PathBuf) -> Result<Self, AgentError> {
        // (1) A v2 file already exists -> load it directly.
        if path.exists() {
            return Ok(Self {
                file: Self::load_v2(&path)?,
                path,
            });
        }

        // (2) No v2 file. Look for a legacy v1 `automations.json`
        // sibling and migrate it if present.
        let legacy_path = legacy_agents_file_path_for(&path);
        if legacy_path.exists() {
            let file = Self::migrate_v1_to_v2(&legacy_path, &path)?;
            return Ok(Self { file, path });
        }

        // (3) Neither file exists -> fresh empty v2 store.
        Ok(Self {
            path,
            file: AgentsFile::default(),
        })
    }

    /// Read and validate a v2 `agents.json` from disk.
    fn load_v2(path: &Path) -> Result<AgentsFile, AgentError> {
        let raw = std::fs::read(path)?;
        if raw.is_empty() {
            return Ok(AgentsFile::default());
        }
        let parsed: AgentsFile = serde_json::from_slice(&raw)?;
        // Refuse to load files newer than this binary understands —
        // saving them back could downgrade their schema and lose
        // data the future format adds.
        if parsed.version > CURRENT_VERSION {
            return Err(AgentError::InvalidEnv(format!(
                "agents.json schema version {} is newer than this build (supports up to {}); upgrade Claudepot",
                parsed.version, CURRENT_VERSION
            )));
        }
        Ok(parsed)
    }

    /// Migrate a legacy v1 `automations.json` into a v2
    /// `agents.json`. Idempotent and data-preserving:
    ///
    /// - upgrades each record (new fields default; `lifecycle` is
    ///   forced to `Installed` — existing automations are armed and
    ///   must not become inert drafts);
    /// - writes the v2 `agents.json` atomically;
    /// - renames `~/.claudepot/automations/` -> `agents/` if it
    ///   exists (a failed rename is logged, not fatal);
    /// - renames the old `automations.json` to
    ///   `automations.json.pre-v2-backup` as a safety net.
    fn migrate_v1_to_v2(
        legacy_path: &Path,
        v2_path: &Path,
    ) -> Result<AgentsFile, AgentError> {
        let raw = std::fs::read(legacy_path)?;
        let mut file = if raw.is_empty() {
            AgentsFile::default()
        } else {
            let v1: AgentsFileV1 = serde_json::from_slice(&raw)?;
            if v1.version > LEGACY_VERSION {
                return Err(AgentError::InvalidEnv(format!(
                    "automations.json schema version {} is newer than the legacy format this build can migrate (v{}); upgrade Claudepot",
                    v1.version, LEGACY_VERSION
                )));
            }
            // serde already filled the new fields with their
            // `#[serde(default)]` values during deserialization.
            // The one record-level override the migration owes is
            // `lifecycle`: a pre-existing automation is already
            // armed, so it migrates to `Installed`, never the
            // `Draft` default.
            let mut agents = v1.agents;
            for a in &mut agents {
                a.lifecycle = Lifecycle::Installed;
            }
            AgentsFile {
                version: CURRENT_VERSION,
                agents,
            }
        };
        file.version = CURRENT_VERSION;

        // Write the v2 file first — this is the load-bearing step.
        if let Some(parent) = v2_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&file)?;
        fs_utils::atomic_write(v2_path, &bytes)?;

        // Rename the legacy runs dir. Best-effort: a cross-device
        // rename or a permissions error must not abort the
        // migration — the store is fully usable without it (a fresh
        // runs dir is created lazily on the next run).
        let data_dir = claudepot_data_dir();
        let legacy_runs = data_dir.join("automations");
        let v2_runs = data_dir.join("agents");
        if legacy_runs.exists() && !v2_runs.exists() {
            if let Err(e) = std::fs::rename(&legacy_runs, &v2_runs) {
                tracing::warn!(
                    error = %e,
                    from = %legacy_runs.display(),
                    to = %v2_runs.display(),
                    "agent store migration: runs dir rename failed; continuing"
                );
            }
        }

        // Back up the legacy JSON rather than deleting it.
        let backup = legacy_backup_path_for(legacy_path);
        if let Err(e) = std::fs::rename(legacy_path, &backup) {
            tracing::warn!(
                error = %e,
                from = %legacy_path.display(),
                to = %backup.display(),
                "agent store migration: legacy file backup rename failed; continuing"
            );
        }

        Ok(file)
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
        // Cross-field invariant: an event-triggered agent MUST carry
        // a rate limit (PRD D9). Without one, a reactive agent could
        // fire on every settled session unbounded — events × agents
        // × Claude is the dominant cost-runaway risk. It must not be
        // possible to install an unthrottled event agent.
        validate_event_rate_limit(&agent)?;
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
        if let Some(v) = patch.disallowed_tools {
            a.disallowed_tools = v;
        }
        if let Some(v) = patch.mcp_servers {
            a.mcp_servers = v;
        }
        if let Some(v) = patch.run_as {
            a.run_as = nz(v);
        }
        if let Some(v) = patch.task_budget {
            // 0 means "clear to None" (a zero token ceiling is
            // meaningless); any positive value sets the cap.
            a.task_budget = if v == 0 { None } else { Some(v) };
        }
        if let Some(v) = patch.rate_limit {
            // An all-None RateLimit means "clear to None": there is
            // nothing to enforce, so collapse it to absent.
            a.rate_limit = if v.min_interval_secs.is_none() && v.max_per_day.is_none() {
                None
            } else {
                Some(v)
            };
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
        // An event-triggered agent must keep a rate limit through
        // every mutation — a patch that switches the trigger to
        // `event` (or strips the rate limit off an event agent) is
        // rejected here, the last gate before persistence.
        validate_event_rate_limit(&a)?;
        a.updated_at = chrono::Utc::now();
        // Commit the validated clone back into the live store. Any
        // earlier `Err` return path leaves `self.file.agents[idx]`
        // untouched, preserving the previous valid record.
        self.file.agents[idx] = a;
        Ok(())
    }

    /// Arm a draft agent: flip its `lifecycle` from `Draft` to
    /// `Installed` and return a clone of the now-armed record.
    ///
    /// This is the in-memory half of the human-only draft->install
    /// gate (PRD §8.2 / D8). It is the **only** way a `lifecycle`
    /// reaches `Installed` outside the v1->v2 migration: there is no
    /// patch field for `lifecycle`, so `update` cannot touch it. The
    /// caller (the GUI's `agent_install` Tauri command) materializes
    /// the scheduler artifact *after* this returns and *before*
    /// `save`, so a failed registration can be rolled back.
    ///
    /// Refuses an agent that is already `Installed` — arming is not
    /// idempotent at this layer; a second call is a caller bug
    /// (double-click, stale UI) and surfaces as an error rather
    /// than silently re-materializing an artifact.
    pub fn arm(&mut self, id: &AgentId) -> Result<Agent, AgentError> {
        let idx = self
            .file
            .agents
            .iter()
            .position(|a| &a.id == id)
            .ok_or_else(|| AgentError::NotFound(id.to_string()))?;
        if self.file.agents[idx].lifecycle == Lifecycle::Installed {
            return Err(AgentError::InvalidEnv(format!(
                "agent {id} is already installed — drafts arm exactly once"
            )));
        }
        self.file.agents[idx].lifecycle = Lifecycle::Installed;
        self.file.agents[idx].updated_at = chrono::Utc::now();
        Ok(self.file.agents[idx].clone())
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

/// Reject an `Event`-triggered agent that carries no usable
/// `rate_limit` (PRD D9). An event trigger fires reactively, so an
/// unthrottled one is a cost-runaway hazard; the store refuses to
/// persist one. A rate limit "counts" only if it actually
/// constrains — an all-`None` [`RateLimit`] is treated as absent.
///
/// Non-event triggers (cron, manual) are unaffected — their
/// frequency is already bounded by the schedule / the Run-Now
/// button.
fn validate_event_rate_limit(agent: &Agent) -> Result<(), AgentError> {
    if !agent.trigger.is_event() {
        return Ok(());
    }
    let has_usable_limit = agent.rate_limit.as_ref().is_some_and(|r| {
        r.min_interval_secs.is_some() || r.max_per_day.is_some()
    });
    if !has_usable_limit {
        return Err(AgentError::InvalidEnv(
            "an event-triggered agent must carry a rate_limit \
             (a min interval and/or a max per day)"
                .into(),
        ));
    }
    Ok(())
}

/// Canonical path: `<claudepot_data_dir>/agents.json` (v2).
pub fn agents_file_path() -> PathBuf {
    claudepot_data_dir().join("agents.json")
}

/// Legacy v1 path: `<claudepot_data_dir>/automations.json`.
/// Read only by the v1 -> v2 migration in [`AgentStore::open_at`].
fn legacy_agents_file_path() -> PathBuf {
    claudepot_data_dir().join("automations.json")
}

/// Resolve the legacy v1 file that sits as a sibling of the given
/// v2 path. For the canonical data-dir path this is exactly
/// [`legacy_agents_file_path`]; for an arbitrary test path it is
/// `automations.json` in the same directory, so tests can exercise
/// the migration without touching the real data dir.
fn legacy_agents_file_path_for(v2_path: &Path) -> PathBuf {
    if v2_path == agents_file_path() {
        legacy_agents_file_path()
    } else {
        match v2_path.parent() {
            Some(dir) => dir.join("automations.json"),
            None => PathBuf::from("automations.json"),
        }
    }
}

/// Backup name for a migrated legacy file: append
/// `.pre-v2-backup` to the original file name.
fn legacy_backup_path_for(legacy_path: &Path) -> PathBuf {
    let mut name = legacy_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("automations.json"));
    name.push(".pre-v2-backup");
    match legacy_path.parent() {
        Some(dir) => dir.join(name),
        None => PathBuf::from(name),
    }
}

/// Per-agent directory inside the data dir.
pub fn agent_dir(id: &AgentId) -> PathBuf {
    claudepot_data_dir()
        .join("agents")
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
            disallowed_tools: vec![],
            mcp_servers: vec![],
            run_as: None,
            task_budget: None,
            rate_limit: None,
            lifecycle: Lifecycle::Installed,
            drafted_by: None,
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
        // Hand-author a v2 JSON file with an unknown field; verify
        // it round-trips harmlessly. serde drops unknown fields by
        // default, and the version envelope tolerates them.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        std::fs::write(
            &path,
            r#"{"version":2,"agents":[],"future_field":"ignored"}"#,
        )
        .unwrap();
        let store = AgentStore::open_at(path).unwrap();
        assert!(store.list().is_empty());
    }

    /// A literal v1 record: every Phase-0/pre-Phase-1 field present,
    /// none of the Phase-1 spec fields. Drives the migration golden.
    const V1_FIXTURE: &str = r#"{
      "version": 1,
      "automations": [
        {
          "id": "11111111-1111-1111-1111-111111111111",
          "name": "morning-pr",
          "enabled": true,
          "binary": { "kind": "first_party" },
          "model": "sonnet",
          "cwd": "/tmp",
          "prompt": "say hi",
          "permission_mode": "dontAsk",
          "allowed_tools": ["Read"],
          "output_format": "json",
          "trigger": { "kind": "cron", "cron": "0 9 * * *" },
          "created_at": "2026-01-01T00:00:00Z",
          "updated_at": "2026-01-01T00:00:00Z"
        }
      ]
    }"#;

    #[test]
    fn migrate_v1_upgrades_record_with_phase1_defaults() {
        // GOLDEN: a literal v1 file migrates to v2 with every new
        // Phase-1 field at its default — and `lifecycle == Installed`
        // (a pre-existing automation is already armed; it must NOT
        // silently become an inert draft).
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");
        std::fs::write(&v1_path, V1_FIXTURE).unwrap();

        let store = AgentStore::open_at(v2_path.clone()).unwrap();
        assert_eq!(store.list().len(), 1);
        let a = &store.list()[0];
        assert_eq!(a.name, "morning-pr");
        // Phase-1 spec fields default.
        assert!(a.disallowed_tools.is_empty());
        assert!(a.mcp_servers.is_empty());
        assert_eq!(a.run_as, None);
        assert_eq!(a.task_budget, None);
        assert_eq!(a.rate_limit, None);
        assert_eq!(a.drafted_by, None);
        // The load-bearing override: migrated records are Installed.
        assert_eq!(a.lifecycle, Lifecycle::Installed);

        // The migration wrote a v2 file...
        assert!(v2_path.exists());
        let raw = std::fs::read_to_string(&v2_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["version"], 2);
        assert!(v["agents"].is_array());
        // ...and backed up the legacy file rather than deleting it.
        assert!(!v1_path.exists());
        assert!(dir.path().join("automations.json.pre-v2-backup").exists());
    }

    #[test]
    fn migrate_v1_is_idempotent() {
        // Re-opening after a migration loads the v2 file directly and
        // does not re-run the migration (the legacy file is gone).
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");
        std::fs::write(&v1_path, V1_FIXTURE).unwrap();

        let first = AgentStore::open_at(v2_path.clone()).unwrap();
        assert_eq!(first.list().len(), 1);

        let second = AgentStore::open_at(v2_path.clone()).unwrap();
        assert_eq!(second.list().len(), 1);
        assert_eq!(second.list()[0].lifecycle, Lifecycle::Installed);
        assert_eq!(second.list()[0].name, "morning-pr");
    }

    #[test]
    fn v2_file_round_trips() {
        // GOLDEN: a v2 store with the new fields populated saves and
        // reopens byte-faithfully through the v2 path.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        let mut a = sample("evening-digest");
        a.lifecycle = Lifecycle::Draft;
        a.drafted_by = Some("claude-code@2026-05-22".into());
        a.disallowed_tools = vec!["Bash".into()];
        a.mcp_servers = vec![McpServerRef::ClaudepotMemory];
        a.run_as = Some("dev@example.com".into());
        a.task_budget = Some(50_000);
        a.rate_limit = Some(RateLimit {
            min_interval_secs: Some(3600),
            max_per_day: Some(12),
        });
        let id = a.id;
        store.add(a.clone()).unwrap();
        store.save().unwrap();

        // The on-disk envelope is v2 with the native `agents` key.
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["version"], 2);
        assert!(v["agents"].is_array());

        let reopened = AgentStore::open_at(path).unwrap();
        let back = reopened.get(&id).unwrap();
        assert_eq!(back, &a);
    }

    #[test]
    fn schema_version_too_new_is_refused() {
        // The guard still fires: a v2 file claiming a future schema
        // version is rejected rather than silently downgraded.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        std::fs::write(&path, r#"{"version":999,"agents":[]}"#).unwrap();
        match AgentStore::open_at(path) {
            Err(AgentError::InvalidEnv(m)) => {
                assert!(m.contains("999"), "expected version in message, got {m}");
            }
            Err(other) => panic!("expected schema-too-new InvalidEnv, got {other:?}"),
            Ok(_) => panic!("expected schema-too-new guard to reject a v999 file"),
        }
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

    #[test]
    fn arm_flips_draft_to_installed() {
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut draft = sample("draft-agent");
        draft.lifecycle = Lifecycle::Draft;
        let id = draft.id;
        store.add(draft).unwrap();
        assert_eq!(store.get(&id).unwrap().lifecycle, Lifecycle::Draft);

        let armed = store.arm(&id).unwrap();
        assert_eq!(armed.lifecycle, Lifecycle::Installed);
        assert_eq!(store.get(&id).unwrap().lifecycle, Lifecycle::Installed);
    }

    #[test]
    fn arm_rejects_already_installed_agent() {
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        // `sample` returns an Installed agent.
        let installed = sample("armed-agent");
        let id = installed.id;
        store.add(installed).unwrap();
        let err = store.arm(&id).unwrap_err();
        assert!(matches!(err, AgentError::InvalidEnv(_)));
    }

    #[test]
    fn arm_unknown_id_returns_not_found() {
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        assert!(matches!(
            store.arm(&Uuid::new_v4()),
            Err(AgentError::NotFound(_))
        ));
    }
}
