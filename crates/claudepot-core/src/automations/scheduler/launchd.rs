//! macOS scheduler adapter — launchd LaunchAgents.
//!
//! Each automation maps to one `~/Library/LaunchAgents/<label>.plist`
//! registered via `launchctl bootstrap gui/$UID <plist>`. Operations
//! are idempotent: replacing a registration unloads + reloads;
//! "not found" on unregister is a success.
//!
//! The plist's `ProgramArguments` invokes `/bin/sh <run.sh>` (the
//! per-automation helper shim from `super::super::shim`), not
//! `claude` directly. The shim handles per-run dir allocation,
//! prompt-via-stdin, and the post-exit `claudepot automation
//! _record-run` callback.
//!
//! launchd LaunchAgents intentionally do not support
//! `WakeToRun`-style waking, missed-run catchup, or running while
//! the user is logged out — `capabilities()` reports `false` for
//! all three.

use std::path::PathBuf;
use std::process::Command;

use chrono::{DateTime, Utc};

use crate::automations::cron::{self, LaunchSlot};
use crate::automations::error::AutomationError;
use crate::automations::store::automation_dir;
use crate::automations::types::{Automation, AutomationId, Trigger};

use super::xml::{indent, xml_escape};
use super::{cron_next_runs, RegisteredEntry, Scheduler, SchedulerCapabilities};

const LABEL_PREFIX: &str = "io.claudepot.automation.";

pub struct LaunchdScheduler;

/// Top-level reverse-DNS label for an automation.
pub fn label_for(id: &AutomationId) -> String {
    format!("{LABEL_PREFIX}{id}")
}

/// Path to the plist this adapter writes for the given automation.
pub fn plist_path_for(id: &AutomationId) -> Result<PathBuf, AutomationError> {
    let home = dirs::home_dir().ok_or(AutomationError::NoHomeDir)?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL_PREFIX}{id}.plist")))
}

impl Scheduler for LaunchdScheduler {
    fn register(&self, automation: &Automation) -> Result<(), AutomationError> {
        let label = label_for(&automation.id);
        let plist_path = plist_path_for(&automation.id)?;
        let xml = render_plist(automation)?;

        // Best-effort unload first so a stale registration is
        // replaced cleanly.
        let _ = run_launchctl(&["bootout", &gui_target(&label)]);

        // Pre-create the stable launchd stdout/stderr log files so
        // launchd's open(2) for `StandardOutPath` /
        // `StandardErrorPath` finds existing files at bootstrap time.
        // The plist points at `<auto_dir>/launchd-{stdout,stderr}.log`;
        // the helper shim writes its own per-run logs into
        // `<auto_dir>/runs/<run-id>/`, so these stable files only
        // capture out-of-band launchd noise (registration errors,
        // pre-shim failures) — useful for forensics.
        let auto_dir = automation_dir(&automation.id);
        std::fs::create_dir_all(&auto_dir)?;
        for f in ["launchd-stdout.log", "launchd-stderr.log"] {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(auto_dir.join(f));
        }

        if let Some(parent) = plist_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::fs_utils::atomic_write(&plist_path, xml.as_bytes())?;
        // launchctl bootstrap requires the plist file already on disk.
        let bootstrap_target = gui_target_root();
        run_launchctl(&[
            "bootstrap",
            &bootstrap_target,
            plist_path.to_str().unwrap_or(""),
        ])
        .map_err(|e| {
            AutomationError::Io(std::io::Error::other(format!(
                "launchctl bootstrap failed for {label}: {e}"
            )))
        })?;
        Ok(())
    }

    fn unregister(&self, id: &AutomationId) -> Result<(), AutomationError> {
        let label = label_for(id);
        // bootout is idempotent — non-zero exit on "not loaded" is fine.
        let _ = run_launchctl(&["bootout", &gui_target(&label)]);
        if let Ok(path) = plist_path_for(id) {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
        Ok(())
    }

    fn kickstart(&self, id: &AutomationId) -> Result<(), AutomationError> {
        let label = label_for(id);
        run_launchctl(&["kickstart", "-k", &gui_target(&label)]).map_err(|e| {
            AutomationError::Io(std::io::Error::other(format!(
                "launchctl kickstart failed for {label}: {e}"
            )))
        })
    }

    fn list_managed(&self) -> Result<Vec<RegisteredEntry>, AutomationError> {
        let home = dirs::home_dir().ok_or(AutomationError::NoHomeDir)?;
        let dir = home.join("Library").join("LaunchAgents");
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
            if !name.starts_with(LABEL_PREFIX) || !name.ends_with(".plist") {
                continue;
            }
            let identifier = name.trim_end_matches(".plist").to_string();
            let claudepot_managed = std::fs::read_to_string(&path)
                .map(|s| s.contains("<key>claudepot_managed</key>"))
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
            wake_to_run: false,
            catch_up_if_missed: false,
            run_when_logged_out: false,
            native_label: "launchd",
            artifact_dir: dirs::home_dir()
                .map(|h| h.join("Library").join("LaunchAgents").display().to_string()),
        }
    }
}

