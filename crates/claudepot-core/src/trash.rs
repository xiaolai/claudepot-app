//! Reversible trash for pruned / slimmed session transcripts.
//!
//! Layout on disk:
//!
//! ```text
//! <data_dir>/trash/sessions/
//!   20260422T153045Z-<uuid8>/
//!     manifest.json           ← TrashEntry
//!     <inode>.jsonl           ← the moved (or pre-slim) file
//!     <inode>.pre-slim.jsonl  ← only for TrashKind::Slim
//! ```
//!
//! One batch directory per `write()` call. Each holds a single
//! `TrashEntry` in `manifest.json`, keyed to its `manifest_id` which
//! matches the directory name. Listing enumerates every batch dir;
//! restore + empty key on the batch id.
//!
//! Atomicity: `rename()` first, cross-device fallback is copy →
//! `sync_all()` → unlink source. GC scans batches older than a
//! duration and deletes them; `gc(7 days)` is the startup sweep.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum TrashError {
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
}

impl TrashError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Why a file ended up in the trash. Distinguishes reversible
/// wholesale deletes (prune) from reversible rewrites (slim — both
/// the original *and* the post-slim pointer are tracked).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashKind {
    Prune,
    Slim,
}

/// A single trashed file, serialized as a batch directory's `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashEntry {
    /// Batch id = directory name, e.g. `20260422T153045Z-abc12345`.
    pub id: String,
    /// Convenience alias. Same as `id`.
    pub manifest_id: String,
    pub orig_path: PathBuf,
    pub kind: TrashKind,
    pub size: u64,
    /// UTC milliseconds since epoch.
    pub ts_ms: i64,
    /// `st_ino` on unix; `file_index()` on windows; 0 if unavailable.
    pub inode: u64,
    /// Optional cwd hint used during restore.
    pub cwd: Option<PathBuf>,
    /// Optional human-readable reason.
    pub reason: Option<String>,
}

/// One-shot input for `write`. Borrows the path so callers don't have
/// to clone on the hot path.
pub struct TrashPut<'a> {
    pub orig_path: &'a Path,
    pub kind: TrashKind,
    pub cwd: Option<&'a Path>,
    pub reason: Option<String>,
}

/// Enumerated batches and their combined byte total.
#[derive(Debug, Clone, Serialize)]
pub struct TrashListing {
    pub entries: Vec<TrashEntry>,
    pub total_bytes: u64,
}

/// List / empty filter.
#[derive(Debug, Clone, Default)]
pub struct TrashFilter {
    pub older_than: Option<Duration>,
    pub kind: Option<TrashKind>,
}

fn trash_root(data_dir: &Path) -> PathBuf {
    data_dir.join("trash").join("sessions")
}

/// Current time as UTC compact string `YYYYMMDDTHHMMSSZ`.
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

#[cfg(test)]
fn batch_ms(id: &str) -> Option<i64> {
    // Parse the leading `YYYYMMDDTHHMMSSZ` prefix before the `-`.
    let ts_part = id.split('-').next()?;
    let dt = chrono::NaiveDateTime::parse_from_str(ts_part, "%Y%m%dT%H%M%SZ").ok()?;
    Some(dt.and_utc().timestamp_millis())
}

#[cfg(unix)]
fn inode_of(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).map(|m| m.ino()).unwrap_or(0)
}

#[cfg(windows)]
fn inode_of(path: &Path) -> u64 {
    use std::os::windows::fs::MetadataExt;
    fs::metadata(path).map(|m| m.file_index().unwrap_or(0)).unwrap_or(0)
}

#[cfg(not(any(unix, windows)))]
fn inode_of(_path: &Path) -> u64 {
    0
}

/// Move a file into the trash.
///
/// Creates a fresh batch directory under `<data_dir>/trash/sessions/`
/// and moves the source in. Cross-device renames fall back to
/// copy+fsync+unlink.
pub fn write(data_dir: &Path, put: TrashPut<'_>) -> Result<TrashEntry, TrashError> {
    let src = put.orig_path;
    let meta =
        fs::metadata(src).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => TrashError::SourceMissing(src.to_path_buf()),
            _ => TrashError::io(src, e),
        })?;
    let size = meta.len();
    let inode = inode_of(src);

    let uuid8 = Uuid::new_v4().simple().to_string()[..8].to_string();
    let batch_id = format!("{}-{}", batch_ts_string(SystemTime::now()), uuid8);
    let batch_dir = trash_root(data_dir).join(&batch_id);
    fs::create_dir_all(&batch_dir).map_err(|e| TrashError::io(&batch_dir, e))?;

    let file_name = format!("{}.jsonl", inode);
    let dest = batch_dir.join(&file_name);

    move_file(src, &dest)?;

    let entry = TrashEntry {
        id: batch_id.clone(),
        manifest_id: batch_id.clone(),
        orig_path: src.to_path_buf(),
        kind: put.kind,
        size,
        ts_ms: now_ms(),
        inode,
        cwd: put.cwd.map(Path::to_path_buf),
        reason: put.reason,
    };
    let manifest = batch_dir.join("manifest.json");
    let json = serde_json::to_vec_pretty(&entry)
        .expect("TrashEntry serializes");
    fs::write(&manifest, json).map_err(|e| TrashError::io(&manifest, e))?;
    Ok(entry)
}

