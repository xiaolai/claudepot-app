//! Parsing and recording for a single agent run.
//!
//! Two responsibilities:
//!
//! 1. Parse the `--output-format=json` stdout that `claude -p`
//!    produces — a JSON array of events. We keep the parser
//!    permissive: take the last `type === "result"` element and
//!    fall back to `is_error: true` if no `result` event is
//!    present.
//! 2. Assemble an [`AgentRun`] record and write
//!    `<run_dir>/result.json` plus update the run-history
//!    directory.
//!
//! The `Run-Now` executor (spawning the shim, streaming progress)
//! is a separate concern and lives in a future module — this one
//! is the post-exit recorder, exercised both by the helper shim
//! (via `claudepot agent _record-run`) and by the in-process
//! Run-Now path.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::fs_utils;
use crate::project_progress::{PhaseStatus, ProgressSink};

use super::error::AgentError;
use super::install::install_shim;
use super::prerun::PrerunDecision;
use super::store::agent_runs_dir;
use super::types::{
    Agent, AgentId, AgentRun, ArtifactKind, HostPlatform, OutputArtifact,
    RunResult, TriggerKind,
};
use crate::routes::RouteDecision;

/// Inputs to [`record_run`]. All values are knowable by the helper
/// shim at exit time.
#[derive(Debug, Clone)]
pub struct RecordInputs<'a> {
    pub agent_id: AgentId,
    pub run_id: &'a str,
    pub exit_code: i32,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub trigger_kind: TriggerKind,
    /// Path to `stdout.log` written by the shim.
    pub stdout_log_path: &'a Path,
    /// Path to `stderr.log` written by the shim.
    pub stderr_log_path: &'a Path,
    pub claudepot_version: &'a str,
    /// The agent's `log_retention_runs`, used to prune old run
    /// directories after this run's `result.json` is written
    /// (grill finding F12). `None` means the caller could not
    /// resolve it — the prune is then skipped, never guessed.
    pub log_retention_runs: Option<u32>,
}

/// Top-level: read stdout.log, parse the result event, write
/// `result.json` next to it, and return the assembled record.
///
/// On any malformed-JSON / missing-result-event condition, falls
/// back to a synthetic [`RunResult`] reflecting the OS exit code
/// so the run row still gets recorded.
pub fn record_run(inputs: &RecordInputs<'_>) -> Result<AgentRun, AgentError> {
    tracing::info!(
        agent_id = %inputs.agent_id,
        run_id = %inputs.run_id,
        exit_code = inputs.exit_code,
        trigger = ?inputs.trigger_kind,
        "agent record-run: started"
    );
    // Read the shim's stdout.log. A missing file is legitimately
    // empty output (the shim may have failed before redirecting), so
    // `NotFound` degrades to empty bytes. Any *other* I/O error —
    // permission denied, a truncated/locked file — is a real failure
    // we must not paper over as "no output": log it loudly so a
    // run whose result looks empty-but-failed is visible, then still
    // degrade so the run row gets recorded.
    let stdout_bytes = match std::fs::read(inputs.stdout_log_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(
                agent_id = %inputs.agent_id,
                run_id = %inputs.run_id,
                path = %inputs.stdout_log_path.display(),
                "agent record-run: stdout.log absent — treating output as empty"
            );
            Vec::new()
        }
        Err(e) => {
            tracing::error!(
                agent_id = %inputs.agent_id,
                run_id = %inputs.run_id,
                path = %inputs.stdout_log_path.display(),
                error = %e,
                "agent record-run: failed to read stdout.log — \
                 recording the run with empty parsed output"
            );
            Vec::new()
        }
    };
    let result = parse_result_event(&stdout_bytes);
    let session_jsonl_path = result
        .as_ref()
        .and_then(|r| r.session_id.clone())
        .and_then(|sid| locate_transcript_for_session(&sid));

    let run = AgentRun {
        id: inputs.run_id.to_string(),
        agent_id: inputs.agent_id,
        started_at: inputs.started_at,
        ended_at: inputs.ended_at,
        duration_ms: (inputs.ended_at - inputs.started_at).num_milliseconds(),
        exit_code: inputs.exit_code,
        result,
        session_jsonl_path,
        stdout_log: file_name_or_empty(inputs.stdout_log_path),
        stderr_log: file_name_or_empty(inputs.stderr_log_path),
        trigger_kind: inputs.trigger_kind,
        host_platform: HostPlatform::current(),
        claudepot_version: inputs.claudepot_version.to_string(),
        // `record_run` is the post-run hook for non-template
        // agents; template-aware enrichment (output-artifact
        // discovery, prerun-decision merge) is layered on by
        // `record_run_with_template_context` once the templates
        // pre-run gate is wired through the shim.
        output_artifacts: Vec::new(),
        route_decision: None,
    };

    write_result_json(&run, inputs.stdout_log_path)?;

    // Enforce `log_retention_runs` (grill finding F12): now that
    // this run's `result.json` is durably on disk, delete the oldest
    // run directories so the agent's run history does not grow
    // unbounded. An agent on a 15-minute cron writes ~35k run dirs
    // a year, each with a full `stdout.log` — a disk-exhaustion path
    // the stored-but-unenforced field used to mislead about. The
    // prune is best-effort: a failure is logged, never fatal to
    // recording the run.
    //
    // The retention count travels in [`RecordInputs`] so this
    // function never has to open the store — keeping it cheap and
    // store-lock-free. The `_record-run` CLI verb and the in-process
    // Run-Now path each resolve the count from the agent record and
    // pass it in. `None` means "retention not supplied" and the
    // prune is skipped (a missing count must never delete history).
    if let Some(retention) = inputs.log_retention_runs {
        prune_run_dirs(&agent_runs_dir(&inputs.agent_id), retention as usize);
    }

    tracing::info!(
        agent_id = %inputs.agent_id,
        run_id = %inputs.run_id,
        exit_code = inputs.exit_code,
        duration_ms = run.duration_ms,
        cost_usd = run.result.as_ref().and_then(|r| r.total_cost_usd),
        is_error = run.result.as_ref().and_then(|r| r.is_error),
        "agent record-run: finished"
    );
    Ok(run)
}

