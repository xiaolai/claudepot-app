//! Inline test module for `session_move.rs`. Lives in this sibling file
//! so `session_move.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "session_move_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;
use std::fs;
use std::io::Write;
use std::time::{Duration, SystemTime};

// -----------------------------------------------------------------------
// Fixture: fake `~/.claude` + fake work dirs
// -----------------------------------------------------------------------

struct Fixture {
    /// Fake `~/.claude`.
    config: tempfile::TempDir,
    /// Fake working directory root. Subdirs under here stand in for
    /// live project cwds. Separate from `config` so the two can be
    /// located on different filesystems in principle (matches prod).
    work: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let f = Self {
            config: tempfile::tempdir().expect("tempdir"),
            work: tempfile::tempdir().expect("tempdir"),
        };
        fs::create_dir_all(f.projects_dir()).unwrap();
        f
    }

    fn config_dir(&self) -> &Path {
        self.config.path()
    }

    fn projects_dir(&self) -> PathBuf {
        self.config.path().join("projects")
    }

    fn history_jsonl_path(&self) -> PathBuf {
        self.config.path().join("history.jsonl")
    }

    fn claude_json_path(&self) -> PathBuf {
        // CC stores this as a sibling to `.claude/`, not inside. We
        // colocate for test simplicity and expect move_session to
        // accept an explicit path (mirrors `MoveArgs.claude_json_path`).
        self.config.path().join("claude.json")
    }

    /// Create a live cwd under `work/` and return its canonical path.
    fn make_live_cwd(&self, name: &str) -> PathBuf {
        let p = self.work.path().join(name);
        fs::create_dir_all(&p).unwrap();
        canonicalize_cc_path(&p)
    }

    /// Return the expected slug dir for a cwd (does not create it).
    fn slug_dir(&self, cwd: &Path) -> PathBuf {
        use crate::project_sanitize::sanitize_path;
        self.projects_dir()
            .join(sanitize_path(&cwd.to_string_lossy()))
    }

    /// Create the slug dir for a cwd and return it.
    fn ensure_slug(&self, cwd: &Path) -> PathBuf {
        let s = self.slug_dir(cwd);
        fs::create_dir_all(&s).unwrap();
        s
    }

    /// Write a session JSONL with one `user`/`assistant` pair, both
    /// carrying `cwd` fields. Returns the written path.
    ///
    /// Ages the mtime to an hour ago so the live-session mtime guard
    /// lets the move through by default. Tests that specifically
    /// exercise the live-session refusal bump the mtime back to now.
    fn write_session(&self, cwd: &Path, sid: Uuid, line_count: usize) -> PathBuf {
        let slug = self.ensure_slug(cwd);
        let path = slug.join(format!("{sid}.jsonl"));
        let mut f = fs::File::create(&path).unwrap();
        let cwd_s = cwd.to_string_lossy();
        for i in 0..line_count {
            let ty = if i % 2 == 0 { "user" } else { "assistant" };
            writeln!(
                f,
                r#"{{"type":"{ty}","cwd":"{cwd_s}","sessionId":"{sid}","seq":{i}}}"#
            )
            .unwrap();
        }
        drop(f);
        self.set_mtime(&path, SystemTime::now() - Duration::from_secs(3600));
        path
    }

    /// Touch a file's mtime to a specific SystemTime.
    fn set_mtime(&self, p: &Path, when: SystemTime) {
        let ft = filetime::FileTime::from_system_time(when);
        filetime::set_file_mtime(p, ft).unwrap();
    }

    /// Write a per-session subdir with one subagent + one remote-agent file.
    fn write_session_sidecars(&self, cwd: &Path, sid: Uuid) {
        let slug = self.ensure_slug(cwd);
        let subagents = slug.join(sid.to_string()).join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(
            subagents.join("agent-foo.jsonl"),
            format!(
                r#"{{"cwd":"{}","agentId":"foo"}}{}"#,
                cwd.to_string_lossy(),
                "\n"
            ),
        )
        .unwrap();
        fs::write(
            subagents.join("agent-foo.meta.json"),
            r#"{"agentType":"general-purpose"}"#,
        )
        .unwrap();

        let remote = slug.join(sid.to_string()).join("remote-agents");
        fs::create_dir_all(&remote).unwrap();
        fs::write(
            remote.join("remote-agent-tsk.meta.json"),
            r#"{"taskId":"tsk","remoteTaskType":"review"}"#,
        )
        .unwrap();
    }

    fn write_history(&self, lines: &[&str]) {
        let path = self.history_jsonl_path();
        let mut f = fs::File::create(&path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    fn read_history(&self) -> String {
        fs::read_to_string(self.history_jsonl_path()).unwrap_or_default()
    }

    fn write_claude_json(&self, v: serde_json::Value) {
        fs::write(
            self.claude_json_path(),
            serde_json::to_string_pretty(&v).unwrap(),
        )
        .unwrap();
    }

    fn read_claude_json(&self) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(self.claude_json_path()).unwrap()).unwrap()
    }
}