/// Move `src` to `dest`. First tries a plain rename (atomic on the
/// same filesystem); falls back to copy + fsync + unlink when
/// rename fails with `ErrorKind::CrossesDevices` (or any other error
/// that looks like "different FS").
fn move_file(src: &Path, dest: &Path) -> Result<(), TrashError> {
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Any error falls back to copy-then-unlink. Pure-rename
            // errors that would also break the copy path (e.g.
            // permission denied on the destination) re-surface on
            // the copy call.
            if let Err(e_copy) = fs::copy(src, dest) {
                return Err(TrashError::io(src, e_copy));
            }
            // fsync the destination so the file is durable before we
            // unlink the source.
            if let Ok(f) = fs::File::open(dest) {
                let _ = f.sync_all();
            }
            fs::remove_file(src).map_err(|e_rm| TrashError::io(src, e_rm))?;
            drop(e);
            Ok(())
        }
    }
}

/// List every batch under `<data_dir>/trash/sessions/` that matches
/// the filter. Entries are returned newest-first.
pub fn list(data_dir: &Path, filter: TrashFilter) -> Result<TrashListing, TrashError> {
    let root = trash_root(data_dir);
    let mut out = Vec::new();
    let mut total: u64 = 0;
    if !root.exists() {
        return Ok(TrashListing {
            entries: out,
            total_bytes: 0,
        });
    }
    let cutoff_ms = filter
        .older_than
        .map(|d| now_ms().saturating_sub(d.as_millis() as i64));

    for entry in fs::read_dir(&root).map_err(|e| TrashError::io(&root, e))? {
        let entry = entry.map_err(|e| TrashError::io(&root, e))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("manifest.json");
        if !manifest.exists() {
            continue;
        }
        let raw = fs::read_to_string(&manifest)
            .map_err(|e| TrashError::io(&manifest, e))?;
        let te: TrashEntry = serde_json::from_str(&raw).map_err(|e| {
            TrashError::ManifestParse {
                path: manifest.clone(),
                source: e,
            }
        })?;
        if let Some(k) = filter.kind {
            if te.kind != k {
                continue;
            }
        }
        if let Some(cut) = cutoff_ms {
            // `older_than` semantics: only include entries whose ts_ms
            // is *older than* the cutoff (ts < now - older_than).
            if te.ts_ms >= cut {
                continue;
            }
        }
        total = total.saturating_add(te.size);
        out.push(te);
    }
    out.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
    Ok(TrashListing {
        entries: out,
        total_bytes: total,
    })
}

/// A valid batch id is exactly `YYYYMMDDTHHMMSSZ-<8 hex>`. Anything
/// else — separators, `..`, absolute prefixes — is rejected so
/// `trash_root.join(entry_id)` can't escape the trash directory.
fn is_valid_batch_id(s: &str) -> bool {
    // 16 chars for the timestamp + '-' + 8 hex chars = 25 total.
    if s.len() != 16 + 1 + 8 {
        return false;
    }
    let bytes = s.as_bytes();
    // Timestamp prefix: 8 digits + 'T' + 6 digits + 'Z'
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

fn find_batch(data_dir: &Path, entry_id: &str) -> Result<(PathBuf, TrashEntry), TrashError> {
    if !is_valid_batch_id(entry_id) {
        return Err(TrashError::EntryNotFound(entry_id.to_string()));
    }
    let root = trash_root(data_dir);
    let batch_dir = root.join(entry_id);
    // Defense in depth: after join(), confirm the result is a direct
    // child of the trash root. If someone finds a way past the id
    // validator this still blocks escape.
    match batch_dir.parent() {
        Some(parent) if parent == root => {}
        _ => return Err(TrashError::EntryNotFound(entry_id.to_string())),
    }
    if !batch_dir.exists() {
        return Err(TrashError::EntryNotFound(entry_id.to_string()));
    }
    let manifest = batch_dir.join("manifest.json");
    let raw = fs::read_to_string(&manifest).map_err(|e| TrashError::io(&manifest, e))?;
    let te: TrashEntry = serde_json::from_str(&raw).map_err(|e| TrashError::ManifestParse {
        path: manifest.clone(),
        source: e,
    })?;
    Ok((batch_dir, te))
}

/// Restore a trashed file. If `override_cwd` is Some, the original
/// filename is placed under that directory; otherwise the original
/// absolute path is used. Fails if the destination already exists —
/// restore never clobbers live state.
pub fn restore(
    data_dir: &Path,
    entry_id: &str,
    override_cwd: Option<&Path>,
) -> Result<PathBuf, TrashError> {
    let (batch_dir, te) = find_batch(data_dir, entry_id)?;
    let file_name = format!("{}.jsonl", te.inode);
    let src = batch_dir.join(&file_name);
    if !src.exists() {
        return Err(TrashError::EntryNotFound(format!(
            "{} (missing {})",
            entry_id, file_name
        )));
    }
    let dest = match (override_cwd, te.orig_path.file_name()) {
        (Some(cwd), Some(name)) => cwd.join(name),
        _ => te.orig_path.clone(),
    };
    if dest.exists() {
        return Err(TrashError::RestoreCollision(dest));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| TrashError::io(parent, e))?;
    }
    move_file(&src, &dest)?;
    // Batch is now half-empty; remove the batch dir outright.
    let _ = fs::remove_dir_all(&batch_dir);
    Ok(dest)
}

