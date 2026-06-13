//! `claudepot logs` — locate or tail the diagnostic log.
//!
//! The Tauri GUI writes every `tracing` event and any panic to a
//! rolling daily file at `claudepot_core::paths::log_dir()`. The
//! rotation naming and the `claudepot.log` symlink contract are
//! owned by `claudepot_core::diagnostic_logging` (the same module
//! that builds the appender); this handler only does presentation —
//! printing the directory and spawning `tail` / the OS file manager.

use crate::AppContext;
use std::process::Stdio;

pub async fn run(ctx: &AppContext, open: bool, tail: bool) -> anyhow::Result<()> {
    // Resolve + create the directory via core so `--open` lands on
    // something real even before the GUI has ever booted.
    let dir = claudepot_core::diagnostic_logging::ensure_log_dir()?;

    if ctx.json {
        let active = claudepot_core::diagnostic_logging::resolve_active_log(&dir);
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "log_dir": dir.display().to_string(),
                "active_log": active.as_ref().map(|p| p.display().to_string()),
            }))?
        );
    } else {
        println!("{}", dir.display());
    }

    if tail {
        let active = claudepot_core::diagnostic_logging::resolve_active_log(&dir);
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
