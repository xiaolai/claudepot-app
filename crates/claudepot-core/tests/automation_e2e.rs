//! End-to-end automation smoke tests.
//!
//! `#[ignore]` because they touch the real OS scheduler:
//!   - macOS: writes a plist to ~/Library/LaunchAgents/, calls
//!     launchctl bootstrap/bootout/kickstart against gui/$UID.
//!   - linux: writes systemd-user units, calls systemctl --user.
//!   - windows: registers a Task Scheduler task, calls schtasks.
//!
//! Each test uses a unique `CLAUDEPOT_DATA_DIR` and a dedicated
//! automation id so concurrent runs don't collide. The test
//! cleans up its registration on success and on panic via a Drop
//! guard.
//!
//! Run with:
//!   cargo test -p claudepot-core --test automation_e2e -- --ignored

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use chrono::Utc;
use claudepot_core::automations::{
    active_scheduler, install_shim, store::automation_runs_dir, Automation,
    AutomationBinary, AutomationId, OutputFormat, PermissionMode, PlatformOptions,
    Trigger,
};
use uuid::Uuid;

struct CleanupGuard {
    id: AutomationId,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        // Best-effort: scheduler unregister + remove on-disk dirs.
        let _ = active_scheduler().unregister(&self.id);
    }
}

fn make_fake_claude(stdout: &str) -> PathBuf {
    // A "claude" shim that ignores stdin and prints fixture JSON.
    let dir = std::env::temp_dir().join(format!("claudepot-fake-claude-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join(if cfg!(target_os = "windows") {
        "claude.cmd"
    } else {
        "claude"
    });
    let contents = if cfg!(target_os = "windows") {
        format!("@echo off\r\necho.{}\r\nexit /b 0\r\n", stdout.replace('\n', " "))
    } else {
        format!(
            "#!/bin/sh\ncat > /dev/null\nprintf '%s' '{}'\nexit 0\n",
            stdout.replace('\'', "'\\''")
        )
    };
    std::fs::write(&bin, contents).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&bin).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&bin, p).unwrap();
    }
    bin
}

fn current_claudepot_cli() -> PathBuf {
    // Use the test binary itself as the "cli" — it never gets called
    // since the fake claude returns success without error subtype,
    // but the shim still references it. The shim invokes with
    // --automation-id ... which will fail unless this is actually
    // the claudepot binary. So we point at the built binary if
    // present, else accept that _record-run will fail (the run
    // still produces stdout.log and we read result from there
    // separately).
    let target = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("/usr/local/bin"))
        .join("../../debug/claudepot");
    target.canonicalize().unwrap_or(target)
}

fn make_automation(name: &str) -> Automation {
    let now = Utc::now();
    Automation {
        id: Uuid::new_v4(),
        name: name.to_string(),
        display_name: None,
        description: None,
        enabled: true,
        binary: AutomationBinary::FirstParty,
        model: Some("haiku".to_string()),
        cwd: std::env::temp_dir().display().to_string(),
        prompt: "test".to_string(),
        system_prompt: None,
        append_system_prompt: None,
        permission_mode: PermissionMode::DontAsk,
        allowed_tools: vec!["Read".to_string()],
        add_dir: vec![],
        max_budget_usd: Some(0.05),
        fallback_model: None,
        output_format: OutputFormat::Json,
        json_schema: None,
        bare: false,
        extra_env: Default::default(),
        // Daily at midnight — far enough future not to fire during the
        // test, but kickstart bypasses the schedule.
        trigger: Trigger::Cron {
            cron: "0 0 * * *".to_string(),
            timezone: None,
        },
        platform_options: PlatformOptions::default(),
        log_retention_runs: 10,
        created_at: now,
        updated_at: now,
        claudepot_managed: true,
    }
}

#[test]
#[ignore]
fn end_to_end_register_kickstart_unregister() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path());

    // Fake claude that prints a one-line success result.
    let stdout = r#"[{"type":"system","subtype":"init"},{"type":"result","subtype":"success","is_error":false,"num_turns":1,"total_cost_usd":0.0001,"stop_reason":"end_turn","session_id":"test-sess","errors":[]}]"#;
    let fake_claude = make_fake_claude(stdout);
    let cli = current_claudepot_cli();

    let automation = make_automation(&format!(
        "e2e-test-{}",
        Utc::now().timestamp()
    ));
    let _guard = CleanupGuard { id: automation.id };

    install_shim(
        &automation,
        fake_claude.to_str().unwrap(),
        cli.to_str().unwrap(),
    )
    .expect("install_shim");

    let scheduler = active_scheduler();
    eprintln!("scheduler: {}", scheduler.capabilities().native_label);
    if scheduler.capabilities().native_label == "none" {
        eprintln!("no scheduler on this host — skipping E2E");
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
        return;
    }

    scheduler.register(&automation).expect("register");
    scheduler.kickstart(&automation.id).expect("kickstart");

    // Wait up to 30s for the run dir to appear with a result.json.
    let runs_dir = automation_runs_dir(&automation.id);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut found_result = false;
    while Instant::now() < deadline {
        if runs_dir.exists() {
            for entry in std::fs::read_dir(&runs_dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let result_json = path.join("result.json");
                    if result_json.exists() {
                        let raw = std::fs::read(&result_json).unwrap();
                        let parsed: serde_json::Value =
                            serde_json::from_slice(&raw).unwrap();
                        eprintln!("result.json: {parsed:?}");
                        assert_eq!(parsed["exit_code"], 0);
                        found_result = true;
                        break;
                    }
                }
            }
        }
        if found_result {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    if !found_result {
        // It's still useful to know what happened — surface the stdout/stderr.
        if runs_dir.exists() {
            for entry in std::fs::read_dir(&runs_dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    for log in ["stdout.log", "stderr.log"] {
                        let p = path.join(log);
                        if p.exists() {
                            eprintln!(
                                "--- {log} ---\n{}",
                                std::fs::read_to_string(&p).unwrap_or_default()
                            );
                        }
                    }
                }
            }
        }
        let _ = scheduler.unregister(&automation.id);
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
        panic!("no result.json materialized within 30s");
    }

    let _ = scheduler.unregister(&automation.id);
    std::env::remove_var("CLAUDEPOT_DATA_DIR");
}

#[test]
#[ignore]
fn dry_run_shim_renders_and_is_executable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path());

    let automation = make_automation("e2e-shim-render");
    let _guard = CleanupGuard { id: automation.id };
    let path = install_shim(&automation, "/bin/echo", "/bin/false").expect("install");
    assert!(path.exists(), "shim should exist at {path:?}");
    let contents = std::fs::read_to_string(&path).unwrap();
    if cfg!(target_os = "windows") {
        assert!(contents.contains("@echo off"));
    } else {
        assert!(contents.starts_with("#!/bin/sh"));
    }
    // Just exercising — leave cleanup to the guard.
    std::env::remove_var("CLAUDEPOT_DATA_DIR");
}

// Silence `Command` import unused on platforms that don't use it.
#[allow(dead_code)]
fn _silence_unused() {
    let _ = Command::new("true");
}