// -----------------------------------------------------------------------
// Section A — canonicalize parity with CC
// -----------------------------------------------------------------------

#[test]
fn canonicalize_resolves_symlinks() {
    // On macOS, /tmp is a symlink to /private/tmp — the same case CC
    // cares about (sessionStoragePortable.ts:336-345 comment).
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("real");
    fs::create_dir(&target).unwrap();
    let link = tmp.path().join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&target, &link).unwrap();

    let got = canonicalize_cc_path(&link);
    let want = std::fs::canonicalize(&target).unwrap();
    assert_eq!(
        got,
        PathBuf::from(want.to_string_lossy().to_string()),
        "canonicalize_cc_path should resolve symlinks like CC does"
    );
}

#[test]
fn canonicalize_nonexistent_path_falls_back_to_nfc() {
    // Orphan-adoption case: source cwd is deleted, realpath will fail,
    // we must still return a usable normalized string (CC's behavior
    // per sessionStoragePortable.ts:341-343 try/catch).
    let ghost = Path::new("/nowhere/this/does/not/exist/whatsoever");
    let got = canonicalize_cc_path(ghost);
    assert_eq!(got, PathBuf::from(ghost));
}

#[test]
fn canonicalize_normalizes_nfd_to_nfc() {
    // "é" can be a single precomposed codepoint (NFC, U+00E9) or an
    // "e" + combining acute (NFD, U+0065 U+0301). macOS APFS uses
    // NFD natively; CC forces NFC for slug stability.
    let nfd = "cafe\u{0301}"; // "café" as e + combining acute
    let nfc_expected = "caf\u{00E9}";
    let got = canonicalize_cc_path(Path::new(nfd));
    assert_eq!(got.to_string_lossy(), nfc_expected);
}

// -----------------------------------------------------------------------
// Section B — move mechanics
// -----------------------------------------------------------------------

#[test]
fn move_session_happy_path_rewrites_cwd_in_every_line() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    let original = f.write_session(&from, sid, 5);

    let report = move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect("happy-path move should succeed");

    assert_eq!(report.jsonl_lines_rewritten, 5);
    assert!(!original.exists(), "source JSONL should be gone after move");
    let moved = f.slug_dir(&to).join(format!("{sid}.jsonl"));
    assert!(moved.exists(), "target JSONL should exist at {moved:?}");

    let contents = fs::read_to_string(&moved).unwrap();
    let to_s = to.to_string_lossy();
    let from_s = from.to_string_lossy();
    assert!(
        !contents.contains(&*from_s),
        "no line should still reference the old cwd"
    );
    assert_eq!(
        contents.matches(&*to_s).count(),
        5,
        "every one of the 5 lines should now carry the new cwd"
    );
}

