//! Session move — relocate a single session transcript from one project
//! cwd to another, with all CC-adjacent surfaces kept consistent.
//!
//! # Primary use case: orphan adoption
//!
//! When a git worktree is deleted (and pruned), sessions that were
//! created inside it become orphaned in three ways at once:
//!
//!   1. Their on-disk slug (`~/.claude/projects/-tmp-wt-foo`) points at
//!      a directory that no longer exists.
//!   2. Every JSONL line inside carries `cwd: "/tmp/wt-foo"` — a dead
//!      path that `--resume` will `cd` into and fail.
//!   3. CC's `resolveSessionFilePath` worktree fallback
//!      (`sessionStoragePortable.ts:425`) consults `git worktree list`,
//!      which no longer mentions the pruned worktree — so the session
//!      is effectively invisible from the main repo.
//!
//! `adopt_orphan_project` moves every session in a dead project's dir
//! into a live target (usually the main worktree's cwd), rewriting
//! every surface CC reads.
//!
//! # Surface map (verified against CC v2.1.88 source)
//!
//! | # | Surface                                                              | Action                              |
//! |---|----------------------------------------------------------------------|-------------------------------------|
//! | 1 | `<projects>/<slug_from>/<S>.jsonl` → `<projects>/<slug_to>/<S>.jsonl` | move + rewrite every line's `cwd`   |
//! | 2 | `<projects>/<slug_from>/<S>/{subagents,remote-agents}/**`            | move whole `<S>/` dir to `<slug_to>`|
//! | 3 | `~/.claude/history.jsonl` lines where `sessionId == S`               | rewrite `project` field             |
//! | 4 | `~/.claude.json → projects[from_cwd].lastSessionId`                  | clear if `== S`                     |
//! | 5 | `~/.claude.json → projects[from_cwd].activeWorktreeSession.sessionId`| clear if `== S`                     |
//! | 6 | source `<slug_from>/` dir                                            | remove if empty after move          |
//!
//! Untouched (all top-level + session-keyed in CC, not per-project):
//!   `~/.claude/file-history/<S>/`, `~/.claude/tasks/<S>/`,
//!   `~/.claude/paste-cache/`, `~/.claude/shell-snapshots/`,
//!   `~/.claude/todos/` (dead in current CC).

use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MoveSessionOpts {
    /// Proceed even if the source JSONL's mtime is within the live-session
    /// threshold. Default false — refuse to move a session CC may be writing.
    pub force_live_session: bool,
    /// Proceed even if a Syncthing `.sync-conflict-*.jsonl` sibling exists.
    /// Default false — refuse to silently discard conflict state.
    pub force_sync_conflict: bool,
    /// After a successful move, remove the source project dir if it now
    /// contains no JSONL files and no session subdirs.
    pub cleanup_source_if_empty: bool,
}

#[derive(Debug, Default, Clone)]
pub struct MoveSessionReport {
    pub session_id: Option<Uuid>,
    pub from_slug: String,
    pub to_slug: String,
    /// Count of lines whose `cwd` field was rewritten in the primary
    /// session JSONL. Matches total line count for sessions whose every
    /// line stayed in one cwd (the normal case).
    pub jsonl_lines_rewritten: usize,
    /// Count of files moved from `<slug_from>/<S>/subagents/**`.
    pub subagent_files_moved: usize,
    /// Count of files moved from `<slug_from>/<S>/remote-agents/**`.
    pub remote_agent_files_moved: usize,
    /// Count of history.jsonl lines whose `project` field was rewritten
    /// (lines where `sessionId == S` AND `project == canonical(from_cwd)`).
    pub history_entries_moved: usize,
    /// Count of history.jsonl lines that likely belong to this session
    /// but could not be attributed — typically pre-sessionId CC versions.
    /// Left as-is.
    pub history_entries_unmapped: usize,
    /// 0, 1, or 2: how many of the two possible `.claude.json` session
    /// pointers were pointing at this session and had to be cleared.
    pub claude_json_pointers_cleared: u8,
    /// True iff `opts.cleanup_source_if_empty` and the source dir was empty.
    pub source_dir_removed: bool,
}

