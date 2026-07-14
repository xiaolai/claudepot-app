//! grill X27: integration smoke test for the wiring `agent_install`
//! (the Tauri command) builds around `install_draft`.
//!
//! The pure helper `install_draft` is already exhaustively covered by
//! `claudepot_core::agent::install_gate::tests` — happy path,
//! shim/save/register failures, X1 Draft gate, X24 `updated_at`
//! restore. What was *not* covered: the *closure shape* the Tauri
//! command builds out of `route_lookup_fn`, `current_claudepot_cli`,
//! `resolve_binary`, and `install_shim`. A refactor of any of those
//! that breaks the closure's type contract (e.g. changing
//! `resolve_binary`'s lookup-argument signature) compiles inside
//! `claudepot_core` but explodes at the Tauri-layer call site.
//!
//! This smoke test reconstructs the exact closure
//! `commands::agents::agent_install` passes to `install_draft`, drives
//! it through a controlled temp `CLAUDEPOT_DATA_DIR`, and asserts the
//! happy-path side effects (in-store lifecycle flip, on-disk save,
//! shim file emitted, scheduler.register fired). Going through the
//! real Tauri IPC layer would require booting a webview-less
//! `tauri::Builder`, which is far more invasive than the regression
//! signal needs; the closure shape is what historically drifts.
//!
//! Run via `cargo test -p claudepot-tauri` from the workspace root.

use std::cell::RefCell;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use claudepot_core::agent::{
    current_claudepot_cli, install_draft, install_shim, resolve_binary,
    scheduler::{RegisteredEntry, SchedulerCapabilities},
    Agent, AgentBinary, AgentError, AgentId, AgentStore, CreatedVia, Lifecycle, OutputFormat,
    PermissionMode, PlatformOptions, Scheduler, Trigger,
};
// A5: import the REAL route_lookup_fn the Tauri command builds. A
// refactor that changes its signature (e.g. adds an `&AccountStore`
// parameter) now fails to compile here — the smoke test no longer
// re-implements the wiring it claims to lock down.
use claudepot_tauri_lib::commands::agents::route_lookup_fn;
use uuid::Uuid;

/// A Scheduler stub that records the agents it registered. Same shape
/// as the `FakeScheduler` inside `install_gate::tests` — but the
/// install_gate tests cannot reach the closure that Tauri builds, so
/// we duplicate the minimum surface here.
struct FakeScheduler {
    registered: RefCell<Vec<AgentId>>,
}

impl FakeScheduler {
    fn new() -> Self {
        Self {
            registered: RefCell::new(Vec::new()),
        }
    }
}

