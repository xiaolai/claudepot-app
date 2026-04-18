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

use crate::project_sanitize::sanitize_path;
use crate::session_move_helpers::{
    clear_claude_json_session_pointers, extract_session_id_from_path, has_sync_conflict,
    is_recently_modified, list_sessions_in_slug, move_session_subdir, read_first_cwd,
    remove_empty_subdirs, remove_if_empty, validate_slug,
};
use crate::session_move_jsonl::{rewrite_history_jsonl, stream_rewrite_jsonl};
pub use crate::session_move_types::{
    AdoptReport, MoveSessionError, MoveSessionOpts, MoveSessionReport, OrphanedProject,
    INVALID_SLUG_MSG,
};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

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
    config_dir: &Path,
    session_id: Uuid,
    from_cwd: &Path,
    to_cwd: &Path,
    opts: MoveSessionOpts,
) -> Result<MoveSessionReport, MoveSessionError> {
    if !config_dir.is_dir() {
        return Err(MoveSessionError::InvalidConfigDir(config_dir.to_path_buf()));
    }

    let from_canonical = canonicalize_cc_path(from_cwd);
    let to_canonical = canonicalize_cc_path(to_cwd);
    if from_canonical == to_canonical {
        return Err(MoveSessionError::SameCwd);
    }

    let projects_dir = config_dir.join("projects");
    let from_slug = sanitize_path(&from_canonical.to_string_lossy());
    let to_slug = sanitize_path(&to_canonical.to_string_lossy());
    let from_proj = projects_dir.join(&from_slug);
    let to_proj = projects_dir.join(&to_slug);

    let session_file_name = format!("{session_id}.jsonl");
    let from_session = from_proj.join(&session_file_name);
    if !from_session.is_file() {
        return Err(MoveSessionError::SessionNotFound(
            session_id,
            from_proj.clone(),
        ));
    }

    // Guard: sync-conflict siblings. Any file whose name starts with
    // `<session>.sync-conflict-` is a Syncthing artifact and signals
    // unresolved divergence between nodes. Refuse to move — we'd silently
    // orphan the conflict copy in the source slug.
    if !opts.force_sync_conflict && has_sync_conflict(&from_proj, session_id)? {
        return Err(MoveSessionError::SyncConflictPresent(session_id));
    }

    // Guard: live session. mtime freshness is an approximation of "CC
    // may still be writing". Not perfect — a crashed session looks live
    // for the first few seconds — but matches the CC flush cadence.
    if !opts.force_live_session && is_recently_modified(&from_session)? {
        return Err(MoveSessionError::LiveSession(session_id));
    }

    // Guard: target collision. Overwriting a same-uuid file in the target
    // would fuse two histories silently.
    let to_session = to_proj.join(&session_file_name);
    if to_session.exists() {
        return Err(MoveSessionError::TargetCollision(session_id));
    }

    fs::create_dir_all(&to_proj)?;

    // Phase 1: rewrite + place the primary JSONL atomically in the target.
    // We stream from source → target tempfile, then rename into place,
    // then unlink the source. That ordering means a crash mid-way leaves
    // the source intact (worst case: an orphaned tempfile in the target).
    let from_str = from_canonical.to_string_lossy();
    let to_str = to_canonical.to_string_lossy();
    let lines_rewritten =
        stream_rewrite_jsonl(&from_session, &to_session, &from_str, &to_str)?;

    // Phase 2: sibling per-session dir (subagents/, remote-agents/).
    let from_sub = from_proj.join(session_id.to_string());
    let (subagent_files_moved, remote_agent_files_moved) = if from_sub.is_dir() {
        let to_sub = to_proj.join(session_id.to_string());
        move_session_subdir(&from_sub, &to_sub)?
    } else {
        (0, 0)
    };

    // Phase 3: history.jsonl — rewrite lines keyed by sessionId. Also
    // counts lines that look like ours (project matches source_cwd) but
    // lack sessionId so we can surface "some history couldn't be
    // attributed" to the caller.
    let history_path = config_dir.join("history.jsonl");
    let (history_entries_moved, history_entries_unmapped) = if history_path.is_file() {
        rewrite_history_jsonl(&history_path, session_id, &from_str, &to_str)?
    } else {
        (0, 0)
    };

    // Phase 4: .claude.json session pointers. CC stores this file at
    // `$HOME/.claude.json` — a sibling of `$HOME/.claude/`, NOT inside
    // config_dir. The caller is responsible for passing the correct
    // path; if they don't (`None`), Phase 4 is skipped.
    let claude_json_pointers_cleared = match opts.claude_json_path.as_deref() {
        Some(path) => clear_claude_json_session_pointers(path, &from_canonical, session_id)?,
        None => 0,
    };

    // Now it's safe to unlink the source JSONL. If anything above failed
    // we returned early and the source is preserved.
    fs::remove_file(&from_session)?;

    // Phase 5: optional cleanup of an empty source project dir.
    let source_dir_removed =
        opts.cleanup_source_if_empty && remove_if_empty(&from_proj)?;

    Ok(MoveSessionReport {
        session_id: Some(session_id),
        from_slug,
        to_slug,
        jsonl_lines_rewritten: lines_rewritten,
        subagent_files_moved,
        remote_agent_files_moved,
        history_entries_moved,
        history_entries_unmapped,
        claude_json_pointers_cleared,
        source_dir_removed,
    })
}