#[derive(Debug, Error)]
pub enum MoveSessionError {
    #[error("session {0} not found under project dir {1:?}")]
    SessionNotFound(Uuid, PathBuf),

    #[error("sync-conflict sibling present for session {0} — resolve manually or pass force_sync_conflict")]
    SyncConflictPresent(Uuid),

    #[error("session {0} appears live (mtime < threshold) — quit Claude Code first or pass force_live_session")]
    LiveSession(Uuid),

    #[error("target slug already contains a file for session {0}")]
    TargetCollision(Uuid),

    #[error("from_cwd and to_cwd canonicalize to the same path")]
    SameCwd,

    #[error("source cwd {0:?} is still a live git worktree of target — CC already handles cross-worktree resume, no move needed")]
    WorktreeSiblingStillLive(PathBuf),

    #[error("config_dir does not exist or is unreadable: {0:?}")]
    InvalidConfigDir(PathBuf),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Port of CC's `canonicalizePath` (`sessionStoragePortable.ts:339-345`).
/// Tries `realpath(p).normalize('NFC')`. On error (non-existent path,
/// permission denied), returns `p.normalize('NFC')`. **Does not require
/// the path to exist** — this is deliberate, because the primary caller
/// (orphan adoption) has a source cwd that is guaranteed not to exist.
pub fn canonicalize_cc_path(p: &Path) -> PathBuf {
    use unicode_normalization::UnicodeNormalization;
    let attempted = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    // Normalize the stringified path to NFC. Path → str is lossy for
    // non-UTF8 bytes on Unix; accept that lossy step because CC's input
    // comes from JavaScript strings (UTF-16) and is already UTF-8-clean.
    let normalized: String = attempted.to_string_lossy().nfc().collect();
    PathBuf::from(normalized)
}

/// Move a single session from `from_cwd`'s project dir to `to_cwd`'s.
/// See module docs for the full surface map. Idempotent: running the
/// same move twice returns `SessionNotFound` on the second call (the
/// file no longer exists in the source slug).
pub fn move_session(
    _config_dir: &Path,
    _session_id: Uuid,
    _from_cwd: &Path,
    _to_cwd: &Path,
    _opts: MoveSessionOpts,
) -> Result<MoveSessionReport, MoveSessionError> {
    todo!("see surface map in module docs")
}

/// A project directory whose internal `cwd` refers to a non-existent
/// directory and has no live-worktree escape hatch. The user's primary
/// cue that a move / adopt is warranted.
#[derive(Debug, Clone)]
pub struct OrphanedProject {
    /// The sanitized directory name under `<config_dir>/projects/`.
    pub slug: String,
    /// Canonical cwd extracted from the first JSONL line in the dir.
    /// `None` if the dir had no parseable JSONL (degenerate orphan).
    pub cwd_from_transcript: Option<PathBuf>,
    pub session_count: usize,
    pub total_size_bytes: u64,
    /// If detectable, a reasonable adoption target — typically the main
    /// worktree of the repo the dead cwd used to be a worktree of, or
    /// the nearest existing ancestor directory.
    pub suggested_adoption_target: Option<PathBuf>,
}

pub fn detect_orphaned_projects(
    _config_dir: &Path,
) -> Result<Vec<OrphanedProject>, MoveSessionError> {
    todo!("scan projects/, stat each internal cwd, cross-check git worktree list")
}

#[derive(Debug, Default, Clone)]
pub struct AdoptReport {
    pub sessions_attempted: usize,
    pub sessions_moved: usize,
    pub sessions_failed: Vec<(Uuid, String)>,
    pub source_dir_removed: bool,
    pub per_session: Vec<MoveSessionReport>,
}

/// Move every session under `<config_dir>/projects/<orphan_slug>/` into
/// `target_cwd`'s project dir. Returns aggregate counts plus per-session
/// detail. Refuses if the orphan slug still resolves to a live directory
/// (not actually orphaned) or if `target_cwd` is the same as the orphan's
/// cwd.
pub fn adopt_orphan_project(
    _config_dir: &Path,
    _orphan_slug: &str,
    _target_cwd: &Path,
) -> Result<AdoptReport, MoveSessionError> {
    todo!("iterate *.jsonl in orphan slug, call move_session for each")
}

// ---------------------------------------------------------------------------
// Tests — the failing matrix.
//
// Stubs above all `todo!()`, so every behavioral test panics at runtime
// and `cargo test` reports them red. Parity tests for `canonicalize_cc_path`
// use the real implementation because it's a <10 line port.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
            serde_json::from_str(&fs::read_to_string(self.claude_json_path()).unwrap())
                .unwrap()
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

