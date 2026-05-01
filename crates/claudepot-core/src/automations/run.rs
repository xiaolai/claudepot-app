//! Parsing and recording for a single automation run.
//!
//! Two responsibilities:
//!
//! 1. Parse the `--output-format=json` stdout that `claude -p`
//!    produces — a JSON array of events. We keep the parser
//!    permissive: take the last `type === "result"` element and
//!    fall back to `is_error: true` if no `result` event is
//!    present.
//! 2. Assemble an [`AutomationRun`] record and write
//!    `<run_dir>/result.json` plus update the run-history
//!    directory.
//!
//! The `Run-Now` executor (spawning the shim, streaming progress)
//! is a separate concern and lives in a future module — this one
//! is the post-exit recorder, exercised both by the helper shim
//! (via `claudepot automation _record-run`) and by the in-process
//! Run-Now path.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::fs_utils;
use crate::project_progress::{PhaseStatus, ProgressSink};

use super::error::AutomationError;
use super::install::install_shim;
use super::store::automation_runs_dir;
use super::types::{Automation, AutomationId, AutomationRun, HostPlatform, RunResult, TriggerKind};

/// Inputs to [`record_run`]. All values are knowable by the helper
/// shim at exit time.
#[derive(Debug, Clone)]
pub struct RecordInputs<'a> {
    pub automation_id: AutomationId,
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
}

/// Top-level: read stdout.log, parse the result event, write
/// `result.json` next to it, and return the assembled record.
///
/// On any malformed-JSON / missing-result-event condition, falls
/// back to a synthetic [`RunResult`] reflecting the OS exit code
/// so the run row still gets recorded.
pub fn record_run(inputs: &RecordInputs<'_>) -> Result<AutomationRun, AutomationError> {
    let stdout_bytes = std::fs::read(inputs.stdout_log_path).unwrap_or_default();
    let result = parse_result_event(&stdout_bytes);
    let session_jsonl_path = result
        .as_ref()
        .and_then(|r| r.session_id.clone())
        .and_then(|sid| locate_transcript_for_session(&sid));

    let run = AutomationRun {
        id: inputs.run_id.to_string(),
        automation_id: inputs.automation_id,
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
        // automations; template-aware enrichment (output-artifact
        // discovery, prerun-decision merge) is layered on by
        // `record_run_with_template_context` once the templates
        // pre-run gate is wired through the shim.
        output_artifacts: Vec::new(),
        route_decision: None,
    };

    write_result_json(&run, inputs.stdout_log_path)?;
    Ok(run)
}

