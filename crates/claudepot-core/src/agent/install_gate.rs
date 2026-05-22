//! The draft → install gate, as a pure, testable core helper.
//!
//! [`install_draft`] is the engine behind the GUI's `agent_install`
//! Tauri command (PRD §8.2 / D8 — the human-only half of the
//! draft/install gate). The Tauri command is a thin wrapper: it
//! resolves the binary path, builds the real shim-installer closure
//! and the OS scheduler, then calls this. The helper itself never
//! touches the webview and takes the scheduler as a `&dyn Scheduler`
//! plus the shim-install step as an injected closure, so the whole
//! arm → install-shim → register → save ordering — and both of its
//! rollback directions — is unit-testable without a real launchd /
//! systemd / Task Scheduler artifact.
//!
//! ## Ordering and rollback
//!
//! The ordering is **arm (in memory) → install shim → save →
//! register** (grill finding F10 inverts the earlier
//! save-after-register order). `arm` is a pure in-memory
//! `lifecycle` flip; the OS scheduler artifact is the *last* thing
//! materialized, so every failure leaves a coherent on-disk state:
//!
//! - **shim-install failure** → the store is *not* saved; the agent
//!   on disk is still an inert `Draft`. The in-memory `lifecycle` is
//!   rolled back to `Draft` so a reused store object stays honest.
//! - **save failure** → store unsaved, lifecycle rolled back in
//!   memory, agent on disk still `Draft`. No scheduler artifact was
//!   registered (register runs only after a clean save). A draft
//!   with no artifact is the safe failure state.
//! - **register failure** → the `Installed` flip is already on disk,
//!   so the lifecycle is rolled back to `Draft` *and re-saved*. The
//!   agent ends as a `Draft` with no artifact — the same safe state
//!   as the other two failures.
//!
//! The previous order (register before save) leaked the *harmful*
//! direction: a save failure after a successful register left an
//! armed launchd / systemd / Task Scheduler artifact firing
//! `claude -p` on schedule behind an on-disk `Draft` record — an
//! invisible firing draft. Saving the `Installed` flip first means
//! the only artifact that can ever exist is one whose record is
//! already (or, after a register-failure rollback, again) coherent.

use super::error::AgentError;
use super::scheduler::Scheduler;
use super::store::AgentStore;
use super::types::{Agent, AgentId, Lifecycle};

/// Outcome of a successful [`install_draft`]: the now-armed agent.
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    /// The agent after arming — `lifecycle == Installed`.
    pub agent: Agent,
}

/// Arm a draft agent and materialize its scheduler artifact.
///
/// `install_shim` is the (impure, disk-touching) shim-render step,
/// injected so tests can supply a fake. `scheduler` registers the OS
/// artifact for an *enabled* agent — a disabled draft is armed but
/// not scheduled, exactly as `agents_add` treats a disabled agent.
///
/// Ordering is **arm → install shim → save → register** (grill F10).
/// Every failure leaves the agent as an artifact-free `Draft`:
///
/// - a shim or save failure rolls the in-memory `lifecycle` back to
///   `Draft` and leaves the store unsaved;
/// - a register failure rolls the `lifecycle` back to `Draft` and
///   **re-saves**, because the `Installed` flip is already on disk.
///
/// A `Draft` with no scheduler artifact is the safe failure state —
/// it is invisible to both the OS scheduler and the event
/// orchestrator until a human re-arms it.
pub fn install_draft<F>(
    store: &mut AgentStore,
    id: &AgentId,
    scheduler: &dyn Scheduler,
    mut install_shim: F,
) -> Result<InstallOutcome, AgentError>
where
    F: FnMut(&Agent) -> Result<(), AgentError>,
{
    // `arm` rejects an already-installed agent and returns the armed
    // (Installed) clone. The store mutation is in-memory only until
    // `save` below.
    let armed = store.arm(id)?;

    // Materialize the shim. On failure, undo the in-memory arm and
    // leave the store unsaved — the agent on disk is still a Draft.
    if let Err(e) = install_shim(&armed) {
        rollback_to_draft(store, id, false);
        return Err(e);
    }

    // Persist the Draft -> Installed flip BEFORE registering the OS
    // artifact (grill F10). A save failure here rolls back in memory
    // and leaves the store unsaved; no artifact has been registered
    // yet, so the on-disk record stays a coherent `Draft`.
    if let Err(e) = store.save() {
        rollback_to_draft(store, id, false);
        return Err(e);
    }

    // Only an enabled agent registers a live scheduler artifact —
    // same rule as `agents_add`. A disabled draft is armed but not
    // scheduled until the user enables it.
    if armed.enabled {
        if let Err(e) = scheduler.register(&armed) {
            // The `Installed` flip is already on disk. Roll the
            // lifecycle back to `Draft` AND re-save so the record
            // never claims `Installed` without a live artifact. A
            // draft with no artifact is the safe failure state.
            rollback_to_draft(store, id, true);
            return Err(e);
        }
    }

    Ok(InstallOutcome { agent: armed })
}

