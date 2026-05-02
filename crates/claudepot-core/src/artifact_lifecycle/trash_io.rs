//! IO helpers for the trash module: atomic manifest write,
//! cross-volume payload move/copy, recursive directory copy with
//! symlink handling, payload hashing, and source removal.
//!
//! Sharded out of `trash.rs` to keep that file's public-API surface
//! readable. Cross-platform behavior (Windows symlinks, EXDEV
//! detection) lives here behind cfg gates.

use crate::artifact_lifecycle::error::{LifecycleError, Result};
use crate::artifact_lifecycle::paths::PayloadKind;
use crate::artifact_lifecycle::trash::TrashManifest;
use sha2::{Digest, Sha256};
use std::io::{BufReader, Read};
use std::path::Path;

/// Buffer size for streaming hash. 64 KiB strikes the usual balance:
/// bigger than the typical small-file payload (so two reads do most
/// files) but small enough to keep the resident set quiet on
/// pathological inputs (someone trashes a 2 GiB markdown by accident).
const HASH_BUF_BYTES: usize = 64 * 1024;

/// Atomic manifest write — tempfile + rename so a crash mid-write
/// either leaves the previous manifest (if any) or no manifest at all.
pub(super) fn write_manifest_atomic(staging: &Path, manifest: &TrashManifest) -> Result<()> {
    let body = serde_json::to_vec_pretty(manifest)
        .map_err(|e| LifecycleError::io("serialize manifest")(std::io::Error::other(e)))?;
    let target = staging.join("manifest.json");
    let tmp = staging.join(".manifest.tmp");
    std::fs::write(&tmp, &body).map_err(LifecycleError::io("write manifest tmp"))?;
    std::fs::rename(&tmp, &target).map_err(LifecycleError::io("commit manifest"))?;
    Ok(())
}

/// Move (preferred) or copy (cross-volume fallback) `source` to
/// `target`. Both directories and files are handled; symlinks
/// preserve the link rather than the target.
///
/// EXDEV detection is narrow: only the documented cross-device
/// errno (18 / `EXDEV` on Unix, `ERROR_NOT_SAME_DEVICE` on Windows
/// = 17 via raw_os_error). The previous "any ErrorKind::Other"
/// fallback was too broad — a permission denied on rename would
/// silently widen into a copy+remove path. Now non-EXDEV errors
/// surface directly.
pub(super) fn move_or_copy(source: &Path, target: &Path, kind: PayloadKind) -> Result<()> {
    match std::fs::rename(source, target) {
        Ok(()) => return Ok(()),
        Err(err) => {
            if !is_exdev(&err) {
                return Err(LifecycleError::io("rename to trash")(err));
            }
        }
    }
    // Audit fix for trash_io.rs:55 — preserve symlink shape on the
    // cross-volume fallback. `std::fs::copy` resolves the symlink
    // and copies the target's bytes, so a trashed symlink would
    // restore as a regular file (losing link semantics, possibly
    // copying secret contents from outside the project root). We
    // probe the source via `symlink_metadata` first; if it's a
    // symlink, we preserve it as a symlink at the target. On
    // Windows where unprivileged symlink creation isn't always
    // available, we fall back to copying the resolved target — the
    // trash is recoverable so a lossy fallback is acceptable.
    let src_meta =
        std::fs::symlink_metadata(source).map_err(LifecycleError::io("stat trash source"))?;
    if src_meta.file_type().is_symlink() {
        let link_target =
            std::fs::read_link(source).map_err(LifecycleError::io("read trash symlink"))?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&link_target, target)
                .map_err(LifecycleError::io("write trash symlink"))?;
        }
        #[cfg(windows)]
        {
            // Best-effort: symlinkw if elevated, otherwise copy
            // the resolved bytes (lossy — see copy_dir_recursive).
            let abs = if link_target.is_absolute() {
                link_target.clone()
            } else {
                source
                    .parent()
                    .map(|p| p.join(&link_target))
                    .unwrap_or(link_target.clone())
            };
            if abs.is_dir() {
                copy_dir_recursive(&abs, target)?;
            } else {
                std::fs::copy(&abs, target).map_err(LifecycleError::io("copy symlink target"))?;
            }
        }
        return Ok(());
    }
    match kind {
        PayloadKind::File => {
            std::fs::copy(source, target).map_err(LifecycleError::io("copy file to trash"))?;
        }
        PayloadKind::Directory => {
            copy_dir_recursive(source, target)?;
        }
    }
    Ok(())
}

/// True iff `err` is the documented "source and destination on
/// different filesystems" error. Linux: EXDEV (18). Windows:
/// ERROR_NOT_SAME_DEVICE (17). On Rust 1.85+ we'd prefer
/// `ErrorKind::CrossesDevices`, but checking raw codes works on
/// older toolchains too.
fn is_exdev(err: &std::io::Error) -> bool {
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

/// Recursive directory copy that preserves symlinks on Unix; Windows
/// can't create symlinks without elevation, so we copy the resolved
/// target as a fallback (lossy but trash is recoverable).
pub(super) fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target).map_err(LifecycleError::io("create copy target"))?;
    for entry in std::fs::read_dir(source).map_err(LifecycleError::io("read copy source"))? {
        let entry = entry.map_err(LifecycleError::io("read copy entry"))?;
        let src = entry.path();
        let dst = target.join(entry.file_name());
        let ft = entry
            .file_type()
            .map_err(LifecycleError::io("file type during copy"))?;
        if ft.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if ft.is_symlink() {
            #[cfg(unix)]
            {
                let link = std::fs::read_link(&src).map_err(LifecycleError::io("read symlink"))?;
                std::os::unix::fs::symlink(link, &dst)
                    .map_err(LifecycleError::io("write symlink"))?;
            }
            #[cfg(windows)]
            {
                let resolved = crate::path_utils::canonicalize_simplified(&src)
                    .map_err(LifecycleError::io("resolve symlink"))?;
                if resolved.is_dir() {
                    copy_dir_recursive(&resolved, &dst)?;
                } else {
                    std::fs::copy(&resolved, &dst)
                        .map_err(LifecycleError::io("copy symlink target"))?;
                }
            }
        } else {
            std::fs::copy(&src, &dst).map_err(LifecycleError::io("copy file"))?;
        }
    }
    Ok(())
}

