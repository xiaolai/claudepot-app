//! Shared filesystem utilities.

use std::path::{Path, PathBuf};

/// Recursively copy a directory, skipping symlinks.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        let ft = entry.metadata()?.file_type();
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
    let which_cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
    let claude_name = if cfg!(target_os = "windows") { "claude.cmd" } else { "claude" };
    if let Ok(output) = std::process::Command::new(which_cmd).arg(claude_name).output() {
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