/// Flip an in-memory `Installed` record back to `Draft` after a
/// failed install step, so the store is not left claiming an agent
/// is installed when its scheduler artifact never materialized.
///
/// `persist` controls whether the rollback is also written to disk:
/// pass `true` only when the `Installed` flip already reached disk
/// (the register-failure path), so the on-disk record is corrected
/// too. A failed re-save is logged but not propagated — the caller
/// is already returning the original install error, and surfacing a
/// second error would mask it; the in-memory rollback still keeps a
/// reused [`AgentStore`] honest. Silent if the id vanished — there
/// is nothing to roll back.
fn rollback_to_draft(store: &mut AgentStore, id: &AgentId, persist: bool) {
    store.set_lifecycle(id, Lifecycle::Draft);
    if persist {
        if let Err(e) = store.save() {
            tracing::error!(
                agent_id = %id,
                error = %e,
                "install_draft: register failed and the rollback re-save \
                 ALSO failed — the on-disk record still claims Installed \
                 with no live scheduler artifact; the next boot \
                 reconciliation will demote it"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::scheduler::{
        cron_next_runs, RegisteredEntry, SchedulerCapabilities,
    };
    use crate::agent::types::*;
    use chrono::{DateTime, Utc};
    use std::cell::RefCell;
    use tempfile::tempdir;
    use uuid::Uuid;

    /// A `Scheduler` fake whose `register` outcome the test picks.
    struct FakeScheduler {
        register_fails: bool,
        registered: RefCell<Vec<AgentId>>,
    }

    impl FakeScheduler {
        fn ok() -> Self {
            Self {
                register_fails: false,
                registered: RefCell::new(Vec::new()),
            }
        }
        fn failing() -> Self {
            Self {
                register_fails: true,
                registered: RefCell::new(Vec::new()),
            }
        }
    }

    impl Scheduler for FakeScheduler {
        fn register(&self, agent: &Agent) -> Result<(), AgentError> {
            if self.register_fails {
                return Err(AgentError::UnsupportedPlatform(
                    "fake scheduler: register forced to fail",
                ));
            }
            self.registered.borrow_mut().push(agent.id);
            Ok(())
        }
        fn unregister(&self, _id: &AgentId) -> Result<(), AgentError> {
            Ok(())
        }
        fn kickstart(&self, _id: &AgentId) -> Result<(), AgentError> {
            Ok(())
        }
        fn list_managed(&self) -> Result<Vec<RegisteredEntry>, AgentError> {
            Ok(Vec::new())
        }
        fn expected_identifier(&self, id: &AgentId) -> String {
            format!("fake.agent.{id}")
        }
        fn next_runs(
            &self,
            trigger: &Trigger,
            from: DateTime<Utc>,
            n: usize,
        ) -> Result<Vec<DateTime<Utc>>, AgentError> {
            match trigger {
                Trigger::Cron { cron, .. } => cron_next_runs(cron, from, n),
                Trigger::Manual | Trigger::Event { .. } => Ok(Vec::new()),
            }
        }
        fn capabilities(&self) -> SchedulerCapabilities {
            SchedulerCapabilities {
                wake_to_run: false,
                catch_up_if_missed: false,
                run_when_logged_out: false,
                native_label: "fake",
                artifact_dir: None,
            }
        }
    }

    fn draft_agent(name: &str, enabled: bool) -> Agent {
        let now = Utc::now();
        Agent {
            id: Uuid::new_v4(),
            name: name.into(),
            display_name: None,
            description: None,
            enabled,
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
            lifecycle: Lifecycle::Draft,
            drafted_by: Some("claude-code@test".into()),
        }
    }

    #[test]
    fn install_draft_happy_path_arms_registers_and_saves() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        let agent = draft_agent("morning-pr", true);
        let id = agent.id;
        store.add(agent).unwrap();
        store.save().unwrap();

        let sched = FakeScheduler::ok();
        let mut shim_calls = 0;
        let outcome = install_draft(&mut store, &id, &sched, |_a| {
            shim_calls += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(outcome.agent.lifecycle, Lifecycle::Installed);
        assert_eq!(shim_calls, 1);
        assert_eq!(sched.registered.borrow().len(), 1);
        // In memory: installed.
        assert_eq!(store.get(&id).unwrap().lifecycle, Lifecycle::Installed);
        // On disk: installed (save ran). Drop the store first so the
        // reopen does not block on the advisory lock.
        drop(store);
        let reopened = AgentStore::open_at(path).unwrap();
        assert_eq!(reopened.get(&id).unwrap().lifecycle, Lifecycle::Installed);
    }

    #[test]
    fn install_draft_register_failure_rolls_back_to_draft_on_disk() {
        // grill F10: with save-before-register ordering, the
        // `Installed` flip reaches disk BEFORE register runs. A
        // register failure must therefore roll the lifecycle back to
        // `Draft` AND re-save, so the on-disk record never claims
        // `Installed` behind a missing scheduler artifact.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        let agent = draft_agent("evening-digest", true);
        let id = agent.id;
        store.add(agent).unwrap();
        store.save().unwrap();

        let sched = FakeScheduler::failing();
        let result = install_draft(&mut store, &id, &sched, |_a| Ok(()));
        assert!(result.is_err(), "register failure must surface as Err");

        // In memory: rolled back to Draft.
        assert_eq!(store.get(&id).unwrap().lifecycle, Lifecycle::Draft);
        // On disk: ALSO Draft — the rollback re-saved. The previous
        // (pre-F10) order would have left an `Installed` record here
        // with no live artifact.
        drop(store);
        let reopened = AgentStore::open_at(path).unwrap();
        assert_eq!(reopened.get(&id).unwrap().lifecycle, Lifecycle::Draft);
        // No artifact was registered.
        assert!(sched.registered.borrow().is_empty());
    }

    #[test]
    fn install_draft_shim_failure_leaves_agent_draft_unsaved() {
        // grill F6: a shim-install failure (before register) is the
        // earliest rollback point — same invariant.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        let agent = draft_agent("noon-report", true);
        let id = agent.id;
        store.add(agent).unwrap();
        store.save().unwrap();

        let sched = FakeScheduler::ok();
        let result = install_draft(&mut store, &id, &sched, |_a| {
            Err(AgentError::InvalidPath(
                "/auto".into(),
                "fake shim render failed",
            ))
        });
        assert!(result.is_err());

        assert_eq!(store.get(&id).unwrap().lifecycle, Lifecycle::Draft);
        drop(store);
        let reopened = AgentStore::open_at(path).unwrap();
        assert_eq!(reopened.get(&id).unwrap().lifecycle, Lifecycle::Draft);
        // Register was never reached.
        assert!(sched.registered.borrow().is_empty());
    }

    #[test]
    fn install_draft_disabled_agent_arms_without_registering() {
        // A disabled draft arms (lifecycle flips, save runs) but no
        // scheduler artifact is materialized — same as `agents_add`.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        let agent = draft_agent("paused-agent", false);
        let id = agent.id;
        store.add(agent).unwrap();
        store.save().unwrap();

        // Even a *failing* scheduler is fine: a disabled agent never
        // calls register.
        let sched = FakeScheduler::failing();
        let outcome = install_draft(&mut store, &id, &sched, |_a| Ok(())).unwrap();
        assert_eq!(outcome.agent.lifecycle, Lifecycle::Installed);
        assert!(sched.registered.borrow().is_empty());

        drop(store);
        let reopened = AgentStore::open_at(path).unwrap();
        assert_eq!(reopened.get(&id).unwrap().lifecycle, Lifecycle::Installed);
    }

    #[test]
    fn install_draft_save_failure_surfaces_as_err() {
        // grill F6: the save-failure path is exercised. We force a
        // save failure by pointing the store at a path whose parent
        // is a *file*, so `create_dir_all` in `save` fails.
        let dir = tempdir().unwrap();
        // `blocker` is a regular file; using it as a directory
        // component makes `create_dir_all` fail.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let path = blocker.join("nested").join("agents.json");

        // Seed an in-memory store directly (open_at would itself
        // fail on this path). Build it via a writable temp first,
        // then re-point — instead, construct through a sibling dir.
        let seed_path = dir.path().join("seed.json");
        let mut store = AgentStore::open_at(seed_path).unwrap();
        let agent = draft_agent("brittle-agent", true);
        let id = agent.id;
        store.add(agent).unwrap();

        // Swap the store's path to the un-creatable one so `save`
        // fails. `set_path` exists for exactly this kind of test
        // seam.
        store.set_path(path);

        let sched = FakeScheduler::ok();
        let result = install_draft(&mut store, &id, &sched, |_a| Ok(()));
        assert!(
            result.is_err(),
            "a failed save must surface as Err, not be swallowed"
        );
        // grill F10: with save-before-register ordering, a save
        // failure happens BEFORE any scheduler artifact is
        // registered. The lifecycle is rolled back to `Draft` and
        // no artifact exists — the safe failure state. The register
        // step is never reached.
        assert_eq!(store.get(&id).unwrap().lifecycle, Lifecycle::Draft);
        assert!(
            sched.registered.borrow().is_empty(),
            "register must not run after a save failure"
        );
    }
}
