//! Integration tests for `LiveRuntime` — wires registry + tail +
//! status + bus end-to-end against a fake filesystem.
//!
//! Included from `runtime.rs` via `#[cfg(test)] #[path] mod tests;`
//! so the file stays focused on the orchestrator.

use super::*;
use crate::project_sanitize::sanitize_path;
use crate::session_live::registry::ProcessCheck;
use std::collections::HashSet;
use std::io::Write;
use tempfile::TempDir;

/// Synthetic `ProcessCheck` for tests — declares a fixed set of
/// PIDs "alive" without touching real processes.
#[derive(Default, Clone)]
struct FakeCheck {
    alive: std::sync::Arc<std::sync::Mutex<HashSet<u32>>>,
}

impl FakeCheck {
    fn set_alive(&self, pids: &[u32]) {
        let mut a = self.alive.lock().unwrap();
        a.clear();
        a.extend(pids.iter().copied());
    }
}

impl ProcessCheck for FakeCheck {
    fn is_running(&self, pid: u32) -> bool {
        self.alive.lock().unwrap().contains(&pid)
    }
}

fn write_pid_file(dir: &std::path::Path, pid: u32, body: &str) {
    let mut f = std::fs::File::create(dir.join(format!("{pid}.json"))).unwrap();
    f.write_all(body.as_bytes()).unwrap();
}

fn write_transcript(projects_dir: &std::path::Path, cwd: &str, sid: &str, body: &str) {
    let slug = sanitize_path(cwd);
    let dir = projects_dir.join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join(format!("{sid}.jsonl"))).unwrap();
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

struct Fixture {
    _td: TempDir,
    sessions_dir: std::path::PathBuf,
    projects_dir: std::path::PathBuf,
    check: FakeCheck,
    runtime: std::sync::Arc<LiveRuntime>,
}

fn fixture() -> Fixture {
    let td = TempDir::new().unwrap();
    let sessions_dir = td.path().join("sessions");
    let projects_dir = td.path().join("projects");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&projects_dir).unwrap();
    let check = FakeCheck::default();
    let runtime = LiveRuntime::with_dirs(
        std::sync::Arc::new(check.clone()) as _,
        sessions_dir.clone(),
        projects_dir.clone(),
    );
    Fixture {
        _td: td,
        sessions_dir,
        projects_dir,
        check,
        runtime,
    }
}

#[tokio::test]
async fn tick_attaches_live_sessions_and_publishes_aggregate() {
    let f = fixture();
    // CC registers a live session.
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    // CC has already written an initial transcript line.
    write_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "sess-a",
        r#"{"type":"custom-title","customTitle":"t","sessionId":"sess-a"}
"#,
    );
    f.check.set_alive(&[12345]);

    f.runtime.tick().await.unwrap();

    let snap = f.runtime.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].session_id, "sess-a");
    assert_eq!(snap[0].pid, 12345);
    assert_eq!(snap[0].cwd, "/tmp/proj");
    assert_eq!(snap[0].status, Status::Idle);
}

#[tokio::test]
async fn tick_drops_sessions_when_pid_dies() {
    let f = fixture();
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/proj", "sess-a", "");
    f.check.set_alive(&[12345]);
    f.runtime.tick().await.unwrap();
    assert_eq!(f.runtime.snapshot().len(), 1);

    // Process dies — stale sweep removes the pid file, then next
    // tick drops the session.
    f.check.set_alive(&[]);
    f.runtime.tick().await.unwrap();
    assert_eq!(f.runtime.snapshot().len(), 0);
}

#[tokio::test]
async fn tick_ingests_appended_lines_and_flips_status() {
    let f = fixture();
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/proj", "sess-a", "");
    f.check.set_alive(&[12345]);
    // First tick: attach at EOF — no events yet, status Idle.
    f.runtime.tick().await.unwrap();
    assert_eq!(f.runtime.snapshot()[0].status, Status::Idle);

    // CC appends: user message + unmatched tool_use.
    append_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "sess-a",
        concat!(
            r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"sess-a","version":"2.1","timestamp":"2026-04-21T10:00:00.000Z","uuid":"u1","type":"user","message":{"role":"user","content":"go"}}"#,
            "\n",
            r#"{"parentUuid":"u1","isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"sess-a","version":"2.1","timestamp":"2026-04-21T10:00:05.000Z","uuid":"u2","type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"pnpm test"}}]}}"#,
            "\n",
        ),
    );
    f.runtime.tick().await.unwrap();
    let s = &f.runtime.snapshot()[0];
    assert_eq!(s.status, Status::Busy);
    assert_eq!(s.model.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(s.current_action.as_deref(), Some("Bash: pnpm test"));
}

