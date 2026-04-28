//! Windows scheduler adapter — Task Scheduler via `schtasks.exe`.
//!
//! Each automation maps to a registered task at
//! `\Claudepot\automation_<id>` whose XML lives at
//! `<claudepot_data_dir>/scheduled-tasks/<id>.xml`. Operations:
//! `schtasks /Create /XML <path> /TN ...` to register, `/Delete`
//! to unregister, `/Run` to kickstart.
//!
//! Capabilities: wake_to_run via `<WakeToRun>true</WakeToRun>`,
//! catch_up_if_missed via `<StartWhenAvailable>true</StartWhenAvailable>`,
//! run_when_logged_out controlled by `<LogonType>` —
//! `InteractiveToken` (logged-in only) vs `Password` (run as
//! stored credentials, requires user typing the password into the
//! OS prompt at registration).

use std::path::PathBuf;
use std::process::Command;

use chrono::{DateTime, Utc};

use crate::automations::cron::{self, LaunchSlot};
use crate::automations::error::AutomationError;
use crate::automations::store::automation_dir;
use crate::automations::types::{Automation, AutomationId, Trigger};

use super::xml::{indent, xml_escape};
use super::{cron_next_runs, RegisteredEntry, Scheduler, SchedulerCapabilities};

const TASK_PATH_PREFIX: &str = r"\Claudepot\automation_";

pub struct SchtasksScheduler;

pub fn task_path_for(id: &AutomationId) -> String {
    format!("{TASK_PATH_PREFIX}{id}")
}

/// Disk path to the persisted XML registration file.
pub fn xml_path_for(id: &AutomationId) -> PathBuf {
    crate::paths::claudepot_data_dir()
        .join("scheduled-tasks")
        .join(format!("{id}.xml"))
}

impl Scheduler for SchtasksScheduler {
    fn register(&self, automation: &Automation) -> Result<(), AutomationError> {
        let xml = render_xml(automation)?;
        let xml_path = xml_path_for(&automation.id);
        if let Some(parent) = xml_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Task Scheduler reads XML in UTF-16 LE with BOM.
        let utf16: Vec<u8> = std::iter::once(0xFF)
            .chain(std::iter::once(0xFE))
            .chain(
                xml.encode_utf16()
                    .flat_map(|c| [(c & 0xFF) as u8, (c >> 8) as u8]),
            )
            .collect();
        crate::fs_utils::atomic_write(&xml_path, &utf16)?;
        let task_path = task_path_for(&automation.id);
        run_schtasks(&[
            "/Create",
            "/XML",
            xml_path.to_str().unwrap_or(""),
            "/TN",
            &task_path,
            "/F",
        ])
        .map_err(io_err)?;
        Ok(())
    }

    fn unregister(&self, id: &AutomationId) -> Result<(), AutomationError> {
        let _ = run_schtasks(&["/Delete", "/TN", &task_path_for(id), "/F"]);
        let xml_path = xml_path_for(id);
        if xml_path.exists() {
            std::fs::remove_file(&xml_path)?;
        }
        Ok(())
    }

    fn kickstart(&self, id: &AutomationId) -> Result<(), AutomationError> {
        run_schtasks(&["/Run", "/TN", &task_path_for(id)]).map_err(io_err)
    }

    fn list_managed(&self) -> Result<Vec<RegisteredEntry>, AutomationError> {
        // Query the Claudepot folder; CSV output for parseability.
        let out = match Command::new("schtasks")
            .args(["/Query", "/TN", r"\Claudepot\*", "/FO", "CSV", "/NH"])
            .output()
        {
            Ok(o) => o,
            Err(_) => return Ok(Vec::new()),
        };
        if !out.status.success() {
            // No tasks under \Claudepot\ → empty.
            return Ok(Vec::new());
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut entries = Vec::new();
        for line in stdout.lines() {
            // CSV row: "TaskName","Next Run Time","Status"
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let task_name = trimmed
                .trim_start_matches('"')
                .split('"')
                .next()
                .unwrap_or("");
            if task_name.is_empty() {
                continue;
            }
            // The presence of the task under our namespace IS the
            // managed marker for now; XML parsing of the source
            // attribute is a future refinement.
            entries.push(RegisteredEntry {
                identifier: task_name.to_string(),
                claudepot_managed: task_name.starts_with(r"\Claudepot\"),
            });
        }
        Ok(entries)
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
            // `run_when_logged_out` requires `<LogonType>Password</LogonType>`
            // which in turn requires storing user credentials at
            // registration time. We do not yet have a secure
            // credential capture flow (the GUI would need to prompt
            // the user for their Windows password and pass it via
            // `schtasks /RU /RP`, then zeroize). Until that lands,
            // report `false` so the UI greys out the toggle.
            run_when_logged_out: false,
            native_label: "Task Scheduler",
            artifact_dir: Some(
                crate::paths::claudepot_data_dir()
                    .join("scheduled-tasks")
                    .display()
                    .to_string(),
            ),
        }
    }
}

fn io_err(s: String) -> AutomationError {
    AutomationError::Io(std::io::Error::other(s))
}

fn run_schtasks(args: &[&str]) -> Result<(), String> {
    let out = Command::new("schtasks")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(format!(
        "schtasks {} failed (status {}): {}",
        args.join(" "),
        out.status,
        stderr.trim()
    ))
}

/// Render Task Scheduler XML for one automation. Pure: no OS calls.
/// Output is byte-stable for golden tests.
pub fn render_xml(automation: &Automation) -> Result<String, AutomationError> {
    let auto_dir = automation_dir(&automation.id);
    let shim_path = auto_dir.join("run.cmd");
    let display_label = automation
        .display_name
        .clone()
        .unwrap_or_else(|| automation.name.clone());
    let user_id = std::env::var("USERNAME")
        .ok()
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| String::from("user"));