#[test]
fn move_session_moves_subagent_and_remote_agent_subdirs() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    f.write_session(&from, sid, 2);
    f.write_session_sidecars(&from, sid);

    let report = move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect("move with sidecars");

    assert_eq!(report.subagent_files_moved, 2); // .jsonl + .meta.json
    assert_eq!(report.remote_agent_files_moved, 1);

    let from_sub = f.slug_dir(&from).join(sid.to_string());
    assert!(
        !from_sub.exists(),
        "source session subdir should be gone: {from_sub:?}"
    );
    let to_sub = f.slug_dir(&to).join(sid.to_string());
    assert!(to_sub.join("subagents").join("agent-foo.jsonl").exists());
    assert!(to_sub
        .join("subagents")
        .join("agent-foo.meta.json")
        .exists());
    assert!(to_sub
        .join("remote-agents")
        .join("remote-agent-tsk.meta.json")
        .exists());
}

#[test]
fn move_session_preserves_non_cwd_fields_byte_exact() {
    // The bar is intentionally high: a JSONL line with nested objects,
    // arrays, unicode, and multiple non-cwd fields should come out
    // the other side with ONLY the cwd string changed. The rewriter
    // must not reorder keys, drop unknown fields, or re-escape.
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    let from_s = from.to_string_lossy().to_string();

    // Build a line by hand so we control byte order exactly.
    let line = format!(
        r#"{{"parentUuid":"abc","type":"user","cwd":"{from}","sessionId":"{sid}","message":{{"role":"user","content":[{{"type":"text","text":"héllo — «world»"}}]}},"timestamp":"2026-04-18T22:31:00Z"}}"#,
        from = from_s,
        sid = sid
    );
    let slug = f.ensure_slug(&from);
    let path = slug.join(format!("{sid}.jsonl"));
    fs::write(&path, format!("{line}\n")).unwrap();
    // Age past the live-session guard (the fixture helper is
    // bypassed in this test, so we do it manually).
    f.set_mtime(&path, SystemTime::now() - Duration::from_secs(3600));

    move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect("byte-fidelity move");

    let moved = f.slug_dir(&to).join(format!("{sid}.jsonl"));
    let got = fs::read_to_string(&moved).unwrap();
    let expected = line.replace(&from_s, &to.to_string_lossy());
    assert_eq!(
        got.trim_end_matches('\n'),
        expected,
        "non-cwd fields must be preserved byte-for-byte"
    );
}

#[test]
fn move_session_refuses_on_sync_conflict_sibling() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    f.write_session(&from, sid, 1);
    // Syncthing conflict copy
    let conflict = f
        .slug_dir(&from)
        .join(format!("{sid}.sync-conflict-20260415-145538-NJCB7YU.jsonl"));
    fs::write(&conflict, "{}").unwrap();

    let err = move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect_err("must refuse when a sync-conflict sibling is present");

    assert!(
        matches!(err, MoveSessionError::SyncConflictPresent(got) if got == sid),
        "got: {err}"
    );
}

#[test]
fn move_session_refuses_when_source_mtime_is_live() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    let path = f.write_session(&from, sid, 1);
    // Bump mtime to "just now" — simulating an in-flight session.
    f.set_mtime(&path, SystemTime::now());

    let err = move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect_err("must refuse a live-session move without force flag");

    assert!(
        matches!(err, MoveSessionError::LiveSession(got) if got == sid),
        "got: {err}"
    );
}

#[test]
fn move_session_refuses_target_collision() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    f.write_session(&from, sid, 1);
    // Target already has a file with this sessionId — an alarming
    // state (probably two histories to reconcile); we should not
    // silently overwrite.
    f.write_session(&to, sid, 1);

    let err = move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect_err("must refuse when target already has this session");

    assert!(
        matches!(err, MoveSessionError::TargetCollision(got) if got == sid),
        "got: {err}"
    );
}