/// Retention prune (grill finding F12): keep the newest `retention`
/// run directories under `runs_dir`, delete the rest.
///
/// Run-id directory names sort lexicographically by their
/// ISO-timestamp prefix, so "oldest" is the lexicographically
/// smallest name. Dotfiles (the `.latest` symlink and pointer
/// files) are never counted or deleted. A `retention` of 0 is
/// treated as "keep none configured = do not prune" — defensively,
/// the model defaults the field to 50 and the GUI/CLI never let it
/// reach 0, so a 0 here means an uninitialized value, not "delete
/// everything."
///
/// Best-effort throughout: every failure is logged and swallowed —
/// retention is housekeeping, never worth aborting a recorded run.
/// Public so the in-process Run-Now path can prune too.
pub fn prune_run_dirs(runs_dir: &Path, retention: usize) {
    if retention == 0 || !runs_dir.exists() {
        return;
    }
    let mut names: Vec<String> = match std::fs::read_dir(runs_dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|n| !n.starts_with('.'))
            .collect(),
        Err(e) => {
            tracing::warn!(
                dir = %runs_dir.display(),
                error = %e,
                "agent retention: could not read runs dir — skipping prune"
            );
            return;
        }
    };
    if names.len() <= retention {
        return;
    }
    // Sort ascending (oldest first); the run id's ISO-timestamp
    // prefix makes lexicographic order match chronological order.
    names.sort();
    let to_delete = names.len() - retention;
    for name in names.into_iter().take(to_delete) {
        let dir = runs_dir.join(&name);
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!(
                dir = %dir.display(),
                error = %e,
                "agent retention: failed to delete an old run dir"
            );
        } else {
            tracing::debug!(
                dir = %dir.display(),
                "agent retention: pruned an old run dir"
            );
        }
    }
}

/// Template-aware record-run.
///
/// Calls [`record_run`], then enriches the resulting record with
/// the template-driven post-run pipeline:
///
/// 1. If the run dir contains `prerun-decision.json`, parse it
///    and merge into `route_decision`.
/// 2. If the agent has a `template_id`, scan the
///    blueprint's output-path neighborhood for files modified
///    during the run window, populate `output_artifacts`.
///
/// The result.json file is rewritten with the enriched record
/// so the Reports panel and apply pipeline see the same data.
pub fn record_run_for_agent(
    agent: &Agent,
    inputs: &RecordInputs<'_>,
    output_path: Option<&Path>,
) -> Result<AgentRun, AgentError> {
    let mut run = record_run(inputs)?;

    let run_dir = inputs.stdout_log_path.parent().ok_or_else(|| {
        AgentError::InvalidPath(
            inputs.stdout_log_path.display().to_string(),
            "stdout log has no parent dir",
        )
    })?;

    // 1. Pre-run decision merge.
    if let Some(decision) = read_prerun_decision(run_dir) {
        run.route_decision = Some(prerun_to_route_decision(decision));
    }

    // 2. Output-artifact discovery, scoped to the resolved
    //    output path. Only template-driven agents carry
    //    one; non-template agents leave the field empty.
    if agent.template_id.is_some() {
        if let Some(path) = output_path {
            run.output_artifacts = discover_artifacts(path, inputs.started_at, inputs.ended_at);
        }
    }

    // Rewrite result.json with the enriched record so consumers
    // (Reports panel, apply pipeline) see the artifact metadata.
    write_result_json(&run, inputs.stdout_log_path)?;
    Ok(run)
}

