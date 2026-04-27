//! Materialize a per-automation directory on disk: helper shim,
//! per-run dirs, and (on unix) executable permission bits.
//!
//! Called from the scheduler register path and from the Run-Now
//! path. Idempotent.

use std::path::PathBuf;

use crate::automations::error::AutomationError;
use crate::automations::env::default_path_segments;
use crate::automations::shim::{render_unix, render_windows, ShimInputs};
use crate::automations::store::automation_dir;
use crate::automations::types::{Automation, AutomationBinary};
use crate::fs_utils;
use crate::paths::claudepot_data_dir;

/// Produce + write the helper shim for an automation. Returns the
/// path the scheduler should reference.
pub fn install_shim(
    automation: &Automation,
    binary_abs_path: &str,
    claudepot_cli_abs_path: &str,
) -> Result<PathBuf, AutomationError> {
    let auto_dir = automation_dir(&automation.id);
    let runs_dir = auto_dir.join("runs");
    std::fs::create_dir_all(&runs_dir)?;

    let bin_dir = claudepot_data_dir().join("bin");
    let path_segments = default_path_segments(&bin_dir.display().to_string());

    let inputs = ShimInputs {
        binary_abs_path,
        claudepot_cli_abs_path,
        automation_dir: &auto_dir.display().to_string(),
        path_segments: &path_segments,
        extra_env: &automation.extra_env,
    };

    let (shim_path, contents) = if cfg!(target_os = "windows") {
        (auto_dir.join("run.cmd"), render_windows(automation, &inputs))
    } else {
        (auto_dir.join("run.sh"), render_unix(automation, &inputs))
    };

    fs_utils::atomic_write(&shim_path, contents.as_bytes())?;

    // Mark executable on unix (atomic_write sets 0600; we need 0700
    // so the scheduler can /bin/sh the shim).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&shim_path)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&shim_path, perms)?;
    }

    Ok(shim_path)
}

/// Resolve the absolute path of the binary the automation should
/// invoke. For first-party, walks `PATH` for `claude` (or
/// `claude.exe` on Windows). For routes, returns
/// `<claudepot_data_dir>/bin/<wrapper-name>`.
pub fn resolve_binary(
    automation: &Automation,
    route_lookup: &dyn Fn(&uuid::Uuid) -> Option<String>,
) -> Result<String, AutomationError> {
    match &automation.binary {
        AutomationBinary::FirstParty => which_claude()
            .ok_or_else(|| AutomationError::InvalidPath(
                "claude".into(),
                "first-party `claude` binary not found on PATH",
            )),
        AutomationBinary::Route { route_id } => {
            let wrapper_name = route_lookup(route_id).ok_or_else(|| {
                AutomationError::NotFound(format!("route {route_id}"))
            })?;
            let bin = claudepot_data_dir()
                .join("bin")
                .join(&wrapper_name);
            if !bin.exists() {
                return Err(AutomationError::InvalidPath(
                    bin.display().to_string(),
                    "route wrapper missing on disk",
                ));
            }
            Ok(bin.display().to_string())
        }
    }
}

fn which_claude() -> Option<String> {
    let exe = if cfg!(target_os = "windows") {
        "claude.exe"
    } else {
        "claude"
    };
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }
    None
}

/// Resolve the path to the running `claudepot` CLI binary. The
/// helper shim calls back into this for `_record-run`.
pub fn current_claudepot_cli() -> Result<String, AutomationError> {
    let exe = std::env::current_exe()
        .map_err(|e| AutomationError::Io(std::io::Error::other(format!(
            "current_exe failed: {e}"
        ))))?;
    Ok(exe.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations::types::*;
    use chrono::Utc;
    use parking_lot::Mutex;
    use tempfile::tempdir;
    use uuid::Uuid;

    /// Serialize tests that mutate `CLAUDEPOT_DATA_DIR` — Cargo
    /// runs tests in parallel within one binary by default.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn auto() -> Automation {
        let now = Utc::now();
        Automation {
            id: Uuid::new_v4(),
            name: "test".into(),
            display_name: None,
            description: None,
            enabled: true,
            binary: AutomationBinary::FirstParty,
            model: Some("sonnet".into()),
            cwd: "/tmp".into(),
            prompt: "hi".into(),
            system_prompt: None,
            append_system_prompt: None,
            permission_mode: PermissionMode::DontAsk,
            allowed_tools: vec!["Read".into()],
            add_dir: vec![],
            max_budget_usd: Some(0.5),
            fallback_model: None,
            output_format: OutputFormat::Json,
            json_schema: None,
            bare: false,
            extra_env: Default::default(),
            trigger: Trigger::Cron { cron: "0 9 * * *".into(), timezone: None },
            platform_options: PlatformOptions::default(),
            log_retention_runs: 50,
            created_at: now,
            updated_at: now,
            claudepot_managed: true,
        }
    }

    #[test]
    fn install_shim_writes_executable_file() {
        let _guard = ENV_LOCK.lock();
        let dir = tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());
        let a = auto();
        let path = install_shim(&a, "/usr/local/bin/claude", "/path/to/claudepot").unwrap();
        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        if cfg!(target_os = "windows") {
            assert!(path.extension().and_then(|e| e.to_str()) == Some("cmd"));
            assert!(contents.contains("@echo off"));
        } else {
            assert!(path.extension().and_then(|e| e.to_str()) == Some("sh"));
            assert!(contents.starts_with("#!/bin/sh"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o700);
            }
        }
        // Runs dir was created.
        let runs = path.parent().unwrap().join("runs");
        assert!(runs.exists());
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn resolve_binary_route_missing_returns_err() {
        let _guard = ENV_LOCK.lock();
        let mut a = auto();
        let route_id = Uuid::new_v4();
        a.binary = AutomationBinary::Route { route_id };
        let lookup = |_id: &uuid::Uuid| Some("claude-mywrapper".to_string());
        let dir = tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());
        let res = resolve_binary(&a, &lookup);
        assert!(matches!(res, Err(AutomationError::InvalidPath(..))));
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn resolve_binary_route_unknown_id_returns_not_found() {
        let mut a = auto();
        a.binary = AutomationBinary::Route { route_id: Uuid::new_v4() };
        let lookup = |_id: &uuid::Uuid| None;
        let res = resolve_binary(&a, &lookup);
        assert!(matches!(res, Err(AutomationError::NotFound(_))));
    }
}
