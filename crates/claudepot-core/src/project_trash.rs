//! Reversible trash for whole CC project artifact directories.
//!
//! Layout on disk:
//!
//! ```text
//! <data_dir>/trash/projects/
//!   20260426T153045Z-<uuid8>/
//!     manifest.json     ← ProjectTrashEntry (slug, recovered cwd,
//!                         pruned ~/.claude.json entry, pruned
//!                         history.jsonl lines)
//!     payload/          ← the entire moved <slug>/ artifact dir
//!       <session-uuid>.jsonl
//!       ...
//! ```
//!
//! Sister to `crate::trash`, which trashes single `.jsonl` files keyed
//! by inode. The shapes diverge enough (directory payload, sibling-
//! state snapshot in the manifest) that splitting kept each module
//! focused.
//!
//! Atomicity: `rename()` of `<projects>/<slug>` → `<batch>/payload`,
//! cross-device fallback is recursive copy → fsync → recursive remove.
//! GC scans batches older than a duration and deletes them; `gc(30 days)`
//! is the default startup sweep for project trash.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ProjectTrashError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("source path not found: {0}")]
    SourceMissing(PathBuf),
    #[error("trash entry not found: {0}")]
    EntryNotFound(String),
    #[error("manifest parse error at {path}: {source}")]
    ManifestParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("restore target already exists: {0}")]
    RestoreCollision(PathBuf),
    #[error("invalid slug: {0:?}")]
    InvalidSlug(String),
}

impl ProjectTrashError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// One trashed project, persisted as `<batch>/manifest.json`.
///
/// Sibling-state snapshots (`claude_json_entry`, `history_lines`) are
/// captured at trash time and re-applied on restore. Held inside the
/// manifest rather than alongside it because a partial restore (dir
/// moves back, manifest read fails) is worse than an all-or-nothing
/// read of one JSON document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTrashEntry {
    /// Batch id = directory name, e.g. `20260426T153045Z-abc12345`.
    pub id: String,
    /// Convenience alias. Same as `id`.
    pub manifest_id: String,
    /// CC sanitized name (e.g. `-Users-joker`). The original
    /// directory under `<config_dir>/projects/` was named this.
    pub slug: String,
    /// Best-effort recovered cwd from session metadata. Display-only
    /// — never used to reconstruct anything on restore.
    pub original_path: Option<String>,
    pub bytes: u64,
    pub session_count: usize,
    /// UTC milliseconds since epoch.
    pub ts_ms: i64,
    /// Pulled `~/.claude.json` `projects.<original_path>` value, if
    /// any. Restore re-inserts it under the same key.
    pub claude_json_entry: Option<serde_json::Value>,
    /// `history.jsonl` lines whose `project` field matched
    /// `original_path`. Restore re-appends them.
    pub history_lines: Vec<String>,
    /// Optional human-readable reason ("user-initiated", "GUI", etc.).
    pub reason: Option<String>,
}

/// One-shot input for `write`. Borrows so callers don't have to clone
/// on the hot path.
pub struct ProjectTrashPut<'a> {
    /// Directory to move *into* the trash. Typically
    /// `<config_dir>/projects/<slug>/`.
    pub source_dir: &'a Path,
    pub slug: &'a str,
    pub original_path: Option<&'a str>,
    pub bytes: u64,
    pub session_count: usize,
    pub claude_json_entry: Option<serde_json::Value>,
    pub history_lines: Vec<String>,
    pub reason: Option<String>,
}

/// Enumerated batches and their combined byte total.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectTrashListing {
    pub entries: Vec<ProjectTrashEntry>,
    pub total_bytes: u64,
}

/// List / empty filter.
#[derive(Debug, Clone, Default)]
pub struct ProjectTrashFilter {
    pub older_than: Option<Duration>,
}

/// Result of a successful restore.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectRestoreReport {
    pub restored_dir: PathBuf,
    pub claude_json_restored: bool,
    pub history_lines_restored: usize,
}

fn trash_root(data_dir: &Path) -> PathBuf {
    data_dir.join("trash").join("projects")
}

