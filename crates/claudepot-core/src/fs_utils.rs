//! Shared filesystem utilities.

use crate::proc_utils::NoWindowExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-global monotonic counter feeding [`atomic_write`]'s temp
/// suffix. The pid alone is not unique enough: a multi-threaded
/// process (Tauri always is) can issue two concurrent writes to the
/// same target, both computing `.{name}.tmp.{pid}` and interleaving
/// their `write_all` into one file. The counter makes every temp
/// path distinct within a process; the pid keeps it distinct across
/// processes (grill finding F9).
static TEMP_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Recursively copy a directory, skipping symlinks.
///
/// Uses `DirEntry::file_type` (not `metadata`) so symlinks are
/// identified as symlinks instead of being resolved to their target's
/// type — the previous code used `entry.metadata()?.file_type()` which
/// silently followed the link and could copy through it (a symlink to
/// a regular file looked like a regular file, a symlink to a directory
/// could trigger unbounded recursion outside the source tree).
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        } else if ft.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            copy_file_retried(&entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

/// Copy a single file, retrying on Windows ERROR_SHARING_VIOLATION (os error 32).
///
/// MSIX-virtualized Chromium profile files (Cookies, Local Storage, IndexedDB)
/// can stay transiently locked for up to ~2 s after the Electron process tree
/// exits, even after `is_running()` returns false. The 1 s post-quit settle
/// delay in `desktop_prelude` handles the common case; this retry is the
/// belt-and-suspenders fallback for slower machines or heavier profiles.
pub fn copy_file_retried(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        const MAX_RETRIES: u32 = 4;
        for attempt in 0..MAX_RETRIES {
            match std::fs::copy(src, dst) {
                Ok(_) => return Ok(()),
                Err(e) if attempt + 1 < MAX_RETRIES && e.raw_os_error() == Some(32) => {
                    tracing::debug!(
                        "copy_file_retried: sharing violation on {}, retry {}/{}",
                        src.display(),
                        attempt + 1,
                        MAX_RETRIES - 1
                    );
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// Atomically write `contents` to `path`: write to a sibling temp file,
/// `fsync` it, then rename into place. On Unix, the final file is
/// chmodded to `0o600` (owner read/write only) because every caller we
/// have writes credential-adjacent data.
///
/// Centralizes the temp+rename+chmod pattern that was previously
/// duplicated in `cli_backend/storage.rs` and `cli_backend/credfile.rs`.
pub fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("atomic_write target has no parent: {}", path.display()),
        )
    })?;
    std::fs::create_dir_all(parent)?;

    // Sibling temp so rename stays atomic (same filesystem). The
    // suffix carries both the pid (cross-process uniqueness) and a
    // process-global counter (intra-process uniqueness): without the
    // counter, two threads writing the same target concurrently
    // would compute the identical temp path and interleave their
    // `write_all` into one corrupt file (grill finding F9).
    let tmp = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("claudepot"),
        std::process::id(),
        TEMP_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    // Write + fsync under a scope so the file handle is closed before
    // rename. On Unix the temp file is opened with mode 0o600 from the
    // start, so secrets are never world-readable, even briefly between
    // the create() and a follow-up chmod.
    {
        #[cfg(unix)]
        let mut f = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?
        };
        #[cfg(not(unix))]
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }

    // rename is atomic within a filesystem. If it fails, clean up the
    // temp to avoid littering.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Find the `claude` CLI binary. Probes every well-known install
/// location (see [`crate::path_env::tool_dirs`]) then falls back to
/// a PATH search via `which`/`where`.
///
/// Both the probe list and the `which` fallback go through the
/// enriched PATH so the binary resolves even when Claudepot is
/// launched from Dock/Finder with a minimal `PATH` — without that,
/// onboarding's `claude auth login` would silently fail to find a
/// Homebrew- or toolchain-installed `claude`.
pub fn find_claude_binary() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    #[cfg(not(target_os = "windows"))]
    {
        for dir in crate::path_env::tool_dirs() {
            candidates.push(dir.join("claude"));
        }
        // System dir — not in `tool_dirs` (already on PATH), but a
        // `claude` could still be installed there.
        candidates.push(PathBuf::from("/usr/bin/claude"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local").join("bin").join("claude.exe"));
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            candidates.push(PathBuf::from(appdata).join("npm").join("claude.cmd"));
        }
    }

    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }

    // Fallback: PATH search, enriched so a Dock launch still hits
    // Homebrew / per-user toolchain directories.
    let which_cmd = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };
    let claude_name = if cfg!(target_os = "windows") {
        "claude.cmd"
    } else {
        "claude"
    };
    if let Ok(output) = std::process::Command::new(which_cmd)
        .arg(claude_name)
        .env("PATH", crate::path_env::enriched_path())
        .no_window()
        .output()
    {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout)
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }

    None
}

