//! Filesystem + config helpers for `session_move` (everything except
//! the JSONL stream rewriters, which live in `session_move_jsonl`).
//!
//! Crate-public so `session_move.rs` can call them; not part of the
//! external API.

use crate::session_move_types::{MoveSessionError, INVALID_SLUG_MSG, LIVE_SESSION_MTIME_THRESHOLD};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Input validation
// ---------------------------------------------------------------------------

/// Reject slugs that would escape `<config_dir>/projects/` when joined.
/// CC's `sanitize_path` only ever emits `[A-Za-z0-9-]+`; any string
/// with a separator, a `..` component, or control characters is
/// untrusted input (likely a traversal attempt via CLI/GUI) and must
/// be rejected at the library boundary even if the joined path happens
/// to be a real directory on disk.
pub(crate) fn validate_slug(slug: &str) -> Result<(), MoveSessionError> {
    let invalid = slug.is_empty()
        || slug == "."
        || slug == ".."
        || slug.contains('/')
        || slug.contains('\\')
        || slug.contains('\0')
        || slug
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
    if invalid {
        return Err(MoveSessionError::InvalidSlug(
            slug.to_string(),
            INVALID_SLUG_MSG,
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

pub(crate) fn has_sync_conflict(
    project_dir: &Path,
    session_id: Uuid,
) -> Result<bool, MoveSessionError> {
    let prefix = format!("{session_id}.sync-conflict-");
    for entry in fs::read_dir(project_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn is_recently_modified(path: &Path) -> Result<bool, MoveSessionError> {
    let meta = fs::metadata(path)?;
    let mtime = meta.modified()?;
    match SystemTime::now().duration_since(mtime) {
        Ok(age) => Ok(age < LIVE_SESSION_MTIME_THRESHOLD),
        // Clock skew: mtime is in the future. Treat as live to err
        // on the side of not corrupting a concurrent writer.
        Err(_) => Ok(true),
    }
}

// ---------------------------------------------------------------------------
// Per-session sidecar dir move (subagents + remote-agents)
// ---------------------------------------------------------------------------

/// Move a per-session dir (containing subagents/ and/or remote-agents/)
/// to its new parent, reporting the count of files in each subtree.
/// Uses rename when the src and dst share a filesystem (the common case
/// — both under ~/.claude); falls back to copy-then-remove if rename
/// fails with EXDEV.
/// Fires `on_progress(done, total)` after each file is moved or
/// copied. `total` is the precounted file count across `subagents/`
/// + `remote-agents/`. Pass `&mut |_, _| {}` to suppress.
pub(crate) fn move_session_subdir_with_progress(
    from_sub: &Path,
    to_sub: &Path,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<(usize, usize), MoveSessionError> {
    let subagent_count = count_files(&from_sub.join("subagents"))?;
    let remote_agent_count = count_files(&from_sub.join("remote-agents"))?;
    let total = subagent_count + remote_agent_count;

    if let Some(parent) = to_sub.parent() {
        fs::create_dir_all(parent)?;
    }
    if to_sub.exists() {
        // Unusual but possible if the target slug had residue from a
        // prior partial move. Merge by moving files individually rather
        // than refusing — the session-file collision check upstream is
        // the real gate.
        let mut done = 0usize;
        copy_tree_then_remove_with_progress(from_sub, to_sub, &mut done, total, on_progress)?;
    } else if let Err(err) = fs::rename(from_sub, to_sub) {
        // Cross-device rename is the only expected failure mode.
        // Fall through to copy+delete.
        if err.raw_os_error() == Some(libc::EXDEV) {
            let mut done = 0usize;
            copy_tree_then_remove_with_progress(
                from_sub,
                to_sub,
                &mut done,
                total,
                on_progress,
            )?;
        } else {
            return Err(err.into());
        }
    } else {
        // Atomic rename moved everything in one syscall — emit a single
        // `total/total` tick so the sink sees S2 finish.
        if total > 0 {
            on_progress(total, total);
        }
    }

    Ok((subagent_count, remote_agent_count))
}

fn count_files(dir: &Path) -> Result<usize, MoveSessionError> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut n = 0usize;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            n += count_files(&entry.path())?;
        } else if ft.is_file() {
            n += 1;
        }
    }
    Ok(n)
}

/// Ticks `on_progress(done, total)` after each file is copied. `done`
/// is shared across recursive calls so the count keeps increasing even
/// as the walker descends. Pass `total = 0` + a no-op callback to
/// disable progress.
fn copy_tree_then_remove_with_progress(
    from: &Path,
    to: &Path,
    done: &mut usize,
    total: usize,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<(), MoveSessionError> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_tree_then_remove_with_progress(&src, &dst, done, total, on_progress)?;
        } else {
            // File-level collision check. Without it, a partial prior
            // move (e.g. a previous run that crashed mid-copy) silently
            // clobbers the target's transcript with the source's. The
            // primary `<sid>.jsonl` is gated upstream by
            // `MoveSessionError::TargetCollision`, but per-session
            // sidecar files (subagents/<x>.jsonl,
            // remote-agents/<x>.jsonl) live under a session-scoped
            // subdir and bypass that gate. Refuse the merge so the
            // caller can resolve the conflict explicitly.
            if dst.exists() {
                return Err(MoveSessionError::SidecarCollision(dst));
            }
            fs::copy(&src, &dst)?;
            *done += 1;
            if total > 0 {
                on_progress(*done, total);
            }
        }
    }
    fs::remove_dir_all(from)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// .claude.json session-pointer clearing
// ---------------------------------------------------------------------------

/// Clear per-project session pointers in `~/.claude.json` when they
/// reference the moved session. Returns 0/1/2 — the count of pointers
/// actually cleared. No-op when the file is missing (e.g. a first-run
/// CC install where the config hasn't been written yet).
pub(crate) fn clear_claude_json_session_pointers(
    path: &Path,
    from_cwd: &Path,
    session_id: Uuid,
) -> Result<u8, MoveSessionError> {
    if !path.is_file() {
        return Ok(0);
    }

    let contents = fs::read_to_string(path)?;
    let mut root: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return Ok(0),
    };

    let from_key = from_cwd.to_string_lossy().to_string();
    let sid_str = session_id.to_string();
    let mut cleared = 0u8;

    if let Some(projects) = root.get_mut("projects").and_then(|v| v.as_object_mut()) {
        if let Some(entry) = projects.get_mut(&from_key).and_then(|v| v.as_object_mut()) {
            // lastSessionId — clear if it matches.
            if entry.get("lastSessionId").and_then(|v| v.as_str()) == Some(&sid_str) {
                entry.insert("lastSessionId".to_string(), serde_json::Value::Null);
                cleared += 1;
            }
            // activeWorktreeSession.sessionId — clear if it matches.
            if let Some(aws) = entry
                .get_mut("activeWorktreeSession")
                .and_then(|v| v.as_object_mut())
            {
                if aws.get("sessionId").and_then(|v| v.as_str()) == Some(&sid_str) {
                    aws.insert("sessionId".to_string(), serde_json::Value::Null);
                    cleared += 1;
                }
            }
        }
    }

    if cleared == 0 {
        return Ok(0);
    }

    // Atomic replace: tempfile in the same dir, then rename. Matches
    // project_config_rewrite.rs's approach — and accepts the same
    // tradeoff (serde_json's BTreeMap-backed Map reorders keys on
    // output; acceptable since CC itself rewrites this file freely).
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    let new_json = serde_json::to_string_pretty(&root)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    tmp.write_all(new_json.as_bytes())?;
    tmp.write_all(b"\n")?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(cleared)
}

// ---------------------------------------------------------------------------
// Directory tree helpers
// ---------------------------------------------------------------------------

/// Remove a directory if it contains no files or non-empty subdirs.
/// Returns true iff removal happened.
pub(crate) fn remove_if_empty(dir: &Path) -> Result<bool, MoveSessionError> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let empty = fs::read_dir(dir)?.next().is_none();
    if empty {
        fs::remove_dir(dir)?;
        return Ok(true);
    }
    Ok(false)
}

/// Recursively remove empty subdirectories, bottom-up. Used by
/// `adopt_orphan_project` to sweep residue before the final dir removal.
pub(crate) fn remove_empty_subdirs(dir: &Path) -> Result<(), MoveSessionError> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            let p = entry.path();
            remove_empty_subdirs(&p)?;
            let _ = remove_if_empty(&p);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Session listing + cwd extraction
// ---------------------------------------------------------------------------

/// List non-conflict `*.jsonl` files directly under a slug dir, sorted
/// lexicographically for deterministic caller behavior.
pub(crate) fn list_sessions_in_slug(slug_dir: &Path) -> Result<Vec<PathBuf>, MoveSessionError> {
    let mut out = Vec::new();
    for entry in fs::read_dir(slug_dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if !ft.is_file() {
            continue;
        }
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        // Skip Syncthing conflict copies — they're repair targets, not
        // primary sessions.
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.contains(".sync-conflict-") {
            continue;
        }
        out.push(p);
    }
    out.sort();
    Ok(out)
}

/// Extract the `cwd` field from the first parseable object line in a
/// JSONL file. `None` if no line is both parseable and carries a `cwd`
/// string.
pub(crate) fn read_first_cwd(jsonl: &Path) -> Result<Option<PathBuf>, MoveSessionError> {
    let f = fs::File::open(jsonl)?;
    let reader = BufReader::new(f);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let parsed: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(cwd) = parsed.get("cwd").and_then(|v| v.as_str()) {
            return Ok(Some(PathBuf::from(cwd)));
        }
    }
    Ok(None)
}

/// Parse a session's UUID from its `<uuid>.jsonl` filename. Returns
/// `None` for unconventional filenames.
pub(crate) fn extract_session_id_from_path(jsonl: &Path) -> Option<Uuid> {
    let stem = jsonl.file_stem()?.to_str()?;
    Uuid::parse_str(stem).ok()
}