fn batch_ts_string(now: SystemTime) -> String {
    let dt: DateTime<Utc> = now.into();
    dt.format("%Y%m%dT%H%M%SZ").to_string()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A slug is a CC sanitized name. By construction it can't contain
/// path separators or traversal sequences — but slugs come from
/// callers, so we re-validate at the boundary. Defense in depth.
fn validate_slug(slug: &str) -> Result<(), ProjectTrashError> {
    if slug.is_empty()
        || slug == "."
        || slug == ".."
        || slug.contains('/')
        || slug.contains('\\')
        || slug.contains('\0')
        || slug.starts_with('.')
    {
        return Err(ProjectTrashError::InvalidSlug(slug.to_string()));
    }
    Ok(())
}

/// Move a project artifact directory into the trash.
///
/// Creates a fresh batch directory under `<data_dir>/trash/projects/`,
/// moves `source_dir` to `<batch>/payload/`, and writes the manifest.
/// Cross-device renames fall back to recursive copy + fsync + recursive
/// remove. Source-dir-missing surfaces as an explicit error.
pub fn write(
    data_dir: &Path,
    put: ProjectTrashPut<'_>,
) -> Result<ProjectTrashEntry, ProjectTrashError> {
    validate_slug(put.slug)?;

    let src = put.source_dir;
    let meta = fs::metadata(src).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => ProjectTrashError::SourceMissing(src.to_path_buf()),
        _ => ProjectTrashError::io(src, e),
    })?;
    if !meta.is_dir() {
        return Err(ProjectTrashError::SourceMissing(src.to_path_buf()));
    }

    let uuid8 = Uuid::new_v4().simple().to_string()[..8].to_string();
    let batch_id = format!("{}-{}", batch_ts_string(SystemTime::now()), uuid8);
    let batch_dir = trash_root(data_dir).join(&batch_id);
    fs::create_dir_all(&batch_dir).map_err(|e| ProjectTrashError::io(&batch_dir, e))?;

    // Audit fix for project_trash.rs:214 — write the manifest BEFORE
    // moving the payload. The previous order moved the directory
    // first and wrote the manifest second, so a manifest-write
    // failure stranded an unlisted payload that `list()` filtered
    // away. With manifest-first, a failed payload move can be
    // rolled back by removing the manifest+batch_dir.
    let entry = ProjectTrashEntry {
        id: batch_id.clone(),
        manifest_id: batch_id.clone(),
        slug: put.slug.to_string(),
        original_path: put.original_path.map(str::to_string),
        bytes: put.bytes,
        session_count: put.session_count,
        ts_ms: now_ms(),
        claude_json_entry: put.claude_json_entry,
        history_lines: put.history_lines,
        reason: put.reason,
    };
    let manifest_path = batch_dir.join("manifest.json");
    let json = serde_json::to_vec_pretty(&entry).expect("ProjectTrashEntry serializes");
    write_atomic(&manifest_path, &json)?;

    let payload = batch_dir.join("payload");
    if let Err(e) = move_dir(src, &payload) {
        // Roll back: payload didn't land, the manifest-only batch
        // would mislead list(). Best-effort cleanup.
        let _ = fs::remove_file(&manifest_path);
        let _ = fs::remove_dir_all(&batch_dir);
        return Err(e);
    }

    Ok(entry)
}

/// Atomic write: tmp + rename. Used so a partial manifest is
/// never observable.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ProjectTrashError> {
    let parent = path.parent().ok_or_else(|| {
        ProjectTrashError::io(path, io::Error::other("manifest has no parent"))
    })?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("manifest.json"),
        std::process::id()
    ));
    fs::write(&tmp, bytes).map_err(|e| ProjectTrashError::io(&tmp, e))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(ProjectTrashError::io(path, e));
    }
    Ok(())
}

