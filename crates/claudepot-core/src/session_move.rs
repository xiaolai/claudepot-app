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

use crate::path_utils::simplify_windows_path;
use crate::project_progress::{NoopSink, PhaseStatus, ProgressSink};
use crate::project_sanitize::sanitize_path;
use crate::session_move_helpers::{
    clear_claude_json_session_pointers, extract_session_id_from_path, has_sync_conflict,
    is_recently_modified, list_sessions_in_slug, move_session_subdir_with_progress, read_first_cwd,
    remove_empty_subdirs, remove_if_empty, validate_slug,
};
use crate::session_move_jsonl::{
    rewrite_history_jsonl_with_progress, stream_rewrite_jsonl_with_progress,
};
pub use crate::session_move_types::{
    AdoptReport, DiscardReport, MoveSessionError, MoveSessionOpts, MoveSessionReport,
    OrphanedProject, INVALID_SLUG_MSG,
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
    // Strip the Windows `\\?\` verbatim prefix that canonicalize adds —
    // CC never writes that form into slugs, so feeding it into
    // `sanitize_path` would produce a slug that doesn't match the
    // on-disk directory. No-op on Unix.
    let simplified = simplify_windows_path(&attempted.to_string_lossy());
    // Normalize to NFC. CC's input comes from JavaScript strings
    // (UTF-16) and is already UTF-8-clean.
    let normalized: String = simplified.nfc().collect();
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
    move_session_with_progress(config_dir, session_id, from_cwd, to_cwd, opts, &NoopSink)
}

