//! `claudepot logs` — locate or tail the diagnostic log.
//!
//! The Tauri GUI writes every `tracing` event and any panic to a
//! rolling daily file at `claudepot_core::paths::log_dir()`. The
//! daily rotation produces files named `claudepot.log.YYYY-MM-DD`
//! (the date suffix is always present — there is no "active without
//! suffix" form), and the GUI's `RollingFileAppender::builder` is
//! configured to maintain a `claudepot.log` symlink that always
//! points to today's dated file. `--tail` follows that symlink so
//! it stays correct across midnight rollovers.

use std::process::Stdio;

const ACTIVE_LOG_SYMLINK: &str = "claudepot.log";

pub async fn run(open: bool, tail: bool) -> anyhow::Result<()> {
    let dir = claudepot_core::paths::log_dir();
    if !dir.exists() {
        // Create the directory eagerly so `--open` lands on
        // something real even before the GUI has ever booted. Do
        // NOT pre-create the log file or symlink — that would
        // shadow the GUI's later symlink creation if the CLI ran
        // first.
        std::fs::create_dir_all(&dir)?;
    }
    println!("{}", dir.display());

    if tail {
        let active = resolve_active_log(&dir);
        let Some(active) = active else {
            eprintln!("no log files yet — launch the Claudepot GUI to generate one");
            return Ok(());
        };
        tail_file(&active).await?;
        return Ok(());
    }
    if open {
        open_dir(&dir).await?;
    }
    Ok(())
}

/// Find what to tail. Preferred path: the `claudepot.log` symlink
/// the GUI maintains. Fallback: if the symlink hasn't been created
/// yet (older GUI build, manual deletion), pick the
/// lexically-latest `claudepot.log.YYYY-MM-DD` file in the
/// directory. `None` means nothing exists to tail.
fn resolve_active_log(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let symlink = dir.join(ACTIVE_LOG_SYMLINK);
    // `exists()` follows symlinks, so this returns true when the
    // symlink points to a real file. We accept either a real file
    // or a working symlink.
    if symlink.exists() {
        return Some(symlink);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut candidates: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            // The rolled files are `claudepot.log.YYYY-MM-DD`.
            // Lexical sort on this prefix is also chronological,
            // so the lexically-latest entry is today's active file.
            if name.starts_with("claudepot.log.") {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    candidates.sort();
    candidates.pop()
}

async fn tail_file(path: &std::path::Path) -> anyhow::Result<()> {
    use tokio::process::Command;
    #[cfg(unix)]
    {
        let status = Command::new("tail")
            .arg("-f")
            .arg(path)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("tail -f exited with {status}");
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        // PowerShell ships everywhere; `Get-Content -Wait` is the
        // canonical tail replacement.
        let status = Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg(format!("Get-Content -Path \"{}\" -Wait", path.display()))
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("powershell Get-Content -Wait exited with {status}");
        }
        Ok(())
    }
}

async fn open_dir(path: &std::path::Path) -> anyhow::Result<()> {
    use tokio::process::Command;
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("/usr/bin/open").arg(path).status().await?;
        if !status.success() {
            anyhow::bail!("open exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let status = Command::new("xdg-open").arg(path).status().await?;
        if !status.success() {
            anyhow::bail!("xdg-open exited with {status}");
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        // `explorer <path>` opens the directory in Explorer. Unlike
        // `explorer /select,<path>` this exits 0 reliably.
        let status = Command::new("explorer").arg(path).status().await?;
        // explorer.exe is famous for returning 1 even on success;
        // accept anything ≤ 1 the same way `reveal_in_finder` does.
        let code = status.code().unwrap_or(0);
        if code > 1 {
            anyhow::bail!("explorer exited {code}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_active_log;
    use std::fs::File;

    #[test]
    fn empty_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(resolve_active_log(tmp.path()).is_none());
    }

    #[test]
    fn picks_lexically_latest_dated_file_when_no_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        File::create(tmp.path().join("claudepot.log.2026-06-03")).unwrap();
        File::create(tmp.path().join("claudepot.log.2026-06-05")).unwrap();
        File::create(tmp.path().join("claudepot.log.2026-06-04")).unwrap();
        File::create(tmp.path().join("unrelated.txt")).unwrap();
        let active = resolve_active_log(tmp.path()).expect("should find a dated file");
        assert_eq!(active.file_name().unwrap(), "claudepot.log.2026-06-05");
    }

    #[test]
    #[cfg(unix)]
    fn prefers_symlink_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        File::create(tmp.path().join("claudepot.log.2026-06-04")).unwrap();
        let dated = tmp.path().join("claudepot.log.2026-06-05");
        File::create(&dated).unwrap();
        let symlink = tmp.path().join("claudepot.log");
        std::os::unix::fs::symlink(&dated, &symlink).unwrap();
        let active = resolve_active_log(tmp.path()).expect("symlink should be picked");
        assert_eq!(active.file_name().unwrap(), "claudepot.log");
    }
}
