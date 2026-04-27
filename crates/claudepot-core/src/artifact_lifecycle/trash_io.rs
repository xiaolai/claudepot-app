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
use std::path::Path;

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
        return raw == Some(libc::EXDEV);
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
                let resolved =
                    std::fs::canonicalize(&src).map_err(LifecycleError::io("resolve symlink"))?;
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
/// hash; directories report total size only (computing a tree-hash
/// is expensive and rarely useful in practice).
pub(super) fn hash_payload(path: &Path, kind: PayloadKind) -> Result<(u64, Option<String>)> {
    match kind {
        PayloadKind::File => {
            let bytes = std::fs::read(path).map_err(LifecycleError::io("hash file"))?;
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            Ok((bytes.len() as u64, Some(hex::encode(hasher.finalize()))))
        }
        PayloadKind::Directory => {
            let mut total = 0u64;
            walk_dir(path, &mut |p| {
                if let Ok(meta) = std::fs::metadata(p) {
                    total += meta.len();
                }
            })?;
            Ok((total, None))
        }
    }
}

fn walk_dir(root: &Path, f: &mut dyn FnMut(&Path)) -> Result<()> {
    if root.is_file() {
        f(root);
        return Ok(());
    }
    for entry in std::fs::read_dir(root).map_err(LifecycleError::io("walk dir"))? {
        let entry = entry.map_err(LifecycleError::io("walk dir entry"))?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, f)?;
        } else {
            f(&path);
        }
    }
    Ok(())
}
