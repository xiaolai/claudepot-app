//! E2E smoke test for the Activity data plane.
//!
//! Satisfies the plan's WI-12 exit criterion ("synthetic PID + JSONL
//! appends; row appears in < 1 s") without pulling in Playwright +
//! Tauri-dev infrastructure. The Rust-level path is what actually
//! contains the orchestration logic — the UI layers on top are thin
//! wrappers around the snapshot + subscribe surface this test
//! exercises directly.
//!
//! Runs via `cargo test --workspace` or targeted
//! `cargo test -p claudepot-core --test activity_e2e`.

use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use claudepot_core::project_sanitize::sanitize_path;
use claudepot_core::session_live::registry::ProcessCheck;
use claudepot_core::session_live::types::{LiveDeltaKind, Status};
use claudepot_core::session_live::LiveRuntime;
use tempfile::TempDir;

/// Synthetic process check — tests declare which PIDs are "alive"
/// dynamically, without touching real processes.
#[derive(Default, Clone)]
struct FakeProcessCheck {
    alive: Arc<Mutex<HashSet<u32>>>,
}

impl FakeProcessCheck {
    fn set_alive(&self, pids: &[u32]) {
        let mut a = self.alive.lock().unwrap();
        a.clear();
        a.extend(pids.iter().copied());
    }
}

impl ProcessCheck for FakeProcessCheck {
    fn is_running(&self, pid: u32) -> bool {
        self.alive.lock().unwrap().contains(&pid)
    }
}

fn write_pid_file(sessions_dir: &std::path::Path, pid: u32, body: &str) {
    let path = sessions_dir.join(format!("{pid}.json"));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
}

fn write_transcript(projects_dir: &std::path::Path, cwd: &str, sid: &str, body: &str) {
    let slug = sanitize_path(cwd);
    let dir = projects_dir.join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{sid}.jsonl"));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
}

fn append_transcript(projects_dir: &std::path::Path, cwd: &str, sid: &str, body: &str) {
    let slug = sanitize_path(cwd);
    let path = projects_dir.join(slug).join(format!("{sid}.jsonl"));
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    f.write_all(body.as_bytes()).unwrap();
}

