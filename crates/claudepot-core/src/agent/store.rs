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
//! The migration is **crash-safe and concurrency-safe** (grill
//! findings F7/F8). The legacy `automations.json` is renamed to its
//! `.pre-v2-backup` *before* the v2 `agents.json` is written, so at
//! any instant exactly one of {v1, v2} is present under its
//! canonical name — a crash mid-migration can never leave both.
//! The whole open→migrate→save critical section is held under an
//! advisory file lock, so a concurrent CLI + GUI first-boot cannot
//! race the migration.
//!
//! ## Concurrent-write protection
//!
//! `agents.json` is a JSON read-modify-write store with two live
//! writers (the CLI `agent draft` verb and the GUI `agents_add` /
//! `agent_install` commands). To stop a stale read from one writer
//! clobbering the other's committed write, every mutating
//! [`AgentStore`] holds an advisory exclusive file lock on a
//! sibling `agents.json.lock` for its whole open→mutate→save
//! lifetime (grill finding F7). The lock is released on `Drop`, so
//! a single process that opens the store, mutates, saves, drops it,
//! then opens it again works without deadlock — the sequential
//! pattern every CLI command and Tauri command uses.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs_utils;
use crate::paths::claudepot_data_dir;

mod lock;
use lock::StoreLock;

use super::draft::{
    validate_agent_inputs, validate_cwd, validate_event_trigger_numerics,
    validate_rate_limit_numerics, validate_trigger_timezone,
};
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
///
/// Holds an advisory file lock ([`StoreLock`]) for its whole
/// lifetime — acquired in [`AgentStore::open_at`], released on
/// `Drop` — so a concurrent CLI + GUI open→mutate→save cannot lose
/// writes (grill finding F7). The lock is exclusive, so the store
/// is a serialization point: open it, mutate, `save`, drop it
/// promptly. A long-lived `AgentStore` held open across unrelated
/// work would block every other writer. The lock holds only a
/// `File`, so an `AgentStore` is still `Send` and may cross an
/// `.await` in a Tauri async command.
pub struct AgentStore {
    path: PathBuf,
    file: AgentsFile,
    /// The advisory lock held for the store's lifetime. Field order
    /// places it last so it drops *after* `file`/`path` — purely
    /// cosmetic (none of them have side-effecting `Drop` beyond the
    /// lock), but it keeps "lock released last" obvious.
    _lock: StoreLock,
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
        // Acquire the advisory lock BEFORE touching the filesystem.
        // It is held for the store's whole lifetime and covers the
        // migration too (grill findings F7/F8): a concurrent CLI +
        // GUI first-boot can no longer race the v1 -> v2 migration,
        // and no two writers can interleave open -> mutate -> save.
        let lock = StoreLock::acquire(&path)?;

        // (1) A v2 file already exists -> load it directly.
        if path.exists() {
            return Ok(Self {
                file: Self::load_v2(&path)?,
                path,
                _lock: lock,
            });
        }

        // (2) No v2 file. Look for a legacy v1 `automations.json`
        // sibling and migrate it if present.
        let legacy_path = legacy_agents_file_path_for(&path);
        if legacy_path.exists() {
            let file = Self::migrate_v1_to_v2(&legacy_path, &path)?;
            return Ok(Self {
                file,
                path,
                _lock: lock,
            });
        }