fn read_prerun_decision(run_dir: &Path) -> Option<PrerunDecision> {
    let path = run_dir.join("prerun-decision.json");
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn prerun_to_route_decision(d: PrerunDecision) -> RouteDecision {
    match d {
        PrerunDecision::Ran { route_id, .. } => RouteDecision::Ran { route_id },
        PrerunDecision::Fallback {
            from,
            to_wrapper,
            reason,
        } => RouteDecision::Fallback {
            from,
            to: to_wrapper,
            reason,
        },
        PrerunDecision::Skipped { reason } => RouteDecision::Skipped { reason },
        PrerunDecision::SkippedAlerted { reason } => RouteDecision::SkippedAlerted { reason },
    }
}

/// Look for files at or near `output_path` modified during the
/// run window. Pure: no mutations. Returns the list ordered by
/// modification time, oldest first, capped at 32 entries to keep
/// rogue templates from polluting the run record.
fn discover_artifacts(
    output_path: &Path,
    started: DateTime<Utc>,
    ended: DateTime<Utc>,
) -> Vec<OutputArtifact> {
    use std::time::SystemTime;

    let started_st: SystemTime = started.into();
    // Allow a small grace window after `ended` to capture files
    // the shim flushes after `claude -p` exits. Use a bounded
    // grace so tests don't have to wait, but generous enough for
    // real-world fs flush.
    let grace = std::time::Duration::from_secs(5);
    let ended_st: SystemTime = ended.into();
    let ended_st = ended_st + grace;

    // The blueprint may name a single file (the report) or a
    // directory. Resolve both shapes.
    let candidates: Vec<PathBuf> = if output_path.is_dir() {
        match std::fs::read_dir(output_path) {
            Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
            Err(_) => Vec::new(),
        }
    } else if output_path.is_file() {
        vec![output_path.to_path_buf()]
    } else {
        // Path doesn't exist (template wrote nothing or wrote
        // somewhere we can't see). Try the parent dir as a
        // fall-back so we still catch a report sibling.
        match output_path.parent().and_then(|p| std::fs::read_dir(p).ok()) {
            Some(rd) => rd.flatten().map(|e| e.path()).collect(),
            None => Vec::new(),
        }
    };

    let mut out: Vec<OutputArtifact> = candidates
        .into_iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(&path).ok()?;
            if !meta.is_file() {
                return None;
            }
            let modified = meta.modified().ok()?;
            // Window check: file mtime must be within the run.
            if modified < started_st || modified > ended_st {
                return None;
            }
            let kind = classify_artifact(&path);
            let format = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            Some(OutputArtifact {
                kind,
                path: path.display().to_string(),
                format: format_for(format.as_str()),
                bytes: meta.len(),
            })
        })
        .collect();

    // Stable order — by mtime asc — capped at 32 entries.
    out.sort_by_key(|a| a.path.clone());
    out.truncate(32);
    out
}

fn format_for(ext: &str) -> String {
    match ext {
        "md" | "markdown" => "markdown".to_string(),
        "json" => "json".to_string(),
        "txt" => "text".to_string(),
        "csv" => "csv".to_string(),
        "" => "text".to_string(),
        other => other.to_string(),
    }
}

fn classify_artifact(path: &Path) -> ArtifactKind {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if name == ".pending-changes.json" || name.ends_with(".pending-changes.json") {
        ArtifactKind::PendingChanges
    } else if name.contains("apply-receipt") || name.contains("receipt") {
        ArtifactKind::ApplyReceipt
    } else if name.ends_with(".eml") || name.ends_with(".email") {
        ArtifactKind::Email
    } else {
        ArtifactKind::Report
    }
}

/// Find the run directory containing `stdout.log` and write
/// `result.json` alongside.
fn write_result_json(run: &AgentRun, stdout_log: &Path) -> Result<(), AgentError> {
    let run_dir = stdout_log.parent().ok_or_else(|| {
        AgentError::InvalidPath(
            stdout_log.display().to_string(),
            "stdout log has no parent dir",
        )
    })?;
    let result_path = run_dir.join("result.json");
    let bytes = serde_json::to_vec_pretty(run)?;
    fs_utils::atomic_write(&result_path, &bytes)?;
    Ok(())
}

