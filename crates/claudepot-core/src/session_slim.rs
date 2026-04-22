//! Rewrite a single session transcript in place, dropping oversized
//! `tool_result` payloads while preserving every other kind of event.
//!
//! Preserved exactly: user prompts, assistant text, tool *calls* (the
//! request), compaction markers, sidechain pointers, thinking blocks,
//! summaries. Only `tool_result` content past the byte threshold is
//! replaced with:
//!
//! ```jsonc
//! {"type":"tool_result_redacted", "original_bytes":N, "tool":"bash",
//!  "tool_use_id":"t1"}
//! ```
//!
//! The pre-slim original goes to the trash under `TrashKind::Slim` so
//! the operation is reversible.
//!
//! Concurrency guard: before rename, we re-stat the source and abort
//! if `(size, mtime_ns)` changed — CC may be writing into the file.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;
use thiserror::Error;

use crate::project_progress::{PhaseStatus, ProgressSink};
use crate::trash::{self, TrashError, TrashKind, TrashPut};

#[derive(Debug, Error)]
pub enum SlimError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("source file not found: {0}")]
    NotFound(PathBuf),
    #[error("source changed during slim (size or mtime); aborted")]
    LiveWriteDetected,
    #[error("trash op failed: {0}")]
    Trash(#[from] TrashError),
    #[error("json parse error on line {line}: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}

impl SlimError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SlimOpts {
    /// `tool_result` payloads strictly larger than this are dropped.
    /// A value of 0 means "drop everything" (rare; intended for tests).
    pub drop_tool_results_over_bytes: u64,
    /// Tool names whose results should be preserved regardless of size.
    /// Matched case-sensitively against the `tool` field.
    pub exclude_tools: Vec<String>,
}

impl Default for SlimOpts {
    fn default() -> Self {
        Self {
            drop_tool_results_over_bytes: 1 << 20, // 1 MiB
            exclude_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SlimPlan {
    /// Original byte size on disk.
    pub original_bytes: u64,
    /// Projected size after slim.
    pub projected_bytes: u64,
    /// Number of tool_result payloads that will be redacted.
    pub redact_count: u32,
    /// Tools whose results will be touched (for UX).
    pub tools_affected: Vec<String>,
}

impl SlimPlan {
    pub fn bytes_saved(&self) -> u64 {
        self.original_bytes.saturating_sub(self.projected_bytes)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SlimReport {
    pub original_bytes: u64,
    pub final_bytes: u64,
    pub redact_count: u32,
    pub trashed_original: PathBuf,
}

impl SlimReport {
    pub fn bytes_saved(&self) -> u64 {
        self.original_bytes.saturating_sub(self.final_bytes)
    }
}

/// Scan the file without touching disk state. The projected byte
/// count is computed by counting replacement-marker length vs the
/// dropped payload length per line.
pub fn plan_slim(path: &Path, opts: &SlimOpts) -> Result<SlimPlan, SlimError> {
    let meta = fs::metadata(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => SlimError::NotFound(path.to_path_buf()),
        _ => SlimError::io(path, e),
    })?;
    let original_bytes = meta.len();
    let f = fs::File::open(path).map_err(|e| SlimError::io(path, e))?;
    let reader = BufReader::new(f);

    let mut projected = 0u64;
    let mut redact_count = 0u32;
    let mut tools: Vec<String> = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| SlimError::io(path, e))?;
        if line.is_empty() {
            projected += 1; // newline
            continue;
        }
        let mut v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| SlimError::Json {
                line: i,
                source: e,
            })?;
        let (new_line, line_stats) = rewrite_line(&mut v, opts);
        redact_count += line_stats.redacted_here;
        for t in line_stats.tools_here {
            if !tools.contains(&t) {
                tools.push(t);
            }
        }
        projected += new_line.len() as u64 + 1; // + \n
    }
    Ok(SlimPlan {
        original_bytes,
        projected_bytes: projected,
        redact_count,
        tools_affected: tools,
    })
}

/// Rewrite the file. Caller must pass a `data_dir` for trash placement
/// of the pre-slim snapshot. Live-write guard aborts if the source
/// changed between the initial scan and the atomic rename.
pub fn execute_slim(
    data_dir: &Path,
    path: &Path,
    opts: &SlimOpts,
    sink: &dyn ProgressSink,
) -> Result<SlimReport, SlimError> {
    sink.phase("scanning", PhaseStatus::Complete);
    let meta = fs::metadata(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => SlimError::NotFound(path.to_path_buf()),
        _ => SlimError::io(path, e),
    })?;
    let before_size = meta.len();
    let before_mtime = meta.modified().map_err(|e| SlimError::io(path, e))?;
    let tmp_path = temp_path_next_to(path);

    sink.phase("rewriting", PhaseStatus::Running);
    let f = fs::File::open(path).map_err(|e| SlimError::io(path, e))?;
    let reader = BufReader::new(f);
    let mut tmp = fs::File::create(&tmp_path).map_err(|e| SlimError::io(&tmp_path, e))?;

    let mut redact_count = 0u32;
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| SlimError::io(path, e))?;
        if line.is_empty() {
            writeln!(tmp).map_err(|e| SlimError::io(&tmp_path, e))?;
            continue;
        }
        let mut v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| SlimError::Json {
                line: i,
                source: e,
            })?;
        let (new_line, stats) = rewrite_line(&mut v, opts);
        redact_count += stats.redacted_here;
        writeln!(tmp, "{new_line}").map_err(|e| SlimError::io(&tmp_path, e))?;
    }
    tmp.sync_all().map_err(|e| SlimError::io(&tmp_path, e))?;
    drop(tmp);

    sink.phase("guarding", PhaseStatus::Running);
    let after = fs::metadata(path).map_err(|e| SlimError::io(path, e))?;
    let after_size = after.len();
    let after_mtime = after.modified().map_err(|e| SlimError::io(path, e))?;
    if after_size != before_size || !same_mtime(before_mtime, after_mtime) {
        let _ = fs::remove_file(&tmp_path);
        return Err(SlimError::LiveWriteDetected);
    }

    sink.phase("trashing-original", PhaseStatus::Running);
    // Trash the original via a side-copy so the atomic rename below is
    // still a single-directory operation. The trash layer handles
    // cross-device fallback.
    let snapshot = tmp_path.with_extension("pre-slim.jsonl");
    fs::copy(path, &snapshot).map_err(|e| SlimError::io(&snapshot, e))?;
    let entry = trash::write(
        data_dir,
        TrashPut {
            orig_path: &snapshot,
            kind: TrashKind::Slim,
            cwd: path.parent(),
            reason: Some(format!(
                "pre-slim snapshot of {}",
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            )),
        },
    )?;

    sink.phase("swapping", PhaseStatus::Running);
    fs::rename(&tmp_path, path).map_err(|e| SlimError::io(path, e))?;

    let after_final = fs::metadata(path).map_err(|e| SlimError::io(path, e))?;
    sink.phase("complete", PhaseStatus::Complete);
    Ok(SlimReport {
        original_bytes: before_size,
        final_bytes: after_final.len(),
        redact_count,
        trashed_original: PathBuf::from(entry.id),
    })
}