fn gui_target_root() -> String {
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}")
}

fn gui_target(label: &str) -> String {
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}/{label}")
}

fn run_launchctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(format!(
        "launchctl {} failed (status {}): {}",
        args.join(" "),
        out.status,
        stderr.trim()
    ))
}

/// Render a launchd plist for an automation. Pure: does not touch
/// the OS. Output is byte-stable for golden tests.
pub fn render_plist(automation: &Automation) -> Result<String, AutomationError> {
    let label = label_for(&automation.id);
    let auto_dir = automation_dir(&automation.id);
    let shim_path = auto_dir.join("run.sh");
    // launchd opens StandardOutPath/StandardErrorPath BEFORE
    // launching ProgramArguments, so the target path must already
    // exist as a writeable file (or be in a directory that does).
    // The previous design pointed at `runs/.latest/stdout.log` but
    // `.latest` was created by the shim AFTER launchd's open(2),
    // racing the redirect. We instead point at stable per-automation
    // log files that the shim appends to from inside its own
    // per-run dir; launchd captures any out-of-band stderr
    // (e.g. shim startup errors before mkdir) into these stable
    // files where they're useful for debugging registration issues.
    let stdout_target = auto_dir.join("launchd-stdout.log");
    let stderr_target = auto_dir.join("launchd-stderr.log");

    let slots = match &automation.trigger {
        Trigger::Cron { cron: expr, .. } => cron::expand(expr)?,
    };

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    );
    xml.push_str("<plist version=\"1.0\">\n");
    xml.push_str("<dict>\n");

    push_kv_string(&mut xml, 1, "Label", &label);

    indent(&mut xml, 1);
    xml.push_str("<key>ProgramArguments</key>\n");
    indent(&mut xml, 1);
    xml.push_str("<array>\n");
    push_string(&mut xml, 2, "/bin/sh");
    push_string(&mut xml, 2, &shim_path.display().to_string());
    indent(&mut xml, 1);
    xml.push_str("</array>\n");

    push_kv_string(&mut xml, 1, "WorkingDirectory", &automation.cwd);
    push_kv_string(
        &mut xml,
        1,
        "StandardOutPath",
        &stdout_target.display().to_string(),
    );
    push_kv_string(
        &mut xml,
        1,
        "StandardErrorPath",
        &stderr_target.display().to_string(),
    );

    push_kv_bool(&mut xml, 1, "RunAtLoad", false);
    push_kv_string(&mut xml, 1, "ProcessType", "Background");
    indent(&mut xml, 1);
    xml.push_str("<key>Nice</key>\n");
    indent(&mut xml, 1);
    xml.push_str("<integer>5</integer>\n");

    indent(&mut xml, 1);
    xml.push_str("<key>StartCalendarInterval</key>\n");
    if slots.len() == 1 {
        let slot = slots[0];
        indent(&mut xml, 1);
        xml.push_str("<dict>\n");
        emit_slot_dict(&mut xml, 2, &slot);
        indent(&mut xml, 1);
        xml.push_str("</dict>\n");
    } else {
        indent(&mut xml, 1);
        xml.push_str("<array>\n");
        for slot in &slots {
            indent(&mut xml, 2);
            xml.push_str("<dict>\n");
            emit_slot_dict(&mut xml, 3, slot);
            indent(&mut xml, 2);
            xml.push_str("</dict>\n");
        }
        indent(&mut xml, 1);
        xml.push_str("</array>\n");
    }

    push_kv_bool(&mut xml, 1, "claudepot_managed", true);
    push_kv_string(&mut xml, 1, "claudepot_version", env!("CARGO_PKG_VERSION"));

    xml.push_str("</dict>\n");
    xml.push_str("</plist>\n");
    Ok(xml)
}

fn emit_slot_dict(out: &mut String, level: usize, slot: &LaunchSlot) {
    push_kv_int(out, level, "Minute", slot.minute as i64);
    push_kv_int(out, level, "Hour", slot.hour as i64);
    if let Some(d) = slot.day_of_month {
        push_kv_int(out, level, "Day", d as i64);
    }
    if let Some(m) = slot.month {
        push_kv_int(out, level, "Month", m as i64);
    }
    if let Some(w) = slot.day_of_week {
        // launchd's Weekday uses 0=Sunday..6=Saturday — same as our internal form.
        push_kv_int(out, level, "Weekday", w as i64);
    }
}