/// Move `src` directory to `dest`. Plain rename first; on a genuine
/// cross-device error (EXDEV/ERROR_NOT_SAME_DEVICE), fall back to
/// recursive copy + fsync + remove via a unique staging dir, then
/// atomic rename into place.
///
/// Audit fix for project_trash.rs:226 — restrict the copy fallback
/// to EXDEV. The previous shape fell back on ANY rename error,
/// which masked permission errors AND opened a race where a
/// concurrent process creating `dest` between the failed rename and
/// the copy would let copy_dir_recursive merge into the existing
/// dir, then `remove_dir_all(src)` would delete the source — losing
/// project data. Restricting to EXDEV plus copying to a staging
/// path first eliminates both.
fn move_dir(src: &Path, dest: &Path) -> Result<(), ProjectTrashError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| ProjectTrashError::io(parent, e))?;
    }
    let rename_err = match fs::rename(src, dest) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    if !is_exdev(&rename_err) {
        return Err(ProjectTrashError::io(src, rename_err));
    }

    // EXDEV path: copy to a unique staging dir adjacent to dest,
    // then atomic-rename into place. Avoids the race where a
    // concurrent process creates `dest` between rename and copy.
    let stage = dest.with_file_name(format!(
        "{}.staging.{}",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("payload"),
        std::process::id()
    ));
    let copy_result = copy_dir_recursive(src, &stage);
    if let Err(e) = copy_result {
        let _ = fs::remove_dir_all(&stage);
        return Err(e);
    }
    if let Err(e) = fs::rename(&stage, dest) {
        let _ = fs::remove_dir_all(&stage);
        return Err(ProjectTrashError::io(dest, e));
    }
    fs::remove_dir_all(src).map_err(|e| ProjectTrashError::io(src, e))?;
    Ok(())
}

fn is_exdev(err: &io::Error) -> bool {
    let raw = err.raw_os_error();
    #[cfg(unix)]
    {
        raw == Some(libc::EXDEV)
    }
    #[cfg(windows)]
    {
        // ERROR_NOT_SAME_DEVICE == 17
        return raw == Some(17);
    }
    #[cfg(not(any(unix, windows)))]
    {
        return raw == Some(18);
    }
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), ProjectTrashError> {
    fs::create_dir_all(dest).map_err(|e| ProjectTrashError::io(dest, e))?;
    for entry in fs::read_dir(src).map_err(|e| ProjectTrashError::io(src, e))? {
        let entry = entry.map_err(|e| ProjectTrashError::io(src, e))?;
        let ft = entry
            .file_type()
            .map_err(|e| ProjectTrashError::io(entry.path(), e))?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ft.is_symlink() {
            // Symlinks under a CC project dir are unexpected. Copy by
            // reading the target and recreating; if that fails, copy
            // the file content under the symlink as a fallback.
            #[cfg(unix)]
            {
                if let Ok(target) = fs::read_link(&from) {
                    if std::os::unix::fs::symlink(&target, &to).is_ok() {
                        continue;
                    }
                }
            }
            fs::copy(&from, &to).map_err(|e| ProjectTrashError::io(&from, e))?;
        } else {
            fs::copy(&from, &to).map_err(|e| ProjectTrashError::io(&from, e))?;
            if let Ok(f) = fs::File::open(&to) {
                let _ = f.sync_all();
            }
        }
    }
    Ok(())
}

fn remove_dir_recursive(dir: &Path) -> Result<(), ProjectTrashError> {
    fs::remove_dir_all(dir).map_err(|e| ProjectTrashError::io(dir, e))
}

/// List every batch under `<data_dir>/trash/projects/` matching the
/// filter. Newest-first.
pub fn list(
    data_dir: &Path,
    filter: ProjectTrashFilter,
) -> Result<ProjectTrashListing, ProjectTrashError> {
    let root = trash_root(data_dir);
    let mut out = Vec::new();
    let mut total: u64 = 0;
    if !root.exists() {
        return Ok(ProjectTrashListing {
            entries: out,
            total_bytes: 0,
        });
    }
    let cutoff_ms = filter
        .older_than
        .map(|d| now_ms().saturating_sub(d.as_millis() as i64));

    for entry in fs::read_dir(&root).map_err(|e| ProjectTrashError::io(&root, e))? {
        let entry = entry.map_err(|e| ProjectTrashError::io(&root, e))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("manifest.json");
        if !manifest.exists() {
            continue;
        }
        let raw = fs::read_to_string(&manifest).map_err(|e| ProjectTrashError::io(&manifest, e))?;
        let te: ProjectTrashEntry =
            serde_json::from_str(&raw).map_err(|e| ProjectTrashError::ManifestParse {
                path: manifest.clone(),
                source: e,
            })?;
        if let Some(cut) = cutoff_ms {
            if te.ts_ms >= cut {
                continue;
            }
        }
        total = total.saturating_add(te.bytes);
        out.push(te);
    }
    out.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
    Ok(ProjectTrashListing {
        entries: out,
        total_bytes: total,
    })
}