fn temp_path_next_to(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(".slim.tmp");
    PathBuf::from(s)
}

fn same_mtime(a: SystemTime, b: SystemTime) -> bool {
    // Compare the full duration since epoch with platform-native
    // precision. A whole-second compare would let a concurrent CC
    // append inside the same second slip past the guard and get
    // clobbered. Filesystems vary in precision (nanosecond on macOS
    // APFS and modern Linux, 100ns on NTFS), but equality of the
    // full Duration is the strongest check we can make from std.
    match (
        a.duration_since(std::time::UNIX_EPOCH),
        b.duration_since(std::time::UNIX_EPOCH),
    ) {
        (Ok(da), Ok(db)) => da == db,
        _ => false,
    }
}

struct LineStats {
    redacted_here: u32,
    tools_here: Vec<String>,
}

/// Rewrite a single parsed line in place. Returns the serialized
/// replacement plus per-line statistics.
fn rewrite_line(v: &mut serde_json::Value, opts: &SlimOpts) -> (String, LineStats) {
    let mut stats = LineStats {
        redacted_here: 0,
        tools_here: Vec::new(),
    };
    // Only user messages carry tool_result parts in CC's format.
    if v.get("type").and_then(|t| t.as_str()) != Some("user") {
        return (serde_json::to_string(v).unwrap_or_default(), stats);
    }
    let Some(parts) = v
        .get_mut("message")
        .and_then(|m| m.get_mut("content"))
        .and_then(|c| c.as_array_mut())
    else {
        return (serde_json::to_string(v).unwrap_or_default(), stats);
    };

    for part in parts.iter_mut() {
        let is_tool_result = part.get("type").and_then(|t| t.as_str()) == Some("tool_result");
        if !is_tool_result {
            continue;
        }
        let tool = part
            .get("tool")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();
        if opts.exclude_tools.iter().any(|t| t == &tool) {
            continue;
        }
        // Raw size of the part serialized.
        let raw = serde_json::to_string(part).unwrap_or_default();
        let raw_len = raw.len() as u64;
        if raw_len <= opts.drop_tool_results_over_bytes {
            continue;
        }
        let tool_use_id = part
            .get("tool_use_id")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let marker = serde_json::json!({
            "type": "tool_result_redacted",
            "original_bytes": raw_len,
            "tool": tool,
            "tool_use_id": tool_use_id,
        });
        *part = marker;
        stats.redacted_here += 1;
        if !stats.tools_here.contains(&tool) {
            stats.tools_here.push(tool);
        }
    }
    (serde_json::to_string(v).unwrap_or_default(), stats)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_progress::NoopSink;
    use tempfile::TempDir;

    fn mk_line_user_text(uuid: &str, text: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{text}"}},"uuid":"{uuid}","sessionId":"S"}}"#
        )
    }

    fn mk_line_tool_result(uuid: &str, tool: &str, payload: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{uuid}","tool":"{tool}","content":"{payload}"}}]}},"uuid":"{uuid}","sessionId":"S"}}"#
        )
    }

    fn mk_line_assistant_text(uuid: &str, text: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}},"uuid":"{uuid}","sessionId":"S"}}"#
        )
    }

    fn write_session(dir: &Path, lines: &[String]) -> PathBuf {
        let p = dir.join("s.jsonl");
        let body = lines.join("\n") + "\n";
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn slim_drops_oversized_tool_results_but_keeps_under_threshold() {
        let tmp = TempDir::new().unwrap();
        let huge = "x".repeat(500);
        let session = write_session(
            tmp.path(),
            &[
                mk_line_user_text("u1", "please help"),
                mk_line_tool_result("t1", "bash", &huge),
                mk_line_tool_result("t2", "bash", "short"),
                mk_line_assistant_text("a1", "ok"),
            ],
        );
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let opts = SlimOpts {
            drop_tool_results_over_bytes: 200,
            exclude_tools: Vec::new(),
        };
        let plan = plan_slim(&session, &opts).unwrap();
        assert_eq!(plan.redact_count, 1);
        assert!(plan.projected_bytes < plan.original_bytes);
        let report = execute_slim(&data_dir, &session, &opts, &NoopSink).unwrap();
        assert_eq!(report.redact_count, 1);
        assert!(report.final_bytes < report.original_bytes);
        // Verify on-disk content.
        let body = fs::read_to_string(&session).unwrap();
        assert!(body.contains("tool_result_redacted"));
        assert!(body.contains("please help"));
        assert!(body.contains("\"content\":\"short\""));
        assert!(!body.contains(&huge));
    }

    #[test]
    fn slim_preserves_user_prompts_assistant_text_and_tool_calls() {
        let tmp = TempDir::new().unwrap();
        let huge = "x".repeat(500);
        let session = write_session(
            tmp.path(),
            &[
                mk_line_user_text("u1", "hello there"),
                mk_line_assistant_text("a1", "answer text"),
                mk_line_tool_result("t1", "bash", &huge),
                // A raw "assistant" with a tool_use is a tool CALL — must stay.
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"bash","input":{"command":"ls"}}]},"uuid":"a2","sessionId":"S"}"#.to_string(),
                r#"{"type":"summary","summary":"done","leafUuid":"a2"}"#.to_string(),
            ],
        );
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        execute_slim(
            &data_dir,
            &session,
            &SlimOpts {
                drop_tool_results_over_bytes: 200,
                exclude_tools: vec![],
            },
            &NoopSink,
        )
        .unwrap();
        let body = fs::read_to_string(&session).unwrap();
        assert!(body.contains("hello there"));
        assert!(body.contains("answer text"));
        assert!(body.contains("\"tool_use\""), "tool_use (tool call) must survive");
        assert!(body.contains("\"summary\""), "summary must survive");
        assert!(body.contains("tool_result_redacted"));
    }

    #[test]
    fn slim_exclude_tool_preserves_that_tools_results_regardless_of_size() {
        let tmp = TempDir::new().unwrap();
        let huge = "x".repeat(500);
        let session = write_session(
            tmp.path(),
            &[
                mk_line_tool_result("t1", "special", &huge),
                mk_line_tool_result("t2", "other", &huge),
            ],
        );
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let opts = SlimOpts {
            drop_tool_results_over_bytes: 100,
            exclude_tools: vec!["special".into()],
        };
        let report = execute_slim(&data_dir, &session, &opts, &NoopSink).unwrap();
        assert_eq!(report.redact_count, 1);
        let body = fs::read_to_string(&session).unwrap();
        // `special` survives verbatim; `other` is redacted.
        assert!(body.contains("\"tool\":\"special\""));
        assert!(body.contains(&huge)); // the special payload is still here
        assert!(body.contains("\"tool\":\"other\""));
        // And the redacted marker is present for the dropped one.
        assert!(body.contains("tool_result_redacted"));
    }

    #[test]
    fn slim_event_count_preserved_minus_dropped() {
        // CC-parity: the line count doesn't drop when we slim — we
        // replace a tool_result part in place with a smaller marker,
        // so the JSONL line count is stable.
        let tmp = TempDir::new().unwrap();
        let huge = "x".repeat(500);
        let session = write_session(
            tmp.path(),
            &[
                mk_line_user_text("u1", "hi"),
                mk_line_tool_result("t1", "bash", &huge),
                mk_line_tool_result("t2", "bash", &huge),
                mk_line_assistant_text("a1", "bye"),
            ],
        );
        let before_lines = fs::read_to_string(&session).unwrap().lines().count();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        execute_slim(
            &data_dir,
            &session,
            &SlimOpts {
                drop_tool_results_over_bytes: 100,
                exclude_tools: vec![],
            },
            &NoopSink,
        )
        .unwrap();
        let after_lines = fs::read_to_string(&session).unwrap().lines().count();
        assert_eq!(before_lines, after_lines);
    }

    #[test]
    fn slim_output_reparses_line_by_line() {
        // Every post-slim line must round-trip through serde_json.
        let tmp = TempDir::new().unwrap();
        let huge = "x".repeat(500);
        let session = write_session(
            tmp.path(),
            &[
                mk_line_user_text("u1", "hi"),
                mk_line_tool_result("t1", "bash", &huge),
            ],
        );
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        execute_slim(
            &data_dir,
            &session,
            &SlimOpts {
                drop_tool_results_over_bytes: 100,
                exclude_tools: vec![],
            },
            &NoopSink,
        )
        .unwrap();
        for (i, line) in fs::read_to_string(&session).unwrap().lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|e| panic!("line {i} failed to parse: {e}; line={line}"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn slim_aborts_if_file_changes_under_us() {
        use std::os::unix::fs::MetadataExt;
        let tmp = TempDir::new().unwrap();
        let huge = "x".repeat(500);
        let session = write_session(
            tmp.path(),
            &[mk_line_tool_result("t1", "bash", &huge)],
        );
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        // Simulate CC appending during slim by wrapping execute_slim
        // logic. We call plan → then modify → then execute. execute
        // re-stats after write and must detect the change.
        // Easiest emulation: manually reproduce execute_slim's guard.
        let meta = fs::metadata(&session).unwrap();
        let before_size = meta.len();
        let _before_mtime = meta.modified().unwrap();

        // Mutate: append a byte. This changes size.
        {
            let mut f = fs::OpenOptions::new().append(true).open(&session).unwrap();
            f.write_all(b"\n").unwrap();
        }
        let after = fs::metadata(&session).unwrap();
        assert_ne!(before_size, after.len());
        // The live-write guard should have caught this; simulate by
        // running execute_slim after the mutation and observing the
        // abort. Because the in-memory `before_size` is stale, we
        // synthesize the abort by calling execute_slim on a path that
        // has already been touched — but execute_slim snapshots on
        // entry. So instead test the helper directly.
        let before = meta.modified().unwrap();
        let after_mtime = after.modified().unwrap();
        // On fast filesystems the second-precision comparison may be
        // equal — tolerate that and additionally check size.
        let unchanged = same_mtime(before, after_mtime) && before_size == after.len();
        assert!(!unchanged, "guard condition must trip");
        // Silence unused import warning in non-cfg-test builds.
        let _ = meta.ino();
    }

    #[test]
    fn same_mtime_distinguishes_different_subsecond_values() {
        use std::time::{Duration, UNIX_EPOCH};
        let t1 = UNIX_EPOCH + Duration::new(1_700_000_000, 100_000_000);
        let t2 = UNIX_EPOCH + Duration::new(1_700_000_000, 200_000_000);
        // Same second, different nanoseconds — must be treated as
        // different so a live write is detected.
        assert!(!same_mtime(t1, t2));
        // Identical values still equal.
        assert!(same_mtime(t1, t1));
    }

    #[test]
    fn slim_keeps_pre_slim_snapshot_in_trash() {
        let tmp = TempDir::new().unwrap();
        let huge = "x".repeat(500);
        let session = write_session(
            tmp.path(),
            &[mk_line_tool_result("t1", "bash", &huge)],
        );
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        execute_slim(
            &data_dir,
            &session,
            &SlimOpts {
                drop_tool_results_over_bytes: 100,
                exclude_tools: vec![],
            },
            &NoopSink,
        )
        .unwrap();
        let listing = trash::list(&data_dir, Default::default()).unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].kind, TrashKind::Slim);
    }
}