impl Scheduler for FakeScheduler {
    fn register(&self, agent: &Agent) -> Result<(), AgentError> {
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
        _trigger: &Trigger,
        _from: DateTime<Utc>,
        _n: usize,
    ) -> Result<Vec<DateTime<Utc>>, AgentError> {
        Ok(Vec::new())
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

/// Build a fresh installable agent — Manual trigger so the scheduler
/// is exercised by the helper for the X1 "enabled" branch without
/// needing a real cron evaluator. Manual is the simplest enabled
/// trigger that NoopScheduler/FakeScheduler can register without
/// platform support. (NoopScheduler short-circuits Manual to Ok.)
fn sample_draft(name: &str) -> Agent {
    let now = Utc::now();
    Agent {
        id: Uuid::new_v4(),
        name: name.into(),
        display_name: None,
        description: None,
        enabled: true,
        binary: AgentBinary::FirstParty,
        model: Some("sonnet".into()),
        // Host-absolute temp dir (NOT a hardcoded "/tmp" — that fails
        // `validate_cwd`'s host-native is_absolute() check on Windows,
        // where the new src-tauri CI test step runs).
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
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
        trigger: Trigger::Manual,
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
        drafted_by: Some("smoke@test".into()),
        created_via: CreatedVia::CliDraft,
        result_sink: None,
    }
}

/// Env tests share a process; `CLAUDEPOT_DATA_DIR` and
/// `CLAUDEPOT_CLI_PATH` are process-global. Serialize the tests so
/// they don't trample each other.
static ENV_GUARD: Mutex<()> = Mutex::new(());

/// Drive the exact closure shape that
/// `commands::agents::agent_install` builds: `route_lookup_fn` +
/// `current_claudepot_cli` + `resolve_binary` + `install_shim`. A
/// refactor that breaks any of those signatures (e.g. changing the
/// route_lookup arg shape) will fail to compile here, even though
/// the four helpers are individually unit-tested in
/// `claudepot_core`.
#[test]
fn agent_install_closure_shape_drives_install_draft_happy_path() {
    let _lock = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());

    // `current_claudepot_cli` needs to resolve to *something* — the
    // shim renders the path into the dispatched command. Point it at
    // a known existing file inside the temp dir.
    let fake_cli = dir.path().join("claudepot-fake-cli");
    std::fs::write(&fake_cli, b"#!/bin/sh\nexit 0\n").unwrap();
    std::env::set_var("CLAUDEPOT_CLI_PATH", &fake_cli);

    // Seed the store with a Draft agent.
    let store_path = dir.path().join("agents.json");
    let mut store = AgentStore::open_at(store_path.clone()).unwrap();
    let agent = sample_draft("smoke-agent");
    let id = agent.id;
    store.add(agent).unwrap();
    store.save().unwrap();

    let scheduler = FakeScheduler::new();

    // ----- Construct the EXACT closure `agent_install` builds. -----
    // If any of `current_claudepot_cli`, `route_lookup_fn`,
    // `resolve_binary`, or `install_shim` changes its signature, this
    // block fails to compile — the regression signal X27 exists for.
    let cli_path = current_claudepot_cli().expect("CLAUDEPOT_CLI_PATH resolves");
    // A5: drive the REAL route_lookup_fn from `commands::agents`.
    // With no routes registered in this temp CLAUDEPOT_DATA_DIR, it
    // returns `None` for every id — correct for a first-party agent.
    let lookup = route_lookup_fn();

    let outcome = install_draft(&mut store, &id, &scheduler, |a| {
        let binary_path = resolve_binary(a, &lookup)?;
        install_shim(a, &binary_path, &cli_path).map(|_| ())
    })
    .expect("install_draft happy path must succeed");

    // ----- Assertions -----

    // 1. The lifecycle flipped in memory and persisted to disk.
    assert_eq!(outcome.agent.lifecycle, Lifecycle::Installed);
    assert_eq!(
        store.get(&id).unwrap().lifecycle,
        Lifecycle::Installed,
        "in-memory store reflects the install"
    );
    drop(store);
    let reopened = AgentStore::open_at(store_path).unwrap();
    assert_eq!(
        reopened.get(&id).unwrap().lifecycle,
        Lifecycle::Installed,
        "on-disk store reflects the install — save ran in the right order"
    );

    // 2. The scheduler saw a `register` call.
    assert_eq!(
        scheduler.registered.borrow().len(),
        1,
        "scheduler.register fired exactly once"
    );
    assert_eq!(scheduler.registered.borrow()[0], id);

    // 3. The shim landed on disk — proof the closure's
    //    `install_shim(a, &binary_path, &cli_path)` call reached real
    //    I/O and not a half-wired plumbing path. Path shape matches
    //    `agent_dir(&id)/run.{sh,cmd}` per
    //    `claudepot_core::agent::install`.
    let agent_dir = claudepot_core::agent::agent_dir(&id);
    let shim_name = if cfg!(target_os = "windows") {
        "run.cmd"
    } else {
        "run.sh"
    };
    let shim_path = agent_dir.join(shim_name);
    assert!(
        shim_path.exists(),
        "shim file must be written at {} after install_draft completes",
        shim_path.display()
    );

    // Cleanup so other tests in the suite see a clean env.
    std::env::remove_var("CLAUDEPOT_DATA_DIR");
    std::env::remove_var("CLAUDEPOT_CLI_PATH");
}

/// Belt-and-suspenders: the same closure shape under the X1 gate.
/// If anyone restructures `install_draft` so the closure no longer
/// receives the post-arm `Agent`, this test catches the drift.
#[test]
fn agent_install_closure_shape_rejects_register_failure_cleanly() {
    let _lock = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

    /// FakeScheduler that fails `register` so the helper's
    /// rollback runs — exercises the closure-shape contract on the
    /// error path too.
    struct FailingScheduler;
    impl Scheduler for FailingScheduler {
        fn register(&self, _agent: &Agent) -> Result<(), AgentError> {
            Err(AgentError::UnsupportedPlatform(
                "smoke test: register forced to fail",
            ))
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
            format!("fail.agent.{id}")
        }
        fn next_runs(
            &self,
            _trigger: &Trigger,
            _from: DateTime<Utc>,
            _n: usize,
        ) -> Result<Vec<DateTime<Utc>>, AgentError> {
            Ok(Vec::new())
        }
        fn capabilities(&self) -> SchedulerCapabilities {
            SchedulerCapabilities {
                wake_to_run: false,
                catch_up_if_missed: false,
                run_when_logged_out: false,
                native_label: "failing",
                artifact_dir: None,
            }
        }
    }

    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());
    let fake_cli = dir.path().join("claudepot-fake-cli");
    std::fs::write(&fake_cli, b"#!/bin/sh\nexit 0\n").unwrap();
    std::env::set_var("CLAUDEPOT_CLI_PATH", &fake_cli);