    let slots = match &automation.trigger {
        Trigger::Cron { cron: expr, .. } => cron::expand(expr)?,
    };

    let opts = &automation.platform_options;
    // We force `InteractiveToken` until the credential-capture flow
    // for `run_when_logged_out=true` ships. Capabilities() returns
    // `false` for this knob, so the UI shouldn't even let it through;
    // this is belt-and-braces in case a stale record arrives.
    let logon_type = "InteractiveToken";
    let _ = opts.run_when_logged_out;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n");
    xml.push_str(
        "<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n",
    );

    // Registration info — the managed marker lives here.
    indent(&mut xml, 1);
    xml.push_str("<RegistrationInfo>\n");
    indent(&mut xml, 2);
    xml.push_str(&format!(
        "<Description>Claudepot automation: {}</Description>\n",
        xml_escape(&display_label)
    ));
    indent(&mut xml, 2);
    xml.push_str("<Source>claudepot</Source>\n");
    indent(&mut xml, 2);
    xml.push_str(&format!(
        "<URI>{}</URI>\n",
        xml_escape(&task_path_for(&automation.id))
    ));
    indent(&mut xml, 1);
    xml.push_str("</RegistrationInfo>\n");

    // Triggers.
    indent(&mut xml, 1);
    xml.push_str("<Triggers>\n");
    for slot in &slots {
        emit_calendar_trigger(&mut xml, 2, slot);
    }
    indent(&mut xml, 1);
    xml.push_str("</Triggers>\n");

    // Principals.
    indent(&mut xml, 1);
    xml.push_str("<Principals>\n");
    indent(&mut xml, 2);
    xml.push_str("<Principal id=\"Author\">\n");
    indent(&mut xml, 3);
    xml.push_str(&format!("<UserId>{}</UserId>\n", xml_escape(&user_id)));
    indent(&mut xml, 3);
    xml.push_str(&format!("<LogonType>{logon_type}</LogonType>\n"));
    indent(&mut xml, 3);
    xml.push_str("<RunLevel>LeastPrivilege</RunLevel>\n");
    indent(&mut xml, 2);
    xml.push_str("</Principal>\n");
    indent(&mut xml, 1);
    xml.push_str("</Principals>\n");

    // Settings.
    indent(&mut xml, 1);
    xml.push_str("<Settings>\n");
    indent(&mut xml, 2);
    xml.push_str("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n");
    indent(&mut xml, 2);
    xml.push_str("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n");
    indent(&mut xml, 2);
    xml.push_str("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n");
    indent(&mut xml, 2);
    xml.push_str("<AllowHardTerminate>true</AllowHardTerminate>\n");
    indent(&mut xml, 2);
    xml.push_str(&format!(
        "<StartWhenAvailable>{}</StartWhenAvailable>\n",
        opts.catch_up_if_missed
    ));
    indent(&mut xml, 2);
    xml.push_str(&format!("<WakeToRun>{}</WakeToRun>\n", opts.wake_to_run));
    indent(&mut xml, 2);
    xml.push_str("<RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>\n");
    indent(&mut xml, 2);
    xml.push_str("<Hidden>false</Hidden>\n");
    indent(&mut xml, 1);
    xml.push_str("</Settings>\n");