/// Scan `<config_dir>/projects/` for slugs whose internal `cwd` (read
/// from the first JSONL line) no longer exists on disk. These are the
/// candidates the `adopt_orphan_project` flow will rescue.
pub fn detect_orphaned_projects(
    config_dir: &Path,
) -> Result<Vec<OrphanedProject>, MoveSessionError> {
    let projects_dir = config_dir.join("projects");
    if !projects_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in fs::read_dir(&projects_dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if !ft.is_dir() {
            continue;
        }
        let slug_path = entry.path();
        let slug = entry.file_name().to_string_lossy().to_string();

        let sessions = list_sessions_in_slug(&slug_path)?;
        if sessions.is_empty() {
            // Empty project dir — degenerate orphan. The existing
            // project-cleanup flow owns this case.
            continue;
        }

        let cwd_from_transcript = read_first_cwd(&sessions[0]).ok().flatten();
        let is_orphan = match &cwd_from_transcript {
            Some(cwd) => !cwd.is_dir(),
            // No parseable cwd but the slug has sessions — treat as
            // orphan so the user can rescue the transcripts.
            None => true,
        };
        if !is_orphan {
            continue;
        }

        let total_size_bytes = sessions
            .iter()
            .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();

        out.push(OrphanedProject {
            slug,
            cwd_from_transcript,
            session_count: sessions.len(),
            total_size_bytes,
            // TODO: git-worktree-aware target suggestion. For now we
            // leave it to the caller — the UX can pre-fill from recently
            // used projects or let the user pick.
            suggested_adoption_target: None,
        });
    }
    Ok(out)
}

/// Move every session under `<config_dir>/projects/<orphan_slug>/` into
/// `target_cwd`'s project dir. Returns aggregate counts plus per-session
/// detail. Refuses if the orphan slug contains path separators or `..`
/// (which would escape `projects_dir`), if the resolved dir is not an
/// actual project dir, or if `target_cwd` is the same as the orphan's
/// cwd.
///
/// `claude_json_path`: where to find CC's `~/.claude.json` (see
/// `MoveSessionOpts::claude_json_path`). Threaded through to each
/// per-session `move_session` call so Phase 4 runs in production.
pub fn adopt_orphan_project(
    config_dir: &Path,
    orphan_slug: &str,
    target_cwd: &Path,
    claude_json_path: Option<PathBuf>,
) -> Result<AdoptReport, MoveSessionError> {
    validate_slug(orphan_slug)?;
    let projects_dir = config_dir.join("projects");
    let orphan_dir = projects_dir.join(orphan_slug);
    if !orphan_dir.is_dir() {
        return Err(MoveSessionError::InvalidConfigDir(orphan_dir));
    }

    let sessions = list_sessions_in_slug(&orphan_dir)?;
    // Derive the orphan's cwd from its first session so we can pass the
    // correct `from_cwd` to move_session (the slug itself is lossy for
    // any path containing non-alnum chars).
    let orphan_cwd = sessions
        .iter()
        .find_map(|p| read_first_cwd(p).ok().flatten())
        .ok_or_else(|| MoveSessionError::InvalidConfigDir(orphan_dir.clone()))?;

    let mut report = AdoptReport {
        sessions_attempted: sessions.len(),
        ..Default::default()
    };

    for jsonl in &sessions {
        let sid = match extract_session_id_from_path(jsonl) {
            Some(id) => id,
            None => {
                // Filenames that aren't <uuid>.jsonl — skip. The user
                // can rename them manually if they want them adopted.
                continue;
            }
        };
        // Force past the live-session mtime guard: orphan adoption
        // targets sessions in a slug whose original cwd doesn't exist,
        // so they can't be live by definition. Still honor sync-conflict
        // refusal — those require manual resolution.
        let opts = MoveSessionOpts {
            force_live_session: true,
            force_sync_conflict: false,
            cleanup_source_if_empty: false,
            claude_json_path: claude_json_path.clone(),
        };
        match move_session(config_dir, sid, &orphan_cwd, target_cwd, opts) {
            Ok(r) => {
                report.sessions_moved += 1;
                report.per_session.push(r);
            }
            Err(e) => {
                report.sessions_failed.push((sid, e.to_string()));
            }
        }
    }

    // Clean up the orphan slug dir if every session moved successfully.
    if report.sessions_failed.is_empty() {
        let _ = remove_empty_subdirs(&orphan_dir);
        if remove_if_empty(&orphan_dir)? {
            report.source_dir_removed = true;
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Tests
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
        // Age past the live-session guard (the fixture helper is
        // bypassed in this test, so we do it manually).
        f.set_mtime(&path, SystemTime::now() - Duration::from_secs(3600));

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

        let from_slug = crate::project_sanitize::sanitize_path(
            &dead_cwd.to_string_lossy(),
        );

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
}