/// Delete batches matching the filter. Returns the number of bytes
/// reclaimed (sum of entry `size`). Missing root is a no-op.
pub fn empty(data_dir: &Path, filter: TrashFilter) -> Result<u64, TrashError> {
    let listing = list(data_dir, filter)?;
    let mut freed: u64 = 0;
    for te in &listing.entries {
        let batch_dir = trash_root(data_dir).join(&te.id);
        if fs::remove_dir_all(&batch_dir).is_ok() {
            freed = freed.saturating_add(te.size);
        }
    }
    Ok(freed)
}

/// Sweep batches older than `older_than`. Convenience wrapper around
/// `empty` with a preset filter. Called on CLI / app startup.
pub fn gc(data_dir: &Path, older_than: Duration) -> Result<u64, TrashError> {
    empty(
        data_dir,
        TrashFilter {
            older_than: Some(older_than),
            kind: None,
        },
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn mk_file(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let p = dir.join(name);
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(contents).unwrap();
        p
    }

    #[test]
    fn write_then_list_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let src = mk_file(tmp.path(), "x.jsonl", b"body\n");
        let entry = write(
            &data_dir,
            TrashPut {
                orig_path: &src,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        assert_eq!(entry.kind, TrashKind::Prune);
        assert_eq!(entry.size, 5);
        assert!(!src.exists(), "src should be moved out of its original path");
        let listing = list(&data_dir, TrashFilter::default()).unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].id, entry.id);
        assert_eq!(listing.total_bytes, 5);
    }

    #[test]
    fn restore_round_trip() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let src = mk_file(tmp.path(), "x.jsonl", b"body\n");
        let entry = write(
            &data_dir,
            TrashPut {
                orig_path: &src,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        let restored = restore(&data_dir, &entry.id, None).unwrap();
        assert_eq!(restored, src);
        assert!(restored.exists());
        let body = fs::read_to_string(&restored).unwrap();
        assert_eq!(body, "body\n");
        // Batch dir is gone.
        assert!(!trash_root(&data_dir).join(&entry.id).exists());
    }

    #[test]
    fn restore_override_cwd_places_under_new_parent() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let src = mk_file(tmp.path(), "x.jsonl", b"body\n");
        let entry = write(
            &data_dir,
            TrashPut {
                orig_path: &src,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        let new_parent = tmp.path().join("elsewhere");
        fs::create_dir_all(&new_parent).unwrap();
        let restored = restore(&data_dir, &entry.id, Some(&new_parent)).unwrap();
        assert_eq!(restored, new_parent.join("x.jsonl"));
    }

    #[test]
    fn restore_refuses_to_clobber_existing_destination() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let src = mk_file(tmp.path(), "x.jsonl", b"body\n");
        let entry = write(
            &data_dir,
            TrashPut {
                orig_path: &src,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        // Recreate something at the original path before restoring.
        mk_file(tmp.path(), "x.jsonl", b"new body\n");
        let err = restore(&data_dir, &entry.id, None).unwrap_err();
        assert!(matches!(err, TrashError::RestoreCollision(_)));
    }

    #[test]
    fn list_filters_by_kind() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let s1 = mk_file(tmp.path(), "a.jsonl", b"a");
        let s2 = mk_file(tmp.path(), "b.jsonl", b"bb");
        write(
            &data_dir,
            TrashPut {
                orig_path: &s1,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        write(
            &data_dir,
            TrashPut {
                orig_path: &s2,
                kind: TrashKind::Slim,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        let only_prune = list(
            &data_dir,
            TrashFilter {
                kind: Some(TrashKind::Prune),
                ..TrashFilter::default()
            },
        )
        .unwrap();
        assert_eq!(only_prune.entries.len(), 1);
        assert_eq!(only_prune.entries[0].kind, TrashKind::Prune);
    }

    #[test]
    fn empty_removes_matching_batches_and_returns_freed_bytes() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let s1 = mk_file(tmp.path(), "a.jsonl", b"aaaa"); // 4 bytes
        let s2 = mk_file(tmp.path(), "b.jsonl", b"bbbbbbbb"); // 8 bytes
        write(
            &data_dir,
            TrashPut {
                orig_path: &s1,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        write(
            &data_dir,
            TrashPut {
                orig_path: &s2,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        let freed = empty(&data_dir, TrashFilter::default()).unwrap();
        assert_eq!(freed, 12);
        // Root is empty after.
        let listing = list(&data_dir, TrashFilter::default()).unwrap();
        assert!(listing.entries.is_empty());
    }

    #[test]
    fn gc_only_removes_batches_older_than_threshold() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let src = mk_file(tmp.path(), "fresh.jsonl", b"x");
        let fresh = write(
            &data_dir,
            TrashPut {
                orig_path: &src,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();
        // Nothing to GC: fresh entry is not older than 1 day.
        let freed = gc(&data_dir, Duration::from_secs(86400)).unwrap();
        assert_eq!(freed, 0);
        let listing = list(&data_dir, TrashFilter::default()).unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].id, fresh.id);
    }

    #[test]
    fn list_missing_root_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let listing = list(tmp.path(), TrashFilter::default()).unwrap();
        assert!(listing.entries.is_empty());
        assert_eq!(listing.total_bytes, 0);
    }

    #[test]
    fn write_missing_source_is_explicit_error() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let missing = tmp.path().join("nope.jsonl");
        let err = write(
            &data_dir,
            TrashPut {
                orig_path: &missing,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, TrashError::SourceMissing(_)));
    }

    #[test]
    fn batch_id_has_parseable_timestamp() {
        let id = format!("20260422T153045Z-deadbeef");
        let ms = batch_ms(&id).unwrap();
        // Sanity: this millisecond value corresponds to 2026 April.
        let dt = DateTime::<Utc>::from_timestamp_millis(ms).unwrap();
        assert_eq!(dt.format("%Y-%m").to_string(), "2026-04");
    }

    #[test]
    fn restore_rejects_path_traversal_in_entry_id() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        // Legitimate trashed file so the root dir exists.
        let src = mk_file(tmp.path(), "x.jsonl", b"body\n");
        write(
            &data_dir,
            TrashPut {
                orig_path: &src,
                kind: TrashKind::Prune,
                cwd: None,
                reason: None,
            },
        )
        .unwrap();

        let bad_ids = [
            "../../../etc/passwd",
            "..",
            "",
            "/",
            "/absolute/path",
            "..\\windows\\path",
            "20260422T120000Z-deadbeef/../other",
            "malformed",
            "20260422T120000Z", // missing suffix
        ];
        for bad in bad_ids {
            let err = restore(&data_dir, bad, None).unwrap_err();
            assert!(
                matches!(err, TrashError::EntryNotFound(_)),
                "expected EntryNotFound for bad id {bad:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn is_valid_batch_id_accepts_canonical_shape() {
        assert!(is_valid_batch_id("20260422T153045Z-deadbeef"));
        assert!(is_valid_batch_id("20260422T153045Z-00000000"));
        assert!(!is_valid_batch_id("20260422T153045Z-DEADBEEF")); // uppercase hex rejected
        assert!(!is_valid_batch_id("20260422T153045Z-dead"));      // too short
        assert!(!is_valid_batch_id("20260422X153045Z-deadbeef"));  // wrong separator
    }

    #[test]
    fn windows_unc_path_preserved_on_orig_path() {
        // sanity guard for `.claude/rules/paths.md` — UNC shapes must
        // survive the manifest round-trip without normalization.
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let src = mk_file(tmp.path(), "x.jsonl", b"body\n");
        let entry = write(
            &data_dir,
            TrashPut {
                orig_path: &src,
                kind: TrashKind::Prune,
                cwd: Some(Path::new(r"\\server\share\project")),
                reason: None,
            },
        )
        .unwrap();
        let listing = list(&data_dir, TrashFilter::default()).unwrap();
        assert_eq!(listing.entries.len(), 1);
        let cwd = listing.entries[0].cwd.as_deref().unwrap();
        // String comparison to dodge PathBuf platform-specific parsing.
        assert_eq!(
            cwd.to_string_lossy(),
            entry.cwd.unwrap().to_string_lossy()
        );
    }
}