        // (3) Neither file exists -> fresh empty v2 store.
        Ok(Self {
            path,
            file: AgentsFile::default(),
            _lock: lock,
        })
    }

    /// Best-effort non-blocking open (grill finding X10). Returns
    /// `Ok(None)` when the advisory lock is currently held by
    /// another process or thread, so the caller can degrade
    /// gracefully instead of blocking the way [`open`](Self::open)
    /// does. A real failure (FS error, corrupt v2 file, migration
    /// problem) still propagates as `Err`.
    ///
    /// Designed for short-lived best-effort *readers* — the
    /// `_record-run` CLI verb in particular needs only the agent's
    /// `log_retention_runs` field to drive the post-record prune.
    /// Blocking on the GUI's open mutex just to read a single
    /// number stalls every `claude -p` exit; skipping a single
    /// retention pass under contention is the correct trade-off.
    /// New skip-on-contention readers should funnel through here so
    /// the "no, but not an error" path stays in one place.
    pub fn try_open() -> Result<Option<Self>, AgentError> {
        Self::try_open_at(agents_file_path())
    }

    /// Non-blocking variant of [`open_at`](Self::open_at). See
    /// [`try_open`](Self::try_open) for the rationale.
    pub fn try_open_at(path: PathBuf) -> Result<Option<Self>, AgentError> {
        let lock = match StoreLock::try_acquire(&path)? {
            Some(l) => l,
            None => return Ok(None),
        };

        if path.exists() {
            return Ok(Some(Self {
                file: Self::load_v2(&path)?,
                path,
                _lock: lock,
            }));
        }

        let legacy_path = legacy_agents_file_path_for(&path);
        if legacy_path.exists() {
            let file = Self::migrate_v1_to_v2(&legacy_path, &path)?;
            return Ok(Some(Self {
                file,
                path,
                _lock: lock,
            }));
        }

        Ok(Some(Self {
            path,
            file: AgentsFile::default(),
            _lock: lock,
        }))
    }

    /// Read and validate a v2 `agents.json` from disk.
    ///
    /// **Threat-model note (grill finding F15).** `lifecycle` is a
    /// plain serde field: `load_v2` does not — and cannot —
    /// cross-check that an `Installed` record was legitimately armed
    /// through [`arm`](Self::arm). The draft/install gate is
    /// airtight against the *CLI verb surface* (there is no install
    /// or edit verb, and `AgentPatch` has no `lifecycle` field), but
    /// **not** against a same-user process editing `agents.json`
    /// directly: anything that can write the file can set
    /// `"lifecycle":"installed"`. That is the universal "same user =
    /// same trust" limit, not a closable hole. The mitigation is
    /// boot-time reconciliation against the OS scheduler — see
    /// [`reconcile_with_scheduler`] — which loudly logs any
    /// `Installed` record with no live scheduler artifact.
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
    /// `agents.json`. Crash-safe, concurrency-safe, idempotent, and
    /// data-preserving (grill finding F8).
    ///
    /// The caller ([`open_at`](Self::open_at)) holds the advisory
    /// [`StoreLock`] across this whole function, so a concurrent
    /// CLI + GUI first-boot cannot race the migration.
    ///
    /// **Crash safety — backup before write.** The ordering is:
    ///
    /// 1. parse + upgrade the v1 records in memory;
    /// 2. move the legacy runs dir (`automations/` -> `agents/`);
    /// 3. rename the legacy `automations.json` ->
    ///    `automations.json.pre-v2-backup` — the single atomic gate;
    /// 4. write the v2 `agents.json`.
    ///
    /// Renaming the legacy JSON aside *before* writing the v2 file
    /// means that at every instant exactly one of {v1, v2} exists
    /// under its canonical name. A crash between steps 3 and 4
    /// leaves only the `.pre-v2-backup` file: the next `open_at`
    /// finds neither `agents.json` nor `automations.json` and starts
    /// a fresh empty v2 store — the records sit safely in the
    /// backup for manual recovery. The previous order (write v2,
    /// then rename) could leave *both* files present, two competing
    /// sources of truth.
    ///
    /// **Runs-dir rename.** A cross-device (`EXDEV`) rename falls
    /// back to a recursive copy + remove rather than silently
    /// orphaning the run history under the un-renamed
    /// `automations/` directory. If even the copy fails the
    /// migration aborts with an error — run history is not
    /// discarded silently.
    fn migrate_v1_to_v2(legacy_path: &Path, v2_path: &Path) -> Result<AgentsFile, AgentError> {
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

        // Step 2 — move the legacy runs dir. Done before the JSON
        // rename so the run history follows the store. An `EXDEV`
        // (cross-device) rename falls back to copy + remove; any
        // hard failure aborts the migration so run history is never
        // silently orphaned under the old directory name.
        let data_dir = claudepot_data_dir();
        let legacy_runs = data_dir.join("automations");
        let v2_runs = data_dir.join("agents");
        if legacy_runs.exists() && !v2_runs.exists() {
            migrate_runs_dir(&legacy_runs, &v2_runs)?;
        }

        // Step 3 — rename the legacy JSON aside. This is the single
        // atomic gate: after this rename succeeds the legacy file no
        // longer exists under its canonical name, so a crash before
        // the v2 write below cannot leave two sources of truth.
        // A failure here is fatal — without it the next open would
        // re-run the migration and a half-written state could
        // diverge.
        let backup = legacy_backup_path_for(legacy_path);
        std::fs::rename(legacy_path, &backup)?;

        // Step 4 — write the v2 file. The backup already exists, so
        // even if this write fails the legacy data is preserved in
        // `.pre-v2-backup` and the next open starts a clean empty
        // v2 store. We still surface the error.
        if let Some(parent) = v2_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&file)?;
        fs_utils::atomic_write(v2_path, &bytes)?;

        Ok(file)
    }

    pub fn list(&self) -> &[Agent] {
        &self.file.agents
    }

    pub fn get(&self, id: &AgentId) -> Option<&Agent> {
        self.file.agents.iter().find(|a| &a.id == id)
    }

    /// Best-effort lookup of an agent's `lifecycle` field without
    /// holding the store across the caller's whole operation.
    ///
    /// grill X17: `agents_run_now_start` was reading the store under
    /// the exclusive open-lock just to gate on `Lifecycle::Draft`
    /// before spawning the run. The full open is needed later (the
    /// spawned task clones the agent for the closure), but the
    /// pre-spawn validation is cheap and structurally "lock first,
    /// validate second" — under GUI contention an installed-agent
    /// run-now would block on a different user's mid-install for
    /// nothing.
    ///
    /// Implementation reuses the X10 `try_open` machinery: on a
    /// free lock we read the file, return the lifecycle, and drop
    /// the lock before the caller proceeds to its real open. On
    /// contention we return `Ok(None)` so the caller falls through
    /// to the full open path — the gate will still run, just
    /// behind the lock (the original behavior). A real I/O failure
    /// propagates as `Err`.
    pub fn lifecycle_of(id: &AgentId) -> Result<Option<Lifecycle>, AgentError> {
        Self::lifecycle_of_at(agents_file_path(), id)
    }

    /// Path-injected variant of [`lifecycle_of`](Self::lifecycle_of)
    /// for tests.
    pub fn lifecycle_of_at(path: PathBuf, id: &AgentId) -> Result<Option<Lifecycle>, AgentError> {
        match Self::try_open_at(path)? {
            Some(store) => Ok(store.get(id).map(|a| a.lifecycle)),
            None => Ok(None),
        }
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
        // Re-validate the working directory at the store boundary —
        // the last gate before persistence (grill finding F4).
        validate_cwd(&agent.cwd)?;
        // grill X13: hoist the F18 per-field caps + control-char
        // gate to the persistence boundary. `build_draft` enforces
        // these for the AI-drafting path; the GUI verbs go through
        // `add`/`update`, which previously skipped them entirely —
        // meaning a renderer-supplied 10 MB `prompt` or
        // control-character `system_prompt` could land on disk.
        // Defense-in-depth: a draft path already validated; an
        // unchecked GUI/template path did not.
        validate_agent_inputs(&agent)?;
        // Reject a cron trigger carrying an IANA timezone — no
        // scheduler adapter honors it (grill finding F11). This
        // gates the GUI Add-Agent path too, not just the CLI draft.
        validate_trigger_timezone(&agent.trigger)?;
        // Numeric bounds (grill finding F20). Gates the GUI path
        // too — a `debounce_secs` past the 7-day ceiling, or a
        // zero-valued rate-limit slot, is rejected here as well as
        // at draft time.
        validate_event_trigger_numerics(&agent.trigger)?;
        validate_rate_limit_numerics(agent.rate_limit.as_ref())?;
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
        if self.file.agents.iter().any(|a| a.name == agent.name) {
            return Err(AgentError::DuplicateName(agent.name));
        }
        if self.file.agents.iter().any(|a| a.id == agent.id) {
            return Err(AgentError::DuplicateName(format!("id {}", agent.id)));
        }
        self.file.agents.push(agent);
        Ok(())
    }

    /// Apply a patch to an existing agent. Bumps `updated_at`.
    pub fn update(&mut self, id: &AgentId, patch: AgentPatch) -> Result<(), AgentError> {
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
            // grill X3 / F26: the F3 fix banned `Custom` MCP servers
            // in `build_draft`, but `update()` previously replaced
            // `mcp_servers` wholesale with no re-validation — a
            // compromised renderer (or any IPC caller) could swap a
            // `Custom { command, args }` config onto an already-
            // installed agent and `claude -p --mcp-config` would
            // spawn it on every run. Hoist the gate to the
            // persistence boundary: reject any `Custom` server in the
            // patch that is not already present (verbatim) in the
            // prior record. An unchanged update is a no-op (no new
            // Custom entries); the v1->v2 migration of records that
            // already carry a `Custom` server is unaffected.
            //
            // `McpServerRef` cannot derive `Hash`/`Eq` (it carries a
            // `serde_json::Value`), so we use a linear scan + the
            // existing `PartialEq` derive. The list is bounded by
            // `MAX_MCP_SERVERS_ELEMS`, so O(n×m) is fine here.
            let prior = &self.file.agents[idx].mcp_servers;
            for entry in &v {
                if matches!(entry, super::types::McpServerRef::Custom { .. })
                    && !prior.iter().any(|p| p == entry)
                {
                    let name = match entry {
                        super::types::McpServerRef::Custom { name, .. } => name.as_str(),
                        _ => "?",
                    };
                    return Err(AgentError::InvalidEnv(format!(
                        "custom MCP server {name:?} is not allowed via update — \
                         only the Claudepot memory server may be attached \
                         to an installed agent"
                    )));
                }
            }
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
        // A patch that switches to (or keeps) a cron trigger with an
        // IANA timezone is rejected — no adapter honors it (F11).
        validate_trigger_timezone(&a.trigger)?;
        // Numeric bounds (F20) re-checked on update so a patch that
        // sets `debounce_secs` past the ceiling, or zeroes a
        // rate-limit slot, is rejected before persistence.
        validate_event_trigger_numerics(&a.trigger)?;
        validate_rate_limit_numerics(a.rate_limit.as_ref())?;
        // grill X13: per-field byte caps + control-char rejection
        // (F18). Same rationale as `add` — the GUI's `agents_update`
        // would otherwise let an arbitrary-size `prompt` /
        // `system_prompt` patch reach disk. Validated against the
        // post-merge clone so partial patches that *re-set* a now-
        // oversize field are caught, not the pre-merge stale view.
        validate_agent_inputs(&a)?;
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

    /// Force an agent's `lifecycle` to a specific value in memory.
    ///
    /// Unlike [`arm`](Self::arm) — the *only* sanctioned Draft →
    /// Installed transition — this is an unconditional setter used
    /// for **rollback**: the install gate ([`install_gate`]) calls
    /// it to revert a failed arm back to `Draft` so a reused store
    /// object never claims an agent is installed when its scheduler
    /// artifact never materialized. No-op (and not an error) when
    /// the id is unknown — there is nothing to roll back.
    ///
    /// [`install_gate`]: super::install_gate
    pub fn set_lifecycle(&mut self, id: &AgentId, lifecycle: Lifecycle) {
        if let Some(a) = self.file.agents.iter_mut().find(|a| &a.id == id) {
            a.lifecycle = lifecycle;
        }
    }

    /// Force an agent's `updated_at` to a specific value in memory.
    ///
    /// Test seam / rollback helper. Used by
    /// [`install_gate::apply_lifecycle_change`] to restore the
    /// pre-mutation timestamp when a rollback fires — `arm` and
    /// `update` both bump `updated_at = now()`, so a rolled-back
    /// mutation otherwise leaves the agent claiming it was just
    /// edited even though every field is back to where it started
    /// (grill X24). No-op when the id is unknown.
    ///
    /// Same trust posture as [`set_lifecycle`](Self::set_lifecycle):
    /// the store-boundary API gives the install gate the
    /// surgical-write seam it needs without widening `AgentPatch`
    /// (which has no `updated_at` field by design — the timestamp
    /// is not a renderer-supplied value).
    ///
    /// [`install_gate::apply_lifecycle_change`]:
    ///     super::install_gate::apply_lifecycle_change
    pub fn set_updated_at(&mut self, id: &AgentId, updated_at: chrono::DateTime<chrono::Utc>) {
        if let Some(a) = self.file.agents.iter_mut().find(|a| &a.id == id) {
            a.updated_at = updated_at;
        } else {
            // A7: previously a silent no-op. The helper at
            // `install_gate::apply_lifecycle_change` calls this from
            // every rollback branch; if the rollback closure removed
            // the agent (the `agents_add` shape), the id is genuinely
            // absent and the no-op is correct. Log at DEBUG so the
            // rare "rollback removed the row, then helper still tried
            // to restore its timestamp" sequence is visible during
            // diagnosis without spamming the typical happy path.
            tracing::debug!(
                agent_id = %id,
                "set_updated_at: id not found; no-op (typical for an \
                 add-shaped rollback that already removed the record)"
            );
        }
    }

    /// Re-point the store at a different path. A test seam only —
    /// lets a test force a `save` failure by aiming the store at an
    /// un-creatable path after seeding it through a writable one.
    #[cfg(test)]
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = path;
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

/// One discrepancy found by [`reconcile_installed_agents`]: an agent
/// whose stored `lifecycle` is `Installed` but for which the OS
/// scheduler reports no live artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanInstalled {
    /// The agent id whose record claims `Installed`.
    pub agent_id: AgentId,
    /// The agent's name, for the log line.
    pub name: String,
}

