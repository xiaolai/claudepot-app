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
    // (grill findings F12 + X10).
    //
    // Previously this called the blocking `AgentStore::open()` —
    // which acquires the store's exclusive advisory lock — just to
    // read one field. Under GUI contention (the user mid-installing
    // an agent while a different agent's `_record-run` shim runs)
    // every `claude -p` exit stalled until the GUI released the
    // lock. We now use the non-blocking `try_open`:
    //
    // - lock free → read the field, prune as usual.
    // - lock held → log at `debug!` (skipped retention is harmless;
    //   the next run will prune) and proceed without a count.
    // - real I/O / migration error → log at `warn!` and proceed
    //   without a count (the run record itself must still be
    //   written).
    //
    // The retention behavior remains best-effort by design; skipping
    // a single pass while the GUI mid-installs is the right trade.
    let log_retention_runs = match AgentStore::try_open() {
        Ok(Some(store)) => store.get(&id).map(|a| a.log_retention_runs),
        Ok(None) => {
            tracing::debug!(
                agent_id = %id,
                run_id = %run_id,
                "record-run: agent store lock busy — skipping retention prune \
                 this call (a later run will catch up)"
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                agent_id = %id,
                run_id = %run_id,
                error = %e,
                "record-run: agent store open failed — skipping retention prune \
                 this call; the run record will still be written"
            );
            None
        }
    };

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
            // Drain the agent's result_sink, if it has one. This is the
            // deterministic half of the harvester: the model emitted
            // JSON, and Claudepot — not the model — writes it into the
            // knowledge base. See `shared_memory::proposal`.
            //
            // Best-effort on purpose. A failed ingest must not fail the
            // run: the run *happened*, `result.json` is on disk, and a
            // later harvest can re-read it. Losing the run record to
            // save a proposal would be the wrong trade.
            if exit == 0 {
                if let Err(e) = drain_result_sink(&id, &run_dir) {
                    tracing::warn!(
                        agent_id = %id, run_id = %run_id, error = %e,
                        "record-run: result_sink ingest failed; result.json is still on disk"
                    );
                }
            }
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

