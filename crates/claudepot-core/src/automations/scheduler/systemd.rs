//! Linux scheduler adapter — systemd-user timers.
//!
//! Each automation maps to a pair of unit files in
//! `~/.config/systemd/user/`:
//!
//! - `claudepot-automation-<id>.service` — runs the helper shim once.
//! - `claudepot-automation-<id>.timer` — fires the service on cron.
//!
//! Operations: `daemon-reload` after writing units, then
//! `enable --now <name>.timer` to register, `disable --now` to
//! unregister, `start <name>.service` to kickstart out of band.
//!
//! Capabilities: catch_up_if_missed via `Persistent=true`,
//! wake_to_run via `WakeSystem=true`, run_when_logged_out
//! requires `loginctl enable-linger $USER` (caller decides
//! whether to prompt).

use std::path::PathBuf;
use std::process::Command;

use chrono::{DateTime, Utc};

use crate::automations::cron::{self, LaunchSlot};
use crate::automations::error::AutomationError;
use crate::automations::store::automation_dir;
use crate::automations::types::{Automation, AutomationId, Trigger};

use super::{cron_next_runs, RegisteredEntry, Scheduler, SchedulerCapabilities};

const UNIT_PREFIX: &str = "claudepot-automation-";

pub struct SystemdScheduler;

pub fn unit_base_for(id: &AutomationId) -> String {
    format!("{UNIT_PREFIX}{id}")
}

pub fn unit_dir() -> Result<PathBuf, AutomationError> {
    let home = dirs::home_dir().ok_or(AutomationError::NoHomeDir)?;
    Ok(home.join(".config").join("systemd").join("user"))
}

pub fn timer_path_for(id: &AutomationId) -> Result<PathBuf, AutomationError> {
    Ok(unit_dir()?.join(format!("{UNIT_PREFIX}{id}.timer")))
}

pub fn service_path_for(id: &AutomationId) -> Result<PathBuf, AutomationError> {
    Ok(unit_dir()?.join(format!("{UNIT_PREFIX}{id}.service")))
}

impl Scheduler for SystemdScheduler {
    fn register(&self, automation: &Automation) -> Result<(), AutomationError> {
        let timer_path = timer_path_for(&automation.id)?;
        let service_path = service_path_for(&automation.id)?;
        let (timer, service) = render_units(automation)?;

        // Idempotent unregister-first.
        let base = unit_base_for(&automation.id);
        let _ = run_systemctl(&["--user", "disable", "--now", &format!("{base}.timer")]);

        if let Some(parent) = timer_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::fs_utils::atomic_write(&service_path, service.as_bytes())?;
        crate::fs_utils::atomic_write(&timer_path, timer.as_bytes())?;
        run_systemctl(&["--user", "daemon-reload"]).map_err(io_err)?;
        run_systemctl(&["--user", "enable", "--now", &format!("{base}.timer")]).map_err(io_err)?;
        Ok(())
    }

    fn unregister(&self, id: &AutomationId) -> Result<(), AutomationError> {
        let base = unit_base_for(id);
        let _ = run_systemctl(&["--user", "disable", "--now", &format!("{base}.timer")]);
        if let Ok(p) = timer_path_for(id) {
            if p.exists() {
                std::fs::remove_file(&p)?;
            }
        }
        if let Ok(p) = service_path_for(id) {
            if p.exists() {
                std::fs::remove_file(&p)?;
            }
        }
        let _ = run_systemctl(&["--user", "daemon-reload"]);
        Ok(())
    }

    fn kickstart(&self, id: &AutomationId) -> Result<(), AutomationError> {
        let base = unit_base_for(id);
        run_systemctl(&["--user", "start", &format!("{base}.service")]).map_err(io_err)
    }

    fn list_managed(&self) -> Result<Vec<RegisteredEntry>, AutomationError> {
        let dir = match unit_dir() {
            Ok(d) => d,
            Err(_) => return Ok(Vec::new()),
        };
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !name.starts_with(UNIT_PREFIX) || !name.ends_with(".timer") {
                continue;
            }
            let identifier = name.trim_end_matches(".timer").to_string();
            let claudepot_managed = std::fs::read_to_string(&path)
                .map(|s| s.contains("# claudepot_managed: true"))
                .unwrap_or(false);
            out.push(RegisteredEntry {
                identifier,
                claudepot_managed,
            });
        }
        Ok(out)
    }

    fn next_runs(
        &self,
        trigger: &Trigger,
        from: DateTime<Utc>,
        n: usize,
    ) -> Result<Vec<DateTime<Utc>>, AutomationError> {
        match trigger {
            Trigger::Cron { cron, .. } => cron_next_runs(cron, from, n),
        }
    }

    fn capabilities(&self) -> SchedulerCapabilities {
        SchedulerCapabilities {
            wake_to_run: true,
            catch_up_if_missed: true,
            run_when_logged_out: true,
            native_label: "systemd-user",
            artifact_dir: unit_dir().ok().map(|p| p.display().to_string()),
        }
    }
}