        let report = move_session(
            f.config_dir(),
            sid,
            &from,
            &to,
            MoveSessionOpts::default(),
        )
        .expect("happy-path move should succeed");

        assert_eq!(report.jsonl_lines_rewritten, 5);
        assert!(
            !original.exists(),
            "source JSONL should be gone after move"
        );
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

        let report = move_session(
            f.config_dir(),
            sid,
            &from,
            &to,
            MoveSessionOpts::default(),
        )
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

        move_session(
            f.config_dir(),
            sid,
            &from,
            &to,
            MoveSessionOpts::default(),
        )
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

        let err = move_session(
            f.config_dir(),
            sid,
            &from,
            &to,
            MoveSessionOpts::default(),
        )
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

        let err = move_session(
            f.config_dir(),
            sid,
            &from,
            &to,
            MoveSessionOpts::default(),
        )
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

        let err = move_session(
            f.config_dir(),
            sid,
            &from,
            &to,
            MoveSessionOpts::default(),
        )
        .expect_err("must refuse when target already has this session");

        assert!(
            matches!(err, MoveSessionError::TargetCollision(got) if got == sid),
            "got: {err}"
        );
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

        move_session(
            f.config_dir(),
            sid,
            &from,
            &to,
            MoveSessionOpts::default(),
        )
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
            format!(r#"{{"display":"p2","timestamp":2,"project":"{from_s}","sessionId":"{other_sid}"}}"#),
            format!(r#"{{"display":"p3","timestamp":3,"project":"{from_s}"}}"#),
            format!(r#"{{"display":"p4","timestamp":4,"project":"{from_s}","sessionId":"{sid}"}}"#),
        ];
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        f.write_history(&line_refs);

        let report = move_session(
            f.config_dir(),
            sid,
            &from,
            &to,
            MoveSessionOpts::default(),
        )
        .expect("history rewrite");

        assert_eq!(report.history_entries_moved, 2, "p1 and p4");

        let after = f.read_history();
        let to_s = to.to_string_lossy();
        // p1 and p4 now carry target project
        assert_eq!(
            after.matches(&format!(r#""project":"{to_s}""#)).count(),
            2
        );
        // p2 (other session) and p3 (sid-less) still carry source project
        assert_eq!(
            after.matches(&format!(r#""project":"{from_s}""#)).count(),
            2
        );
        // Relative order of lines must be preserved (history is read
        // newest-first but lines have their own timestamps; stable order
        // avoids confusing the Up-arrow reader).
        let positions: Vec<usize> = ["p1", "p2", "p3", "p4"]
            .iter()
            .map(|tag| after.find(tag).unwrap_or(usize::MAX))
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "history line order must be preserved: {positions:?}"
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
            MoveSessionOpts::default(),
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
        assert_eq!(v["projects"][&to_key]["lastSessionId"], preserved_sid.to_string());

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
            MoveSessionOpts::default(),
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

        let orphans = detect_orphaned_projects(f.config_dir())
            .expect("orphan detection should succeed");

        assert_eq!(orphans.len(), 1, "exactly one orphan expected");
        let o = &orphans[0];
        assert_eq!(o.cwd_from_transcript, Some(dead_cwd.clone()));
        assert_eq!(o.session_count, 2);
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

        let from_slug = crate::project_sanitize::sanitize_path(
            &dead_cwd.to_string_lossy(),
        );

        let report = adopt_orphan_project(f.config_dir(), &from_slug, &target)
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
}