/// A valid batch id is exactly `YYYYMMDDTHHMMSSZ-<8 hex>`. Anything
/// else is rejected so `trash_root.join(entry_id)` cannot escape the
/// trash directory.
fn is_valid_batch_id(s: &str) -> bool {
    if s.len() != 16 + 1 + 8 {
        return false;
    }
    let bytes = s.as_bytes();
    let digits_ok = |range: std::ops::Range<usize>| -> bool {
        range.into_iter().all(|i| bytes[i].is_ascii_digit())
    };
    if !digits_ok(0..8) || bytes[8] != b'T' || !digits_ok(9..15) || bytes[15] != b'Z' {
        return false;
    }
    if bytes[16] != b'-' {
        return false;
    }
    bytes[17..25]
        .iter()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn find_batch(
    data_dir: &Path,
    entry_id: &str,
) -> Result<(PathBuf, ProjectTrashEntry), ProjectTrashError> {
    if !is_valid_batch_id(entry_id) {
        return Err(ProjectTrashError::EntryNotFound(entry_id.to_string()));
    }
    let root = trash_root(data_dir);
    let batch_dir = root.join(entry_id);
    // Defense in depth: confirm batch_dir is a direct child of the
    // trash root.
    match batch_dir.parent() {
        Some(parent) if parent == root => {}
        _ => return Err(ProjectTrashError::EntryNotFound(entry_id.to_string())),
    }
    if !batch_dir.exists() {
        return Err(ProjectTrashError::EntryNotFound(entry_id.to_string()));
    }
    let manifest = batch_dir.join("manifest.json");
    let raw = fs::read_to_string(&manifest).map_err(|e| ProjectTrashError::io(&manifest, e))?;
    let te: ProjectTrashEntry =
        serde_json::from_str(&raw).map_err(|e| ProjectTrashError::ManifestParse {
            path: manifest.clone(),
            source: e,
        })?;
    Ok((batch_dir, te))
}

/// Restore a trashed project. Moves `<batch>/payload/` back to
/// `<config_dir>/projects/<slug>/`. If `claude_json_path` is provided,
/// re-inserts `projects.<original_path>` from the manifest snapshot
/// (skipped silently if the key already exists — never clobbers live
/// state). If `history_path` is provided, re-appends the snapshotted
/// lines.
///
/// Refuses to clobber an existing target dir. Refuses entries with an
/// invalid slug (defense in depth — should never happen on
/// well-formed trash, but a hand-edited manifest could).
pub fn restore(
    data_dir: &Path,
    entry_id: &str,
    config_dir: &Path,
    claude_json_path: Option<&Path>,
    history_path: Option<&Path>,
) -> Result<ProjectRestoreReport, ProjectTrashError> {
    let (batch_dir, te) = find_batch(data_dir, entry_id)?;
    validate_slug(&te.slug)?;
    let payload = batch_dir.join("payload");
    if !payload.exists() {
        return Err(ProjectTrashError::EntryNotFound(format!(
            "{} (missing payload)",
            entry_id
        )));
    }
    let dest = config_dir.join("projects").join(&te.slug);
    if dest.exists() {
        return Err(ProjectTrashError::RestoreCollision(dest));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| ProjectTrashError::io(parent, e))?;
    }
    move_dir(&payload, &dest)?;

    let mut claude_json_restored = false;
    let mut history_lines_restored = 0;

    if let (Some(path), Some(orig), Some(value)) = (
        claude_json_path,
        te.original_path.as_deref(),
        te.claude_json_entry.as_ref(),
    ) {
        if path.exists() {
            if let Ok(contents) = fs::read_to_string(path) {
                if let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&contents) {
                    let projects = root.as_object_mut().and_then(|m| {
                        m.entry("projects")
                            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
                            .as_object_mut()
                    });
                    if let Some(map) = projects {
                        if !map.contains_key(orig) {
                            map.insert(orig.to_string(), value.clone());
                            claude_json_restored = true;
                            let bytes =
                                serde_json::to_vec_pretty(&root).expect("re-serialize claude.json");
                            // Best-effort write: failures here surface
                            // through Io error.
                            fs::write(path, bytes).map_err(|e| ProjectTrashError::io(path, e))?;
                        }
                    }
                }
            }
        }
    }

    if let Some(path) = history_path {
        if !te.history_lines.is_empty() {
            use std::io::Write;
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| ProjectTrashError::io(path, e))?;
            for line in &te.history_lines {
                writeln!(f, "{}", line.trim_end_matches('\n'))
                    .map_err(|e| ProjectTrashError::io(path, e))?;
                history_lines_restored += 1;
            }
        }
    }

    let _ = remove_dir_recursive(&batch_dir);
    Ok(ProjectRestoreReport {
        restored_dir: dest,
        claude_json_restored,
        history_lines_restored,
    })
}