/// One discrepancy found by [`reconcile_orphan_artifacts`] — the
/// **reverse** direction of [`OrphanInstalled`]: a Claudepot-managed
/// scheduler artifact whose identifier corresponds to no `Installed`
/// agent record (no record at all, or a `Draft`/removed record). The
/// artifact will fire `claude -p` on schedule with no visible record
/// behind it.
///
/// grill finding X9: a hand-edit to `agents.json` (or a third-process
/// crash mid-install before the v2 `agents.json` was rewritten) can
/// leave a stale launchd/systemd/schtasks artifact behind. The
/// existing `reconcile_installed_agents` checks the
/// `Installed → artifact` direction only; this struct + its
/// reconciler close the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanArtifact {
    /// The scheduler identifier the host reports (launchd label,
    /// systemd unit, Task Scheduler path).
    pub identifier: String,
}

/// Pure reconciliation core (grill finding F15).
///
/// Compares every `Installed`, OS-scheduled agent against the set of
/// scheduler artifact identifiers the host actually reports, and
/// returns the agents that claim `Installed` but have no live
/// artifact. An agent whose trigger carries no OS schedule
/// (`Manual` / `Event`) is **never** flagged — those are run by
/// Run-Now or the in-app event orchestrator and legitimately have
/// no scheduler artifact even when `Installed`.
///
/// `expected_identifier` maps an agent id to the scheduler
/// identifier the active adapter would register for it (e.g. the
/// launchd label). Passing it in keeps this function pure and
/// host-agnostic — testable without a real scheduler.
pub fn reconcile_installed_agents(
    agents: &[Agent],
    registered_identifiers: &std::collections::HashSet<String>,
    expected_identifier: impl Fn(&AgentId) -> String,
) -> Vec<OrphanInstalled> {
    agents
        .iter()
        .filter(|a| a.lifecycle == Lifecycle::Installed)
        // Only OS-scheduled triggers should have an artifact. A
        // Manual/Event agent legitimately has none.
        .filter(|a| !a.trigger.has_no_os_schedule())
        // An installed-but-disabled agent has no artifact by design
        // (disabling unregisters it) — not an orphan.
        .filter(|a| a.enabled)
        .filter(|a| !registered_identifiers.contains(&expected_identifier(&a.id)))
        .map(|a| OrphanInstalled {
            agent_id: a.id,
            name: a.name.clone(),
        })
        .collect()
}

/// Boot-time reconciliation of the store against the OS scheduler
/// (grill finding F15).
///
/// Opens the store, asks the active scheduler what artifacts it
/// actually holds, and **loudly logs** every `Installed` agent with
/// no live artifact. This catches the one residual gap the
/// draft/install gate cannot close on its own: a same-user process
/// editing `agents.json` to set `"lifecycle":"installed"` directly
/// (the gate is airtight only against the *CLI verb surface* — see
/// [`AgentStore::load_v2`]). It also surfaces an artifact that a
/// failed install rollback could not re-save (see
/// [`install_gate`](super::install_gate)).
///
/// This is *observability*, not enforcement: it does not mutate the
/// store. Demoting an orphan record to `Draft` automatically would
/// risk silently disarming an agent during a transient scheduler
/// hiccup; logging loudly and leaving the decision to the operator
/// is the conservative choice. The function is best-effort — a
/// store-open or scheduler-query failure is logged and swallowed so
/// it can run unconditionally at boot. Returns the orphan list for
/// callers (and tests) that want to act on it.
pub fn reconcile_with_scheduler() -> Vec<OrphanInstalled> {
    let scheduler = super::scheduler::active_scheduler();
    reconcile_with_scheduler_using(scheduler.as_ref())
}

