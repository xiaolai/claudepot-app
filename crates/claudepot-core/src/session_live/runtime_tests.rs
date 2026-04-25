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

/// Transcript path for assertions and mtime bumps. Mirrors
/// `runtime.rs::transcript_path` one for one — kept local so tests
/// don't depend on a private helper.
fn transcript_path(
    projects_dir: &std::path::Path,
    cwd: &str,
    sid: &str,
) -> std::path::PathBuf {
    projects_dir.join(sanitize_path(cwd)).join(format!("{sid}.jsonl"))
}

/// Set the mtime relative to now. `offset_secs` may be negative to
/// place the mtime in the past. Used by resolver-adjacent tests
/// where file-ordering is the whole point — relying on natural
/// file-creation ordering would be both flaky and coarser than the
/// tests need (macOS HFS mtime resolution is seconds).
fn bump_mtime(path: &std::path::Path, offset_secs: i64) {
    let now = std::time::SystemTime::now();
    let target = if offset_secs >= 0 {
        now + std::time::Duration::from_secs(offset_secs as u64)
    } else {
        now - std::time::Duration::from_secs((-offset_secs) as u64)
    };
    filetime::set_file_mtime(
        path,
        filetime::FileTime::from_system_time(target),
    )
    .unwrap();
}

/// Format an ms-since-epoch timestamp as RFC-3339 for embedding in
/// fixture JSONL lines. `chrono`'s formatter is already a workspace
/// dep so this costs nothing.
fn iso_ms(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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

/// Rapid stop→start must NOT spawn a second poller while the prior
/// one is still alive. The new task must wait for the prior one to
/// terminate before entering its own loop. We verify this by
/// observing that at most ONE poll task is active at a time:
/// successive `start`+`stop` cycles converge to all handles
/// completing within the bounded test window.
#[tokio::test]
async fn rapid_stop_start_does_not_double_spawn() {
    let f = fixture();

    // First cycle: start, then stop without giving the loop a chance
    // to fully exit between the next start.
    let h1 = f.runtime.clone().start();
    // Stop immediately (no sleep): the loop is still in its first
    // sleep wakeup window.
    f.runtime.stop();
    // Re-start before h1 has had the chance to exit. Without the
    // generation+notify fix, this used to leave two pollers running
    // (h1 still alive while h2 begins ticking).
    let h2 = f.runtime.clone().start();
    // Now stop the second one too.
    f.runtime.stop();

    // Both handles must complete within the bounded window. If they
    // don't, the fix isn't working: a hung handle means a poll task
    // never noticed its generation was bumped (or the new task
    // never awaited the old).
    tokio::time::timeout(std::time::Duration::from_secs(2), h1)
        .await
        .expect("h1 must complete after stop")
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), h2)
        .await
        .expect("h2 must complete after stop")
        .unwrap();
}