/// Progress-aware variant of [`move_session`]. Emits structured
/// [`PhaseStatus`] events on `sink` keyed by these stable phase ids
/// — the frontend reads these by name and renders matching labels:
///
/// | id | label                              |
/// |----|------------------------------------|
/// | S1 | Rewriting primary transcript       |
/// | S2 | Moving sidecar dirs                |
/// | S3 | Updating history.jsonl             |
/// | S4 | Clearing .claude.json pointers     |
/// | S5 | Cleaning up source dir             |
///
/// `sub_progress` fires per JSONL line in S1, per sidecar file in S2,
/// per history line scanned in S3. S4 and S5 each fire a single
/// `Complete` event (atomic / single-step phases).
pub fn move_session_with_progress(
    config_dir: &Path,
    session_id: Uuid,
    from_cwd: &Path,
    to_cwd: &Path,
    opts: MoveSessionOpts,
    sink: &dyn ProgressSink,
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

    // Phase S1: rewrite + place the primary JSONL atomically in the
    // target. We stream from source → target tempfile, then rename into
    // place, then unlink the source. That ordering means a crash mid-way
    // leaves the source intact (worst case: an orphaned tempfile in the
    // target).
    let from_str = from_canonical.to_string_lossy();
    let to_str = to_canonical.to_string_lossy();
    let lines_rewritten = match stream_rewrite_jsonl_with_progress(
        &from_session,
        &to_session,
        &from_str,
        &to_str,
        &mut |done, total| sink.sub_progress("S1", done, total),
    ) {
        Ok(n) => {
            sink.phase("S1", PhaseStatus::Complete);
            n
        }
        Err(e) => {
            sink.phase("S1", PhaseStatus::Error(e.to_string()));
            return Err(e);
        }
    };

    // From here on, any phase failure must roll the target JSONL back
    // out — otherwise both source and target carry the transcript and
    // a retry trips `TargetCollision`. We can't fully undo every side
    // effect (history.jsonl rewrites, .claude.json pointer clears
    // already happened in-place), but unwinding the target file
    // restores the precondition the next attempt needs.
    let history_path = config_dir.join("history.jsonl");
    let from_sub = from_proj.join(session_id.to_string());
    let to_sub = to_proj.join(session_id.to_string());
    // Snapshot whether the target sidecar dir pre-existed. Without this,
    // a rollback that wipes `to_sub` could destroy unrelated user data
    // that lived there before we ever started.
    let to_sub_preexisted = to_sub.exists();
    // Audit fix for session_move.rs:276 — track whether S2 actually
    // moved sidecars from `from_sub` to `to_sub`, so a rollback after
    // S3/S4 failure can move them BACK instead of deleting `to_sub`
    // (which destroyed user data in the previous shape).
    let mut s2_moved_sidecars = false;
    let result: Result<(usize, usize, usize, usize, u8), (MoveSessionError, &'static str)> =
        (|| {
            // Phase S2: sibling per-session dir (subagents/, remote-agents/).
            let (subagent_files_moved, remote_agent_files_moved) = if from_sub.is_dir() {
                let moved = move_session_subdir_with_progress(
                    &from_sub,
                    &to_sub,
                    &mut |done, total| sink.sub_progress("S2", done, total),
                )
                .map_err(|e| (e, "S2"))?;
                if moved.0 > 0 || moved.1 > 0 {
                    s2_moved_sidecars = true;
                }
                moved
            } else {
                (0, 0)
            };
            sink.phase("S2", PhaseStatus::Complete);

            // Phase S3: history.jsonl — rewrite lines keyed by sessionId.
            // Also counts lines that look like ours (project matches
            // source_cwd) but lack sessionId so we can surface "some history
            // couldn't be attributed" to the caller.
            let (history_entries_moved, history_entries_unmapped) = if history_path.is_file() {
                rewrite_history_jsonl_with_progress(
                    &history_path,
                    session_id,
                    &from_str,
                    &to_str,
                    &mut |done, total| sink.sub_progress("S3", done, total),
                )
                .map_err(|e| (e, "S3"))?
            } else {
                (0, 0)
            };
            sink.phase("S3", PhaseStatus::Complete);

            // Phase S4: .claude.json session pointers. CC stores this file at
            // `$HOME/.claude.json` — a sibling of `$HOME/.claude/`, NOT
            // inside config_dir. The caller is responsible for passing the
            // correct path; if they don't (`None`), S4 is skipped.
            let claude_json_pointers_cleared = match opts.claude_json_path.as_deref() {
                Some(path) => clear_claude_json_session_pointers(path, &from_canonical, session_id)
                    .map_err(|e| (e, "S4"))?,
                None => 0,
            };
            sink.phase("S4", PhaseStatus::Complete);

            Ok((
                subagent_files_moved,
                remote_agent_files_moved,
                history_entries_moved,
                history_entries_unmapped,
                claude_json_pointers_cleared,
            ))
        })();

    let (
        subagent_files_moved,
        remote_agent_files_moved,
        history_entries_moved,
        history_entries_unmapped,
        claude_json_pointers_cleared,
    ) = match result {
        Ok(v) => v,
        Err((e, phase)) => {
            sink.phase(phase, PhaseStatus::Error(e.to_string()));
            // Roll the target JSONL back so a retry doesn't hit
            // TargetCollision on the same session that previously failed
            // mid-flight. Best-effort — if the unlink itself fails (rare;
            // the file we just renamed in is owned by us), the original
            // error wins because that's what the caller asked about.
            let _ = fs::remove_file(&to_session);
            // Audit fix for session_move.rs:276 — sidecar rollback.
            // If S2 already MOVED sidecar files from from_sub into
            // to_sub, deleting to_sub now would destroy user data
            // that the source no longer carries. Move them back to
            // from_sub instead. If S2 never moved anything (no
            // from_sub originally, or S2 failed itself before any
            // file moved), then either to_sub is empty or it
            // pre-existed and we leave it alone.
            if s2_moved_sidecars {
                // Best-effort: re-move sidecars back to source. If
                // this fails the user will have the contents under
                // to_sub instead of from_sub — surfaceable as a
                // post-rollback inventory check, but no data loss.
                let _ = fs::create_dir_all(&from_sub);
                let _ = move_session_subdir_with_progress(&to_sub, &from_sub, &mut |_, _| {});
                // After the back-move, to_sub should be empty;
                // remove the now-empty dir we created.
                let _ = fs::remove_dir(&to_sub);
            } else if !to_sub_preexisted {
                // S2 never moved data into to_sub, but we may have
                // created it as a side effect. Remove only the
                // freshly-created empty dir; pre-existing dirs stay.
                let _ = fs::remove_dir_all(&to_sub);
            }
            return Err(e);
        }
    };

    // Now it's safe to unlink the source JSONL. If anything above failed
    // we returned early and the source is preserved.
    fs::remove_file(&from_session)?;

    // Phase S5: optional cleanup of an empty source project dir.
    let source_dir_removed = opts.cleanup_source_if_empty && remove_if_empty(&from_proj)?;
    sink.phase("S5", PhaseStatus::Complete);

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

/// Move an orphan project slug dir (with all its session transcripts and
/// sidecars) to the OS Trash. Reversible by design — the user can restore
/// from Trash if they change their mind.
///
/// Use this when the user wants to forget an orphan whose sessions are
/// no longer valuable. For orphans whose history must be preserved, use
/// [`adopt_orphan_project`] instead.
///
/// Guards:
/// * `orphan_slug` must pass [`validate_slug`] — rejects traversal attempts.
/// * `<config_dir>/projects/<orphan_slug>` must be an actual directory.
///
/// This function does **not** check whether the slug is "really" an orphan.
/// Policy gating belongs in the UI; the backend is mechanism-only so a
/// future power-user CLI path can discard any slug the user owns.
pub fn discard_orphan_project(
    config_dir: &Path,
    orphan_slug: &str,
) -> Result<DiscardReport, MoveSessionError> {
    discard_orphan_project_with(config_dir, orphan_slug, trash_dir)
}

/// OS-Trash remover used by [`discard_orphan_project`] in production.
fn trash_dir(p: &Path) -> Result<(), MoveSessionError> {
    trash::delete(p).map_err(|e| MoveSessionError::TrashFailed(e.to_string()))
}

/// Core discard implementation parameterised on the remover so tests
/// can exercise the slug-validation + counting logic without polluting
/// the host's real Trash. The `remove` closure is called exactly once,
/// after validation and size computation, with the resolved slug dir.
pub(crate) fn discard_orphan_project_with<F>(
    config_dir: &Path,
    orphan_slug: &str,
    remove: F,
) -> Result<DiscardReport, MoveSessionError>
where
    F: FnOnce(&Path) -> Result<(), MoveSessionError>,
{
    validate_slug(orphan_slug)?;
    let projects_dir = config_dir.join("projects");
    let orphan_dir = projects_dir.join(orphan_slug);
    if !orphan_dir.is_dir() {
        return Err(MoveSessionError::InvalidConfigDir(orphan_dir));
    }

    // Snapshot counts BEFORE the trash call so the report remains
    // meaningful even if the dir is gone by the time the caller reads it.
    let sessions = list_sessions_in_slug(&orphan_dir)?;
    let total_size_bytes: u64 = sessions
        .iter()
        .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();
    let sessions_discarded = sessions.len();

    remove(&orphan_dir)?;

    Ok(DiscardReport {
        sessions_discarded,
        total_size_bytes,
        dir_removed: !orphan_dir.exists(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "session_move_tests.rs"]
mod tests;