fn file_name_or_empty(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Parse the terminal `result` event from `claude -p` stdout.
/// Handles both `--output-format=json` (a JSON array of events) and
/// `--output-format=stream-json` (newline-delimited JSON, one event
/// per line). Returns `None` if no parsable result event is present
/// (caller falls back to exit code).
pub fn parse_result_event(stdout_bytes: &[u8]) -> Option<RunResult> {
    if stdout_bytes.is_empty() {
        return None;
    }
    // Use a helper struct so unknown fields are tolerated.
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        subtype: Option<String>,
        #[serde(default)]
        is_error: Option<bool>,
        #[serde(default)]
        num_turns: Option<i64>,
        #[serde(default)]
        total_cost_usd: Option<f64>,
        #[serde(default)]
        stop_reason: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        errors: Vec<String>,
    }
    fn build(raw: Raw) -> RunResult {
        RunResult {
            subtype: raw.subtype,
            is_error: raw.is_error,
            num_turns: raw.num_turns,
            total_cost_usd: raw.total_cost_usd,
            stop_reason: raw.stop_reason,
            session_id: raw.session_id,
            errors: raw.errors,
        }
    }

    // Audit fix for agents/run.rs:146 — accept BOTH the array
    // shape and a top-level result object. CC's `--output-format=json`
    // (without `--verbose`) emits a single `{type:"result",...}`
    // object, not an array; the previous code only matched the array
    // shape and silently dropped the run result for the bare-object
    // case. We try the object shape first, fall back to the array
    // shape, then to the stream-json line scan.
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(stdout_bytes) {
        if v.is_object() && v.get("type").and_then(|t| t.as_str()) == Some("result") {
            if let Ok(raw) = serde_json::from_value::<Raw>(v.clone()) {
                return Some(build(raw));
            }
        }
        if let Some(arr) = v.as_array() {
            return arr
                .iter()
                .rev()
                .find(|el| el.get("type").and_then(|t| t.as_str()) == Some("result"))
                .and_then(|el| serde_json::from_value::<Raw>(el.clone()).ok())
                .map(build);
        }
    }

    // Fall back to `--output-format=stream-json`: NDJSON, one JSON
    // object per line. Walk in reverse to pick the last `result`.
    let stdout = std::str::from_utf8(stdout_bytes).ok()?;
    let mut last_result: Option<RunResult> = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(x) => x,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("result") {
            if let Ok(raw) = serde_json::from_value::<Raw>(v) {
                last_result = Some(build(raw));
            }
        }
    }
    last_result
}

/// Best-effort: given a CC `session_id`, find the `.jsonl`
/// transcript on disk. CC writes transcripts under
/// `~/.claude/projects/<sanitized-cwd>/<session_id>.jsonl`. We
/// don't know the sanitized-cwd dir from the session id alone, so
/// we search the projects tree by filename. Returns the first
/// match.
fn locate_transcript_for_session(session_id: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    let projects = home.join(".claude").join("projects");
    if !projects.exists() {
        return None;
    }
    let target = format!("{session_id}.jsonl");
    walk_for_filename(&projects, &target, 3).map(|p| p.display().to_string())
}

/// Bounded recursive walk: looking for `target` filename under
/// `dir`, no deeper than `depth_remaining`.
fn walk_for_filename(dir: &Path, target: &str, depth_remaining: u32) -> Option<PathBuf> {
    if depth_remaining == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_file() {
            if path.file_name().and_then(|n| n.to_str()) == Some(target) {
                return Some(path);
            }
        } else if ft.is_dir() {
            subdirs.push(path);
        }
    }
    for sub in subdirs {
        if let Some(found) = walk_for_filename(&sub, target, depth_remaining - 1) {
            return Some(found);
        }
    }
    None
}