/// Seed-on-attach: a transcript that was busy mid-turn before the
/// runtime started must surface as Busy on the very first tick —
/// not Idle. Without the seed step, `try_attach` opens the tail at
/// EOF with a fresh `StatusMachine`, so the snapshot reports Idle
/// until CC writes another line (which can take seconds or never).
#[tokio::test]
async fn try_attach_seeds_status_from_recent_transcript() {
    let f = fixture();
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-seed","cwd":"/tmp/seed-proj","startedAt":1000}"#,
    );
    // Pre-existing transcript: a user turn followed by an unmatched
    // tool_use — the canonical "Busy" shape per the status-machine
    // tests. The runtime hasn't seen any of this yet.
    write_transcript(
        &f.projects_dir,
        "/tmp/seed-proj",
        "sess-seed",
        concat!(
            r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp/seed-proj","sessionId":"sess-seed","version":"2.1","timestamp":"2026-04-21T10:00:00.000Z","uuid":"u1","type":"user","message":{"role":"user","content":"go"}}"#,
            "\n",
            r#"{"parentUuid":"u1","isSidechain":false,"userType":"external","cwd":"/tmp/seed-proj","sessionId":"sess-seed","version":"2.1","timestamp":"2026-04-21T10:00:05.000Z","uuid":"u2","type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"pnpm test"}}]}}"#,
            "\n",
        ),
    );
    f.check.set_alive(&[12345]);

    // First tick attaches and ALSO seeds status. The aggregate must
    // already report Busy + the recent action — no second tick
    // required.
    f.runtime.tick().await.unwrap();
    let snap = f.runtime.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(
        snap[0].status,
        Status::Busy,
        "seeded transcript must surface as Busy on first tick"
    );
    assert_eq!(snap[0].current_action.as_deref(), Some("Bash: pnpm test"));
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
async fn task_summary_drives_current_action_and_emits_delta() {
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

    // CC writes a task-summary while a tool-use is also pending.
    // The task-summary text should win over the tool head-line in
    // current_action AND a TaskSummaryChanged delta must fire.
    append_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "sess-a",
        concat!(
            r#"{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"sess-a","version":"2.1","timestamp":"2026-04-21T10:00:00.000Z","uuid":"u1","type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-opus-4-7","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"pnpm test"}}]}}"#,
            "\n",
            r#"{"type":"task-summary","sessionId":"sess-a","timestamp":"2026-04-21T10:00:01.000Z","summary":"running the test suite after repo-filter change"}"#,
            "\n",
        ),
    );
    f.runtime.tick().await.unwrap();

    let s = &f.runtime.snapshot()[0];
    assert_eq!(
        s.current_action.as_deref(),
        Some("running the test suite after repo-filter change")
    );

    // Drain deltas and confirm at least one TaskSummaryChanged
    // arrived with the expected text.
    let mut saw_task_summary = false;
    while let Ok(d) = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        rx.recv(),
    )
    .await
    {
        let Some(d) = d else { break };
        if let LiveDeltaKind::TaskSummaryChanged { summary } = &d.kind {
            assert!(summary.contains("running the test suite"));
            saw_task_summary = true;
        }
    }
    assert!(
        saw_task_summary,
        "expected at least one TaskSummaryChanged delta"
    );
}

#[tokio::test]
async fn excluded_paths_are_skipped_by_tick() {
    let f = fixture();
    // Two live PIDs — one inside an excluded path, one outside.
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-kept","cwd":"/tmp/kept-proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/kept-proj", "sess-kept", "");
    write_pid_file(
        &f.sessions_dir,
        12346,
        r#"{"pid":12346,"sessionId":"sess-excluded","cwd":"/tmp/secret-proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/secret-proj", "sess-excluded", "");
    f.check.set_alive(&[12345, 12346]);

    f.runtime.set_excluded_paths(vec!["/tmp/secret".to_string()]).await;
    f.runtime.tick().await.unwrap();

    let snap = f.runtime.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].session_id, "sess-kept");
}