fn push_kv_string(out: &mut String, level: usize, key: &str, value: &str) {
    indent(out, level);
    out.push_str(&format!("<key>{}</key>\n", xml_escape(key)));
    push_string(out, level, value);
}

fn push_kv_bool(out: &mut String, level: usize, key: &str, value: bool) {
    indent(out, level);
    out.push_str(&format!("<key>{}</key>\n", xml_escape(key)));
    indent(out, level);
    out.push_str(if value { "<true/>\n" } else { "<false/>\n" });
}

fn push_kv_int(out: &mut String, level: usize, key: &str, value: i64) {
    indent(out, level);
    out.push_str(&format!("<key>{}</key>\n", xml_escape(key)));
    indent(out, level);
    out.push_str(&format!("<integer>{value}</integer>\n"));
}

fn push_string(out: &mut String, level: usize, value: &str) {
    indent(out, level);
    out.push_str(&format!("<string>{}</string>\n", xml_escape(value)));
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
            display_name: None,
            description: None,
            enabled: true,
            binary: AutomationBinary::FirstParty,
            model: Some("sonnet".into()),
            cwd: "/Users/me/repo".into(),
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
    fn label_format_matches_reverse_dns() {
        let id = Uuid::nil();
        assert_eq!(
            label_for(&id),
            "io.claudepot.automation.00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn render_plist_minimal_daily_at_9() {
        let a = auto("morning-pr", "0 9 * * *");
        let xml = render_plist(&a).unwrap();
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist"));
        assert!(xml.contains("<key>Label</key>"));
        assert!(xml.contains(
            "<string>io.claudepot.automation.00000000-0000-0000-0000-000000000000</string>"
        ));
        assert!(xml.contains("<key>ProgramArguments</key>"));
        assert!(xml.contains("<string>/bin/sh</string>"));
        assert!(xml.contains("run.sh"));
        assert!(xml.contains("<key>WorkingDirectory</key>"));
        assert!(xml.contains("<string>/Users/me/repo</string>"));
        assert!(xml.contains("<key>StartCalendarInterval</key>"));
        assert!(xml.contains("<key>Hour</key>"));
        assert!(xml.contains("<integer>9</integer>"));
        assert!(xml.contains("<key>Minute</key>"));
        assert!(xml.contains("<integer>0</integer>"));
        assert!(xml.contains("<key>RunAtLoad</key>"));
        assert!(xml.contains("<false/>"));
        assert!(xml.contains("<key>claudepot_managed</key>"));
        assert!(xml.contains("<true/>"));
        // Single-slot form is `<dict>`, not an `<array><dict>...`.
        assert!(!xml.contains("<key>StartCalendarInterval</key>\n  <array>"));
    }

    #[test]
    fn render_plist_weekday_uses_array() {
        let a = auto("biz-hours", "0 9 * * 1-5");
        let xml = render_plist(&a).unwrap();
        assert!(xml.contains("<key>StartCalendarInterval</key>\n  <array>"));
        // 5 weekday slots
        let weekday_count = xml.matches("<key>Weekday</key>").count();
        assert_eq!(weekday_count, 5);
    }

    #[test]
    fn render_plist_escapes_xml_in_cwd() {
        let mut a = auto("escape", "0 9 * * *");
        a.cwd = "/tmp/<weird & path>".into();
        let xml = render_plist(&a).unwrap();
        assert!(xml.contains("/tmp/&lt;weird &amp; path&gt;"));
        assert!(!xml.contains("/tmp/<weird"));
    }

    #[test]
    fn render_plist_carries_managed_marker() {
        let a = auto("m", "0 9 * * *");
        let xml = render_plist(&a).unwrap();
        assert!(xml.contains("<key>claudepot_managed</key>\n  <true/>"));
        assert!(xml.contains("<key>claudepot_version</key>"));
    }

    #[test]
    fn capabilities_reports_launchd_truths() {
        let s = LaunchdScheduler;
        let caps = s.capabilities();
        assert_eq!(caps.native_label, "launchd");
        assert!(!caps.wake_to_run);
        assert!(!caps.catch_up_if_missed);
        assert!(!caps.run_when_logged_out);
        assert!(caps.artifact_dir.is_some());
    }

    #[test]
    fn next_runs_works_via_trait() {
        let s = LaunchdScheduler;
        let trigger = Trigger::Cron {
            cron: "0 9 * * *".into(),
            timezone: None,
        };
        let from = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 4, 28, 8, 0, 0).unwrap();
        let next = s.next_runs(&trigger, from, 2).unwrap();
        assert_eq!(next.len(), 2);
    }
}