    // Actions.
    indent(&mut xml, 1);
    xml.push_str("<Actions Context=\"Author\">\n");
    indent(&mut xml, 2);
    xml.push_str("<Exec>\n");
    indent(&mut xml, 3);
    xml.push_str(&format!(
        "<Command>{}</Command>\n",
        xml_escape(r"C:\Windows\System32\cmd.exe")
    ));
    indent(&mut xml, 3);
    xml.push_str(&format!(
        "<Arguments>/C \"{}\"</Arguments>\n",
        xml_escape(&shim_path.display().to_string())
    ));
    indent(&mut xml, 3);
    xml.push_str(&format!(
        "<WorkingDirectory>{}</WorkingDirectory>\n",
        xml_escape(&automation.cwd)
    ));
    indent(&mut xml, 2);
    xml.push_str("</Exec>\n");
    indent(&mut xml, 1);
    xml.push_str("</Actions>\n");

    xml.push_str("</Task>\n");
    Ok(xml)
}

fn emit_calendar_trigger(out: &mut String, level: usize, slot: &LaunchSlot) {
    indent(out, level);
    out.push_str("<CalendarTrigger>\n");

    // StartBoundary — required. We use a year-2000 anchor; the
    // actual schedule is driven by the schedule sub-elements.
    let hh = format!("{:02}", slot.hour);
    let mm = format!("{:02}", slot.minute);
    indent(out, level + 1);
    out.push_str(&format!(
        "<StartBoundary>2000-01-01T{hh}:{mm}:00</StartBoundary>\n"
    ));
    indent(out, level + 1);
    out.push_str("<Enabled>true</Enabled>\n");

    // Branch order matters: a slot with both `day_of_week` and
    // `month` set is a "this weekday in this month" schedule; emit
    // a `ScheduleByMonthDayOfWeek` so the month constraint is
    // preserved. Plain weekday-only collapses to ScheduleByWeek.
    // DOM-with-or-without month falls through to ScheduleByMonth.
    // Everything else is a daily.
    let all_months_array = |out: &mut String, lvl: usize| {
        for m in [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ] {
            indent(out, lvl);
            out.push_str(&format!("<{m} />\n"));
        }
    };
    let day_tag_for = |w: u8| match w {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        _ => "Saturday",
    };
    if let (Some(w), Some(m)) = (slot.day_of_week, slot.month) {
        // Weekday + month: ScheduleByMonthDayOfWeek (monthly XX-day,
        // first/second/.../last week — but the cron we accept doesn't
        // distinguish weeks-of-month, so we use Weeks::All).
        indent(out, level + 1);
        out.push_str("<ScheduleByMonthDayOfWeek>\n");
        indent(out, level + 2);
        out.push_str("<Weeks><Week>1</Week><Week>2</Week><Week>3</Week><Week>4</Week><Week>Last</Week></Weeks>\n");
        indent(out, level + 2);
        out.push_str("<DaysOfWeek>\n");
        let day_tag = day_tag_for(w);
        indent(out, level + 3);
        out.push_str(&format!("<{day_tag} />\n"));
        indent(out, level + 2);
        out.push_str("</DaysOfWeek>\n");
        indent(out, level + 2);
        out.push_str("<Months>\n");
        let mtag = match m {
            1 => "January",
            2 => "February",
            3 => "March",
            4 => "April",
            5 => "May",
            6 => "June",
            7 => "July",
            8 => "August",
            9 => "September",
            10 => "October",
            11 => "November",
            _ => "December",
        };
        indent(out, level + 3);
        out.push_str(&format!("<{mtag} />\n"));
        indent(out, level + 2);
        out.push_str("</Months>\n");
        indent(out, level + 1);
        out.push_str("</ScheduleByMonthDayOfWeek>\n");
    } else if let Some(w) = slot.day_of_week {
        // Weekly only.
        indent(out, level + 1);
        out.push_str("<ScheduleByWeek>\n");
        indent(out, level + 2);
        out.push_str("<DaysOfWeek>\n");
        let day_tag = day_tag_for(w);
        indent(out, level + 3);
        out.push_str(&format!("<{day_tag} />\n"));
        indent(out, level + 2);
        out.push_str("</DaysOfWeek>\n");
        indent(out, level + 2);
        out.push_str("<WeeksInterval>1</WeeksInterval>\n");
        indent(out, level + 1);
        out.push_str("</ScheduleByWeek>\n");
    } else if slot.day_of_month.is_some() || slot.month.is_some() {
        // Monthly. If `day_of_month` is wildcard but `month` is set,
        // fan out across every day of the month rather than
        // collapsing to day 1 (the previous behavior silently dropped
        // 30 of the 31 fire days).
        indent(out, level + 1);
        out.push_str("<ScheduleByMonth>\n");
        indent(out, level + 2);
        out.push_str("<DaysOfMonth>\n");
        if let Some(d) = slot.day_of_month {
            indent(out, level + 3);
            out.push_str(&format!("<Day>{d}</Day>\n"));
        } else {
            for d in 1u8..=31 {
                indent(out, level + 3);
                out.push_str(&format!("<Day>{d}</Day>\n"));
            }
        }
        indent(out, level + 2);
        out.push_str("</DaysOfMonth>\n");
        indent(out, level + 2);
        out.push_str("<Months>\n");
        if let Some(m) = slot.month {
            let mtag = match m {
                1 => "January",
                2 => "February",
                3 => "March",
                4 => "April",
                5 => "May",
                6 => "June",
                7 => "July",
                8 => "August",
                9 => "September",
                10 => "October",
                11 => "November",
                _ => "December",
            };
            indent(out, level + 3);
            out.push_str(&format!("<{mtag} />\n"));
        } else {
            all_months_array(out, level + 3);
        }
        indent(out, level + 2);
        out.push_str("</Months>\n");
        indent(out, level + 1);
        out.push_str("</ScheduleByMonth>\n");
    } else {
        indent(out, level + 1);
        out.push_str("<ScheduleByDay>\n");
        indent(out, level + 2);
        out.push_str("<DaysInterval>1</DaysInterval>\n");
        indent(out, level + 1);
        out.push_str("</ScheduleByDay>\n");
    }

    indent(out, level);
    out.push_str("</CalendarTrigger>\n");
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
            display_name: Some("Pretty".into()),
            description: None,
            enabled: true,
            binary: AutomationBinary::FirstParty,
            model: Some("sonnet".into()),
            cwd: r"C:\Users\me\repo".into(),
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
    fn render_xml_daily_at_9() {
        let a = auto("morning", "0 9 * * *");
        let xml = render_xml(&a).unwrap();
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"));
        assert!(xml.contains("<Source>claudepot</Source>"));
        assert!(
            xml.contains("<URI>\\Claudepot\\automation_00000000-0000-0000-0000-000000000000</URI>")
        );
        assert!(xml.contains("<CalendarTrigger>"));
        assert!(xml.contains("<StartBoundary>2000-01-01T09:00:00</StartBoundary>"));
        assert!(xml.contains("<ScheduleByDay>"));
        assert!(xml.contains("<DaysInterval>1</DaysInterval>"));
        assert!(xml.contains("<Command>C:\\Windows\\System32\\cmd.exe</Command>"));
        assert!(xml.contains("run.cmd"));
        assert!(xml.contains("<WorkingDirectory>C:\\Users\\me\\repo</WorkingDirectory>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains("<WakeToRun>false</WakeToRun>"));
    }

    #[test]
    fn render_xml_weekday_emits_5_calendar_triggers() {
        let a = auto("biz", "0 9 * * 1-5");
        let xml = render_xml(&a).unwrap();
        let trig_count = xml.matches("<CalendarTrigger>").count();
        assert_eq!(trig_count, 5);
        assert!(xml.contains("<Monday />"));
        assert!(xml.contains("<Friday />"));
        assert!(!xml.contains("<Sunday />"));
    }

    // Currently stubbed: production force-`InteractiveToken` (see the
    // `let logon_type = "InteractiveToken";` block above) until the
    // Windows credential-capture flow ships. The test documents what
    // SHOULD happen when that lands; ignored so it doesn't block CI.
    #[test]
    #[ignore = "run_when_logged_out path is stubbed pending credential-capture flow"]
    fn render_xml_logon_type_password_when_logged_out() {
        let mut a = auto("logged-out", "0 9 * * *");
        a.platform_options.run_when_logged_out = true;
        let xml = render_xml(&a).unwrap();
        assert!(xml.contains("<LogonType>Password</LogonType>"));
    }

    #[test]
    fn render_xml_escapes_ampersand_in_cwd() {
        let mut a = auto("amp", "0 9 * * *");
        a.cwd = r"C:\Users\me\path & more".into();
        let xml = render_xml(&a).unwrap();
        assert!(xml.contains("path &amp; more"));
        assert!(!xml.contains("path & more"));
    }

    #[test]
    fn task_path_format() {
        let id = Uuid::nil();
        assert_eq!(
            task_path_for(&id),
            r"\Claudepot\automation_00000000-0000-0000-0000-000000000000"
        );
    }

    // `capabilities().run_when_logged_out` is intentionally `false`
    // until the credential-capture flow lands (see the explanatory
    // comment in `capabilities()`). Ignored so the assertion that
    // documents the eventual contract doesn't block CI.
    #[test]
    #[ignore = "run_when_logged_out cap is stubbed pending credential-capture flow"]
    fn capabilities_reports_schtasks_truths() {
        let s = SchtasksScheduler;
        let caps = s.capabilities();
        assert_eq!(caps.native_label, "Task Scheduler");
        assert!(caps.wake_to_run);
        assert!(caps.catch_up_if_missed);
        assert!(caps.run_when_logged_out);
    }
}