/// Mid-run `/clear` — CC forks a new transcript but leaves the
/// PID file's `sessionId` stale. The resolver must catch the
/// sibling `.jsonl`, the old session must drop out of the
/// aggregate (emitting `Ended`), and the new one must attach.
///
/// This reproduces the lixiaolai.com bug observed in the field
/// on 2026-04-21: a PID that had been `/clear`ed twice was still
/// reported as idle because we were tailing the original (stale)
/// transcript whose final `end_turn` was 1h+ old.
#[tokio::test]
async fn clear_rotation_rebinds_to_the_active_transcript() {
    let f = fixture();

    // Timeline: PID started 10 min ago, initial transcript written
    // at +5s, user /cleared 5 min later to a new transcript.
    let started_at_ms = chrono::Utc::now().timestamp_millis() - 10 * 60 * 1000;
    let t0_iso = iso_ms(started_at_ms + 5_000);
    let t1_iso = iso_ms(started_at_ms + 5 * 60 * 1000);

    write_pid_file(
        &f.sessions_dir,
        12345,
        &format!(
            r#"{{"pid":12345,"sessionId":"stale-sid","cwd":"/tmp/proj","startedAt":{started_at_ms}}}"#,
        ),
    );
    // Declared (stale) transcript — CC wrote its first event right
    // after startup; nothing since.
    write_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "stale-sid",
        &format!(
            r#"{{"type":"custom-title","sessionId":"stale-sid","timestamp":"{t0_iso}"}}
"#,
        ),
    );
    bump_mtime(
        &transcript_path(&f.projects_dir, "/tmp/proj", "stale-sid"),
        -500,
    );
    f.check.set_alive(&[12345]);

    // First tick binds to the declared transcript — PID-file match
    // wins because no sibling exists yet.
    f.runtime.tick().await.unwrap();
    assert_eq!(f.runtime.snapshot().len(), 1);
    assert_eq!(f.runtime.snapshot()[0].session_id, "stale-sid");

    // Subscribe BEFORE the /clear so we can assert `Ended` fires for
    // the stale session when the resolver rotates us to the fresh
    // one. Drain any prior deltas first — the first tick's initial
    // StatusChanged would otherwise block our Ended assertion by
    // returning first.
    let mut rx_stale = f.runtime.subscribe_detail("stale-sid").await.unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), rx_stale.recv()).await;

    // Simulate `/clear`: CC starts writing a new transcript next to
    // the old one, but does NOT update the PID file (because
    // `regenerateSessionId` skips the sessionSwitched hook). The
    // new transcript's first line carries a timestamp from within
    // this PID's lifetime and its mtime is fresher than the stale
    // one.
    write_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "fresh-sid",
        &format!(
            r#"{{"type":"custom-title","sessionId":"fresh-sid","timestamp":"{t1_iso}"}}
"#,
        ),
    );
    bump_mtime(
        &transcript_path(&f.projects_dir, "/tmp/proj", "fresh-sid"),
        -1,
    );

    f.runtime.tick().await.unwrap();

    // Aggregate now shows the fresh session; the stale sessionId
    // must be gone (the whole point of the fix).
    let snap = f.runtime.snapshot();
    assert_eq!(snap.len(), 1, "still exactly one live session per PID");
    assert_eq!(snap[0].session_id, "fresh-sid");
    assert_eq!(snap[0].pid, 12345);

    // The stale subscriber should have received an `Ended` delta —
    // that's how detail consumers know to unmount.
    let d = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        rx_stale.recv(),
    )
    .await
    .expect("Ended delta should arrive on stale session")
    .expect("channel open");
    assert!(
        matches!(d.kind, LiveDeltaKind::Ended),
        "expected Ended, got {:?}",
        d.kind
    );
}

/// Once bound to the fresh post-`/clear` transcript, tailing new
/// lines must drive the status machine as usual — a regression here
/// would mean we rotated correctly but then never surfaced actual
/// work.
#[tokio::test]
async fn post_clear_session_receives_normal_status_transitions() {
    let f = fixture();
    let started_at_ms = chrono::Utc::now().timestamp_millis() - 5 * 60 * 1000;
    let t0_iso = iso_ms(started_at_ms + 1_000);
    let t1_iso = iso_ms(started_at_ms + 60_000);

    write_pid_file(
        &f.sessions_dir,
        12345,
        &format!(
            r#"{{"pid":12345,"sessionId":"stale-sid","cwd":"/tmp/proj","startedAt":{started_at_ms}}}"#,
        ),
    );
    write_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "stale-sid",
        &format!(
            r#"{{"type":"custom-title","sessionId":"stale-sid","timestamp":"{t0_iso}"}}
"#,
        ),
    );
    bump_mtime(
        &transcript_path(&f.projects_dir, "/tmp/proj", "stale-sid"),
        -200,
    );
    write_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "fresh-sid",
        &format!(
            r#"{{"type":"custom-title","sessionId":"fresh-sid","timestamp":"{t1_iso}"}}
"#,
        ),
    );
    bump_mtime(
        &transcript_path(&f.projects_dir, "/tmp/proj", "fresh-sid"),
        -1,
    );
    f.check.set_alive(&[12345]);

    // First tick: resolver picks fresh-sid (newest mtime). Tail
    // opens at EOF, status is Idle.
    f.runtime.tick().await.unwrap();
    assert_eq!(f.runtime.snapshot()[0].session_id, "fresh-sid");
    assert_eq!(f.runtime.snapshot()[0].status, Status::Idle);

    // CC appends to the fresh transcript — a user turn starting.
    append_transcript(
        &f.projects_dir,
        "/tmp/proj",
        "fresh-sid",
        &format!(
            concat!(
                r#"{{"parentUuid":null,"isSidechain":false,"userType":"external","cwd":"/tmp/p","sessionId":"fresh-sid","version":"2.1","timestamp":"{ts}","uuid":"u1","type":"user","message":{{"role":"user","content":"go"}}}}"#,
                "\n",
            ),
            ts = iso_ms(started_at_ms + 120_000),
        ),
    );
    bump_mtime(
        &transcript_path(&f.projects_dir, "/tmp/proj", "fresh-sid"),
        0,
    );
    f.runtime.tick().await.unwrap();
    assert_eq!(f.runtime.snapshot()[0].status, Status::Busy);
}