fn io_err(s: String) -> AutomationError {
    AutomationError::Io(std::io::Error::other(s))
}

fn run_systemctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(format!(
        "systemctl {} failed (status {}): {}",
        args.join(" "),
        out.status,
        stderr.trim()
    ))
}

/// Check whether `loginctl show-user $USER --property=Linger` reports `Linger=yes`.
pub fn linger_status() -> Result<bool, AutomationError> {
    let user = std::env::var("USER").unwrap_or_else(|_| String::from("nobody"));
    let out = Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger"])
        .output()
        .map_err(|e| io_err(e.to_string()))?;
    if !out.status.success() {
        return Err(io_err(format!(
            "loginctl exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.trim().eq_ignore_ascii_case("Linger=yes"))
}

/// Render `(timer, service)` unit-file contents for an automation.
/// Pure: no OS calls. Outputs are byte-stable for goldens.
pub fn render_units(automation: &Automation) -> Result<(String, String), AutomationError> {
    let auto_dir = automation_dir(&automation.id);
    let shim_path = auto_dir.join("run.sh");
    let display_label = automation
        .display_name
        .clone()
        .unwrap_or_else(|| automation.name.clone());

    // --- service ---
    let mut svc = String::new();
    svc.push_str("# claudepot_managed: true\n");
    svc.push_str(&format!(
        "# claudepot_version: {}\n",
        env!("CARGO_PKG_VERSION")
    ));
    svc.push_str("[Unit]\n");
    svc.push_str(&format!(
        "Description=Claudepot automation: {}\n",
        unit_value_escape(&display_label)
    ));
    svc.push('\n');
    svc.push_str("[Service]\n");
    svc.push_str("Type=oneshot\n");
    svc.push_str(&format!(
        "WorkingDirectory={}\n",
        unit_value_escape(&automation.cwd)
    ));
    // Quote ExecStart's argument so paths with spaces (or any
    // shell-relevant char) don't get split by systemd's tokenizer.
    // systemd accepts `"…"` quoting around individual args.
    svc.push_str(&format!(
        "ExecStart=/bin/sh \"{}\"\n",
        unit_value_escape(&shim_path.display().to_string())
            // Reject embedded `"` inside quoted ExecStart arg to keep
            // tokenization unambiguous. The path validator already
            // accepts only printable ASCII, so this is defense in
            // depth.
            .replace('"', "")
    ));
    svc.push_str("Nice=5\n");

    // --- timer ---
    let slots = match &automation.trigger {
        Trigger::Cron { cron: expr, .. } => cron::expand(expr)?,
    };
    let mut timer = String::new();
    timer.push_str("# claudepot_managed: true\n");
    timer.push_str(&format!(
        "# claudepot_version: {}\n",
        env!("CARGO_PKG_VERSION")
    ));
    timer.push_str("[Unit]\n");
    timer.push_str(&format!(
        "Description=Claudepot automation timer: {}\n",
        unit_value_escape(&display_label)
    ));
    timer.push('\n');
    timer.push_str("[Timer]\n");
    for slot in &slots {
        timer.push_str(&format!("OnCalendar={}\n", on_calendar_for(slot)));
    }
    timer.push_str(&format!(
        "Persistent={}\n",
        if automation.platform_options.catch_up_if_missed {
            "true"
        } else {
            "false"
        }
    ));
    timer.push_str(&format!(
        "WakeSystem={}\n",
        if automation.platform_options.wake_to_run {
            "true"
        } else {
            "false"
        }
    ));
    timer.push_str(&format!("Unit={UNIT_PREFIX}{}.service\n", automation.id));
    timer.push('\n');
    timer.push_str("[Install]\n");
    timer.push_str("WantedBy=timers.target\n");

    Ok((timer, svc))
}

/// Render one `OnCalendar=` value from a launch slot. systemd's
/// `OnCalendar` syntax is `DOW YYYY-MM-DD HH:MM:SS`; a `*` means
/// "any". We always use minute precision.
fn on_calendar_for(slot: &LaunchSlot) -> String {
    let dow = match slot.day_of_week {
        Some(0) => "Sun",
        Some(1) => "Mon",
        Some(2) => "Tue",
        Some(3) => "Wed",
        Some(4) => "Thu",
        Some(5) => "Fri",
        Some(6) => "Sat",
        _ => "*-*",
    };
    let mon = slot
        .month
        .map(|m| format!("{m:02}"))
        .unwrap_or_else(|| "*".into());
    let day = slot
        .day_of_month
        .map(|d| format!("{d:02}"))
        .unwrap_or_else(|| "*".into());
    let hour = format!("{:02}", slot.hour);
    let minute = format!("{:02}", slot.minute);
    if slot.day_of_week.is_some() {
        format!("{dow} *-{mon}-{day} {hour}:{minute}:00")
    } else {
        format!("*-{mon}-{day} {hour}:{minute}:00")
    }
}

fn sanitize_one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Escape a string for embedding in a systemd unit-file value:
/// strip newlines (one-line invariant) and double `%` so it isn't
/// interpreted as a systemd specifier (e.g. `%h` for home dir).
/// systemd's specifier syntax is documented in
/// `systemd.unit(5)` under "SPECIFIERS"; we don't use any specifiers
/// in our generated units, so escaping all `%` is the safe default.
fn unit_value_escape(s: &str) -> String {
    let single = sanitize_one_line(s);
    single.replace('%', "%%")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations::types::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn auto(name: &str, cron: &str) -> Automation {
        let now = Utc::now();
        Automation {
            id: Uuid::nil(),
            name: name.into(),
            display_name: Some("Pretty Name".into()),
            description: None,
            enabled: true,
            binary: AutomationBinary::FirstParty,
            model: Some("sonnet".into()),
            cwd: "/home/me/repo".into(),
            prompt: "say hi".into(),
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
            trigger: Trigger::Cron {
                cron: cron.into(),
                timezone: None,
            },
            platform_options: PlatformOptions::default(),
            log_retention_runs: 50,
            created_at: now,
            updated_at: now,
            claudepot_managed: true,
        }
    }

    #[test]
    fn render_units_daily_at_9() {
        let a = auto("morning-pr", "0 9 * * *");
        let (timer, service) = render_units(&a).unwrap();
        assert!(service.contains("[Service]"));
        assert!(service.contains("Type=oneshot"));
        assert!(service.contains("WorkingDirectory=/home/me/repo"));
        assert!(service.contains("ExecStart=/bin/sh "));
        assert!(service.contains("run.sh"));
        assert!(service.contains("# claudepot_managed: true"));

        assert!(timer.contains("[Timer]"));
        assert!(timer.contains("OnCalendar=*-*-* 09:00:00"));
        assert!(timer.contains("Persistent=true")); // default catch_up_if_missed=true
        assert!(timer.contains("WakeSystem=false"));
        assert!(timer.contains("WantedBy=timers.target"));
        assert!(timer.contains(&format!("Unit={UNIT_PREFIX}{}.service", a.id)));
    }

    #[test]
    fn render_units_weekday_emits_5_oncalendar_lines() {
        let a = auto("biz", "0 9 * * 1-5");
        let (timer, _) = render_units(&a).unwrap();
        let oc_count = timer.matches("OnCalendar=").count();
        assert_eq!(oc_count, 5);
        assert!(timer.contains("OnCalendar=Mon "));
        assert!(timer.contains("OnCalendar=Fri "));
    }

    #[test]
    fn render_units_honors_platform_options() {
        let mut a = auto("wake", "0 9 * * *");
        a.platform_options.wake_to_run = true;
        a.platform_options.catch_up_if_missed = false;
        let (timer, _) = render_units(&a).unwrap();
        assert!(timer.contains("Persistent=false"));
        assert!(timer.contains("WakeSystem=true"));
    }

    #[test]
    fn render_units_strips_newlines_from_display_name() {
        let mut a = auto("nl", "0 9 * * *");
        a.display_name = Some("evil\ninjection".into());
        let (_, service) = render_units(&a).unwrap();
        assert!(!service.contains("\nevil\ninjection"));
        assert!(service.contains("evil injection"));
    }

    #[test]
    fn capabilities_reports_systemd_truths() {
        let s = SystemdScheduler;
        let caps = s.capabilities();
        assert_eq!(caps.native_label, "systemd-user");
        assert!(caps.wake_to_run);
        assert!(caps.catch_up_if_missed);
        assert!(caps.run_when_logged_out);
    }

    #[test]
    fn unit_base_format() {
        let id = Uuid::nil();
        assert_eq!(
            unit_base_for(&id),
            "claudepot-automation-00000000-0000-0000-0000-000000000000"
        );
    }
}
