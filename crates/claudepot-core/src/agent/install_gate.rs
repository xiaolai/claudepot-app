//! The draft → install gate and the shared install-ordering helper,
//! as pure, testable core helpers.
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
//! [`apply_lifecycle_change`] hoists the same critical section out
//! of the four parallel GUI Tauri commands (`agents_add`,
//! `agents_update`, `agent_install`, `agents_set_enabled`) — grill
//! finding X2. Before X2 each verb re-implemented the
//! mutate → save → register sequence with **three different rollback
//! shapes**, and two of them (`agents_update` and
//! `agents_set_enabled`) had no rollback at all — reintroducing the
//! F10 orphan that `install_draft` had already closed. The X2 fix
//! is one helper; every verb delegates. The Draft-rejection gate
//! (X1) lives inside the helper, so every enabling verb enforces
//! it, not just `agents_set_enabled`.
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
    install_shim: F,
) -> Result<InstallOutcome, AgentError>
where
    F: FnOnce(&Agent) -> Result<(), AgentError>,
{
    // Snapshot the original `Draft` lifecycle so the rollback closure
    // restores exactly that state — the helper sees only "an in-memory
    // mutation that needs materializing" and a way to undo it.
    let original_id = *id;
    let agent = apply_lifecycle_change(
        store,
        id,
        // `mutate`: flip Draft → Installed in memory. `store.arm`
        // already rejects an already-installed agent; the helper's
        // Draft-rejection gate (X1) is irrelevant here because the
        // post-mutation record is `Installed`.
        |store| store.arm(&original_id),
        // `rollback`: restore the in-memory `Draft` flip. The helper
        // decides whether to also persist this rollback (it does iff
        // the failure happens after the `save`).
        |store| store.set_lifecycle(&original_id, Lifecycle::Draft),
        // `install_shim`: caller-supplied.
        install_shim,
        scheduler,
    )?;
    Ok(InstallOutcome { agent })
}