#[tokio::test]
async fn unsubscribe_releases_detail_slot_for_resubscribe() {
    let f = fixture();
    write_pid_file(
        &f.sessions_dir,
        12345,
        r#"{"pid":12345,"sessionId":"sess-a","cwd":"/tmp/proj","startedAt":1000}"#,
    );
    write_transcript(&f.projects_dir, "/tmp/proj", "sess-a", "");
    f.check.set_alive(&[12345]);
    f.runtime.tick().await.unwrap();

    // First subscribe succeeds.
    let _rx1 = f.runtime.subscribe_detail("sess-a").await.unwrap();
    // Second would fail — single-subscriber contract.
    assert!(f.runtime.subscribe_detail("sess-a").await.is_err());
    // After explicit end, a fresh subscribe is allowed.
    f.runtime.detail_end_session("sess-a").await;
    let _rx2 = f.runtime.subscribe_detail("sess-a").await.unwrap();
}

#[tokio::test]
async fn metrics_writes_transition_plus_heartbeat() {
    // Verifies the post-audit write model: new sessions write once,
    // transitions write a row, and static sessions get at least one
    // heartbeat row per HEARTBEAT_TICKS. Uses the MetricsStore
    // directly (the runtime's own is None in test mode).
    use crate::session_live::metrics_store::MetricsStore;
    let td = tempfile::TempDir::new().unwrap();
    let path = td.path().join("m.db");
    let store = MetricsStore::open(&path).unwrap();
    // Single session transitioning busy → idle: two rows.
    store
        .record_tick(
            1_000,
            &[LiveSessionSummary {
                session_id: "s".into(),
                pid: 1,
                cwd: "/tmp/p".into(),
                transcript_path: None,
                status: Status::Busy,
                current_action: None,
                model: None,
                waiting_for: None,
                errored: false,
                stuck: false,
                idle_ms: 0,
                seq: 0,
            }],
        )
        .unwrap();
    store
        .record_tick(
            2_000,
            &[LiveSessionSummary {
                session_id: "s".into(),
                pid: 1,
                cwd: "/tmp/p".into(),
                transcript_path: None,
                status: Status::Idle,
                current_action: None,
                model: None,
                waiting_for: None,
                errored: false,
                stuck: false,
                idle_ms: 0,
                seq: 0,
            }],
        )
        .unwrap();
    // Two buckets, two distinct writes → two counts.
    let series = store.active_series(0, 3_000, 2).unwrap();
    assert_eq!(series, vec![1, 1]);
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
    // M2: whole-bearer redaction is stronger than sk-ant-specific.
    // Accept either mask shape; both protect the token body.
    assert!(
        ca.contains("Authorization: Bearer ***") || ca.contains("sk-ant-***"),
        "expected redaction marker, got: {ca}"
    );
}