/// Deposit a finished run's structured output into its declared sink.
///
/// Only [`ResultSink::MemoryProposals`] exists today: parse the run's
/// model output as distilled claims and file them as **proposals**
/// awaiting human review. Nothing here can produce an accepted memory —
/// that transition is the human's, and only the human's.
///
/// Reads **`stdout.log`**, NOT `result.json`: `record_run` has already
/// overwritten `result.json` with the `AgentRun` metadata record (which
/// has no `claims` field), so reading it back would always parse to an
/// empty harvest and the sink would silently do nothing. `stdout.log`
/// holds the raw `claude -p --output-format json` output — the
/// `{"type":"result","result":"…"}` envelope `parse_claims` already
/// unwraps.
fn drain_result_sink(id: &AgentId, run_dir: &std::path::Path) -> Result<()> {
    use claudepot_core::agent::ResultSink;
    use claudepot_core::shared_memory::proposal;

    // Non-blocking: if the GUI holds the store lock we skip this pass
    // rather than stall the shim. `stdout.log` stays on disk, so a
    // later `claudepot lesson harvest` picks up the source session.
    let Some(store) = AgentStore::try_open()? else {
        tracing::debug!(agent_id = %id, "record-run: store lock busy — deferring sink drain");
        return Ok(());
    };
    let Some(agent) = store.get(id) else {
        return Ok(());
    };
    let Some(sink) = agent.result_sink else {
        return Ok(());
    };
    let cwd = agent.cwd.clone();
    let created_by = format!("agent:{}", agent.name);
    drop(store); // release the lock before touching sessions.db

    match sink {
        ResultSink::MemoryProposals => {
            let raw = std::fs::read_to_string(run_dir.join("stdout.log"))
                .context("read stdout.log for the memory sink")?;
            let claims = proposal::parse_claims(&raw).context("parse distilled claims")?;
            if claims.claims.is_empty() {
                // The common case, and a correct one: most sessions
                // teach nothing. Say so at debug, not warn.
                tracing::debug!(agent_id = %id, "distiller returned no claims");
                return Ok(());
            }

            let db = claudepot_core::paths::claudepot_data_dir().join("sessions.db");
            let idx = claudepot_core::session_index::SessionIndex::open(&db)
                .context("open sessions.db to file proposals")?;

            // The transcript that taught the lesson — the orchestrator
            // hands it to the run in the same env var the prompt reads.
            let file_path = std::env::var("CLAUDEPOT_EVENT_SESSION_PATH").ok();
            let origin = proposal::ProposalOrigin {
                project_path: &cwd,
                file_path: file_path.as_deref(),
                exchange_id: None,
                created_by: &created_by,
            };
            let now_ms = chrono::Utc::now().timestamp_millis();
            let report = proposal::ingest_proposals(&idx, &claims, &origin, now_ms)
                .context("file distilled claims as proposals")?;
            tracing::info!(
                agent_id = %id,
                proposed = report.proposed,
                skipped = report.total_skipped(),
                "filed distilled claims for review"
            );
            Ok(())
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
    fn the_memory_sink_reads_stdout_log_and_files_a_proposal() {
        // Regression for the Critical: drain_result_sink used to read
        // result.json, which record_run had already overwritten with the
        // AgentRun metadata (no `claims` field) — so the agent-triggered
        // harvest was a silent no-op. It must read stdout.log, the raw
        // `claude -p --output-format json` output. This test writes BOTH
        // a claims-free result.json AND a claims-bearing stdout.log, then
        // asserts the claims landed — pinning that stdout.log is the
        // source. Runs against the CLAUDEPOT_DATA_DIR the test runner
        // isolates (repo-invariants guard 5).
        use claudepot_core::agent::templates::knowledge_distiller;
        use claudepot_core::agent::{AgentStore, Lifecycle};

        let run_dir = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let cwd_str = cwd.path().to_string_lossy().into_owned();

        // A distiller agent (result_sink = MemoryProposals) in the store.
        let mut agent = knowledge_distiller(&cwd_str, Utc::now());
        agent.lifecycle = Lifecycle::Installed; // add() accepts installed
        let id = agent.id;
        let mut store = AgentStore::open().expect("open agent store");
        store.add(agent).expect("add distiller");
        store.save().expect("save");
        // Release the store lock — drain uses the NON-blocking try_open()
        // and would otherwise skip silently while we hold it.
        drop(store);

        // result.json as record_run leaves it: metadata, NO claims.
        std::fs::write(
            run_dir.path().join("result.json"),
            r#"{"agent_id":"x","exit_code":0,"result":{"is_error":false}}"#,
        )
        .unwrap();
        // stdout.log: the real model output — a claims envelope.
        std::fs::write(
            run_dir.path().join("stdout.log"),
            "{\"type\":\"result\",\"result\":\"{\\\"claims\\\":[{\\\"claim\\\":\
             \\\"always run preflight before pushing\\\",\\\"directive\\\":\
             \\\"Run scripts/preflight.sh before pushing.\\\",\\\"kind\\\":\
             \\\"constraint\\\",\\\"evidence\\\":\\\"CI went red\\\",\\\"confidence\\\":90}]}\"}",
        )
        .unwrap();

        drain_result_sink(&id, run_dir.path()).expect("drain");

        // A proposal must now exist in sessions.db under the agent's cwd.
        let db = claudepot_core::paths::claudepot_data_dir().join("sessions.db");
        let idx = claudepot_core::session_index::SessionIndex::open(&db).unwrap();
        let rows =
            claudepot_core::shared_memory::review::list(&idx, Some(&cwd_str), None, 50).unwrap();
        assert_eq!(rows.len(), 1, "the stdout.log claim must have been filed");
        assert_eq!(rows[0].review_state, "proposed");
        assert!(rows[0].content.contains("preflight"));
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

        assert!(
            res.is_err(),
            "record-run must fail when result.json is a dir"
        );
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