/// End-to-end smoke: synthetic CC session appears → sidebar strip's
/// aggregate snapshot reflects it → user-visible status transitions
/// within 1 s. This is the plan's M1 acceptance criterion as a
/// deterministic test.
#[tokio::test]
async fn synthetic_session_appears_and_transitions_within_1s() {
    let td = TempDir::new().unwrap();
    let sessions_dir = td.path().join("sessions");
    let projects_dir = td.path().join("projects");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&projects_dir).unwrap();

    let check = FakeProcessCheck::default();
    let runtime = LiveRuntime::with_dirs(
        Arc::new(check.clone()) as _,
        sessions_dir.clone(),
        projects_dir.clone(),
    );

    // No PID file yet → snapshot is empty.
    assert!(runtime.snapshot().is_empty());

    // Simulate a CC process starting: drop a PID file and write the
    // first transcript line. Mark the PID alive.
    write_pid_file(
        &sessions_dir,
        42099,
        r#"{"pid":42099,"sessionId":"sess-e2e","cwd":"/tmp/e2e-proj","startedAt":1000}"#,
    );
    write_transcript(&projects_dir, "/tmp/e2e-proj", "sess-e2e", "");
    check.set_alive(&[42099]);

    let t0 = std::time::Instant::now();
    runtime.tick().await.unwrap();
    let t_attach = t0.elapsed();
    assert!(
        t_attach < Duration::from_secs(1),
        "attach tick took {t_attach:?}, expected < 1s"
    );

    let snap = runtime.snapshot();
    assert_eq!(snap.len(), 1, "aggregate should carry the new session");
    assert_eq!(snap[0].session_id, "sess-e2e");
    assert_eq!(
        snap[0].status,
        Status::Idle,
        "freshly-attached session with no events is Idle"
    );

    // CC writes a user prompt + unmatched tool_use. Expect a
    // StatusChanged delta to flip the status to Busy and
    // current_action to reflect the tool.
    let mut rx = runtime.subscribe_detail("sess-e2e").await.unwrap();
    append_transcript(
        &projects_dir,
        "/tmp/e2e-proj",
        "sess-e2e",
        concat!(
            r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"sess-e2e","version":"2.1","timestamp":"2026-04-21T10:00:00.000Z","uuid":"u1","type":"user","message":{"role":"user","content":"run tests"}}"#,
            "\n",
            r#"{"parentUuid":"u1","isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"sess-e2e","version":"2.1","timestamp":"2026-04-21T10:00:01.000Z","uuid":"u2","type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"pnpm test"}}]}}"#,
            "\n",
        ),
    );

    let t_transition_start = std::time::Instant::now();
    runtime.tick().await.unwrap();
    let t_transition = t_transition_start.elapsed();
    assert!(
        t_transition < Duration::from_secs(1),
        "transition tick took {t_transition:?}, expected < 1s"
    );

    let snap = runtime.snapshot();
    assert_eq!(snap[0].status, Status::Busy);
    assert_eq!(snap[0].model.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(snap[0].current_action.as_deref(), Some("Bash: pnpm test"));

    // A StatusChanged delta must have arrived.
    let d = tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("delta should arrive")
        .unwrap();
    match d.kind {
        LiveDeltaKind::StatusChanged { status, .. } => {
            assert_eq!(status, Status::Busy);
        }
        other => panic!("expected StatusChanged, got {other:?}"),
    }

    // Simulate the session ending. Expect an Ended delta (possibly
    // interleaved with a late Overlay/Status transition from the
    // prior tick) and the aggregate drops to empty.
    check.set_alive(&[]);
    runtime.tick().await.unwrap();
    assert!(runtime.snapshot().is_empty());

    // Drain deltas until we see Ended, with a bounded timeout.
    // Intermediate StatusChanged / OverlayChanged deltas from the
    // second tick are fine — they just happen before the end.
    let mut saw_ended = false;
    for _ in 0..8 {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(d)) if matches!(d.kind, LiveDeltaKind::Ended) => {
                saw_ended = true;
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
    assert!(saw_ended, "Ended delta did not arrive within window");
}

/// E2E for the task-summary path: CC writes a type:"task-summary"
/// line and the runtime emits a TaskSummaryChanged delta with the
/// text post-redaction. Protects the M2 contract that
/// current_action prefers task-summary over tool head-lines.
#[tokio::test]
async fn task_summary_path_end_to_end() {
    let td = TempDir::new().unwrap();
    let sessions_dir = td.path().join("sessions");
    let projects_dir = td.path().join("projects");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&projects_dir).unwrap();

    let check = FakeProcessCheck::default();
    let runtime = LiveRuntime::with_dirs(
        Arc::new(check.clone()) as _,
        sessions_dir.clone(),
        projects_dir.clone(),
    );

    write_pid_file(
        &sessions_dir,
        42100,
        r#"{"pid":42100,"sessionId":"sess-ts","cwd":"/tmp/ts-proj","startedAt":1000}"#,
    );
    write_transcript(&projects_dir, "/tmp/ts-proj", "sess-ts", "");
    check.set_alive(&[42100]);
    runtime.tick().await.unwrap();

    let mut rx = runtime.subscribe_detail("sess-ts").await.unwrap();

    append_transcript(
        &projects_dir,
        "/tmp/ts-proj",
        "sess-ts",
        concat!(
            r#"{"type":"task-summary","sessionId":"sess-ts","timestamp":"2026-04-21T10:00:01.000Z","summary":"investigating the test flake"}"#,
            "\n",
        ),
    );
    runtime.tick().await.unwrap();

    let snap = runtime.snapshot();
    assert_eq!(
        snap[0].current_action.as_deref(),
        Some("investigating the test flake"),
        "task-summary text must win over the (absent) tool head-line"
    );

    let mut saw_task_summary = false;
    while let Ok(Some(d)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
        if let LiveDeltaKind::TaskSummaryChanged { summary } = &d.kind {
            assert!(summary.contains("investigating the test flake"));
            saw_task_summary = true;
        }
    }
    assert!(
        saw_task_summary,
        "expected at least one TaskSummaryChanged delta"
    );
}

/// E2E for the redaction boundary: a bearer-wrapped sk-ant key in a
/// Bash input must never surface unredacted in the aggregate DTO.
#[tokio::test]
async fn redaction_boundary_end_to_end() {
    let td = TempDir::new().unwrap();
    let sessions_dir = td.path().join("sessions");
    let projects_dir = td.path().join("projects");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&projects_dir).unwrap();

    let check = FakeProcessCheck::default();
    let runtime = LiveRuntime::with_dirs(
        Arc::new(check.clone()) as _,
        sessions_dir.clone(),
        projects_dir.clone(),
    );

    write_pid_file(
        &sessions_dir,
        42101,
        r#"{"pid":42101,"sessionId":"sess-r","cwd":"/tmp/r-proj","startedAt":1000}"#,
    );
    write_transcript(&projects_dir, "/tmp/r-proj", "sess-r", "");
    check.set_alive(&[42101]);
    runtime.tick().await.unwrap();

    append_transcript(
        &projects_dir,
        "/tmp/r-proj",
        "sess-r",
        concat!(
            r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"sess-r","version":"2.1","timestamp":"2026-04-21T10:00:00.000Z","uuid":"u1","type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"curl -H 'Authorization: Bearer sk-ant-Abc123DEF456_xyz'"}}]}}"#,
            "\n",
        ),
    );
    runtime.tick().await.unwrap();

    let ca = runtime.snapshot()[0]
        .current_action
        .clone()
        .expect("current_action set");
    assert!(
        !ca.contains("sk-ant-Abc123DEF456_xyz"),
        "raw key leaked through aggregate DTO in E2E: {ca}"
    );
    // Either mask form is acceptable; both protect the body.
    assert!(
        ca.contains("Authorization: Bearer ***") || ca.contains("sk-ant-***"),
        "expected redaction marker in: {ca}"
    );
}