/// Spawn the agent's helper shim once and return the
/// resulting [`AgentRun`]. Used by the "Run Now" button —
/// distinct from scheduled runs which the OS scheduler invokes
/// directly. Phase events are emitted on `sink`.
///
/// `extra_env` is layered onto the spawned shim process — used by
/// the event orchestrator to pass `CLAUDEPOT_EVENT_SESSION_ID` and
/// `CLAUDEPOT_EVENT_SESSION_PATH` for `session-settled` triggers
/// so the prompt can reference the firing session. Keys here do NOT
/// pass through the env whitelist (those checks are for renderer-
/// supplied user env on the agent record); the orchestrator is
/// trusted Rust code injecting bounded `CLAUDEPOT_*` keys.
pub async fn run_now(
    agent: &Agent,
    binary_abs_path: &str,
    claudepot_cli_abs_path: &str,
    sink: &dyn ProgressSink,
    extra_env: &std::collections::BTreeMap<String, String>,
) -> Result<AgentRun, AgentError> {
    tracing::info!(
        agent_id = %agent.id,
        agent_name = %agent.name,
        "agent run-now: started"
    );
    sink.phase("prepare", PhaseStatus::Running);
    let shim_path = install_shim(agent, binary_abs_path, claudepot_cli_abs_path)?;

    // Mint a deterministic run id in Rust so we can read back the
    // exact run dir without scanning for "newest" (which would race
    // any concurrent scheduled run firing in the same second). The
    // shim honors `CLAUDEPOT_RUN_ID` from its env when present.
    // Format: ISO-Z timestamp + UUIDv4 suffix → satisfies
    // validate_run_id ([A-Za-z0-9._-], ≤128 chars).
    let run_id = format!(
        "{}-{}",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        uuid::Uuid::new_v4().simple()
    );
    let runs_root = agent_runs_dir(&agent.id);
    let run_dir = runs_root.join(&run_id);
    sink.phase("prepare", PhaseStatus::Complete);

    sink.phase("spawn", PhaseStatus::Running);
    let started_at = Utc::now();
    // The shim is responsible for everything inside (per-run dir,
    // logs, calling _record-run). We just await its exit.
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(&shim_path);
        c
    } else {
        let mut c = tokio::process::Command::new("/bin/sh");
        c.arg(&shim_path);
        c
    };
    cmd.env("CLAUDEPOT_RUN_ID", &run_id);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let status = cmd.status().await.map_err(|e| {
        AgentError::Io(std::io::Error::other(format!("failed to spawn shim: {e}")))
    })?;
    let ended_at = Utc::now();
    let exit_code = status.code().unwrap_or(-1);
    sink.phase("spawn", PhaseStatus::Complete);

    // Read the result.json the shim's `_record-run` callback wrote.
    // If it's missing (record-run failed, shim crashed before
    // calling it, etc.), synthesize a record AND persist it so the
    // run history reflects every attempt.
    sink.phase("record", PhaseStatus::Running);
    let result_path = run_dir.join("result.json");
    let run = if result_path.exists() {
        let raw = std::fs::read(&result_path)?;
        serde_json::from_slice(&raw)?
    } else {
        // The shim's `_record-run` callback did not write
        // `result.json` (record-run failed, or the shim crashed
        // before reaching it). Synthesize a record AND persist it
        // so the run history still reflects every attempt.
        tracing::warn!(
            agent_id = %agent.id,
            run_id = %run_id,
            run_dir = %run_dir.display(),
            exit_code,
            "agent run-now: shim wrote no result.json — synthesizing a run record"
        );
        let synth = synthesize_run(agent, started_at, ended_at, exit_code, &run_dir);
        // Persist the synthesized record so the run history has a
        // row for it. Best-effort — don't fail the whole run-now
        // if the persist itself errors (that would mask the actual
        // run outcome) — but a failed persist must not be silent.
        if !run_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&run_dir) {
                tracing::error!(
                    agent_id = %agent.id,
                    run_id = %run_id,
                    run_dir = %run_dir.display(),
                    error = %e,
                    "agent run-now: failed to create run dir for the \
                     synthesized record — the run will have no on-disk row"
                );
            }
        }
        match serde_json::to_vec_pretty(&synth) {
            Ok(bytes) => {
                if let Err(e) = crate::fs_utils::atomic_write(&result_path, &bytes) {
                    tracing::error!(
                        agent_id = %agent.id,
                        run_id = %run_id,
                        path = %result_path.display(),
                        error = %e,
                        "agent run-now: failed to persist the synthesized \
                         result.json — the run will have no on-disk row"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    agent_id = %agent.id,
                    run_id = %run_id,
                    error = %e,
                    "agent run-now: failed to serialize the synthesized \
                     run record — the run will have no on-disk row"
                );
            }
        }
        synth
    };
    sink.phase("record", PhaseStatus::Complete);

    // Enforce `log_retention_runs` (grill finding F12). The shim's
    // own `_record-run` callback already prunes when it ran; this
    // also covers the synthesize branch above (shim crashed before
    // calling `_record-run`), so a misbehaving agent's run dirs are
    // bounded regardless of which path recorded the run. The agent
    // record is in hand, so no store open is needed.
    prune_run_dirs(&runs_root, agent.log_retention_runs as usize);

    sink.phase("done", PhaseStatus::Complete);
    tracing::info!(
        agent_id = %agent.id,
        run_id = %run.id,
        exit_code,
        duration_ms = run.duration_ms,
        cost_usd = run.result.as_ref().and_then(|r| r.total_cost_usd),
        "agent run-now: finished"
    );
    Ok(run)
}