/// Get the version of a claude binary.
pub fn claude_version(path: &Path) -> Option<String> {
    std::process::Command::new(path)
        .arg("--version")
        .no_window()
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

/// Platform-specific file-identity proxy used in the
/// `(size, mtime_ns, inode)` staleness triple stored in `sessions.db`.
///
/// - **Unix**: real POSIX inode via `MetadataExt::ino()`.
/// - **Windows**: `creation_time()` (FILETIME, stable since Rust 1.1).
///   `MetadataExt::file_index()` would be the natural choice but it
///   sits behind nightly-only `windows_by_handle`
///   (rust-lang/rust#63010) and breaks the release build (E0658,
///   v0.1.36). `creation_time()` is strictly safer for the equality-
///   only consumer: changes when the file is created or replaced,
///   stays constant across in-place modifications.
/// - **Other targets**: 0 (the staleness check degrades to size + mtime).
///
/// All three shared-memory writers — `session_index/codec.rs`,
/// `shared_memory/indexer.rs`, `shared_memory/claude_exchanges.rs` —
/// must use this function (not their own copy). Prior drift caused
/// silent re-index of every Claude file on every Windows tick because
/// `session_index` stored 0 and `shared_memory` stored `file_index()`
/// — the triple never matched and `claude_exchanges::tests::
/// second_backfill_skips_unchanged_files` failed on Windows.
pub fn file_identity(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.ino()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.creation_time()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_copy_dir_recursive_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("b.txt"), "world").unwrap();
        fs::write(src.join("c.bin"), &[0u8, 1, 2]).unwrap();

        copy_dir_recursive(&src, &dst).unwrap();

        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(dst.join("b.txt")).unwrap(), "world");
        assert_eq!(fs::read(dst.join("c.bin")).unwrap(), &[0, 1, 2]);
    }

    #[test]
    fn test_copy_dir_recursive_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("sub1/sub2")).unwrap();
        fs::write(src.join("top.txt"), "top").unwrap();
        fs::write(src.join("sub1/mid.txt"), "mid").unwrap();
        fs::write(src.join("sub1/sub2/deep.txt"), "deep").unwrap();

        copy_dir_recursive(&src, &dst).unwrap();

        assert_eq!(fs::read_to_string(dst.join("top.txt")).unwrap(), "top");
        assert_eq!(fs::read_to_string(dst.join("sub1/mid.txt")).unwrap(), "mid");
        assert_eq!(
            fs::read_to_string(dst.join("sub1/sub2/deep.txt")).unwrap(),
            "deep"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_copy_dir_recursive_skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("real.txt"), "data").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", src.join("link")).unwrap();

        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("real.txt").exists());
        assert!(!dst.join("link").exists());
    }

    #[test]
    fn test_copy_dir_recursive_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir(&src).unwrap();

        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.exists());
        assert!(fs::read_dir(&dst).unwrap().next().is_none());
    }

    #[test]
    fn test_copy_dir_recursive_src_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let result = copy_dir_recursive(&tmp.path().join("nonexistent"), &tmp.path().join("dst"));
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_copy_dir_recursive_does_not_follow_directory_symlinks() {
        // Regression guard for the audit finding H8: metadata() followed
        // the link, so a symlinked directory looked like a directory and
        // recursion could escape the source tree.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let escape = tmp.path().join("escape");
        fs::create_dir(&src).unwrap();
        fs::create_dir(&escape).unwrap();
        fs::write(escape.join("outside.txt"), "OOPS").unwrap();
        std::os::unix::fs::symlink(&escape, src.join("link-dir")).unwrap();

        let dst = tmp.path().join("dst");
        copy_dir_recursive(&src, &dst).unwrap();

        // Nothing under dst/link-dir should exist — the symlink must be skipped.
        assert!(!dst.join("link-dir").exists());
        assert!(!dst.join("link-dir/outside.txt").exists());
    }

    #[test]
    fn test_atomic_write_creates_file_with_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nested/sub/file.json");
        atomic_write(&target, b"{\"k\":1}").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"{\"k\":1}");
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("file.txt");
        fs::write(&target, b"old").unwrap();
        atomic_write(&target, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn test_atomic_write_concurrent_same_target_no_collision() {
        // grill F9: two threads writing the SAME target concurrently
        // must not interleave into one corrupt file. With a pid-only
        // temp suffix both threads computed `.{name}.tmp.{pid}` and
        // the second `write_all` could land mid-stream of the first.
        // The atomic counter makes every temp path distinct; the
        // final rename leaves exactly one of the two payloads intact
        // and the file is never byte-interleaved.
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("contended.json");
        // Two distinct, valid, equal-length JSON payloads.
        let payload_a = br#"{"writer":"aaaaaaaaaaaaaaaaaaaaaaa"}"#;
        let payload_b = br#"{"writer":"bbbbbbbbbbbbbbbbbbbbbbb"}"#;

        for _ in 0..50 {
            let t = target.clone();
            let a = *payload_a;
            let h_a = std::thread::spawn(move || atomic_write(&t, &a));
            let t = target.clone();
            let b = *payload_b;
            let h_b = std::thread::spawn(move || atomic_write(&t, &b));
            h_a.join().unwrap().unwrap();
            h_b.join().unwrap().unwrap();

            // The file must be exactly one of the two payloads —
            // never a byte-interleaved mix.
            let got = fs::read(&target).unwrap();
            assert!(
                got == payload_a || got == payload_b,
                "atomic_write produced a corrupt interleaved file: {got:?}"
            );
            // No temp files leaked into the parent dir.
            let leftovers: Vec<_> = fs::read_dir(tmp.path())
                .unwrap()
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|n| n.contains(".tmp."))
                        .unwrap_or(false)
                })
                .collect();
            assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_sets_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("secret");
        atomic_write(&target, b"shh").unwrap();
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