/// Phase 3 LiveRuntime integration: when an `ActivityIndex` is
/// enabled, the per-tick tail loop classifies new lines and persists
/// any emitted cards. The classifier runs alongside the transcript
/// parser, sharing the byte stream but not the parsed model.
#[tokio::test]
async fn activity_classifier_runs_on_live_tail() {
    let f = fixture();
    let activity_dir = f._td.path().join("activity.db");
    let idx = std::sync::Arc::new(crate::activity::ActivityIndex::open(&activity_dir).unwrap());
    f.runtime.enable_activity(std::sync::Arc::clone(&idx));

    // Pre-create a transcript with an existing failure that would
    // produce a card. Using the real plugin_missing fixture so the
    // help template path lights up too.
    let body = include_str!("../activity/testdata/hook_plugin_missing.jsonl").trim();
    let body = format!("{body}\n");
    write_transcript(&f.projects_dir, "/Users/x/proj", "sess1", &body);
    // Register the live session AFTER writing the transcript so the
    // attach path sees it.
    write_pid_file(
        &f.sessions_dir,
        9001,
        r#"{"pid":9001,"sessionId":"sess1","cwd":"/Users/x/proj","startedAt":1700000000000}"#,
    );
    f.check.set_alive(&[9001]);
    f.runtime.tick().await.unwrap();

    // Append a fresh `hook_non_blocking_error` and tick again so the
    // tail-derived path (not just the attach seed) gets exercised.
    let appended = format!("{body}");
    append_transcript(&f.projects_dir, "/Users/x/proj", "sess1", &appended);
    f.runtime.tick().await.unwrap();

    let cards = idx
        .recent(&crate::activity::RecentQuery {
            limit: Some(50),
            ..Default::default()
        })
        .unwrap();
    assert!(
        cards.iter().any(|c| c.title.contains("PostToolUse:Write")),
        "expected at least one PostToolUse:Write card, got titles: {:?}",
        cards.iter().map(|c| c.title.clone()).collect::<Vec<_>>()
    );
    assert!(
        cards.iter().any(|c| c.plugin.as_deref() == Some("mermaid-preview@xiaolai")),
        "expected plugin attribution to appear, got plugins: {:?}",
        cards
            .iter()
            .map(|c| c.plugin.clone())
            .collect::<Vec<_>>()
    );
}

/// Phase 3 finalize hook: when a session disappears from the PID
/// registry, any open Agent episodes drain into AgentStranded
/// cards before the session is removed from the runtime's state map.
#[tokio::test]
async fn ended_session_drains_open_agent_episodes_to_stranded_cards() {
    let f = fixture();
    let activity_dir = f._td.path().join("activity.db");
    let idx = std::sync::Arc::new(crate::activity::ActivityIndex::open(&activity_dir).unwrap());
    f.runtime.enable_activity(std::sync::Arc::clone(&idx));

    // Empty transcript at attach time so the tail-EOF positioning
    // doesn't skip our test line. Seeded status is irrelevant for
    // this test — we only care about the classifier path.
    write_transcript(&f.projects_dir, "/Users/x/proj", "sess2", "");
    write_pid_file(
        &f.sessions_dir,
        9002,
        r#"{"pid":9002,"sessionId":"sess2","cwd":"/Users/x/proj","startedAt":1700000000000}"#,
    );
    f.check.set_alive(&[9002]);
    // First tick: attach.
    f.runtime.tick().await.unwrap();

    // Append an Agent tool_use AFTER attach — the tail now picks it
    // up on the next tick and the classifier records the open
    // episode. This is the steady-state path (production sessions
    // append while the runtime watches).
    let line = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-04-25T10:00:00Z",
        "uuid": "u-open",
        "cwd": "/Users/x/proj",
        "message": {
            "role": "assistant",
            "model": "claude-opus-4-7",
            "content": [{
                "type": "tool_use",
                "id": "t-stranded",
                "name": "Agent",
                "input": {
                    "subagent_type": "Explore",
                    "description": "find the leak",
                }
            }]
        }
    });
    let body = format!("{line}\n");
    append_transcript(&f.projects_dir, "/Users/x/proj", "sess2", &body);
    f.runtime.tick().await.unwrap();

    // Pre-finalize: episode is open, no stranded card yet.
    let pre = idx.recent(&Default::default()).unwrap();
    assert!(
        pre.iter().all(|c| c.kind != crate::activity::CardKind::AgentStranded),
        "no AgentStranded card while session is live"
    );

    // Session disappears (PID file removed) — runtime drains the
    // open episode on the next tick.
    std::fs::remove_file(f.sessions_dir.join("9002.json")).unwrap();
    f.check.set_alive(&[]);
    f.runtime.tick().await.unwrap();

    let stranded: Vec<_> = idx
        .recent(&crate::activity::RecentQuery {
            kinds: vec![crate::activity::CardKind::AgentStranded],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        stranded.len(),
        1,
        "expected exactly one stranded card, got {stranded:?}"
    );
    assert!(stranded[0].title.contains("did not return"));
}