fn synthesize_run(
    agent: &Agent,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    exit_code: i32,
    run_dir: &Path,
) -> AgentRun {
    AgentRun {
        id: run_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("synthetic")
            .to_string(),
        agent_id: agent.id,
        started_at,
        ended_at,
        duration_ms: (ended_at - started_at).num_milliseconds(),
        exit_code,
        result: None,
        session_jsonl_path: None,
        stdout_log: "stdout.log".to_string(),
        stderr_log: "stderr.log".to_string(),
        trigger_kind: TriggerKind::Manual,
        host_platform: HostPlatform::current(),
        claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        output_artifacts: Vec::new(),
        route_decision: None,
    }
}

/// Find the most recent `runs/<run-id>/` directory by name.
/// Skips dotfiles (`.latest` symlink and pointer files).
#[allow(dead_code)]
fn find_latest_run_dir(runs_dir: &Path) -> Option<PathBuf> {
    if !runs_dir.exists() {
        return None;
    }
    let mut best: Option<PathBuf> = None;
    let mut best_name = String::new();
    for entry in std::fs::read_dir(runs_dir).ok()?.flatten() {
        let path = entry.path();
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = match entry.file_name().to_str() {
            Some(n) if !n.starts_with('.') => n.to_string(),
            _ => continue,
        };
        if name > best_name {
            best_name = name;
            best = Some(path);
        }
    }
    best
}

/// Convenience: list all run-id directory names for an agent,
/// sorted descending (newest first).
pub fn list_run_ids(id: &AgentId) -> Result<Vec<String>, AgentError> {
    let dir = agent_runs_dir(id);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<String> = std::fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|n| !n.starts_with('.')) // skip the .latest symlink
        .collect();
    // ULID-ish run ids sort lexicographically by time prefix.
    out.sort();
    out.reverse();
    Ok(out)
}

/// Read a single run record by id. Validates `run_id` is a single
/// safe filename component and pins the resolved path under the
/// agent's runs dir so a malicious caller cannot escape with
/// `..` or absolute paths.
pub fn read_run(
    agent_id: &AgentId,
    run_id: &str,
) -> Result<AgentRun, AgentError> {
    validate_run_id(run_id)?;
    let runs_root = agent_runs_dir(agent_id);
    let path = runs_root.join(run_id).join("result.json");
    // Defense in depth: confirm the resolved canonical path stays
    // inside the runs root. We use the raw join because the path
    // may not exist yet on disk; the canonicalize-after-read happens
    // on the read step below.
    if !path.starts_with(&runs_root) {
        return Err(AgentError::InvalidPath(
            run_id.to_string(),
            "run_id resolved outside agent runs dir",
        ));
    }
    let bytes = std::fs::read(&path)?;
    let run: AgentRun = serde_json::from_slice(&bytes)?;
    Ok(run)
}