#[tokio::test]
async fn detail_subscriber_receives_status_change_delta() {
    let f = fixture();
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/proj", "sess-a", "");
    f.check.set_alive(&[12345]);
    f.runtime.tick().await.unwrap();

    let mut rx = f.runtime.subscribe_detail("sess-a").await.unwrap();

    // Trigger a transition.
    append_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "sess-a",
        concat!(
            r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"sess-a","version":"2.1","timestamp":"2026-04-21T10:00:00.000Z","uuid":"u1","type":"user","message":{"role":"user","content":"go"}}"#,
            "\n",
        ),
    );
    f.runtime.tick().await.unwrap();

    let d = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .expect("delta should arrive")
        .unwrap();
    match d.kind {
        LiveDeltaKind::StatusChanged { status, .. } => {
            assert_eq!(status, Status::Busy);
        }
        other => panic!("expected StatusChanged, got {other:?}"),
    }
    assert!(!d.resync_required);
}

#[tokio::test]
async fn ended_delta_fires_when_session_disappears() {
    let f = fixture();
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/proj", "sess-a", "");
    f.check.set_alive(&[12345]);
    f.runtime.tick().await.unwrap();

    let mut rx = f.runtime.subscribe_detail("sess-a").await.unwrap();

    // Kill the session.
    f.check.set_alive(&[]);
    f.runtime.tick().await.unwrap();

    let d = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .expect("Ended should arrive")
        .unwrap();
    assert!(matches!(d.kind, LiveDeltaKind::Ended));
}

#[tokio::test]
async fn missing_transcript_defers_attach_gracefully() {
    let f = fixture();
    // PID file exists but transcript doesn't — CC hasn't flushed yet.
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    f.check.set_alive(&[12345]);
    // Tick must not panic; snapshot stays empty until transcript appears.
    f.runtime.tick().await.unwrap();
    assert_eq!(f.runtime.snapshot().len(), 0);

    // Transcript lands. Next tick picks it up.
    write_transcript(&f.projects_dir, "/tmp/proj", "sess-a", "");
    f.runtime.tick().await.unwrap();
    assert_eq!(f.runtime.snapshot().len(), 1);
}

#[tokio::test]
async fn start_stop_lifecycle_is_clean() {
    let f = fixture();
    let handle = f.runtime.clone().start();
    // Stop almost immediately.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    f.runtime.stop();
    tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .expect("stop should complete within 2s")
        .unwrap();
}

#[tokio::test]
async fn session_snapshot_returns_live_record_only() {
    let f = fixture();
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/proj", "sess-a", "");
    f.check.set_alive(&[12345]);
    f.runtime.tick().await.unwrap();

    assert!(f.runtime.session_snapshot("sess-a").await.is_some());
    assert!(f.runtime.session_snapshot("unknown").await.is_none());
}

#[tokio::test]
async fn redaction_applied_to_current_action() {
    let f = fixture();
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/proj", "sess-a", "");
    f.check.set_alive(&[12345]);
    f.runtime.tick().await.unwrap();

    append_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "sess-a",
        concat!(
            r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"sess-a","version":"2.1","timestamp":"2026-04-21T10:00:00.000Z","uuid":"u1","type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"curl -H 'Authorization: Bearer sk-ant-Abc123DEF456_xyz'"}}]}}"#,
            "\n",
        ),
    );
    f.runtime.tick().await.unwrap();
    let ca = f.runtime.snapshot()[0]
        .current_action
        .clone()
        .expect("current_action set");
    assert!(
        !ca.contains("sk-ant-Abc123DEF456_xyz"),
        "raw key leaked through aggregate DTO: {ca}"
    );
    assert!(ca.contains("sk-ant-***"));
}