/// Scheduler-injected variant of [`reconcile_with_scheduler`] for
/// tests and for callers that want to drive the boot-time reconcile
/// against a non-active scheduler.
///
/// grill X29: the wired form above takes the active scheduler from
/// the host platform, which makes the outer wiring (open store + ask
/// scheduler + log) impossible to exercise without booting real
/// launchd/systemd/Task Scheduler. The pure
/// [`reconcile_installed_agents`] predicate is covered; this seam
/// lets the wiring (store-load → list_managed → identifier match →
/// log) be exercised end-to-end via a `FakeScheduler` in an
/// integration test.
pub fn reconcile_with_scheduler_using(
    scheduler: &dyn super::scheduler::Scheduler,
) -> Vec<OrphanInstalled> {
    let store = match AgentStore::open() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "agent reconciliation: store open failed; skipping"
            );
            return Vec::new();
        }
    };
    let registered: std::collections::HashSet<String> = match scheduler.list_managed() {
        Ok(entries) => entries.into_iter().map(|e| e.identifier).collect(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "agent reconciliation: scheduler list_managed failed; skipping"
            );
            return Vec::new();
        }
    };
    let orphans = reconcile_installed_agents(store.list(), &registered, |id| {
        scheduler.expected_identifier(id)
    });
    for orphan in &orphans {
        tracing::error!(
            agent_id = %orphan.agent_id,
            agent_name = %orphan.name,
            "agent reconciliation: agent is marked Installed but the OS \
             scheduler reports NO live artifact for it — it will never \
             fire on schedule. Either it was hand-edited into \
             agents.json, or an install rollback could not complete. \
             Re-install it from the GUI to materialize the artifact, or \
             delete it."
        );
    }
    if orphans.is_empty() {
        tracing::debug!(
            "agent reconciliation: every Installed agent has a live \
             scheduler artifact"
        );
    }
    orphans
}

/// Pure reverse-direction reconciler (grill finding X9). Returns the
/// scheduler artifacts the host reports that **do not** correspond to
/// any `Installed` agent in the store.
///
/// `reconcile_installed_agents` checks one direction only:
/// `Installed → artifact`. This closes the other:
/// `artifact → Installed`. A `claudepot_managed` launchd/systemd/
/// schtasks artifact whose identifier maps to no live `Installed`
/// record will fire `claude -p` on schedule with no visible record
/// behind it — the same blind-firing hazard the install gate (X1, X2)
/// closes for the create direction, but reachable through a hand-edit
/// to `agents.json` or a third-process artifact write.
///
/// `expected_identifier` maps an agent id to the scheduler identifier
/// the active adapter would register for it (matches the
/// [`reconcile_installed_agents`] seam). `installed_identifiers`
/// pre-computes the set of identifiers for every `Installed` agent;
/// any reported artifact whose id is not in that set is an orphan.
///
/// `registered_artifacts` is the list the scheduler reports. The
/// `claudepot_managed` filter belongs to the caller — the wired
/// [`reconcile_with_scheduler_full`] keeps only managed entries
/// before passing them in. (Foreign launchd/systemd labels we never
/// installed must not be flagged.)
pub fn reconcile_orphan_artifacts(
    installed_identifiers: &std::collections::HashSet<String>,
    registered_artifacts: &[super::scheduler::RegisteredEntry],
) -> Vec<OrphanArtifact> {
    registered_artifacts
        .iter()
        .filter(|e| e.claudepot_managed)
        .filter(|e| !installed_identifiers.contains(&e.identifier))
        .map(|e| OrphanArtifact {
            identifier: e.identifier.clone(),
        })
        .collect()
}

/// Boot-time wired version of [`reconcile_orphan_artifacts`]. Opens
/// the store, asks the active scheduler what artifacts it holds, and
/// **loudly logs** every Claudepot-managed artifact with no live
/// `Installed` record. Best-effort: a store-open or scheduler-query
/// failure logs + returns empty so it can run unconditionally at
/// boot. **Observability only** — never removes the orphan artifact
/// (same conservative policy as [`reconcile_with_scheduler`]: a
/// transient scheduler hiccup must not destroy a real registration).
pub fn reconcile_orphan_artifacts_now() -> Vec<OrphanArtifact> {
    let scheduler = super::scheduler::active_scheduler();
    reconcile_orphan_artifacts_using(scheduler.as_ref())
}

/// Scheduler-injected variant of [`reconcile_orphan_artifacts_now`]
/// for tests. Same X29 rationale as
/// [`reconcile_with_scheduler_using`]: the inner predicate is pure
/// and well-covered, but the *wiring* (open store + list_managed +
/// identifier expansion + log) needs a seam.
pub fn reconcile_orphan_artifacts_using(
    scheduler: &dyn super::scheduler::Scheduler,
) -> Vec<OrphanArtifact> {
    let store = match AgentStore::open() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "agent reverse reconciliation: store open failed; skipping"
            );
            return Vec::new();
        }
    };
    let registered: Vec<super::scheduler::RegisteredEntry> = match scheduler.list_managed() {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "agent reverse reconciliation: scheduler list_managed failed; skipping"
            );
            return Vec::new();
        }
    };
    let installed: std::collections::HashSet<String> = store
        .list()
        .iter()
        .filter(|a| a.lifecycle == Lifecycle::Installed)
        // Same trigger guard as the other direction: an `Installed`
        // Manual/Event agent legitimately has no scheduler artifact,
        // so its `expected_identifier` would never appear in the
        // host's report and would always look like an orphan from
        // the artifact side too. Filter both sides identically.
        .filter(|a| !a.trigger.has_no_os_schedule())
        .map(|a| scheduler.expected_identifier(&a.id))
        .collect();
    let orphans = reconcile_orphan_artifacts(&installed, &registered);
    for orphan in &orphans {
        // Log loudly — same severity as the other direction. The
        // file:line context comes from `tracing`'s metadata when the
        // subscriber is configured for it.
        tracing::error!(
            identifier = %orphan.identifier,
            "agent reverse reconciliation: a Claudepot-managed \
             scheduler artifact exists with NO matching Installed \
             agent — it will fire `claude -p` on schedule with no \
             visible record. Either agents.json was hand-edited \
             (removing the record but not the artifact), or a third \
             process created the artifact. Inspect the artifact and \
             unregister it manually if it is stale; Claudepot will \
             NOT remove it automatically."
        );
    }
    if orphans.is_empty() {
        tracing::debug!(
            "agent reverse reconciliation: every managed artifact has \
             a matching Installed agent"
        );
    }
    orphans
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
    let has_usable_limit = agent
        .rate_limit
        .as_ref()
        .is_some_and(|r| r.min_interval_secs.is_some() || r.max_per_day.is_some());
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