#[test]
fn move_session_rolls_back_target_on_phase_failure() {
    // Regression: when a post-Phase-1 step fails, we must unwind
    // the target JSONL — otherwise both source and target carry
    // the transcript and a retry trips `TargetCollision` on the
    // same uuid that previously failed mid-flight.
    //
    // We trigger a sidecar collision: a per-session file already
    // exists under the target's per-session subdir, which makes
    // Phase 2's `copy_tree_then_remove` refuse the merge.
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    f.write_session(&from, sid, 2);
    f.write_session_sidecars(&from, sid);

    // Pre-place a colliding sidecar in the target's session subdir.
    // The target dir lives on a different filesystem in principle,
    // but in tests we stay on tmpfs — so `move_session_subdir`
    // takes the rename branch when `to_sub` does NOT exist, and
    // the copy-then-merge branch when it does. Forcing the merge
    // path with a name collision exercises the failure leg.
    let to_slug = f.ensure_slug(&to);
    let to_sub_subagents = to_slug.join(sid.to_string()).join("subagents");
    fs::create_dir_all(&to_sub_subagents).unwrap();
    fs::write(
        to_sub_subagents.join("agent-foo.jsonl"),
        r#"{"agentId":"foo"}"#,
    )
    .unwrap();

    let err = move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect_err("phase-2 sidecar collision must surface as an error");
    assert!(
        matches!(err, MoveSessionError::SidecarCollision(_)),
        "got: {err}"
    );

    // Source must still hold the primary JSONL — Phase 5 must NOT
    // have run. Target's primary JSONL must NOT exist (rolled back).
    let from_session = f.slug_dir(&from).join(format!("{sid}.jsonl"));
    let to_session = f.slug_dir(&to).join(format!("{sid}.jsonl"));
    assert!(
        from_session.exists(),
        "source JSONL must be preserved on phase failure"
    );
    assert!(
        !to_session.exists(),
        "target JSONL must be rolled back on phase failure"
    );
    // The pre-existing target sidecar that triggered the failure
    // must NOT have been wiped by rollback — only this attempt's
    // newly-placed residue should be cleaned up.
    assert!(
        to_sub_subagents.join("agent-foo.jsonl").exists(),
        "rollback must not wipe pre-existing target sidecars"
    );

    // Retry must NOT trip TargetCollision (the rollback removed
    // the placeholder). Clear the colliding sidecar first so Phase
    // 2 can succeed on the second attempt.
    fs::remove_dir_all(to_slug.join(sid.to_string())).unwrap();
    move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect("retry after rollback must succeed");
}

#[test]
fn move_session_rejects_same_canonical_cwd() {
    let f = Fixture::new();
    let from = f.make_live_cwd("only-one");
    let sid = Uuid::new_v4();
    f.write_session(&from, sid, 1);

    let err = move_session(
        f.config_dir(),
        sid,
        &from,
        &from, // same
        MoveSessionOpts::default(),
    )
    .expect_err("from == to must be rejected");

    assert!(matches!(err, MoveSessionError::SameCwd), "got: {err}");
}

#[test]
fn move_session_accepts_aged_source_without_force() {
    // Counterpart to the live-mtime refusal — a session that was last
    // written >60s ago (realistic default threshold) must be movable.
    let f = Fixture::new();
    let from = f.make_live_cwd("old");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    let path = f.write_session(&from, sid, 1);
    let long_ago = SystemTime::now() - Duration::from_secs(60 * 60);
    f.set_mtime(&path, long_ago);

    move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect("aged session should move without force flag");
}

// -----------------------------------------------------------------------
// Section C — history.jsonl + .claude.json
// -----------------------------------------------------------------------

