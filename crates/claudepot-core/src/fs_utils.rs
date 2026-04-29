//! Shared filesystem utilities.

use std::path::{Path, PathBuf};

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
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
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

    // Sibling temp so rename stays atomic (same filesystem).
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("claudepot"),
        std::process::id()
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

/// Find the `claude` CLI binary. Checks common install locations
/// then falls back to PATH via `which`/`where`.
pub fn find_claude_binary() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![];

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/claude"));
    }
    candidates.push(PathBuf::from("/usr/local/bin/claude"));
    candidates.push(PathBuf::from("/usr/bin/claude"));

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            candidates.push(PathBuf::from(appdata).join("npm").join("claude.cmd"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local").join("bin").join("claude.exe"));
        }
    }

    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }

    // Fallback: try PATH
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
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
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