/// Remove the source artifact after a same-volume rename to trash
/// already moved it. Used only by the cross-volume copy fallback —
/// rename leaves nothing behind.
pub(super) fn remove_source(source: &Path, kind: PayloadKind) -> Result<()> {
    match kind {
        PayloadKind::File => {
            std::fs::remove_file(source).map_err(LifecycleError::io("remove source file"))
        }
        PayloadKind::Directory => {
            std::fs::remove_dir_all(source).map_err(LifecycleError::io("remove source dir"))
        }
    }
}

/// Compute (byte_count, sha256?) for the trashed payload. Files
/// hash via a streaming reader so a multi-GB payload doesn't spike
/// the resident set. Directories report total size only (computing a
/// tree-hash is expensive and rarely useful in practice).
pub(super) fn hash_payload(path: &Path, kind: PayloadKind) -> Result<(u64, Option<String>)> {
    match kind {
        PayloadKind::File => {
            let (bytes, hex) = stream_hash_file(path)?;
            Ok((bytes, Some(hex)))
        }
        PayloadKind::Directory => {
            let mut total = 0u64;
            walk_dir(path, &mut |entry_kind, p| match entry_kind {
                WalkEntryKind::File | WalkEntryKind::Symlink => {
                    if let Ok(meta) = std::fs::symlink_metadata(p) {
                        total += meta.len();
                    }
                }
                WalkEntryKind::Directory => {}
            })?;
            Ok((total, None))
        }
    }
}

/// Streaming SHA-256 over a file. Returns (byte_count, hex digest).
/// Reads in `HASH_BUF_BYTES` chunks via `BufReader::read` so memory
/// stays bounded regardless of file size.
pub(crate) fn stream_hash_file(path: &Path) -> Result<(u64, String)> {
    let file = std::fs::File::open(path).map_err(LifecycleError::io("hash file open"))?;
    let mut reader = BufReader::with_capacity(HASH_BUF_BYTES, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_BUF_BYTES];
    let mut total: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(LifecycleError::io("hash file read"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((total, hex::encode(hasher.finalize())))
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn walk_dir_does_not_recurse_into_symlinks() {
        // A symlink loop (link → root) would recurse forever if the
        // walker followed it. The lstat-based walker reports the link
        // as Symlink and stops.
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), b"x").unwrap();
        // Create root/loop → root.
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();

        let mut entries: Vec<(WalkEntryKind, std::path::PathBuf)> = Vec::new();
        walk_dir(&root, &mut |k, p| entries.push((k, p.to_path_buf()))).unwrap();
        assert!(
            entries
                .iter()
                .any(|(k, p)| *k == WalkEntryKind::Symlink && p.ends_with("loop")),
            "loop must surface as a Symlink entry"
        );
        // No nested traversal — only one level of File entries (a.md).
        let file_count = entries
            .iter()
            .filter(|(k, _)| *k == WalkEntryKind::File)
            .count();
        assert_eq!(file_count, 1, "must not recurse into the symlink loop");
    }

    #[test]
    fn stream_hash_file_matches_in_memory_hash() {
        // Sanity: streaming hash agrees with hashing all bytes at once
        // for a moderately-sized file (larger than HASH_BUF_BYTES).
        let tmp = tempdir().unwrap();
        let f = tmp.path().join("big.bin");
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i & 0xff) as u8).collect();
        std::fs::write(&f, &bytes).unwrap();

        let (n, hex) = stream_hash_file(&f).unwrap();
        assert_eq!(n, bytes.len() as u64);

        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let expected = hex::encode(hasher.finalize());
        assert_eq!(hex, expected);
    }
}

/// Tag passed to the `walk_dir` callback so the caller can decide
/// what to do with each entry without re-stat'ing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalkEntryKind {
    File,
    Directory,
    Symlink,
}

/// Recursive walker that uses `symlink_metadata` (lstat) so symlink
/// entries are reported AS symlinks and never traversed. Without
/// this guard a symlink-to-`/` (or a self-referential dir symlink)
/// would loop until stack overflow.
///
/// Callers that want symlink contents traversed must follow the
/// link explicitly — the walker won't do it for them.
pub(crate) fn walk_dir(root: &Path, f: &mut dyn FnMut(WalkEntryKind, &Path)) -> Result<()> {
    let meta = std::fs::symlink_metadata(root).map_err(LifecycleError::io("walk dir root stat"))?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        f(WalkEntryKind::Symlink, root);
        return Ok(());
    }
    if ft.is_file() {
        f(WalkEntryKind::File, root);
        return Ok(());
    }
    // Directory.
    f(WalkEntryKind::Directory, root);
    for entry in std::fs::read_dir(root).map_err(LifecycleError::io("walk dir"))? {
        let entry = entry.map_err(LifecycleError::io("walk dir entry"))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .map_err(LifecycleError::io("walk dir file_type"))?;
        if ft.is_symlink() {
            f(WalkEntryKind::Symlink, &path);
        } else if ft.is_dir() {
            walk_dir(&path, f)?;
        } else {
            f(WalkEntryKind::File, &path);
        }
    }
    Ok(())
}