/// Delete batches matching the filter. Returns bytes reclaimed (sum of
/// entry `bytes`). Missing root is a no-op.
pub fn empty(data_dir: &Path, filter: ProjectTrashFilter) -> Result<u64, ProjectTrashError> {
    let listing = list(data_dir, filter)?;
    let mut freed: u64 = 0;
    for te in &listing.entries {
        let batch_dir = trash_root(data_dir).join(&te.id);
        if fs::remove_dir_all(&batch_dir).is_ok() {
            freed = freed.saturating_add(te.bytes);
        }
    }
    Ok(freed)
}

/// Sweep batches older than `older_than`. Convenience wrapper around
/// `empty`. Default 30 days for project trash.
pub fn gc(data_dir: &Path, older_than: Duration) -> Result<u64, ProjectTrashError> {
    empty(
        data_dir,
        ProjectTrashFilter {
            older_than: Some(older_than),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn mk_project_dir(parent: &Path, slug: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let dir = parent.join(slug);
        fs::create_dir_all(&dir).unwrap();
        for (name, body) in files {
            let mut f = fs::File::create(dir.join(name)).unwrap();
            f.write_all(body).unwrap();
        }
        dir
    }

    #[test]
    fn write_then_list_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let projects = tmp.path().join("projects");
        fs::create_dir_all(&projects).unwrap();
        let dir = mk_project_dir(&projects, "-Users-joker", &[("a.jsonl", b"hello")]);
        let entry = write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &dir,
                slug: "-Users-joker",
                original_path: Some("/Users/joker"),
                bytes: 5,
                session_count: 1,
                claude_json_entry: None,
                history_lines: vec![],
                reason: Some("test".to_string()),
            },
        )
        .unwrap();
        assert_eq!(entry.slug, "-Users-joker");
        assert_eq!(entry.bytes, 5);
        assert!(!dir.exists(), "source dir should be moved");
        let listing = list(&data_dir, ProjectTrashFilter::default()).unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].id, entry.id);
        assert_eq!(listing.total_bytes, 5);
    }

    #[test]
    fn restore_round_trip_brings_back_dir_and_sibling_state() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let config_dir = tmp.path().join(".claude");
        let projects = config_dir.join("projects");
        fs::create_dir_all(&projects).unwrap();
        let dir = mk_project_dir(&projects, "-Users-joker", &[("a.jsonl", b"hello")]);

        let claude_json = tmp.path().join(".claude.json");
        fs::write(
            &claude_json,
            serde_json::json!({
                "projects": {}
            })
            .to_string(),
        )
        .unwrap();
        let history = config_dir.join("history.jsonl");
        fs::write(&history, "").unwrap();

        let snap_value = serde_json::json!({"trustDialogAccepted": true});
        let entry = write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &dir,
                slug: "-Users-joker",
                original_path: Some("/Users/joker"),
                bytes: 5,
                session_count: 1,
                claude_json_entry: Some(snap_value.clone()),
                history_lines: vec![r#"{"project":"/Users/joker","display":"ls"}"#.to_string()],
                reason: None,
            },
        )
        .unwrap();

        let report = restore(
            &data_dir,
            &entry.id,
            &config_dir,
            Some(&claude_json),
            Some(&history),
        )
        .unwrap();
        assert_eq!(report.restored_dir, dir);
        assert!(report.claude_json_restored);
        assert_eq!(report.history_lines_restored, 1);
        assert!(dir.exists(), "dir restored");

        // .claude.json contains the restored entry.
        let cj: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert_eq!(cj["projects"]["/Users/joker"], snap_value);

        // history.jsonl appended the saved line.
        let h = fs::read_to_string(&history).unwrap();
        assert!(h.contains(r#""project":"/Users/joker""#));

        // Batch dir gone after restore.
        assert!(!trash_root(&data_dir).join(&entry.id).exists());
    }

    #[test]
    fn restore_refuses_to_clobber_existing_dir() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let config_dir = tmp.path().join(".claude");
        let projects = config_dir.join("projects");
        fs::create_dir_all(&projects).unwrap();
        let dir = mk_project_dir(&projects, "-Users-joker", &[("a.jsonl", b"hello")]);

        let entry = write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &dir,
                slug: "-Users-joker",
                original_path: Some("/Users/joker"),
                bytes: 5,
                session_count: 1,
                claude_json_entry: None,
                history_lines: vec![],
                reason: None,
            },
        )
        .unwrap();
        // User reran `claude` in $HOME, recreating the dir.
        mk_project_dir(&projects, "-Users-joker", &[("b.jsonl", b"new")]);
        let err = restore(&data_dir, &entry.id, &config_dir, None, None).unwrap_err();
        assert!(matches!(err, ProjectTrashError::RestoreCollision(_)));
    }

    #[test]
    fn restore_does_not_clobber_existing_claude_json_key() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let config_dir = tmp.path().join(".claude");
        let projects = config_dir.join("projects");
        fs::create_dir_all(&projects).unwrap();
        let dir = mk_project_dir(&projects, "-Users-joker", &[("a.jsonl", b"hello")]);

        let claude_json = tmp.path().join(".claude.json");
        fs::write(
            &claude_json,
            serde_json::json!({
                "projects": {"/Users/joker": {"current": true}}
            })
            .to_string(),
        )
        .unwrap();

        let entry = write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &dir,
                slug: "-Users-joker",
                original_path: Some("/Users/joker"),
                bytes: 5,
                session_count: 1,
                claude_json_entry: Some(serde_json::json!({"old": true})),
                history_lines: vec![],
                reason: None,
            },
        )
        .unwrap();

        let report = restore(&data_dir, &entry.id, &config_dir, Some(&claude_json), None).unwrap();
        assert!(!report.claude_json_restored, "live key wins");
        let cj: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert_eq!(cj["projects"]["/Users/joker"]["current"], true);
        assert!(cj["projects"]["/Users/joker"].get("old").is_none());
    }

    #[test]
    fn empty_removes_matching_batches_and_returns_freed_bytes() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let projects = tmp.path().join("projects");
        fs::create_dir_all(&projects).unwrap();
        let d1 = mk_project_dir(&projects, "-a", &[("x", b"aaaa")]);
        let d2 = mk_project_dir(&projects, "-b", &[("y", b"bbbbbbbb")]);
        write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &d1,
                slug: "-a",
                original_path: None,
                bytes: 4,
                session_count: 0,
                claude_json_entry: None,
                history_lines: vec![],
                reason: None,
            },
        )
        .unwrap();
        write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &d2,
                slug: "-b",
                original_path: None,
                bytes: 8,
                session_count: 0,
                claude_json_entry: None,
                history_lines: vec![],
                reason: None,
            },
        )
        .unwrap();
        let freed = empty(&data_dir, ProjectTrashFilter::default()).unwrap();
        assert_eq!(freed, 12);
        let listing = list(&data_dir, ProjectTrashFilter::default()).unwrap();
        assert!(listing.entries.is_empty());
    }

    #[test]
    fn gc_only_removes_batches_older_than_threshold() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let projects = tmp.path().join("projects");
        fs::create_dir_all(&projects).unwrap();
        let d = mk_project_dir(&projects, "-fresh", &[("x", b"x")]);
        write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &d,
                slug: "-fresh",
                original_path: None,
                bytes: 1,
                session_count: 0,
                claude_json_entry: None,
                history_lines: vec![],
                reason: None,
            },
        )
        .unwrap();
        let freed = gc(&data_dir, Duration::from_secs(86400)).unwrap();
        assert_eq!(freed, 0);
        assert_eq!(
            list(&data_dir, ProjectTrashFilter::default())
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn list_missing_root_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let listing = list(tmp.path(), ProjectTrashFilter::default()).unwrap();
        assert!(listing.entries.is_empty());
        assert_eq!(listing.total_bytes, 0);
    }

    #[test]
    fn write_missing_source_is_explicit_error() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let missing = tmp.path().join("nope");
        let err = write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &missing,
                slug: "-a",
                original_path: None,
                bytes: 0,
                session_count: 0,
                claude_json_entry: None,
                history_lines: vec![],
                reason: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ProjectTrashError::SourceMissing(_)));
    }

    #[test]
    fn write_rejects_invalid_slug() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let projects = tmp.path().join("projects");
        fs::create_dir_all(&projects).unwrap();
        // Need a real source dir or the missing-source check fires
        // first; `validate_slug` runs before metadata.
        let dir = mk_project_dir(&projects, "x", &[]);
        for bad in ["..", ".", "", "a/b", "a\\b", ".hidden"] {
            let err = write(
                &data_dir,
                ProjectTrashPut {
                    source_dir: &dir,
                    slug: bad,
                    original_path: None,
                    bytes: 0,
                    session_count: 0,
                    claude_json_entry: None,
                    history_lines: vec![],
                    reason: None,
                },
            )
            .unwrap_err();
            assert!(
                matches!(err, ProjectTrashError::InvalidSlug(_)),
                "expected InvalidSlug for {bad:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn restore_rejects_path_traversal_in_entry_id() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let config_dir = tmp.path().join(".claude");
        let projects = config_dir.join("projects");
        fs::create_dir_all(&projects).unwrap();
        let dir = mk_project_dir(&projects, "-x", &[("a", b"a")]);
        write(
            &data_dir,
            ProjectTrashPut {
                source_dir: &dir,
                slug: "-x",
                original_path: None,
                bytes: 1,
                session_count: 0,
                claude_json_entry: None,
                history_lines: vec![],
                reason: None,
            },
        )
        .unwrap();
        for bad in [
            "../../../etc/passwd",
            "..",
            "",
            "/",
            "/absolute/path",
            "..\\windows\\path",
            "20260422T120000Z-deadbeef/../other",
            "20260422T120000Z", // truncated
            "malformed",
        ] {
            let err = restore(&data_dir, bad, &config_dir, None, None).unwrap_err();
            assert!(
                matches!(err, ProjectTrashError::EntryNotFound(_)),
                "expected EntryNotFound for {bad:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn is_valid_batch_id_accepts_canonical_shape() {
        assert!(is_valid_batch_id("20260426T153045Z-deadbeef"));
        assert!(is_valid_batch_id("20260426T153045Z-00000000"));
        assert!(!is_valid_batch_id("20260426T153045Z-DEADBEEF"));
        assert!(!is_valid_batch_id("20260426T153045Z-dead"));
        assert!(!is_valid_batch_id("20260426X153045Z-deadbeef"));
    }

    #[test]
    fn cross_device_fallback_via_copy_then_remove() {
        // Simulate cross-device by trashing into a different tmp tree.
        // rename() will succeed on the same fs, but we still exercise
        // copy_dir_recursive directly via a test with a nested file.
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        let nested = src.join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("leaf.jsonl"), b"deep").unwrap();
        copy_dir_recursive(&src, &dest).unwrap();
        assert_eq!(fs::read(dest.join("a/b/c/leaf.jsonl")).unwrap(), b"deep");
    }
}