#[test]
fn move_session_rewrites_history_lines_by_session_id() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    let other_sid = Uuid::new_v4();
    f.write_session(&from, sid, 1);

    // Three kinds of history lines:
    //   1. Ours (sid matches, project matches) → REWRITE
    //   2. Other session in same project → LEAVE
    //   3. Ours but sid-less (pre-sessionId CC) → LEAVE, count unmapped
    let from_s = from.to_string_lossy();
    let lines = [
        format!(r#"{{"display":"p1","timestamp":1,"project":"{from_s}","sessionId":"{sid}"}}"#),
        format!(
            r#"{{"display":"p2","timestamp":2,"project":"{from_s}","sessionId":"{other_sid}"}}"#
        ),
        format!(r#"{{"display":"p3","timestamp":3,"project":"{from_s}"}}"#),
        format!(r#"{{"display":"p4","timestamp":4,"project":"{from_s}","sessionId":"{sid}"}}"#),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    f.write_history(&line_refs);

    let report = move_session(f.config_dir(), sid, &from, &to, MoveSessionOpts::default())
        .expect("history rewrite");

    assert_eq!(report.history_entries_moved, 2, "p1 and p4");

    let after = f.read_history();
    let to_s = to.to_string_lossy();
    // p1 and p4 now carry target project
    assert_eq!(after.matches(&format!(r#""project":"{to_s}""#)).count(), 2);
    // p2 (other session) and p3 (sid-less) still carry source project
    assert_eq!(
        after.matches(&format!(r#""project":"{from_s}""#)).count(),
        2
    );
    // Relative order of lines must be preserved (history is read
    // newest-first but lines have their own timestamps; stable order
    // avoids confusing the Up-arrow reader). We check the visible
    // `timestamp` sequence in the output rather than finding short
    // tag strings — the random tempdir path can contain any digit
    // by coincidence and produce spurious find-hits.
    let timestamps: Vec<i64> = after
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("timestamp").and_then(|t| t.as_i64()))
        .collect();
    assert_eq!(
        timestamps,
        vec![1, 2, 3, 4],
        "history line order must be preserved"
    );
}

#[test]
fn move_session_clears_last_session_id_in_claude_json() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    let preserved_sid = Uuid::new_v4();
    f.write_session(&from, sid, 1);

    let from_key = from.to_string_lossy().to_string();
    let to_key = to.to_string_lossy().to_string();
    f.write_claude_json(serde_json::json!({
        "numStartups": 1,
        "projects": {
            &from_key: {
                "lastSessionId": sid.to_string(),
                "lastCost": 1.23,
                "projectOnboardingSeenCount": 2
            },
            &to_key: {
                "lastSessionId": preserved_sid.to_string(),
                "projectOnboardingSeenCount": 1
            },
            "/unrelated/project": {
                "lastSessionId": sid.to_string(), // happens to match, different cwd
                "projectOnboardingSeenCount": 0
            }
        }
    }));

    let report = move_session(
        f.config_dir(),
        sid,
        &from,
        &to,
        MoveSessionOpts {
            claude_json_path: Some(f.claude_json_path()),
            ..Default::default()
        },
    )
    .expect("claude.json update");

    assert!(report.claude_json_pointers_cleared >= 1);

    let v = f.read_claude_json();
    let from_entry = &v["projects"][&from_key];
    assert!(
        from_entry["lastSessionId"].is_null(),
        "source project's lastSessionId must be cleared"
    );
    // Non-session sibling fields must survive untouched
    assert_eq!(from_entry["lastCost"], 1.23);
    assert_eq!(from_entry["projectOnboardingSeenCount"], 2);

    // Target project: lastSessionId untouched (see surface-map rule 8)
    assert_eq!(
        v["projects"][&to_key]["lastSessionId"],
        preserved_sid.to_string()
    );

    // Unrelated project: touching another cwd's lastSessionId would
    // be a correctness bug, even if the UUID happens to match.
    assert_eq!(
        v["projects"]["/unrelated/project"]["lastSessionId"],
        sid.to_string()
    );
}

#[test]
fn move_session_clears_active_worktree_session_pointer() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    f.write_session(&from, sid, 1);

    let from_key = from.to_string_lossy().to_string();
    f.write_claude_json(serde_json::json!({
        "projects": {
            &from_key: {
                "lastSessionId": "somebody-else",
                "activeWorktreeSession": {
                    "originalCwd": "/orig",
                    "worktreePath": from_key,
                    "worktreeName": "feat-x",
                    "sessionId": sid.to_string()
                },
                "projectOnboardingSeenCount": 0
            }
        }
    }));

    move_session(
        f.config_dir(),
        sid,
        &from,
        &to,
        MoveSessionOpts {
            claude_json_path: Some(f.claude_json_path()),
            ..Default::default()
        },
    )
    .expect("activeWorktreeSession clear");

    let v = f.read_claude_json();
    let active = &v["projects"][&from_key]["activeWorktreeSession"];
    assert!(
        active.is_null() || active["sessionId"].is_null(),
        "activeWorktreeSession.sessionId must be cleared (or the whole block nulled): {active}"
    );
}

// -----------------------------------------------------------------------
// Section C2 — progress sink contract
// -----------------------------------------------------------------------

/// Capture every phase + sub_progress event so tests can assert
/// the public phase contract (S1..S5) without binding to internal
/// timings.
#[derive(Default)]
struct RecordingSink {
    phases: std::sync::Mutex<Vec<(String, PhaseStatus)>>,
    subs: std::sync::Mutex<Vec<(String, usize, usize)>>,
}

impl ProgressSink for RecordingSink {
    fn phase(&self, phase: &str, status: PhaseStatus) {
        self.phases
            .lock()
            .unwrap()
            .push((phase.to_string(), status));
    }
    fn sub_progress(&self, phase: &str, done: usize, total: usize) {
        self.subs
            .lock()
            .unwrap()
            .push((phase.to_string(), done, total));
    }
}

#[test]
fn move_session_emits_expected_phases() {
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    f.write_session(&from, sid, 4);
    f.write_session_sidecars(&from, sid);
    // Seed history.jsonl so S3 isn't a no-op.
    let from_s = from.to_string_lossy();
    f.write_history(&[&format!(
        r#"{{"display":"p1","timestamp":1,"project":"{from_s}","sessionId":"{sid}"}}"#
    )]);
    // Provide claude.json so S4 actually runs.
    let from_key = from.to_string_lossy().to_string();
    f.write_claude_json(serde_json::json!({
        "projects": { &from_key: { "lastSessionId": sid.to_string(), "projectOnboardingSeenCount": 0 } }
    }));

    let sink = RecordingSink::default();
    move_session_with_progress(
        f.config_dir(),
        sid,
        &from,
        &to,
        MoveSessionOpts {
            cleanup_source_if_empty: true,
            claude_json_path: Some(f.claude_json_path()),
            ..Default::default()
        },
        &sink,
    )
    .expect("happy path with sink");

    let phases = sink.phases.lock().unwrap();
    let ids: Vec<&str> = phases.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        ids,
        vec!["S1", "S2", "S3", "S4", "S5"],
        "phase order must be S1→S2→S3→S4→S5"
    );
    for (id, status) in phases.iter() {
        assert!(
            matches!(status, PhaseStatus::Complete),
            "phase {id} should be Complete on happy path; got {status:?}"
        );
    }

    // S1 should have fired sub_progress at least once with the
    // line count as total.
    let subs = sink.subs.lock().unwrap();
    let s1_subs: Vec<_> = subs.iter().filter(|(p, _, _)| p == "S1").collect();
    assert!(
        !s1_subs.is_empty(),
        "S1 must emit at least one sub_progress"
    );
    let (_, _, total) = s1_subs.last().unwrap();
    assert_eq!(*total, 4, "S1 total should equal source line count");
}

#[test]
fn move_session_phase_error_propagates() {
    // Force a phase-2 (S2) error via a sidecar collision and
    // assert: (a) sink saw S2 error, (b) primary JSONL was rolled
    // back. Production parity with `move_session_rolls_back_target_on_phase_failure`,
    // but exercises the progress-sink contract.
    let f = Fixture::new();
    let from = f.make_live_cwd("feat-x");
    let to = f.make_live_cwd("main");
    let sid = Uuid::new_v4();
    f.write_session(&from, sid, 2);
    f.write_session_sidecars(&from, sid);

    // Pre-place a colliding sidecar so S2's copy-merge branch
    // refuses.
    let to_slug = f.ensure_slug(&to);
    let to_sub_subagents = to_slug.join(sid.to_string()).join("subagents");
    fs::create_dir_all(&to_sub_subagents).unwrap();
    fs::write(
        to_sub_subagents.join("agent-foo.jsonl"),
        r#"{"agentId":"foo"}"#,
    )
    .unwrap();

    let sink = RecordingSink::default();
    let err = move_session_with_progress(
        f.config_dir(),
        sid,
        &from,
        &to,
        MoveSessionOpts::default(),
        &sink,
    )
    .expect_err("S2 collision must surface as error");
    assert!(
        matches!(err, MoveSessionError::SidecarCollision(_)),
        "got: {err}"
    );

    // S1 should have completed; S2 should have errored.
    let phases = sink.phases.lock().unwrap();
    let s1 = phases.iter().find(|(p, _)| p == "S1").expect("S1 emitted");
    assert!(
        matches!(s1.1, PhaseStatus::Complete),
        "S1 should still be Complete: {:?}",
        s1.1
    );
    let s2 = phases.iter().find(|(p, _)| p == "S2").expect("S2 emitted");
    assert!(
        matches!(s2.1, PhaseStatus::Error(_)),
        "S2 should be Error: {:?}",
        s2.1
    );
    // S3..S5 must NOT have fired — once a phase errors the rest
    // are skipped.
    for skipped in ["S3", "S4", "S5"] {
        assert!(
            !phases.iter().any(|(p, _)| p == skipped),
            "{skipped} must not fire after S2 error"
        );
    }

    // Source primary JSONL preserved; target JSONL rolled back.
    let from_session = f.slug_dir(&from).join(format!("{sid}.jsonl"));
    let to_session = f.slug_dir(&to).join(format!("{sid}.jsonl"));
    assert!(from_session.exists(), "source must be preserved on error");
    assert!(!to_session.exists(), "target must be rolled back on error");
}

// -----------------------------------------------------------------------
// Section D — orphan detection + adoption
// -----------------------------------------------------------------------

#[test]
fn detect_orphaned_projects_flags_dead_cwd() {
    let f = Fixture::new();

    // Live project — existing cwd, should NOT be flagged.
    let live = f.make_live_cwd("live-project");
    f.write_session(&live, Uuid::new_v4(), 1);

    // Orphan project — cwd path does not exist on disk.
    let dead_cwd = PathBuf::from("/this/was/a/worktree/but/is/gone");
    f.write_session(&dead_cwd, Uuid::new_v4(), 1);
    f.write_session(&dead_cwd, Uuid::new_v4(), 1);

    let orphans =
        detect_orphaned_projects(f.config_dir()).expect("orphan detection should succeed");

    assert_eq!(orphans.len(), 1, "exactly one orphan expected");
    let o = &orphans[0];
    assert_eq!(o.cwd_from_transcript, Some(dead_cwd.clone()));
    assert_eq!(o.session_count, 2);
}

#[test]
fn adopt_orphan_project_rejects_traversal_slug() {
    // Library-level defense: slugs from CLI/Tauri are user input
    // that must NEVER be able to escape <config_dir>/projects/.
    // CC's real slugs are all `[A-Za-z0-9-]+`; anything else is
    // rejected regardless of whether the joined path happens to
    // exist on disk.
    let f = Fixture::new();
    let target = f.make_live_cwd("main");
    let bad_slugs = [
        "..",
        "../outside",
        "/etc",
        "a/b",
        "a\\b",
        "",
        ".",
        "foo\0bar",
        "has space",
        "has:colon",
    ];
    for slug in bad_slugs {
        let err = adopt_orphan_project(f.config_dir(), slug, &target, None)
            .expect_err(&format!("must reject slug {slug:?}"));
        let matched = matches!(&err, MoveSessionError::InvalidSlug(got, _) if got == slug);
        assert!(matched, "expected InvalidSlug for {slug:?}, got {err:?}");
    }
}

#[test]
fn adopt_orphan_project_moves_all_sessions_and_removes_empty_source() {
    let f = Fixture::new();
    let dead_cwd = PathBuf::from("/was/a/worktree/but/is/gone");
    let target = f.make_live_cwd("main");

    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    let sid_c = Uuid::new_v4();
    f.write_session(&dead_cwd, sid_a, 3);
    f.write_session(&dead_cwd, sid_b, 1);
    f.write_session(&dead_cwd, sid_c, 5);
    f.write_session_sidecars(&dead_cwd, sid_a);

    let from_slug = crate::project_sanitize::sanitize_path(&dead_cwd.to_string_lossy());

    let report = adopt_orphan_project(f.config_dir(), &from_slug, &target, None)
        .expect("adopt should succeed");

    assert_eq!(report.sessions_attempted, 3);
    assert_eq!(report.sessions_moved, 3);
    assert!(report.sessions_failed.is_empty());
    assert!(
        report.source_dir_removed,
        "empty source slug dir should be removed"
    );

    let to_slug = f.slug_dir(&target);
    for sid in [sid_a, sid_b, sid_c] {
        assert!(
            to_slug.join(format!("{sid}.jsonl")).exists(),
            "session {sid} must exist under target slug"
        );
    }
    assert!(to_slug.join(sid_a.to_string()).join("subagents").exists());
    assert!(
        !f.projects_dir().join(&from_slug).exists(),
        "orphan slug dir must be removed after adopt"
    );
}

#[test]
fn discard_orphan_project_rejects_traversal_slug() {
    // Same guard as adopt — the slug is untrusted input from the UI
    // and must never be allowed to escape <config_dir>/projects/.
    let f = Fixture::new();
    let bad_slugs = [
        "..",
        "../outside",
        "/etc",
        "a/b",
        "a\\b",
        "",
        ".",
        "foo\0bar",
        "has space",
        "has:colon",
    ];
    for slug in bad_slugs {
        let err = discard_orphan_project_with(f.config_dir(), slug, |_| {
            panic!("remove must not be called for invalid slug {slug:?}")
        })
        .expect_err(&format!("must reject slug {slug:?}"));
        let matched = matches!(&err, MoveSessionError::InvalidSlug(got, _) if got == slug);
        assert!(matched, "expected InvalidSlug for {slug:?}, got {err:?}");
    }
}

#[test]
fn discard_orphan_project_reports_counts_and_removes_dir() {
    let f = Fixture::new();
    let dead_cwd = PathBuf::from("/was/a/worktree/but/is/gone");
    let sid_a = Uuid::new_v4();
    let sid_b = Uuid::new_v4();
    f.write_session(&dead_cwd, sid_a, 3);
    f.write_session(&dead_cwd, sid_b, 5);
    f.write_session_sidecars(&dead_cwd, sid_a);

    let slug = crate::project_sanitize::sanitize_path(&dead_cwd.to_string_lossy());
    let slug_dir = f.projects_dir().join(&slug);
    assert!(slug_dir.is_dir(), "sanity: slug dir must exist");
    let expected_size: u64 = fs::read_dir(&slug_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
                .then(|| fs::metadata(&p).map(|m| m.len()).unwrap_or(0))
        })
        .sum();

    // Inject a plain remove_dir_all so we don't pollute the real OS
    // Trash during tests. The trashing path itself is covered by the
    // `trash` crate's own integration tests + a single run-on-macOS
    // smoke test we do manually.
    let report = discard_orphan_project_with(f.config_dir(), &slug, |p| {
        fs::remove_dir_all(p).map_err(Into::into)
    })
    .expect("discard should succeed");

    assert_eq!(report.sessions_discarded, 2);
    assert_eq!(report.total_size_bytes, expected_size);
    assert!(report.dir_removed, "slug dir must be gone post-discard");
    assert!(!slug_dir.exists(), "slug dir must not exist on disk");
}

#[test]
fn discard_orphan_project_errors_when_slug_dir_missing() {
    // A well-formed slug whose directory doesn't exist returns
    // InvalidConfigDir — the UI can surface this as "already gone".
    let f = Fixture::new();
    let err = discard_orphan_project(f.config_dir(), "never-existed")
        .expect_err("missing slug dir must error");
    matches!(err, MoveSessionError::InvalidConfigDir(_));
}