/// The single critical section behind every GUI verb that mutates
/// `agents.json` in a way that may materialize an OS scheduler
/// artifact — `agents_add`, `agents_update`, `agent_install`, and
/// `agents_set_enabled` (grill finding X2).
///
/// Each verb supplies:
///
/// - `mutate(&mut store)` — the in-memory store mutation (insert a
///   new record, apply a patch, flip lifecycle, etc.), returning the
///   post-mutation [`Agent`] clone. Anything that wants to leave the
///   record visible on a successful save belongs here.
/// - `rollback(&mut store)` — the inverse of `mutate`. Run when a
///   later step (shim render, save, register) fails so the store
///   never claims a record exists/installed/enabled when the
///   artifact it implies never materialized. **MUST NOT** touch the
///   filesystem; the helper decides whether to re-save the rolled-
///   back state (it does iff the save already succeeded).
/// - `install_shim(&Agent)` — the (impure, disk-touching) shim
///   render. Injected so tests can supply a fake.
/// - `scheduler` — the OS scheduler adapter; only consulted when the
///   post-mutation record is `enabled`.
///
/// ## Invariants
///
/// 1. **Draft never acquires an artifact** (X1). Before any
///    `install_shim` / scheduler call, the post-mutation record's
///    lifecycle is checked. If it is `Draft` AND the agent is
///    `enabled` — i.e. this mutation would materialize a scheduler
///    artifact for a draft — the helper rolls back and errors. The
///    Draft → Installed transition itself (the legitimate path,
///    used by [`install_draft`]) is the *output* of `mutate`, so it
///    arrives here already flipped to `Installed` and passes.
/// 2. **`register` runs only on success-with-enabled** (F10): we
///    `mutate` in memory → render the shim → `save` → `register`.
///    Every failure direction rolls back to the pre-mutation state.
///    A `Draft`-with-no-artifact (and a Disabled-with-no-artifact)
///    is the safe failure shape.
/// 3. **`unregister` for the disabled-or-removed direction is best-
///    effort.** Failing to unregister is logged but does not roll
///    back the in-store mutation — leaving a stale artifact behind
///    is the same hazard `reconcile_with_scheduler` was built to
///    surface, and refusing the mutation here would block the user
///    from disabling/removing an agent because of a transient
///    scheduler hiccup.
pub fn apply_lifecycle_change<M, R, F>(
    store: &mut AgentStore,
    id: &AgentId,
    mutate: M,
    rollback: R,
    install_shim: F,
    scheduler: &dyn Scheduler,
) -> Result<Agent, AgentError>
where
    M: FnOnce(&mut AgentStore) -> Result<Agent, AgentError>,
    R: FnOnce(&mut AgentStore),
    F: FnOnce(&Agent) -> Result<(), AgentError>,
{
    // grill X24: snapshot the pre-mutation `updated_at` so a
    // rolled-back failed install does not leave the agent claiming
    // it was just edited.
    //
    // - `arm` and `update` both bump `updated_at = now()` inside
    //   the mutate closure.
    // - The verb-supplied rollback closure restores every other
    //   field but does NOT know to revert `updated_at`:
    //     * `agents_update`'s rollback is `remove + add(prior)` —
    //       `add` does not bump `updated_at`, so the prior value
    //       (from the verb's snapshot) is restored. ✓
    //     * `agents_set_enabled`'s rollback calls `update` again
    //       with the old enabled bit — `update` bumps `updated_at`
    //       a second time. ✗ The UI then shows "updated 3 seconds
    //       ago" after a failure.
    //     * `install_draft`'s rollback calls `set_lifecycle` which
    //       does NOT bump — but the earlier `arm` call (inside
    //       `mutate`) did. ✗ Same shape.
    //     * `agents_add`'s rollback removes the record; there is
    //       nothing to restore. ✓
    //
    // We capture the pre-mutation timestamp here, run mutate, and
    // — if any post-mutation step rolls back — restore the snapshot
    // value via `store.set_updated_at`. The capture is `None` when
    // the agent didn't exist pre-mutation (the `agents_add` shape);
    // restoring `None` is a no-op.
    let pre_updated_at = store.get(id).map(|a| a.updated_at);

    // Step 1 — in-memory mutation. A failure here means nothing has
    // changed yet; surface it as-is.
    let post = mutate(store)?;

    // Step 2 — X1 gate. A `Draft` agent must NEVER acquire a live
    // scheduler artifact. The legitimate Draft → Installed
    // transition (`install_draft`) flips the lifecycle *inside*
    // `mutate`, so by the time we get here the post-mutation record
    // is already `Installed`. A `Draft` arriving here means a verb
    // (likely `agents_set_enabled(true)` or an `agents_update` that
    // toggles enabled on a draft) is trying to bypass the install
    // review gate.
    //
    // We only refuse when the draft is also `enabled` — a
    // disabled-Draft mutation has no scheduler consequence and is
    // harmless. Tighten if a future invariant demands.
    if post.lifecycle == Lifecycle::Draft && post.enabled {
        let name = post.name.clone();
        rollback(store);
        // X24: also restore the pre-mutation `updated_at` so a
        // rejected draft mutation does not leave the agent
        // claiming it was just edited.
        if let Some(ts) = pre_updated_at {
            store.set_updated_at(id, ts);
        }
        return Err(AgentError::InvalidEnv(format!(
            "agent '{name}' is a draft — review and install it before \
             enabling. A draft cannot acquire a scheduler artifact via \
             this verb; only the install-review flow may arm it."
        )));
    }

    // Step 3 — shim render. The shim is the per-agent `.sh`/`.cmd`
    // file the scheduler artifact ultimately invokes; without it,
    // a register that succeeded would be loading a label that
    // points at a missing executable. Render before save so a shim
    // failure unwinds with no on-disk change.
    if post.enabled {
        if let Err(e) = install_shim(&post) {
            rollback(store);
            if let Some(ts) = pre_updated_at {
                store.set_updated_at(id, ts);
            }
            return Err(e);
        }
    }

    // Step 4 — persist BEFORE registering the OS artifact (F10). A
    // save failure here unwinds in memory; no artifact has been
    // registered, so the on-disk record is whatever it was before
    // `mutate` ran.
    if let Err(e) = store.save() {
        rollback(store);
        if let Some(ts) = pre_updated_at {
            store.set_updated_at(id, ts);
        }
        return Err(e);
    }

    // Step 5 — register / unregister.
    if post.enabled {
        if let Err(e) = scheduler.register(&post) {
            // The mutation IS on disk. Roll the in-memory state
            // back AND re-save so the on-disk record never claims
            // a live artifact that never materialized. A failed
            // re-save is logged but not propagated — surfacing two
            // errors would mask the original `register` failure.
            rollback(store);
            if let Some(ts) = pre_updated_at {
                store.set_updated_at(id, ts);
            }
            if let Err(save_err) = store.save() {
                tracing::error!(
                    agent_id = %id,
                    error = %save_err,
                    "apply_lifecycle_change: register failed and the \
                     rollback re-save ALSO failed — the on-disk \
                     record now claims a state with no live \
                     scheduler artifact; the next boot reconciliation \
                     will surface it"
                );
            }
            return Err(e);
        }
    } else {
        // Disabled (or just-disabled) record: best-effort unregister
        // so a previously-registered artifact does not outlive the
        // record's enabled bit. Failures are logged, not propagated
        // — see the helper's doc-comment for the rationale.
        if let Err(e) = scheduler.unregister(id) {
            tracing::warn!(
                agent_id = %id,
                error = %e,
                "apply_lifecycle_change: scheduler unregister failed \
                 on the disabled path; the record is saved but a stale \
                 artifact may remain — reconcile_with_scheduler will \
                 surface it"
            );
        }
    }

    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::scheduler::{cron_next_runs, RegisteredEntry, SchedulerCapabilities};
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
            created_via: crate::agent::types::CreatedVia::CliDraft,
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

    // ---- grill X2: apply_lifecycle_change ----------------------
    //
    // The shared helper is the single critical section behind the
    // four GUI verbs (`agents_add`, `agents_update`, `agent_install`,
    // `agents_set_enabled`). Each of these tests exercises one of
    // its four failure shapes plus the X1 Draft gate.

    /// Build an already-`Installed` sample agent for the
    /// apply-change tests, distinct from `draft_agent` which builds
    /// a `Draft`.
    fn installed_agent(name: &str, enabled: bool) -> Agent {
        let mut a = draft_agent(name, enabled);
        a.lifecycle = Lifecycle::Installed;
        a
    }

    #[test]
    fn apply_change_add_happy_path_inserts_saves_and_registers() {
        // Models `agents_add`: `mutate` inserts a brand-new record;
        // rollback removes it; helper renders shim, saves, registers.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        let agent = installed_agent("morning-pr", true);
        let id = agent.id;

        let sched = FakeScheduler::ok();
        let mut shim_calls = 0;
        let agent_for_mutate = agent.clone();
        let id_for_mutate = id;
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                store.add(agent_for_mutate.clone())?;
                store
                    .get(&id_for_mutate)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id_for_mutate.to_string()))
            },
            move |store| {
                let _ = store.remove(&id_for_mutate);
            },
            |_a| {
                shim_calls += 1;
                Ok(())
            },
            &sched,
        );

        let post = result.expect("add happy path must succeed");
        assert_eq!(post.lifecycle, Lifecycle::Installed);
        assert_eq!(shim_calls, 1);
        assert_eq!(sched.registered.borrow().len(), 1);
        assert!(store.get(&id).is_some(), "the record is in the store");
        // On disk: persisted.
        drop(store);
        let reopened = AgentStore::open_at(path).unwrap();
        assert!(reopened.get(&id).is_some());
    }

    #[test]
    fn apply_change_register_failure_rolls_back_add_on_disk() {
        // Models `agents_add` with a failing scheduler: the helper
        // must roll the just-inserted record back AND re-save so the
        // store on disk does not retain a phantom record.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        let agent = installed_agent("ghost", true);
        let id = agent.id;

        let sched = FakeScheduler::failing();
        let agent_for_mutate = agent.clone();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                store.add(agent_for_mutate.clone())?;
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            move |store| {
                let _ = store.remove(&id);
            },
            |_a| Ok(()),
            &sched,
        );
        assert!(result.is_err(), "register failure must surface as Err");

        // In-memory: rolled back (record removed).
        assert!(
            store.get(&id).is_none(),
            "the helper rolled the in-memory insert back"
        );
        // On disk: ALSO rolled back. The previous (pre-X2) per-verb
        // code did this for `agents_add` but NOT for `agents_update`
        // or `agents_set_enabled`; the helper hoists the invariant.
        drop(store);
        let reopened = AgentStore::open_at(path).unwrap();
        assert!(reopened.get(&id).is_none());
    }

    #[test]
    fn apply_change_save_failure_unwinds_with_no_artifact() {
        // F10/X2 cross-test: a `save` failure must roll back the
        // in-memory mutation and NEVER reach `register`. The helper
        // shares this property with `install_draft` (delegates here);
        // the test asserts it independently for an add-shaped change.
        let dir = tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let path = blocker.join("nested").join("agents.json");

        let seed_path = dir.path().join("seed.json");
        let mut store = AgentStore::open_at(seed_path).unwrap();
        store.set_path(path);

        let agent = installed_agent("brittle-add", true);
        let id = agent.id;

        let sched = FakeScheduler::ok();
        let agent_for_mutate = agent.clone();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                store.add(agent_for_mutate.clone())?;
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            move |store| {
                let _ = store.remove(&id);
            },
            |_a| Ok(()),
            &sched,
        );
        assert!(result.is_err(), "save failure must surface as Err");
        assert!(
            store.get(&id).is_none(),
            "rollback must remove the just-inserted record"
        );
        assert!(
            sched.registered.borrow().is_empty(),
            "register must not run after a save failure"
        );
    }

    #[test]
    fn apply_change_shim_failure_unwinds_with_no_artifact() {
        // A failing shim closure must surface as Err, roll back the
        // in-memory mutation, never reach save, never reach register.
        let dir = tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let mut store = AgentStore::open_at(path.clone()).unwrap();
        let agent = installed_agent("shim-fail", true);
        let id = agent.id;

        let sched = FakeScheduler::ok();
        let agent_for_mutate = agent.clone();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                store.add(agent_for_mutate.clone())?;
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            move |store| {
                let _ = store.remove(&id);
            },
            |_a| Err(AgentError::InvalidPath("/x".into(), "fake shim failure")),
            &sched,
        );
        assert!(result.is_err());
        assert!(store.get(&id).is_none(), "rollback must remove the record");
        assert!(sched.registered.borrow().is_empty());

        // On disk: nothing was persisted — `agents.json` may not even
        // exist (no prior save in this test). What matters is that
        // the rolled-back add is not on disk.
        drop(store);
        let reopened = AgentStore::open_at(path).unwrap();
        assert!(reopened.get(&id).is_none());
    }

    #[test]
    fn apply_change_x1_draft_enabled_is_rejected() {
        // X1: the helper REFUSES to materialize an artifact for a
        // Draft + enabled record. This is the headline fix:
        // `agents_set_enabled(true)` on a Draft must error here, no
        // matter which verb invokes the helper.
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        // Seed a Draft + enabled agent the helper can find via id.
        let mut a = draft_agent("draft-victim", true);
        a.lifecycle = Lifecycle::Draft;
        let id = a.id;
        store.add(a.clone()).unwrap();

        let sched = FakeScheduler::ok();
        let mut shim_calls = 0;
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            |_store| {
                // No-op rollback — the mutation is a no-op clone.
            },
            |_a| {
                shim_calls += 1;
                Ok(())
            },
            &sched,
        );
        match result {
            Err(AgentError::InvalidEnv(m)) => assert!(
                m.to_lowercase().contains("draft"),
                "X1 rejection must name 'draft', got {m}"
            ),
            other => panic!("expected X1 InvalidEnv rejection, got {other:?}"),
        }
        assert_eq!(
            shim_calls, 0,
            "shim must NOT run when the helper rejects a draft"
        );
        assert!(sched.registered.borrow().is_empty());
        // The record is still on disk as a Draft.
        assert_eq!(store.get(&id).unwrap().lifecycle, Lifecycle::Draft);
    }

    #[test]
    fn apply_change_draft_but_disabled_is_allowed() {
        // The X1 gate fires only on Draft + enabled. A Draft +
        // disabled mutation has no scheduler consequence and must
        // not be rejected — e.g. a verb that disables a draft is
        // benign.
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut a = draft_agent("draft-disabled", false);
        a.lifecycle = Lifecycle::Draft;
        let id = a.id;
        store.add(a.clone()).unwrap();

        let sched = FakeScheduler::ok();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            |_store| {},
            |_a| panic!("shim must not run for a disabled record"),
            &sched,
        );
        assert!(result.is_ok(), "draft + disabled must be allowed");
        assert!(sched.registered.borrow().is_empty());
    }

    #[test]
    fn apply_change_disabled_record_unregisters() {
        // Models `agents_set_enabled(false)`: the helper must call
        // `scheduler.unregister` (best-effort) for a disabled
        // record. The FakeScheduler's unregister is a no-op `Ok(())`;
        // the assertion here is that the helper completes
        // successfully and never tried to register.
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let a = installed_agent("disabling", true);
        let id = a.id;
        store.add(a).unwrap();
        store.save().unwrap();

        let sched = FakeScheduler::ok();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                // Disable the record.
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(false),
                    ..crate::agent::store::AgentPatch::default()
                };
                store.update(&id, patch)?;
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            move |store| {
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(true),
                    ..crate::agent::store::AgentPatch::default()
                };
                let _ = store.update(&id, patch);
            },
            |_a| panic!("shim must not run for a disabled record"),
            &sched,
        );
        let post = result.expect("set_enabled(false) must succeed");
        assert!(!post.enabled);
        assert!(
            sched.registered.borrow().is_empty(),
            "no register for disabled"
        );
        // Saved.
        assert!(!store.get(&id).unwrap().enabled);
    }

    #[test]
    fn apply_change_rollback_restores_pre_mutation_updated_at() {
        // grill X24: a failed install / failed shim / failed
        // register / failed save / X1 rejection must leave the
        // agent's `updated_at` at its pre-mutation value, not at
        // the timestamp `arm`/`update` stamped while the mutation
        // was in flight. Otherwise the UI shows "updated 3 seconds
        // ago" on a record that is logically unchanged after the
        // rollback.
        //
        // Models the `agents_set_enabled` shape: the verb passes a
        // mutate closure that calls `store.update`, and a rollback
        // closure that calls `store.update` with the old enabled
        // bit. Both `update` calls bump `updated_at = now()`; the
        // helper must reverse that.
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut a = installed_agent("ts-victim", false);
        // Seed the agent with a clearly-old `updated_at` so the
        // bumped-then-restored value is unambiguous.
        let original_ts = Utc::now() - chrono::Duration::days(7);
        a.updated_at = original_ts;
        let id = a.id;
        store.add(a).unwrap();
        store.save().unwrap();

        // The mutate closure flips enabled to true (triggering the
        // X1 Draft check via a normal Installed agent — so the
        // Draft gate does NOT fire; pick a failing scheduler so the
        // rollback path is the register-failure branch).
        let sched = FakeScheduler::failing();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(true),
                    ..crate::agent::store::AgentPatch::default()
                };
                store.update(&id, patch)?;
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            move |store| {
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(false),
                    ..crate::agent::store::AgentPatch::default()
                };
                let _ = store.update(&id, patch);
            },
            |_a| Ok(()),
            &sched,
        );
        assert!(result.is_err(), "register failure must surface as Err");

        // The agent on disk should carry the ORIGINAL `updated_at`,
        // not a freshly-bumped one. Use a wide tolerance — we just
        // need to prove the helper restored to "around the original"
        // and not "around now()".
        let after = store.get(&id).expect("agent still present");
        assert!(
            (after.updated_at - original_ts).num_seconds().abs() < 2,
            "after a rolled-back failed install, `updated_at` must \
             equal the pre-mutation value (original={original_ts}, \
             after={ts}, delta={delta}s)",
            ts = after.updated_at,
            delta = (after.updated_at - original_ts).num_seconds(),
        );
    }

    /// A4 (audit follow-up): the X24 timestamp restore must hold on
    /// EVERY rollback path. The existing tests covered the X1
    /// rejection and the register-failure branch; this one covers
    /// the shim-failure branch — earliest rollback point, before
    /// save. A refactor that drops the `set_updated_at` call from
    /// the shim-failure branch would have landed green without this.
    #[test]
    fn apply_change_shim_failure_restores_updated_at() {
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut a = installed_agent("shim-ts-victim", false);
        let original_ts = Utc::now() - chrono::Duration::days(5);
        a.updated_at = original_ts;
        let original_enabled = a.enabled;
        let id = a.id;
        store.add(a).unwrap();
        store.save().unwrap();

        // The mutate closure enables the agent (bumping `updated_at`
        // inside `update`); the shim closure errors so the rollback
        // branch fires before save.
        let sched = FakeScheduler::ok();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(true),
                    ..crate::agent::store::AgentPatch::default()
                };
                store.update(&id, patch)?;
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            move |store| {
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(original_enabled),
                    ..crate::agent::store::AgentPatch::default()
                };
                let _ = store.update(&id, patch);
            },
            |_a| Err(AgentError::InvalidPath("/x".into(), "shim forced to fail")),
            &sched,
        );
        assert!(result.is_err(), "shim failure must surface as Err");

        let after = store.get(&id).expect("agent still present");
        assert_eq!(
            after.enabled, original_enabled,
            "rollback must restore the prior `enabled` bit"
        );
        assert!(
            (after.updated_at - original_ts).num_seconds().abs() < 2,
            "shim-failure rollback must restore `updated_at` \
             (original={original_ts}, after={ts}, delta={delta}s)",
            ts = after.updated_at,
            delta = (after.updated_at - original_ts).num_seconds(),
        );
        // No artifact was registered (shim failed before save and
        // before register).
        assert!(sched.registered.borrow().is_empty());
    }

    /// A4 (audit follow-up): the save-failure branch is the OTHER
    /// path the existing tests didn't cover for `updated_at`. The
    /// mutate-closure's `store.update` bumps `updated_at` in memory;
    /// the save then fails (un-creatable path); the rollback closure
    /// fires; the helper must also restore `updated_at`.
    #[test]
    fn apply_change_save_failure_restores_updated_at() {
        let dir = tempdir().unwrap();
        // Seed via a writable path so the agent ends up in the store,
        // THEN re-point at an un-creatable path so `save` fails on
        // the next call.
        let seed_path = dir.path().join("seed.json");
        let mut store = AgentStore::open_at(seed_path).unwrap();
        let mut a = installed_agent("save-ts-victim", false);
        let original_ts = Utc::now() - chrono::Duration::days(4);
        a.updated_at = original_ts;
        let original_enabled = a.enabled;
        let id = a.id;
        store.add(a).unwrap();
        store.save().unwrap();

        // Re-point: parent is a regular file, so `create_dir_all`
        // inside `save` will fail.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let unwritable = blocker.join("nested").join("agents.json");
        store.set_path(unwritable);

        let sched = FakeScheduler::ok();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(true),
                    ..crate::agent::store::AgentPatch::default()
                };
                store.update(&id, patch)?;
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            move |store| {
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(original_enabled),
                    ..crate::agent::store::AgentPatch::default()
                };
                let _ = store.update(&id, patch);
            },
            |_a| Ok(()),
            &sched,
        );
        assert!(result.is_err(), "save failure must surface as Err");

        let after = store.get(&id).expect("agent still present in-memory");
        assert_eq!(after.enabled, original_enabled, "rollback restored enabled");
        assert!(
            (after.updated_at - original_ts).num_seconds().abs() < 2,
            "save-failure rollback must restore `updated_at` \
             (original={original_ts}, after={ts}, delta={delta}s)",
            ts = after.updated_at,
            delta = (after.updated_at - original_ts).num_seconds(),
        );
        // Register was never reached (save failed before register).
        assert!(sched.registered.borrow().is_empty());
    }

    #[test]
    fn apply_change_x1_rejection_also_restores_updated_at() {
        // X24 cross-test: the Draft + enabled X1 rejection runs
        // the rollback closure too, and must restore `updated_at`
        // the same way.
        let dir = tempdir().unwrap();
        let mut store = AgentStore::open_at(dir.path().join("a.json")).unwrap();
        let mut a = draft_agent("draft-ts", false);
        a.lifecycle = Lifecycle::Draft;
        let original_ts = Utc::now() - chrono::Duration::days(3);
        a.updated_at = original_ts;
        let id = a.id;
        store.add(a).unwrap();
        store.save().unwrap();

        let sched = FakeScheduler::ok();
        let result = apply_lifecycle_change(
            &mut store,
            &id,
            move |store| {
                // Mutation that would enable the draft — exactly the
                // shape X1 exists to refuse.
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(true),
                    ..crate::agent::store::AgentPatch::default()
                };
                store.update(&id, patch)?;
                store
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| AgentError::NotFound(id.to_string()))
            },
            move |store| {
                let patch = crate::agent::store::AgentPatch {
                    enabled: Some(false),
                    ..crate::agent::store::AgentPatch::default()
                };
                let _ = store.update(&id, patch);
            },
            |_a| Ok(()),
            &sched,
        );
        match result {
            Err(AgentError::InvalidEnv(_)) => {}
            other => panic!("expected X1 rejection, got {other:?}"),
        }

        let after = store.get(&id).expect("agent still present");
        assert!(
            (after.updated_at - original_ts).num_seconds().abs() < 2,
            "X1 rollback must also restore `updated_at`"
        );
    }
}
