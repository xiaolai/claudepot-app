//! `claudepot logs` — locate or tail the diagnostic log.
//!
//! The Tauri GUI writes every `tracing` event and any panic to a
//! rolling daily file at `claudepot_core::paths::log_dir()`. This
//! subcommand resolves and prints the path, opens the directory in
//! the OS file manager (`--open`), or follows the current file
//! (`--tail` / `-f`) when the user wants live diagnostics during a
//! reproducing run.

use std::process::Stdio;

const ACTIVE_LOG_FILENAME: &str = "claudepot.log";

pub async fn run(open: bool, tail: bool) -> anyhow::Result<()> {
    let dir = claudepot_core::paths::log_dir();
    if !dir.exists() {
        // Mirror the GUI's first-boot behavior: create the directory
        // so `--open` lands on something real even before any GUI run
        // has fired. Without this, opening on first install would
        // fail with "no such directory".
        std::fs::create_dir_all(&dir)?;
    }
    println!("{}", dir.display());

    if tail {
        let active = dir.join(ACTIVE_LOG_FILENAME);
        if !active.exists() {
            std::fs::write(&active, b"")?;
        }
        tail_file(&active).await?;
        return Ok(());
    }
    if open {
        open_dir(&dir).await?;
    }
    Ok(())
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