    let store_path = dir.path().join("agents.json");
    let mut store = AgentStore::open_at(store_path.clone()).unwrap();
    let mut agent = sample_draft("rollback-victim");
    // A5 + A4 cross-test: stamp a clearly-old `updated_at` so the
    // X24 rollback-restore assertion below is unambiguous. After the
    // failed install_draft, the on-disk agent must still carry THIS
    // timestamp — not a fresh `now()` bumped by the in-flight `arm`
    // call that the helper rolled back.
    let original_updated_at = Utc::now() - chrono::Duration::days(6);
    agent.updated_at = original_updated_at;
    let id = agent.id;
    store.add(agent).unwrap();
    store.save().unwrap();

    let cli_path = current_claudepot_cli().expect("CLAUDEPOT_CLI_PATH resolves");
    // A5: drive the REAL route_lookup_fn from `commands::agents`.
    // With no routes registered in this temp CLAUDEPOT_DATA_DIR, it
    // returns `None` for every id — correct for a first-party agent.
    let lookup = route_lookup_fn();

    let result = install_draft(&mut store, &id, &FailingScheduler, |a| {
        let binary_path = resolve_binary(a, &lookup)?;
        install_shim(a, &binary_path, &cli_path).map(|_| ())
    });

    assert!(result.is_err(), "FailingScheduler must surface as Err");
    // F10 contract: on register failure the on-disk lifecycle is
    // rolled back to Draft. If a refactor breaks this — e.g. moves
    // the save after register, or swallows the register error — the
    // assertion fires.
    drop(store);
    let reopened = AgentStore::open_at(store_path).unwrap();
    let after = reopened.get(&id).expect("agent still present");
    assert_eq!(
        after.lifecycle,
        Lifecycle::Draft,
        "on-disk lifecycle must be rolled back after a register failure"
    );
    // A5 ties to A4: the rollback must ALSO restore `updated_at`.
    // The `arm` call inside `mutate` bumps `updated_at = now()`; the
    // rollback closure calls `set_lifecycle` which does NOT bump, so
    // without the helper's X24 timestamp snapshot the rolled-back
    // agent would claim it was just edited.
    assert!(
        (after.updated_at - original_updated_at).num_seconds().abs() < 2,
        "X24 / A4: updated_at must be restored to its pre-mutation \
         value after a register-failure rollback \
         (original={original_updated_at}, after={ts}, delta={delta}s)",
        ts = after.updated_at,
        delta = (after.updated_at - original_updated_at).num_seconds(),
    );

    std::env::remove_var("CLAUDEPOT_DATA_DIR");
    std::env::remove_var("CLAUDEPOT_CLI_PATH");
}