/// Move the legacy `automations/` runs dir to `agents/` during the
/// v1 -> v2 migration (grill finding F8).
///
/// A plain `std::fs::rename` is tried first — atomic and instant on
/// the common same-filesystem case. If it fails with a cross-device
/// error (`EXDEV` — the data dir and the runs dir straddle a mount
/// boundary, e.g. a bind-mounted `~/.claudepot`), it falls back to a
/// recursive copy followed by removing the source. Either way the
/// run history ends up under `agents/`; it is never silently
/// orphaned under the old directory name. A hard failure of *both*
/// paths surfaces as an [`AgentError`] so the migration aborts
/// loudly rather than continuing with lost run history.
fn migrate_runs_dir(legacy_runs: &Path, v2_runs: &Path) -> Result<(), AgentError> {
    match std::fs::rename(legacy_runs, v2_runs) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            tracing::warn!(
                from = %legacy_runs.display(),
                to = %v2_runs.display(),
                "agent store migration: runs dir rename is cross-device; \
                 falling back to copy + remove"
            );
            fs_utils::copy_dir_recursive(legacy_runs, v2_runs)?;
            // The copy succeeded; remove the source. A failed remove
            // leaves a harmless stale copy under the old name — log
            // it, but the migration is otherwise complete, so do not
            // abort.
            if let Err(rm) = std::fs::remove_dir_all(legacy_runs) {
                tracing::warn!(
                    error = %rm,
                    path = %legacy_runs.display(),
                    "agent store migration: copied runs dir but failed to \
                     remove the legacy source; a stale copy remains"
                );
            }
            Ok(())
        }
        Err(e) => Err(AgentError::Io(e)),
    }
}

