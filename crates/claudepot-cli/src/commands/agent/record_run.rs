//! The `_record-run` plumbing verb.
//!
//! Invoked by the per-agent helper shim after `claude -p` exits.
//! It parses the redirected `stdout.log`, assembles an [`AgentRun`]
//! record, and writes `result.json` next to the logs.
//!
//! The leading underscore is intentional: this is plumbing, not a
//! user-facing surface. The user-/AI-facing verbs (`draft` /
//! `list` / `show`) live in sibling modules; see the `agent.rs`
//! entry file for the draft/install gate this surface is part of.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use claudepot_core::agent::{
    agent_runs_dir, record_run, AgentId, AgentStore, RecordInputs, TriggerKind,
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

/// Parse a unix timestamp (seconds, integer string). Empty input
/// falls back to "now" — useful when the calling shim can't
/// reliably compute timestamps (e.g. Task Scheduler contexts that
/// don't inherit PowerShell on PATH).
fn parse_unix_seconds(raw: &str) -> Result<DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Utc::now());
    }
    let secs: i64 = trimmed
        .parse()
        .with_context(|| format!("invalid unix timestamp: {raw:?}"))?;
    Utc.timestamp_opt(secs, 0)
        .single()
        .ok_or_else(|| anyhow!("ambiguous or out-of-range unix timestamp: {secs}"))
}

#[allow(clippy::too_many_arguments)]
pub fn record_run_cmd(
    agent_id: &str,
    run_id: &str,
    exit: i32,
    start: &str,
    end: &str,
    trigger: &str,
    run_dir: Option<&str>,
) -> Result<()> {
    let id: AgentId = Uuid::parse_str(agent_id.trim())
        .with_context(|| format!("invalid agent id: {agent_id:?}"))?;
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
        None => agent_runs_dir(&id).join(run_id),
    };
    if !run_dir.exists() {
        return Err(anyhow!(
            "run directory does not exist: {} — did the shim run?",
            run_dir.display()
        ));
    }
    let stdout_log = run_dir.join("stdout.log");
    let stderr_log = run_dir.join("stderr.log");

    // Resolve the agent's `log_retention_runs` so `record_run` can
    // prune old run dirs after writing this run's `result.json`
    // (grill finding F12). `_record-run` is a short-lived CLI
    // process spawned by the shim, so opening the store here is
    // cheap. A store-open / agent-not-found failure resolves to
    // `None` — the prune is skipped, never guessed.
    let log_retention_runs = AgentStore::open()
        .ok()
        .and_then(|store| store.get(&id).map(|a| a.log_retention_runs));

    let inputs = RecordInputs {
        agent_id: id,
        run_id,
        exit_code: exit,
        started_at,
        ended_at,
        trigger_kind,
        stdout_log_path: &stdout_log,
        stderr_log_path: &stderr_log,
        claudepot_version: env!("CARGO_PKG_VERSION"),
        log_retention_runs,
    };
    match record_run(&inputs) {
        Ok(_run) => {
            // The shim sets exit code from the underlying `claude -p`;
            // we just confirm we wrote the record.
            Ok(())
        }
        Err(e) => {
            // The shim invokes `_record-run` with a trailing `|| true`
            // so a failed record-run cannot abort the shim before it
            // re-raises the real `claude -p` exit code (grill F5).
            // That makes a broken record-run *invisible* from the
            // shim's side. So `_record-run` drops a breadcrumb file
            // in the run dir on its own failure: a non-empty
            // `record-run-error.txt` next to the (missing or stale)
            // `result.json` is the durable signal that this run's
            // result was never recorded. The breadcrumb write itself
            // is best-effort — if even that fails we still propagate
            // the original error.
            let breadcrumb = run_dir.join("record-run-error.txt");
            let body = format!(
                "record-run failed for agent={id} run={run_id}\n\
                 dir={}\nexit_code={exit}\n\nerror: {e:#}\n",
                run_dir.display()
            );
            if let Err(write_err) = std::fs::write(&breadcrumb, body) {
                eprintln!(
                    "claudepot: record-run failed AND its breadcrumb write \
                     failed ({write_err}) — run {run_id} is unrecorded"
                );
            }
            Err(e).with_context(|| {
                format!(
                    "failed to record run: agent={id} run={run_id} dir={}",
                    run_dir.display()
                )
            })
        }
    }
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
    }

    #[test]
    fn parse_unix_seconds_empty_falls_back_to_now() {
        let dt = parse_unix_seconds("").unwrap();
        let now = Utc::now();
        // Within 5 seconds of "now" — the shim falls back to
        // current time when it can't compute its own.
        assert!((now - dt).num_seconds().abs() < 5);
        let dt2 = parse_unix_seconds("   ").unwrap();
        assert!((now - dt2).num_seconds().abs() < 5);
    }

    #[test]
    fn record_run_cmd_drops_breadcrumb_when_recording_fails() {
        // grill F5: a failed `_record-run` must not be invisible.
        // Force `record_run` to fail by occupying the `result.json`
        // path with a *directory* — `atomic_write` cannot replace a
        // directory with a file. The run dir itself stays writable,
        // so the `record-run-error.txt` breadcrumb must land.
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let run_dir = dir.path().join("run-1");
        std::fs::create_dir_all(&run_dir).unwrap();
        // A stdout.log so the read step succeeds; the failure we
        // want is the result.json *write*.
        let mut f = std::fs::File::create(run_dir.join("stdout.log")).unwrap();
        f.write_all(b"[]").unwrap();
        drop(f);
        // result.json is a NON-EMPTY directory → renaming a file
        // over it (the last step of `atomic_write`) fails.
        std::fs::create_dir_all(run_dir.join("result.json")).unwrap();
        std::fs::write(run_dir.join("result.json").join("occupant"), b"x").unwrap();

        let agent_id = Uuid::new_v4().to_string();
        let res = record_run_cmd(
            &agent_id,
            "run-1",
            0,
            "1745836800",
            "1745836810",
            "scheduled",
            Some(run_dir.to_str().unwrap()),
        );

        assert!(res.is_err(), "record-run must fail when result.json is a dir");
        let breadcrumb = run_dir.join("record-run-error.txt");
        assert!(
            breadcrumb.exists(),
            "expected a record-run-error.txt breadcrumb on failure"
        );
        let body = std::fs::read_to_string(&breadcrumb).unwrap();
        assert!(body.contains("record-run failed"));
        assert!(body.contains("run-1"));
    }
}