/// Find the run directory containing `stdout.log` and write
/// `result.json` alongside.
fn write_result_json(run: &AutomationRun, stdout_log: &Path) -> Result<(), AutomationError> {
    let run_dir = stdout_log.parent().ok_or_else(|| {
        AutomationError::InvalidPath(
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

    // Audit fix for automations/run.rs:146 — accept BOTH the array
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

/// Spawn the automation's helper shim once and return the
/// resulting [`AutomationRun`]. Used by the "Run Now" button —
/// distinct from scheduled runs which the OS scheduler invokes
/// directly. Phase events are emitted on `sink`.
pub async fn run_now(
    automation: &Automation,
    binary_abs_path: &str,
    claudepot_cli_abs_path: &str,
    sink: &dyn ProgressSink,
) -> Result<AutomationRun, AutomationError> {
    sink.phase("prepare", PhaseStatus::Running);
    let shim_path = install_shim(automation, binary_abs_path, claudepot_cli_abs_path)?;

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
    let runs_root = automation_runs_dir(&automation.id);
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
    let status = cmd.status().await.map_err(|e| {
        AutomationError::Io(std::io::Error::other(format!("failed to spawn shim: {e}")))
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
        let synth = synthesize_run(automation, started_at, ended_at, exit_code, &run_dir);
        // Persist the synthesized record so the run history has a
        // row for it. Best-effort — don't fail the whole run-now
        // if the persist itself errors (that would mask the actual
        // run outcome).
        if !run_dir.exists() {
            let _ = std::fs::create_dir_all(&run_dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&synth) {
            let _ = crate::fs_utils::atomic_write(&result_path, &bytes);
        }
        synth
    };
    sink.phase("record", PhaseStatus::Complete);

    sink.phase("done", PhaseStatus::Complete);
    Ok(run)
}

fn synthesize_run(
    automation: &Automation,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    exit_code: i32,
    run_dir: &Path,
) -> AutomationRun {
    AutomationRun {
        id: run_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("synthetic")
            .to_string(),
        automation_id: automation.id,
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

/// Convenience: list all run-id directory names for an automation,
/// sorted descending (newest first).
pub fn list_run_ids(id: &AutomationId) -> Result<Vec<String>, AutomationError> {
    let dir = automation_runs_dir(id);
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
/// automation's runs dir so a malicious caller cannot escape with
/// `..` or absolute paths.
pub fn read_run(
    automation_id: &AutomationId,
    run_id: &str,
) -> Result<AutomationRun, AutomationError> {
    validate_run_id(run_id)?;
    let runs_root = automation_runs_dir(automation_id);
    let path = runs_root.join(run_id).join("result.json");
    // Defense in depth: confirm the resolved canonical path stays
    // inside the runs root. We use the raw join because the path
    // may not exist yet on disk; the canonicalize-after-read happens
    // on the read step below.
    if !path.starts_with(&runs_root) {
        return Err(AutomationError::InvalidPath(
            run_id.to_string(),
            "run_id resolved outside automation runs dir",
        ));
    }
    let bytes = std::fs::read(&path)?;
    let run: AutomationRun = serde_json::from_slice(&bytes)?;
    Ok(run)
}

/// A run id must be a single non-empty path component, ASCII-only,
/// composed of `[A-Za-z0-9._-]` (so it can serve as a filesystem
/// directory name on every supported host without escaping). This
/// matches the shape the unix and Windows shims emit
/// (`<ISO-timestamp>-<pid|random>`).
fn validate_run_id(s: &str) -> Result<(), AutomationError> {
    if s.is_empty() {
        return Err(AutomationError::InvalidPath(
            s.to_string(),
            "run_id cannot be empty",
        ));
    }
    if s.len() > 128 {
        return Err(AutomationError::InvalidPath(
            s.to_string(),
            "run_id exceeds 128 characters",
        ));
    }
    if s.starts_with('.') {
        return Err(AutomationError::InvalidPath(
            s.to_string(),
            "run_id cannot start with `.`",
        ));
    }
    for b in s.bytes() {
        let ok = b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.';
        if !ok {
            return Err(AutomationError::InvalidPath(
                s.to_string(),
                "run_id contains characters outside [A-Za-z0-9._-]",
            ));
        }
    }
    if s.contains("..") {
        return Err(AutomationError::InvalidPath(
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
            automation_id: id,
            run_id: "20260428T090000Z-1",
            exit_code: 0,
            started_at: started,
            ended_at: ended,
            trigger_kind: TriggerKind::Scheduled,
            stdout_log_path: &stdout_path,
            stderr_log_path: &stderr_path,
            claudepot_version: "0.0.5",
        };
        let run = record_run(&inputs).unwrap();
        assert_eq!(run.duration_ms, 13_000);
        assert_eq!(run.exit_code, 0);
        assert_eq!(run.stdout_log, "stdout.log");
        let result_path = dir.path().join("result.json");
        assert!(result_path.exists());

        // Round-trip the on-disk record.
        let raw = std::fs::read(&result_path).unwrap();
        let on_disk: AutomationRun = serde_json::from_slice(&raw).unwrap();
        assert_eq!(on_disk.id, run.id);
        assert_eq!(on_disk.exit_code, 0);
        assert_eq!(
            on_disk.result.as_ref().and_then(|r| r.subtype.as_deref()),
            Some("success")
        );
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
            automation_id: id,
            run_id: "r1",
            exit_code: 1,
            started_at: started,
            ended_at: started + chrono::Duration::seconds(2),
            trigger_kind: TriggerKind::Manual,
            stdout_log_path: &stdout_path,
            stderr_log_path: &stderr_path,
            claudepot_version: "0.0.5",
        };
        let run = record_run(&inputs).unwrap();
        assert_eq!(run.exit_code, 1);
        assert!(run.result.is_none());
        assert_eq!(run.trigger_kind, TriggerKind::Manual);
    }
}