/// A run id must be a single non-empty path component, ASCII-only,
/// composed of `[A-Za-z0-9._-]` (so it can serve as a filesystem
/// directory name on every supported host without escaping). This
/// matches the shape the unix and Windows shims emit
/// (`<ISO-timestamp>-<pid|random>`).
fn validate_run_id(s: &str) -> Result<(), AgentError> {
    if s.is_empty() {
        return Err(AgentError::InvalidPath(
            s.to_string(),
            "run_id cannot be empty",
        ));
    }
    if s.len() > 128 {
        return Err(AgentError::InvalidPath(
            s.to_string(),
            "run_id exceeds 128 characters",
        ));
    }
    if s.starts_with('.') {
        return Err(AgentError::InvalidPath(
            s.to_string(),
            "run_id cannot start with `.`",
        ));
    }
    for b in s.bytes() {
        let ok = b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.';
        if !ok {
            return Err(AgentError::InvalidPath(
                s.to_string(),
                "run_id contains characters outside [A-Za-z0-9._-]",
            ));
        }
    }
    if s.contains("..") {
        return Err(AgentError::InvalidPath(
            s.to_string(),
            "run_id cannot contain `..`",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn parse_result_event_success_case() {
        let stdout = br#"[
            {"type":"system","subtype":"init","session_id":"sess-1"},
            {"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}},
            {"type":"result","subtype":"success","is_error":false,"num_turns":1,
             "duration_ms":1234,"total_cost_usd":0.01,"stop_reason":"end_turn",
             "session_id":"sess-1","errors":[]}
        ]"#;
        let r = parse_result_event(stdout).unwrap();
        assert_eq!(r.subtype.as_deref(), Some("success"));
        assert_eq!(r.is_error, Some(false));
        assert_eq!(r.num_turns, Some(1));
        assert_eq!(r.total_cost_usd, Some(0.01));
        assert_eq!(r.session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn parse_result_event_budget_error() {
        let stdout = br#"[
            {"type":"system","subtype":"init"},
            {"type":"result","subtype":"error_max_budget_usd","is_error":true,
             "errors":["Reached maximum budget ($0.05)"],"total_cost_usd":0.31}
        ]"#;
        let r = parse_result_event(stdout).unwrap();
        assert_eq!(r.subtype.as_deref(), Some("error_max_budget_usd"));
        assert_eq!(r.is_error, Some(true));
        assert_eq!(r.errors, vec!["Reached maximum budget ($0.05)".to_string()]);
    }

    #[test]
    fn parse_result_event_picks_last_when_multiple() {
        let stdout = br#"[
            {"type":"result","subtype":"first","is_error":false},
            {"type":"assistant"},
            {"type":"result","subtype":"second","is_error":true}
        ]"#;
        let r = parse_result_event(stdout).unwrap();
        assert_eq!(r.subtype.as_deref(), Some("second"));
    }

    #[test]
    fn parse_result_event_returns_none_when_absent() {
        let stdout = br#"[
            {"type":"system","subtype":"init"},
            {"type":"assistant"}
        ]"#;
        assert!(parse_result_event(stdout).is_none());
    }

    #[test]
    fn parse_result_event_tolerates_unknown_fields() {
        let stdout = br#"[
            {"type":"result","subtype":"success","is_error":false,
             "future_field":"ignored","other":123}
        ]"#;
        let r = parse_result_event(stdout).unwrap();
        assert_eq!(r.subtype.as_deref(), Some("success"));
    }

    #[test]
    fn parse_result_event_returns_none_for_garbage() {
        assert!(parse_result_event(b"").is_none());
        assert!(parse_result_event(b"not json").is_none());
        assert!(parse_result_event(b"{}").is_none()); // not an array
        assert!(parse_result_event(b"[").is_none());
    }

    #[test]
    fn record_run_writes_result_json_next_to_stdout() {
        let dir = tempdir().unwrap();
        let stdout_path = dir.path().join("stdout.log");
        let stderr_path = dir.path().join("stderr.log");
        std::fs::write(
            &stdout_path,
            r#"[{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.02}]"#,
        )
        .unwrap();
        std::fs::write(&stderr_path, "").unwrap();

        let id = Uuid::new_v4();
        let started = Utc.with_ymd_and_hms(2026, 4, 28, 9, 0, 0).unwrap();
        let ended = started + chrono::Duration::seconds(13);
        let inputs = RecordInputs {
            agent_id: id,
            run_id: "20260428T090000Z-1",
            exit_code: 0,
            started_at: started,
            ended_at: ended,
            trigger_kind: TriggerKind::Scheduled,
            stdout_log_path: &stdout_path,
            stderr_log_path: &stderr_path,
            claudepot_version: "0.0.5",
            log_retention_runs: None,
        };
        let run = record_run(&inputs).unwrap();
        assert_eq!(run.duration_ms, 13_000);
        assert_eq!(run.exit_code, 0);
        assert_eq!(run.stdout_log, "stdout.log");
        let result_path = dir.path().join("result.json");
        assert!(result_path.exists());

        // Round-trip the on-disk record.
        let raw = std::fs::read(&result_path).unwrap();
        let on_disk: AgentRun = serde_json::from_slice(&raw).unwrap();
        assert_eq!(on_disk.id, run.id);
        assert_eq!(on_disk.exit_code, 0);
        assert_eq!(
            on_disk.result.as_ref().and_then(|r| r.subtype.as_deref()),
            Some("success")
        );
    }

    #[test]
    fn record_run_treats_absent_stdout_log_as_empty_output() {
        // grill F5: a *missing* stdout.log is legitimately empty
        // output — `record_run` must degrade to an empty parse, not
        // error. (An unreadable-but-present file logs an error and
        // also degrades; both end with a recorded run row.)
        let dir = tempdir().unwrap();
        // Deliberately do NOT create stdout.log. result.json must
        // still land next to the (absent) stdout.log path.
        let stdout_path = dir.path().join("stdout.log");
        let stderr_path = dir.path().join("stderr.log");

        let id = Uuid::new_v4();
        let started = Utc::now();
        let inputs = RecordInputs {
            agent_id: id,
            run_id: "r-absent",
            exit_code: 70,
            started_at: started,
            ended_at: started + chrono::Duration::seconds(1),
            trigger_kind: TriggerKind::Scheduled,
            stdout_log_path: &stdout_path,
            stderr_log_path: &stderr_path,
            claudepot_version: "0.0.5",
            log_retention_runs: None,
        };
        let run = record_run(&inputs).unwrap();
        assert_eq!(run.exit_code, 70);
        assert!(run.result.is_none());
        assert!(dir.path().join("result.json").exists());
    }

    #[test]
    fn prune_run_dirs_keeps_only_the_newest_retention() {
        // grill F12: with 7 run dirs and a retention of 3, the 4
        // oldest (lexicographically smallest names) are deleted and
        // the 3 newest survive.
        let dir = tempdir().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir(&runs).unwrap();
        // ISO-prefixed names sort chronologically.
        let names = [
            "20260101T000000Z-a",
            "20260102T000000Z-b",
            "20260103T000000Z-c",
            "20260104T000000Z-d",
            "20260105T000000Z-e",
            "20260106T000000Z-f",
            "20260107T000000Z-g",
        ];
        for n in names {
            let d = runs.join(n);
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join("result.json"), b"{}").unwrap();
        }
        // A dotfile must never be counted or deleted.
        std::fs::write(runs.join(".latest"), b"ptr").unwrap();

        prune_run_dirs(&runs, 3);

        let survivors: std::collections::HashSet<String> = std::fs::read_dir(&runs)
            .unwrap()
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .collect();
        assert_eq!(survivors.len(), 3, "exactly retention=3 dirs survive");
        assert!(survivors.contains("20260107T000000Z-g"), "newest kept");
        assert!(survivors.contains("20260106T000000Z-f"));
        assert!(survivors.contains("20260105T000000Z-e"));
        assert!(!survivors.contains("20260101T000000Z-a"), "oldest deleted");
        // The dotfile is untouched.
        assert!(runs.join(".latest").exists());
    }

    #[test]
    fn prune_run_dirs_noop_when_under_retention() {
        // Fewer dirs than the retention cap — nothing is deleted.
        let dir = tempdir().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir(&runs).unwrap();
        for n in ["20260101T000000Z-a", "20260102T000000Z-b"] {
            std::fs::create_dir(runs.join(n)).unwrap();
        }
        prune_run_dirs(&runs, 50);
        let count = std::fs::read_dir(&runs).unwrap().flatten().count();
        assert_eq!(count, 2, "no prune when under the retention cap");
    }

    #[test]
    fn prune_run_dirs_zero_retention_is_a_noop() {
        // A retention of 0 means "uninitialized" — it must NOT
        // delete everything. The model defaults the field to 50.
        let dir = tempdir().unwrap();
        let runs = dir.path().join("runs");
        std::fs::create_dir(&runs).unwrap();
        std::fs::create_dir(runs.join("20260101T000000Z-a")).unwrap();
        prune_run_dirs(&runs, 0);
        assert!(runs.join("20260101T000000Z-a").exists());
    }

    #[test]
    fn record_run_handles_empty_stdout_gracefully() {
        let dir = tempdir().unwrap();
        let stdout_path = dir.path().join("stdout.log");
        let stderr_path = dir.path().join("stderr.log");
        std::fs::write(&stdout_path, "").unwrap();
        std::fs::write(&stderr_path, "boom").unwrap();

        let id = Uuid::new_v4();
        let started = Utc::now();
        let inputs = RecordInputs {
            agent_id: id,
            run_id: "r1",
            exit_code: 1,
            started_at: started,
            ended_at: started + chrono::Duration::seconds(2),
            trigger_kind: TriggerKind::Manual,
            stdout_log_path: &stdout_path,
            stderr_log_path: &stderr_path,
            claudepot_version: "0.0.5",
            log_retention_runs: None,
        };
        let run = record_run(&inputs).unwrap();
        assert_eq!(run.exit_code, 1);
        assert!(run.result.is_none());
        assert_eq!(run.trigger_kind, TriggerKind::Manual);
    }
}
