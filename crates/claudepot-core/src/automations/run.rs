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

use super::error::AutomationError;
use super::store::automation_runs_dir;
use super::types::{AutomationId, AutomationRun, HostPlatform, RunResult, TriggerKind};

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
    };

    write_result_json(&run, inputs.stdout_log_path)?;
    Ok(run)
}

/// Find the run directory containing `stdout.log` and write
/// `result.json` alongside.
fn write_result_json(run: &AutomationRun, stdout_log: &Path) -> Result<(), AutomationError> {
    let run_dir = stdout_log
        .parent()
        .ok_or_else(|| AutomationError::InvalidPath(
            stdout_log.display().to_string(),
            "stdout log has no parent dir",
        ))?;
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

/// Parse the last `result` event from a `claude -p
/// --output-format=json` stdout dump. Returns `None` if no parsable
/// result event is present (caller should fall back to exit code).
pub fn parse_result_event(stdout_bytes: &[u8]) -> Option<RunResult> {
    if stdout_bytes.is_empty() {
        return None;
    }
    // The output format is a JSON array; tolerate trailing whitespace.
    let v: serde_json::Value = serde_json::from_slice(stdout_bytes).ok()?;
    let arr = v.as_array()?;
    // Walk in reverse — pick the last element with type=="result".
    arr.iter()
        .rev()
        .find(|el| el.get("type").and_then(|t| t.as_str()) == Some("result"))
        .and_then(|result_el| {
            // Use a helper struct so unknown fields are tolerated
            // (serde-untagged default behavior).
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
            let raw: Raw = serde_json::from_value(result_el.clone()).ok()?;
            Some(RunResult {
                subtype: raw.subtype,
                is_error: raw.is_error,
                num_turns: raw.num_turns,
                total_cost_usd: raw.total_cost_usd,
                stop_reason: raw.stop_reason,
                session_id: raw.session_id,
                errors: raw.errors,
            })
        })
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

/// Read a single run record by id.
pub fn read_run(automation_id: &AutomationId, run_id: &str) -> Result<AutomationRun, AutomationError> {
    let path = automation_runs_dir(automation_id)
        .join(run_id)
        .join("result.json");
    let bytes = std::fs::read(&path)?;
    let run: AutomationRun = serde_json::from_slice(&bytes)?;
    Ok(run)
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