/// True for a cross-device-link (`EXDEV`) I/O error. `ErrorKind`
/// has no portable `CrossesDevices` variant on stable Rust, so we
/// match the raw OS error code: `EXDEV` is 18 on Linux/macOS and
/// `ERROR_NOT_SAME_DEVICE` (17) on Windows.
fn is_cross_device(e: &std::io::Error) -> bool {
    match e.raw_os_error() {
        #[cfg(unix)]
        Some(code) => code == libc::EXDEV,
        #[cfg(windows)]
        Some(code) => code == 17,
        #[cfg(not(any(unix, windows)))]
        Some(_) => false,
        None => false,
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
    claudepot_data_dir().join("agents").join(id.to_string())
}

/// Per-agent runs directory.
pub fn agent_runs_dir(id: &AgentId) -> PathBuf {
    agent_dir(id).join("runs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::*;
    use crate::testing::test_cwd;
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
            cwd: test_cwd(),
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
            created_via: crate::agent::types::CreatedVia::Gui,
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
        // The first store must DROP before the second open — the F7
        // advisory lock is exclusive and serializes same-process
        // re-acquires too (the explicit unsupported pattern in the
        // `lock.rs` doc: two `StoreLock`s alive on one thread is a
        // self-deadlock). The original pre-lock pattern (`let mut
        // store = …; …; let reopened = …`) deadlocks after F7
        // landed; an explicit `drop` makes the lifetime obvious.
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        store.add(sample("morning-pr")).unwrap();
        store.add(sample("evening-summary")).unwrap();
        store.save().unwrap();
        drop(store);

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
        // The first store must DROP before the second open — see the
        // analogous explanation in `add_save_reopen_preserves_records`.
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");
        std::fs::write(&v1_path, V1_FIXTURE).unwrap();

        {
            let first = AgentStore::open_at(v2_path.clone()).unwrap();
            assert_eq!(first.list().len(), 1);
        }

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
        // Drop the first store so the lock releases — see the
        // analogous comment in `add_save_reopen_preserves_records`
        // for why this is mandatory under the F7 advisory lock.
        drop(store);

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
        assert!(matches!(store.add(bad), Err(AgentError::InvalidName(..))));
    }

    // ---- grill X13: F18 caps + control-char gate at the
    // persistence boundary ----

    #[test]
    fn add_rejects_oversize_prompt_at_persistence_boundary() {
        // X13: the byte caps that `build_draft` enforces must also
        // fire when `agents_add` / `agent_add_from_template` go
        // through `AgentStore::add`. A 200 KB prompt overflows the
        // 128 KB `MAX_PROMPT_BYTES` ceiling.
        use crate::agent::draft::MAX_PROMPT_BYTES;
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut bad = sample("oversize-agent");
        bad.prompt = "x".repeat(MAX_PROMPT_BYTES + 1);
        let err = store.add(bad).unwrap_err();
        assert!(
            matches!(err, AgentError::InvalidEnv(ref m) if m.contains("prompt")),
            "expected InvalidEnv mentioning prompt, got {err:?}"
        );
    }

    #[test]
    fn add_rejects_control_chars_in_system_prompt() {
        // X13: control characters in any shell-flag-bound field
        // must be rejected at persistence too (the draft path
        // already rejects them).
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut bad = sample("ctrl-agent");
        bad.system_prompt = Some(String::from("hello\u{0007}bell"));
        let err = store.add(bad).unwrap_err();
        assert!(
            matches!(err, AgentError::InvalidEnv(ref m) if m.contains("control")),
            "expected InvalidEnv mentioning control characters, got {err:?}"
        );
    }

    #[test]
    fn update_rejects_oversize_prompt_at_persistence_boundary() {
        // X13 paired test on the update path — `agents_update`
        // hits this gate too, not just `add`.
        use crate::agent::draft::MAX_PROMPT_BYTES;
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let a = sample("u-agent");
        let id = a.id;
        store.add(a).unwrap();
        let patch = AgentPatch {
            prompt: Some("y".repeat(MAX_PROMPT_BYTES + 1)),
            ..Default::default()
        };
        let err = store.update(&id, patch).unwrap_err();
        assert!(
            matches!(err, AgentError::InvalidEnv(ref m) if m.contains("prompt")),
            "expected InvalidEnv on oversize prompt patch, got {err:?}"
        );
        // The record must be unchanged.
        assert_eq!(store.get(&id).unwrap().prompt, "say hi");
    }

    // ---- grill X3 / F26: Custom MCP gate on update ------------

    #[test]
    fn update_rejects_introducing_new_custom_mcp_server() {
        // X3: the F3 ban on `Custom` MCP in drafts must also hold at
        // the persistence boundary; a patch that introduces a Custom
        // server not already present must be rejected so a
        // compromised IPC caller cannot strap a malicious child
        // process onto an already-installed agent.
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut a = sample("victim-agent");
        a.mcp_servers = vec![McpServerRef::ClaudepotMemory];
        let id = a.id;
        store.add(a).unwrap();

        let patch = AgentPatch {
            mcp_servers: Some(vec![
                McpServerRef::ClaudepotMemory,
                McpServerRef::Custom {
                    name: "evil".into(),
                    config: serde_json::json!({"command": "/usr/bin/curl"}),
                },
            ]),
            ..AgentPatch::default()
        };
        let err = store.update(&id, patch).unwrap_err();
        match err {
            AgentError::InvalidEnv(m) => {
                assert!(
                    m.contains("evil"),
                    "rejection must name the server, got {m}"
                );
            }
            other => panic!("expected InvalidEnv for new Custom MCP, got {other:?}"),
        }
        // The store record is untouched — the update never committed.
        let after = store.get(&id).unwrap();
        assert_eq!(after.mcp_servers, vec![McpServerRef::ClaudepotMemory]);
    }

    #[test]
    fn update_allows_no_op_rewrite_of_existing_custom_mcp_server() {
        // X3: a record that already carries a `Custom` entry (e.g. a
        // legacy install from before the F3 gate, or a v1->v2 record
        // that inherited one) must NOT be rejected by a no-op
        // round-trip — the gate is "introduces new Custom", not
        // "carries any Custom". Otherwise every save-after-load
        // round-trip would break.
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let custom = McpServerRef::Custom {
            name: "legacy".into(),
            config: serde_json::json!({"command": "/bin/echo"}),
        };
        let mut a = sample("legacy-agent");
        a.mcp_servers = vec![custom.clone()];
        let id = a.id;
        store.add(a).unwrap();

        // Same Custom entry, possibly reordered — must pass.
        let patch = AgentPatch {
            mcp_servers: Some(vec![custom.clone()]),
            ..AgentPatch::default()
        };
        store
            .update(&id, patch)
            .expect("no-op rewrite must be allowed");
    }

    /// A8b polish on grill X3: the no-op-rewrite gate compares via
    /// `McpServerRef`'s derived `PartialEq`, which delegates to
    /// `serde_json::Value`'s `PartialEq` for the `config` field.
    /// `Value::Object` deduplicates and orders keys via the inner
    /// map, so two JSON strings that differ only in key order parse
    /// to the SAME `Value` and `==` returns `true`. This test pins
    /// that contract: a `Custom` MCP server whose `config` JSON is
    /// hand-typed with a different key ordering must still be
    /// accepted as a no-op rewrite (otherwise round-tripping through
    /// any JSON formatter that reorders keys would fail the X3 gate).
    #[test]
    fn update_allows_identical_custom_with_reordered_json_object_keys() {
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();

        // Two semantically-identical JSON strings, but the second has
        // its top-level keys (`command`, `args`, `env`) in a different
        // order. `serde_json::from_str` materializes them into
        // structurally-equal `Value::Object`s — the X3 prior-equality
        // check must therefore accept the second as a no-op rewrite.
        let original_config: serde_json::Value =
            serde_json::from_str(r#"{"command":"/bin/echo","args":["hi"],"env":{"KEY":"val"}}"#)
                .unwrap();
        let reordered_config: serde_json::Value =
            serde_json::from_str(r#"{"env":{"KEY":"val"},"args":["hi"],"command":"/bin/echo"}"#)
                .unwrap();

        let mut a = sample("reorder-canary");
        a.mcp_servers = vec![McpServerRef::Custom {
            name: "fs".into(),
            config: original_config,
        }];
        let id = a.id;
        store.add(a).unwrap();

        let patch = AgentPatch {
            mcp_servers: Some(vec![McpServerRef::Custom {
                name: "fs".into(),
                config: reordered_config,
            }]),
            ..AgentPatch::default()
        };
        store
            .update(&id, patch)
            .expect("reordered-keys rewrite of the same Custom MCP must pass the X3 gate");
    }

    #[test]
    fn update_allows_dropping_a_custom_mcp_server() {
        // X3: the gate fires only on *introducing* a Custom server,
        // not on dropping one. Removing a previously-attached Custom
        // entry is a strict de-escalation and must pass.
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut a = sample("draining-agent");
        a.mcp_servers = vec![
            McpServerRef::Custom {
                name: "legacy".into(),
                config: serde_json::json!({"command": "/bin/echo"}),
            },
            McpServerRef::ClaudepotMemory,
        ];
        let id = a.id;
        store.add(a).unwrap();

        let patch = AgentPatch {
            mcp_servers: Some(vec![McpServerRef::ClaudepotMemory]),
            ..AgentPatch::default()
        };
        store
            .update(&id, patch)
            .expect("dropping a Custom must be allowed");
        assert_eq!(
            store.get(&id).unwrap().mcp_servers,
            vec![McpServerRef::ClaudepotMemory]
        );
    }

    // ---- arm + reconcile -------------------------------------

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

    // ---- grill F6: the migration's dangerous paths ----

    #[test]
    fn migrate_v1_succeeds_when_runs_dir_rename_is_a_noop() {
        // GOLDEN (F6a): the runs-dir rename is best-effort — when it
        // is a no-op (the legacy `automations/` runs dir is absent)
        // OR is skipped (the destination already exists), the
        // migration must still succeed. This case exercises the
        // no-op: the legacy file migrates, but there is no legacy
        // runs dir to move, so the rename branch is a no-op and the
        // migration completes cleanly.
        //
        // The migration's runs-dir logic reads the process-global
        // `claudepot_data_dir()`; a no-op runs-dir rename is the one
        // shape that is deterministic regardless of that global, so
        // the assertion here is exactly "the migration still
        // succeeds" — which is the load-bearing F6a claim.
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");
        std::fs::write(&v1_path, V1_FIXTURE).unwrap();
        // Deliberately do NOT create an `automations/` runs dir
        // beside the legacy file — the rename branch becomes a no-op.

        let store = AgentStore::open_at(v2_path.clone()).unwrap();
        // The load-bearing claim: a no-op runs-dir rename does not
        // abort the migration — every record still migrates.
        assert_eq!(store.list().len(), 1, "migration must still succeed");
        assert_eq!(store.list()[0].name, "morning-pr");
        assert_eq!(store.list()[0].lifecycle, Lifecycle::Installed);
        assert!(v2_path.exists(), "v2 agents.json was written");
        // The legacy file is backed up, not destroyed.
        assert!(!v1_path.exists());
        assert!(dir.path().join("automations.json.pre-v2-backup").exists());
    }

    #[test]
    fn migrate_v1_corrupt_legacy_file_is_an_error_not_a_panic() {
        // GOLDEN (F6b): a corrupt (non-JSON) `automations.json` must
        // surface as a clean `AgentError`, never a panic, and must
        // NOT silently produce an empty store (that would mask data
        // loss). The backup/v2 writes must not have run.
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");
        std::fs::write(&v1_path, b"{ this is not valid json").unwrap();

        match AgentStore::open_at(v2_path.clone()) {
            Err(AgentError::Json(_)) => { /* expected */ }
            Err(other) => panic!("expected a JSON error, got {other:?}"),
            Ok(_) => panic!("a corrupt legacy file must not load as a store"),
        }
        // The migration aborted before writing v2 or backing up the
        // legacy file — the corrupt file is left untouched for the
        // user to inspect.
        assert!(
            !v2_path.exists(),
            "v2 file must not exist after a failed migration"
        );
        assert!(v1_path.exists(), "corrupt legacy file is left in place");
        assert!(
            !dir.path().join("automations.json.pre-v2-backup").exists(),
            "no backup is created when the migration aborts"
        );
    }

    #[test]
    fn migrate_v1_empty_legacy_file_yields_empty_v2_store() {
        // GOLDEN (F6c): a zero-byte `automations.json` is a benign
        // legacy state (the user never created an automation). It
        // migrates to a fresh, empty v2 store — and still writes the
        // v2 file + backs up the legacy file so the next open is a
        // clean v2 load.
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");
        std::fs::write(&v1_path, b"").unwrap();

        let store = AgentStore::open_at(v2_path.clone()).unwrap();
        assert!(store.list().is_empty());

        // v2 file written, version 2.
        let raw = std::fs::read_to_string(&v2_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["version"], 2);
        // Legacy file backed up, not deleted.
        assert!(!v1_path.exists());
        assert!(dir.path().join("automations.json.pre-v2-backup").exists());
    }

    #[test]
    fn migrate_v1_backs_up_legacy_before_writing_v2() {
        // GOLDEN (F8): the legacy file is renamed to its
        // `.pre-v2-backup` BEFORE the v2 file is written. After a
        // successful migration: the v2 file exists, the legacy file
        // is gone, and the backup holds the original bytes — so a
        // crash at any point leaves exactly one of {v1, v2}, never
        // both, under a canonical name.
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");
        std::fs::write(&v1_path, V1_FIXTURE).unwrap();

        let store = AgentStore::open_at(v2_path.clone()).unwrap();
        assert_eq!(store.list().len(), 1);

        assert!(v2_path.exists(), "v2 file written");
        assert!(!v1_path.exists(), "legacy file renamed away");
        let backup = dir.path().join("automations.json.pre-v2-backup");
        assert!(backup.exists(), "legacy file preserved as backup");
        // The backup carries the original v1 bytes verbatim.
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            V1_FIXTURE,
            "backup must hold the original legacy bytes"
        );
    }

    #[test]
    fn migrate_v1_concurrent_open_is_serialized_by_the_lock() {
        // GOLDEN (F8): two `open_at` calls in sequence on the same
        // path — the second loads the migrated v2 file, never
        // re-runs the migration. The advisory lock guarantees that
        // even a concurrent CLI + GUI first-boot serializes; here we
        // exercise the deterministic sequential equivalent (the lock
        // is released at each `drop`).
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");
        std::fs::write(&v1_path, V1_FIXTURE).unwrap();

        {
            let first = AgentStore::open_at(v2_path.clone()).unwrap();
            assert_eq!(first.list().len(), 1);
        }
        // Second open: the legacy file is gone, the v2 file loads
        // directly, the record count is unchanged (no double-add).
        let second = AgentStore::open_at(v2_path.clone()).unwrap();
        assert_eq!(second.list().len(), 1);
        assert_eq!(second.list()[0].name, "morning-pr");
        assert!(!v1_path.exists());
    }

    // ---- grill F15: store / scheduler reconciliation ----

    #[test]
    fn reconcile_flags_installed_cron_agent_with_no_artifact() {
        // GOLDEN (F15): an `Installed`, enabled, cron-triggered agent
        // whose id is absent from the scheduler's reported artifacts
        // is an orphan — it claims to be armed but nothing fires it.
        let mut a = sample("ghost-agent"); // sample() => Installed, Cron
        a.id = Uuid::nil();
        let orphans = reconcile_installed_agents(
            std::slice::from_ref(&a),
            &std::collections::HashSet::new(), // scheduler reports nothing
            |id| format!("io.claudepot.agent.{id}"),
        );
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].name, "ghost-agent");
    }

    #[test]
    fn reconcile_does_not_flag_an_agent_with_a_live_artifact() {
        let mut a = sample("healthy-agent");
        a.id = Uuid::nil();
        let mut registered = std::collections::HashSet::new();
        registered.insert(format!("io.claudepot.agent.{}", a.id));
        let orphans = reconcile_installed_agents(std::slice::from_ref(&a), &registered, |id| {
            format!("io.claudepot.agent.{id}")
        });
        assert!(orphans.is_empty(), "an agent with a live artifact is fine");
    }

    #[test]
    fn reconcile_ignores_draft_manual_and_disabled_agents() {
        // A draft has no artifact by design; a Manual/Event agent
        // legitimately has none even when Installed; a disabled
        // Installed agent is unregistered by design. None are
        // orphans.
        let mut draft = sample("a-draft");
        draft.lifecycle = Lifecycle::Draft;

        let mut manual = sample("a-manual");
        manual.trigger = Trigger::Manual;

        let mut disabled = sample("a-disabled");
        disabled.enabled = false;

        let agents = vec![draft, manual, disabled];
        let orphans =
            reconcile_installed_agents(&agents, &std::collections::HashSet::new(), |id| {
                format!("io.claudepot.agent.{id}")
            });
        assert!(
            orphans.is_empty(),
            "draft / manual / disabled agents are never reconciliation orphans"
        );
    }

    // ---- grill X9: reverse-direction reconciliation ----

    fn registered(identifier: &str, managed: bool) -> super::super::scheduler::RegisteredEntry {
        super::super::scheduler::RegisteredEntry {
            identifier: identifier.to_string(),
            claudepot_managed: managed,
        }
    }

    #[test]
    fn reverse_reconcile_flags_managed_artifact_with_no_installed_record() {
        // GOLDEN (X9): the scheduler reports a managed artifact, but
        // the store has no `Installed` record whose
        // `expected_identifier` matches. The artifact will fire on
        // schedule with no visible record — exactly the hand-edit
        // hazard X9 exists to surface.
        let installed: std::collections::HashSet<String> = ["io.claudepot.agent.alive".to_string()]
            .into_iter()
            .collect();
        let reported = vec![
            registered("io.claudepot.agent.alive", true),
            registered("io.claudepot.agent.orphan", true),
        ];
        let orphans = reconcile_orphan_artifacts(&installed, &reported);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].identifier, "io.claudepot.agent.orphan");
    }

    #[test]
    fn reverse_reconcile_ignores_foreign_unmanaged_artifacts() {
        // The host can carry plenty of launchd / systemd entries we
        // never installed. The `claudepot_managed` filter must keep
        // them out of the orphan list — otherwise every machine
        // would light up with false positives at boot.
        let installed: std::collections::HashSet<String> = std::collections::HashSet::new();
        let reported = vec![
            registered("com.apple.something.else", false),
            registered("io.claudepot.agent.foo", false),
        ];
        let orphans = reconcile_orphan_artifacts(&installed, &reported);
        assert!(
            orphans.is_empty(),
            "unmanaged entries must never be flagged"
        );
    }

    #[test]
    fn reverse_reconcile_empty_when_every_artifact_has_a_record() {
        let installed: std::collections::HashSet<String> = [
            "io.claudepot.agent.a".to_string(),
            "io.claudepot.agent.b".to_string(),
        ]
        .into_iter()
        .collect();
        let reported = vec![
            registered("io.claudepot.agent.a", true),
            registered("io.claudepot.agent.b", true),
        ];
        let orphans = reconcile_orphan_artifacts(&installed, &reported);
        assert!(orphans.is_empty());
    }

    #[test]
    fn reverse_reconcile_empty_when_nothing_registered() {
        let installed: std::collections::HashSet<String> = std::collections::HashSet::new();
        let orphans = reconcile_orphan_artifacts(&installed, &[]);
        assert!(orphans.is_empty(), "empty inputs cannot produce orphans");
    }

    #[test]
    fn open_at_prefers_v2_and_ignores_a_leftover_legacy_file() {
        // GOLDEN (F6d): crash-mid-migration simulation. Both
        // `agents.json` (v2) and a leftover `automations.json` (v1)
        // are on disk — e.g. the process died after the v2 write but
        // before the legacy backup rename. `open_at` must load the
        // v2 file and IGNORE the legacy file entirely; it must never
        // re-run the migration and clobber the newer v2 data.
        let dir = tempdir().unwrap();
        let v2_path = dir.path().join("agents.json");
        let v1_path = dir.path().join("automations.json");

        // The v2 file holds the authoritative record.
        let mut seed = AgentStore::open_at(v2_path.clone()).unwrap();
        let mut a = sample("post-crash-agent");
        a.prompt = "the v2 truth".into();
        let id = a.id;
        seed.add(a).unwrap();
        seed.save().unwrap();
        // Drop the seed store so the lock releases before the second
        // open below (F7 lock is exclusive; see lock.rs's contract
        // for why two same-thread holders deadlock).
        drop(seed);

        // A stale v1 file is *also* present, with a different agent.
        std::fs::write(&v1_path, V1_FIXTURE).unwrap();

        let store = AgentStore::open_at(v2_path.clone()).unwrap();
        // Exactly the v2 record — the legacy `morning-pr` is ignored.
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get(&id).unwrap().prompt, "the v2 truth");
        assert!(store.get_by_name("morning-pr").is_none());
        // The legacy file is left exactly as-is — the migration did
        // not run, so it was not backed up or consumed.
        assert!(v1_path.exists());
        assert!(!dir.path().join("automations.json.pre-v2-backup").exists());
    }

    // ---- grill X29: outer reconcile_with_scheduler wiring ----
    //
    // The pure inner predicates are well-covered above. The outer
    // wired functions add: open store + ask scheduler + log. The
    // `_using` variants expose the scheduler seam so we can drive
    // the wiring end-to-end with a `FakeScheduler` against a
    // temp `CLAUDEPOT_DATA_DIR`.

    use std::cell::RefCell;
    use std::sync::Mutex;

    /// Tests that set `CLAUDEPOT_DATA_DIR` share process env state;
    /// serialize them so they don't see each other's stores.
    static RECONCILE_ENV_GUARD: Mutex<()> = Mutex::new(());

    /// Scheduler stub for the X29 wiring tests. Returns the
    /// `list_managed` payload the test asks for, and synthesizes
    /// `expected_identifier` so the orphan check has something to
    /// match against.
    struct ScriptedScheduler {
        managed: RefCell<Vec<super::super::scheduler::RegisteredEntry>>,
        list_managed_error: bool,
    }

    impl ScriptedScheduler {
        fn with(entries: Vec<super::super::scheduler::RegisteredEntry>) -> Self {
            Self {
                managed: RefCell::new(entries),
                list_managed_error: false,
            }
        }
        fn list_managed_errors() -> Self {
            Self {
                managed: RefCell::new(Vec::new()),
                list_managed_error: true,
            }
        }
    }

    impl super::super::scheduler::Scheduler for ScriptedScheduler {
        fn register(&self, _agent: &Agent) -> Result<(), AgentError> {
            Ok(())
        }
        fn unregister(&self, _id: &AgentId) -> Result<(), AgentError> {
            Ok(())
        }
        fn kickstart(&self, _id: &AgentId) -> Result<(), AgentError> {
            Ok(())
        }
        fn list_managed(
            &self,
        ) -> Result<Vec<super::super::scheduler::RegisteredEntry>, AgentError> {
            if self.list_managed_error {
                return Err(AgentError::UnsupportedPlatform(
                    "scripted: list_managed forced to fail",
                ));
            }
            Ok(self.managed.borrow().clone())
        }
        fn expected_identifier(&self, id: &AgentId) -> String {
            format!("scripted.agent.{id}")
        }
        fn next_runs(
            &self,
            _trigger: &Trigger,
            _from: chrono::DateTime<Utc>,
            _n: usize,
        ) -> Result<Vec<chrono::DateTime<Utc>>, AgentError> {
            Ok(Vec::new())
        }
        fn capabilities(&self) -> super::super::scheduler::SchedulerCapabilities {
            super::super::scheduler::SchedulerCapabilities {
                wake_to_run: false,
                catch_up_if_missed: false,
                run_when_logged_out: false,
                native_label: "scripted",
                artifact_dir: None,
            }
        }
    }

    /// Seed an `Installed` cron agent at the given data dir. Returns
    /// the agent id so callers can build the scheduler payload.
    fn seed_installed_cron_agent(data_dir: &Path, name: &str) -> AgentId {
        // The data dir override is set by the caller; build the
        // store at the canonical path inside it so `AgentStore::open`
        // (called by `_using`) finds it.
        let store_path = data_dir.join("agents.json");
        let mut store = AgentStore::open_at(store_path).unwrap();
        let mut a = sample(name);
        a.lifecycle = Lifecycle::Installed;
        let id = a.id;
        store.add(a).unwrap();
        store.save().unwrap();
        drop(store);
        id
    }

    #[test]
    fn reconcile_with_scheduler_using_flags_an_installed_agent_with_no_artifact() {
        let _lock = RECONCILE_ENV_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());

        let _id = seed_installed_cron_agent(dir.path(), "ghost-cron");
        // Scheduler reports nothing — every Installed agent is an
        // orphan from the `Installed → artifact` direction.
        let sched = ScriptedScheduler::with(Vec::new());

        let orphans = reconcile_with_scheduler_using(&sched);
        assert_eq!(orphans.len(), 1, "wired reconcile flags the orphan");
        assert_eq!(orphans[0].name, "ghost-cron");

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn reconcile_with_scheduler_using_silent_when_artifact_present() {
        let _lock = RECONCILE_ENV_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());

        let id = seed_installed_cron_agent(dir.path(), "healthy-cron");
        // Scheduler reports the expected identifier — no orphan.
        let entry = super::super::scheduler::RegisteredEntry {
            identifier: format!("scripted.agent.{id}"),
            claudepot_managed: true,
        };
        let sched = ScriptedScheduler::with(vec![entry]);

        let orphans = reconcile_with_scheduler_using(&sched);
        assert!(
            orphans.is_empty(),
            "wired reconcile is quiet on healthy state"
        );

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn reconcile_with_scheduler_using_returns_empty_on_list_managed_error() {
        let _lock = RECONCILE_ENV_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());

        let _id = seed_installed_cron_agent(dir.path(), "would-be-orphan");
        let sched = ScriptedScheduler::list_managed_errors();

        // `list_managed` failing must NOT promote every agent to
        // orphan — that would mean a transient scheduler hiccup
        // disarms every cron agent's audit signal. The wired
        // function swallows + logs the error and returns empty.
        let orphans = reconcile_with_scheduler_using(&sched);
        assert!(
            orphans.is_empty(),
            "a transient scheduler failure must not be reported as orphans"
        );

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn reconcile_orphan_artifacts_using_flags_unmatched_managed_artifact() {
        let _lock = RECONCILE_ENV_GUARD
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());

        let id = seed_installed_cron_agent(dir.path(), "alive");
        // Scheduler reports the alive agent's artifact AND an extra
        // unmatched managed artifact — the latter is an orphan.
        let entries = vec![
            super::super::scheduler::RegisteredEntry {
                identifier: format!("scripted.agent.{id}"),
                claudepot_managed: true,
            },
            super::super::scheduler::RegisteredEntry {
                identifier: "scripted.agent.ghost".to_string(),
                claudepot_managed: true,
            },
        ];
        let sched = ScriptedScheduler::with(entries);

        let orphans = reconcile_orphan_artifacts_using(&sched);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].identifier, "scripted.agent.ghost");

        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }
}
