//! Hidden CLI surface for the Automations feature.
//!
//! v1 exposes exactly one verb — `_record-run` — invoked by the
//! per-automation helper shim after `claude -p` exits. It parses
//! the redirected `stdout.log`, assembles an [`AutomationRun`]
//! record, and writes `result.json` next to the logs.
//!
//! The leading underscore is intentional: this is plumbing, not a
//! user-facing surface. The Automations GUI section is the
//! sanctioned way to manage automations.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use claudepot_core::automations::{
    automation_runs_dir, record_run, AutomationId, RecordInputs, TriggerKind,
};
use uuid::Uuid;

/// Trigger-kind name as accepted on the CLI.
fn parse_trigger(s: &str) -> Result<TriggerKind> {
    match s {
        "scheduled" => Ok(TriggerKind::Scheduled),
        "manual" => Ok(TriggerKind::Manual),
        other => Err(anyhow!(
            "unknown --trigger value '{other}' (expected 'scheduled' or 'manual')"
        )),
    }
}

/// Parse a unix timestamp (seconds, integer string).
fn parse_unix_seconds(raw: &str) -> Result<DateTime<Utc>> {
    let secs: i64 = raw
        .trim()
        .parse()
        .with_context(|| format!("invalid unix timestamp: {raw:?}"))?;
    Utc.timestamp_opt(secs, 0)
        .single()
        .ok_or_else(|| anyhow!("ambiguous or out-of-range unix timestamp: {secs}"))
}

#[allow(clippy::too_many_arguments)]
pub fn record_run_cmd(
    automation_id: &str,
    run_id: &str,
    exit: i32,
    start: &str,
    end: &str,
    trigger: &str,
    run_dir: Option<&str>,
) -> Result<()> {
    let id: AutomationId = Uuid::parse_str(automation_id.trim())
        .with_context(|| format!("invalid automation id: {automation_id:?}"))?;
    let trigger_kind = parse_trigger(trigger)?;
    let started_at = parse_unix_seconds(start)?;
    let ended_at = parse_unix_seconds(end)?;
    if ended_at < started_at {
        return Err(anyhow!(
            "ended_at ({ended_at}) is before started_at ({started_at})"
        ));
    }

    // Locate the run directory. The shim passes --run-dir explicitly
    // (the authoritative source); fall back to the default layout for
    // backward compat / manual invocation.
    let run_dir: PathBuf = match run_dir {
        Some(p) => PathBuf::from(p),
        None => automation_runs_dir(&id).join(run_id),
    };
    if !run_dir.exists() {
        return Err(anyhow!(
            "run directory does not exist: {} — did the shim run?",
            run_dir.display()
        ));
    }
    let stdout_log = run_dir.join("stdout.log");
    let stderr_log = run_dir.join("stderr.log");

    let inputs = RecordInputs {
        automation_id: id,
        run_id,
        exit_code: exit,
        started_at,
        ended_at,
        trigger_kind,
        stdout_log_path: &stdout_log,
        stderr_log_path: &stderr_log,
        claudepot_version: env!("CARGO_PKG_VERSION"),
    };
    let _run = record_run(&inputs).with_context(|| {
        format!(
            "failed to record run: automation={id} run={run_id} dir={}",
            run_dir.display()
        )
    })?;

    // The shim sets exit code from the underlying `claude -p`; we
    // just confirm we wrote the record.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trigger_accepts_known() {
        assert_eq!(parse_trigger("scheduled").unwrap(), TriggerKind::Scheduled);
        assert_eq!(parse_trigger("manual").unwrap(), TriggerKind::Manual);
        assert!(parse_trigger("nope").is_err());
    }

    #[test]
    fn parse_unix_seconds_round_trip() {
        let dt = parse_unix_seconds("1745836800").unwrap();
        assert_eq!(dt.timestamp(), 1745836800);
        assert!(parse_unix_seconds("not a number").is_err());
        assert!(parse_unix_seconds("").is_err());
    }
}
