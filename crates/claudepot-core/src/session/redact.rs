//! Remove content from an indexed session transcript.
//!
//! # Why this exists
//!
//! Agents put things in transcripts that should not stay there. A
//! credential pasted into a prompt, a customer record returned by a
//! tool, a private key echoed by a shell command — Claude Code
//! persists *every* turn, and Claudepot then indexes it into
//! `sessions.db` and its FTS table. Until now there was no supported
//! way to get it back out. The content was there forever, greppable by
//! any agent with the memory server attached.
//!
//! This is the way out. It is deliberately a *movement* primitive, not
//! a text editor: it finds byte-exact strings and replaces them, and
//! it never shows you what it found.
//!
//! # The backup tension
//!
//! [`execute_redact`] trashes the pre-redaction original by default, so
//! a mistake is recoverable via `claudepot session trash restore`. But
//! **the trashed copy still contains the thing you just redacted.** For
//! a leaked *secret*, that is not good enough — a backup you forgot
//! about is the same as no redaction at all.
//!
//! So [`RedactReport::backup`] is `Option`, and the caller is expected
//! to say so out loud. Pass [`RedactOpts::purge`] to skip the backup
//! and make the removal real. The default is safety; the flag is the
//! honest escape hatch, and the CLI prints the trade-off either way.
//!
//! # What it never does
//!
//! It never echoes a match. A redaction tool that prints the secret it
//! removed — into a terminal, a log, a CI transcript — has done
//! nothing. [`RedactPlan`] and [`RedactReport`] carry **counts only**.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::project_progress::{PhaseStatus, ProgressSink};
use crate::redaction::{apply as redact_apply, RedactionPolicy};
use crate::session::slim::{same_mtime, temp_path_next_to, FileGuard};
use crate::trash::{self, TrashError, TrashKind, TrashPut};

/// The default stand-in written where matched content used to be.
pub const REDACTION_MARKER: &str = "[REDACTED by claudepot]";

#[derive(Debug, Error)]
pub enum RedactError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("source file not found: {0}")]
    NotFound(PathBuf),
    #[error("source changed during redact (size or mtime); aborted")]
    LiveWriteDetected,
    #[error("trash op failed: {0}")]
    Trash(#[from] TrashError),
    #[error("json parse error on line {line}: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    /// Refusing to rewrite the whole transcript to nothing.
    #[error("no patterns given — pass at least one --pattern, or --secrets")]
    NoPatterns,
}

impl RedactError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RedactOpts {
    /// Byte-exact strings to remove. Literal, **not** regex: a regex
    /// here would be a footgun (catastrophic backtracking on a
    /// multi-MB transcript) and a false-confidence generator. If you
    /// can't name the string, you don't know what you're removing.
    pub patterns: Vec<String>,
    /// Also run the built-in secret redactor (`sk-ant-…` keys, emails,
    /// `FOO=bar` env assignments) over every string.
    pub secrets: bool,
    /// Replace the **entire** string value that contains a match,
    /// rather than just the matching substring.
    ///
    /// Default (`false`) is surgical. Set this when the match is
    /// evidence that the whole value is tainted — e.g. a tool result
    /// that dumped a table of private records, where excising the one
    /// string you happened to grep for would leave the other 200 rows
    /// sitting there.
    pub whole_value: bool,
    /// Skip the trash backup. The removal becomes irreversible — and
    /// therefore actually a removal. Correct for leaked secrets.
    pub purge: bool,
}

impl RedactOpts {
    fn is_empty(&self) -> bool {
        self.patterns.is_empty() && !self.secrets
    }
}

/// What a redact *would* do. Counts only — never the matched text.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RedactPlan {
    pub original_bytes: u64,
    /// JSONL lines carrying at least one match.
    pub matched_lines: u32,
    /// Individual string values that would be rewritten.
    pub matched_values: u32,
    /// Per-pattern hit counts, in the order the patterns were given.
    /// The pattern itself is echoed back (the caller supplied it); the
    /// surrounding content never is.
    pub hits: Vec<PatternHit>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PatternHit {
    pub pattern: String,
    pub count: u32,
}

impl RedactPlan {
    pub fn is_noop(&self) -> bool {
        self.matched_values == 0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactReport {
    pub original_bytes: u64,
    pub final_bytes: u64,
    pub matched_lines: u32,
    pub matched_values: u32,
    /// Trash batch id of the pre-redaction original — **it still
    /// contains the redacted content**. Restore with
    /// `claudepot session trash restore <id>`; destroy with
    /// `claudepot session trash empty`. `None` when `purge` was set.
    pub backup_trash_id: Option<String>,
}

/// Scan without touching disk state.
pub fn plan_redact(path: &Path, opts: &RedactOpts) -> Result<RedactPlan, RedactError> {
    if opts.is_empty() {
        return Err(RedactError::NoPatterns);
    }
    let meta = fs::metadata(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => RedactError::NotFound(path.to_path_buf()),
        _ => RedactError::io(path, e),
    })?;

    let f = fs::File::open(path).map_err(|e| RedactError::io(path, e))?;
    let mut counts = vec![0u32; opts.patterns.len()];
    let mut matched_lines = 0u32;
    let mut matched_values = 0u32;

    for (i, line) in BufReader::new(f).lines().enumerate() {
        let line = line.map_err(|e| RedactError::io(path, e))?;
        if line.is_empty() {
            continue;
        }
        let mut v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| RedactError::Json { line: i, source: e })?;
        let stats = rewrite_value(&mut v, opts, &mut counts);
        if stats > 0 {
            matched_lines += 1;
            matched_values += stats;
        }
    }

    Ok(RedactPlan {
        original_bytes: meta.len(),
        matched_lines,
        matched_values,
        hits: opts
            .patterns
            .iter()
            .zip(counts)
            .map(|(p, count)| PatternHit {
                pattern: p.clone(),
                count,
            })
            .collect(),
    })
}

/// Rewrite the transcript, removing every match.
///
/// Same concurrency discipline as `slim`: the source is re-stat'd
/// before the atomic rename and the op aborts if `(size, mtime)`
/// moved, because Claude Code may be appending to a live session. The
/// two share one guard implementation on purpose — a second, divergent
/// copy is how a TOCTOU hole gets introduced.
pub fn execute_redact(
    data_dir: &Path,
    path: &Path,
    opts: &RedactOpts,
    sink: &dyn ProgressSink,
) -> Result<RedactReport, RedactError> {
    if opts.is_empty() {
        return Err(RedactError::NoPatterns);
    }
    sink.phase("scanning", PhaseStatus::Complete);
    let meta = fs::metadata(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => RedactError::NotFound(path.to_path_buf()),
        _ => RedactError::io(path, e),
    })?;
    let before_size = meta.len();
    let before_mtime = meta.modified().map_err(|e| RedactError::io(path, e))?;

    let tmp_path = temp_path_next_to(path);
    let mut tmp_guard = FileGuard::new(tmp_path.clone());

    sink.phase("rewriting", PhaseStatus::Running);
    let f = fs::File::open(path).map_err(|e| RedactError::io(path, e))?;
    let reader = BufReader::new(f);
    let mut tmp = fs::File::create(&tmp_path).map_err(|e| RedactError::io(&tmp_path, e))?;

    let mut counts = vec![0u32; opts.patterns.len()];
    let mut matched_lines = 0u32;
    let mut matched_values = 0u32;

    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| RedactError::io(path, e))?;
        if line.is_empty() {
            writeln!(tmp).map_err(|e| RedactError::io(&tmp_path, e))?;
            continue;
        }
        // Fast path: a line with no match is written back byte-for-byte
        // rather than reserialized, so a redact of 2 lines in a 5,000-
        // line transcript leaves 4,998 lines bit-identical. Reserializing
        // everything would churn key order and formatting across the
        // whole file for no reason.
        if !line_may_match(&line, opts) {
            writeln!(tmp, "{line}").map_err(|e| RedactError::io(&tmp_path, e))?;
            continue;
        }
        let mut v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| RedactError::Json { line: i, source: e })?;
        let hits = rewrite_value(&mut v, opts, &mut counts);
        if hits > 0 {
            matched_lines += 1;
            matched_values += hits;
        }
        let out =
            serde_json::to_string(&v).map_err(|e| RedactError::Json { line: i, source: e })?;
        writeln!(tmp, "{out}").map_err(|e| RedactError::io(&tmp_path, e))?;
    }
    tmp.sync_all().map_err(|e| RedactError::io(&tmp_path, e))?;
    drop(tmp);

    sink.phase("guarding", PhaseStatus::Running);
    let after = fs::metadata(path).map_err(|e| RedactError::io(path, e))?;
    if after.len() != before_size
        || !same_mtime(
            before_mtime,
            after.modified().map_err(|e| RedactError::io(path, e))?,
        )
    {
        return Err(RedactError::LiveWriteDetected);
    }

    // Backup — unless the caller asked for a real removal.
    let backup = if opts.purge {
        sink.phase("purging", PhaseStatus::Complete);
        None
    } else {
        sink.phase("trashing-original", PhaseStatus::Running);
        let snapshot = tmp_path.with_extension("pre-redact.jsonl");
        let mut snap_guard = FileGuard::new(snapshot.clone());
        fs::copy(path, &snapshot).map_err(|e| RedactError::io(&snapshot, e))?;
        let entry = trash::write(
            data_dir,
            TrashPut {
                orig_path: &snapshot,
                restore_path: Some(path),
                kind: TrashKind::Slim,
                cwd: path.parent(),
                reason: Some(format!(
                    "pre-redact snapshot of {} — STILL CONTAINS THE REDACTED CONTENT",
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                )),
            },
        )?;
        snap_guard.disarm();
        Some(entry.id.clone())
    };

    sink.phase("swapping", PhaseStatus::Running);
    // Second re-stat immediately before the swap, narrowing the TOCTOU
    // window. If CC appended in between, bail — with the backup (if any)
    // already safely in the trash.
    let after2 = fs::metadata(path).map_err(|e| RedactError::io(path, e))?;
    if after2.len() != before_size
        || !same_mtime(
            before_mtime,
            after2.modified().map_err(|e| RedactError::io(path, e))?,
        )
    {
        return Err(RedactError::LiveWriteDetected);
    }
    fs::rename(&tmp_path, path).map_err(|e| RedactError::io(path, e))?;
    tmp_guard.disarm();

    let final_meta = fs::metadata(path).map_err(|e| RedactError::io(path, e))?;
    sink.phase("complete", PhaseStatus::Complete);
    Ok(RedactReport {
        original_bytes: before_size,
        final_bytes: final_meta.len(),
        matched_lines,
        matched_values,
        backup_trash_id: backup,
    })
}

/// Cheap pre-filter over the raw line. Only meaningful for literal
/// patterns — with `secrets` on we must parse and let the redactor
/// decide, since it matches shapes rather than fixed strings.
fn line_may_match(line: &str, opts: &RedactOpts) -> bool {
    opts.secrets || opts.patterns.iter().any(|p| line.contains(p))
}

/// Walk the JSON, rewriting every string that carries a match.
/// Returns how many string values were rewritten. Structure — keys,
/// arrays, nesting, the UUID chain CC needs to replay the session — is
/// preserved exactly; only leaf strings change.
fn rewrite_value(v: &mut serde_json::Value, opts: &RedactOpts, counts: &mut [u32]) -> u32 {
    match v {
        serde_json::Value::String(s) => {
            let mut hit = false;
            for (i, p) in opts.patterns.iter().enumerate() {
                if p.is_empty() {
                    continue;
                }
                let n = s.matches(p.as_str()).count() as u32;
                if n > 0 {
                    counts[i] += n;
                    hit = true;
                }
            }
            if hit {
                *s = if opts.whole_value {
                    REDACTION_MARKER.to_string()
                } else {
                    let mut out = s.clone();
                    for p in &opts.patterns {
                        if !p.is_empty() {
                            out = out.replace(p.as_str(), REDACTION_MARKER);
                        }
                    }
                    out
                };
            }
            if opts.secrets {
                let masked = redact_apply(s, &secret_policy());
                if masked != *s {
                    *s = masked;
                    if !hit {
                        hit = true;
                    }
                }
            }
            u32::from(hit)
        }
        serde_json::Value::Array(items) => items
            .iter_mut()
            .map(|x| rewrite_value(x, opts, counts))
            .sum(),
        serde_json::Value::Object(map) => map
            .iter_mut()
            .map(|(_, x)| rewrite_value(x, opts, counts))
            .sum(),
        _ => 0,
    }
}

/// The built-in secret shapes. Same policy the MCP boundary uses, so
/// "what an agent could have seen" and "what redact removes" agree.
fn secret_policy() -> RedactionPolicy {
    RedactionPolicy {
        anthropic_keys: true,
        emails: true,
        env_assignments: true,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_progress::NoopSink;
    use tempfile::TempDir;

    /// A transcript carrying a leaked credential and a private record,
    /// shaped like the real thing (tool_result content nested in an
    /// array inside a message object).
    fn write_transcript(dir: &Path) -> PathBuf {
        let p = dir.join("session.jsonl");
        fs::write(
            &p,
            r#"{"type":"user","uuid":"u1","message":{"role":"user","content":[{"type":"text","text":"deploy the thing"}]}}
{"type":"assistant","uuid":"a1","message":{"role":"assistant","content":[{"type":"tool_result","tool_use_id":"t1","content":"AWS_SECRET=hunter2 and customer Jane Roe owes 12345"}]}}
{"type":"user","uuid":"u2","message":{"role":"user","content":[{"type":"text","text":"thanks"}]}}
"#,
        )
        .unwrap();
        p
    }

    #[test]
    fn a_plan_reports_counts_and_never_the_matched_text() {
        // The whole point: a redaction tool that prints what it found
        // has leaked it again — into a terminal, a log, a CI record.
        let tmp = TempDir::new().unwrap();
        let p = write_transcript(tmp.path());
        let opts = RedactOpts {
            patterns: vec!["Jane Roe".into(), "hunter2".into()],
            ..Default::default()
        };
        let plan = plan_redact(&p, &opts).unwrap();
        assert_eq!(plan.matched_lines, 1);
        assert_eq!(plan.matched_values, 1);
        assert_eq!(plan.hits[0].count, 1);
        assert_eq!(plan.hits[1].count, 1);

        let rendered = serde_json::to_string(&plan).unwrap();
        assert!(!rendered.contains("owes 12345"));
        assert!(!rendered.contains("AWS_SECRET=hunter2"));
    }

    #[test]
    fn planning_touches_nothing_on_disk() {
        let tmp = TempDir::new().unwrap();
        let p = write_transcript(tmp.path());
        let before = fs::read(&p).unwrap();
        let opts = RedactOpts {
            patterns: vec!["hunter2".into()],
            ..Default::default()
        };
        plan_redact(&p, &opts).unwrap();
        assert_eq!(fs::read(&p).unwrap(), before);
    }

    #[test]
    fn redacting_removes_the_string_and_leaves_every_other_line_intact() {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let p = write_transcript(tmp.path());

        let opts = RedactOpts {
            patterns: vec!["hunter2".into()],
            ..Default::default()
        };
        let report = execute_redact(&data, &p, &opts, &NoopSink).unwrap();
        assert_eq!(report.matched_values, 1);

        let body = fs::read_to_string(&p).unwrap();
        assert!(!body.contains("hunter2"), "the secret must be gone");
        assert!(body.contains(REDACTION_MARKER));
        // Untouched turns survive byte-for-byte, and the UUID chain CC
        // needs to replay the session is intact.
        assert!(body.contains(r#""uuid":"u1""#));
        assert!(body.contains("deploy the thing"));
        assert!(body.contains("thanks"));
        assert_eq!(body.lines().count(), 3);
    }

    #[test]
    fn whole_value_removes_the_entire_tainted_payload() {
        // Excising only the string you happened to grep for would leave
        // the rest of a dumped record sitting there. whole_value is for
        // "this value is tainted, drop all of it".
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let p = write_transcript(tmp.path());

        let opts = RedactOpts {
            patterns: vec!["Jane Roe".into()],
            whole_value: true,
            ..Default::default()
        };
        execute_redact(&data, &p, &opts, &NoopSink).unwrap();

        let body = fs::read_to_string(&p).unwrap();
        assert!(!body.contains("Jane Roe"));
        assert!(
            !body.contains("12345"),
            "the rest of the record must go too"
        );
        assert!(
            !body.contains("hunter2"),
            "…including the neighbouring secret"
        );
        assert!(
            body.contains("deploy the thing"),
            "other turns still intact"
        );
    }

    #[test]
    fn the_default_keeps_a_backup_that_still_holds_the_secret() {
        // Documented, not accidental: the caller MUST surface this, or
        // a user "redacts" a leaked key and leaves a copy in the trash.
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let p = write_transcript(tmp.path());

        let opts = RedactOpts {
            patterns: vec!["hunter2".into()],
            ..Default::default()
        };
        let report = execute_redact(&data, &p, &opts, &NoopSink).unwrap();
        report
            .backup_trash_id
            .expect("the default keeps a backup in the trash");
        let still_somewhere = walk(&data)
            .iter()
            .filter_map(|f| fs::read_to_string(f).ok())
            .any(|s| s.contains("hunter2"));
        assert!(
            still_somewhere,
            "the backup is a pre-redaction snapshot — it necessarily still \
             contains the secret, which is exactly why --purge exists"
        );
    }

    #[test]
    fn purge_leaves_no_copy_behind() {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let p = write_transcript(tmp.path());

        let opts = RedactOpts {
            patterns: vec!["hunter2".into()],
            purge: true,
            ..Default::default()
        };
        let report = execute_redact(&data, &p, &opts, &NoopSink).unwrap();
        assert!(report.backup_trash_id.is_none());
        assert!(!fs::read_to_string(&p).unwrap().contains("hunter2"));

        // Nothing anywhere under the data dir may still hold it.
        let mut found = false;
        for entry in walk(&data) {
            if let Ok(s) = fs::read_to_string(&entry) {
                if s.contains("hunter2") {
                    found = true;
                }
            }
        }
        assert!(!found, "--purge must not leave the secret in the trash");
    }

    #[test]
    fn the_secrets_mode_catches_shapes_no_one_thought_to_name() {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let p = tmp.path().join("s.jsonl");
        fs::write(
            &p,
            "{\"type\":\"user\",\"message\":{\"content\":\
             [{\"type\":\"text\",\"text\":\"key is sk-ant-oat01-AAAABBBBCCCCDDDD\"}]}}\n",
        )
        .unwrap();

        let opts = RedactOpts {
            secrets: true,
            ..Default::default()
        };
        execute_redact(&data, &p, &opts, &NoopSink).unwrap();
        let body = fs::read_to_string(&p).unwrap();
        assert!(!body.contains("sk-ant-oat01-AAAABBBBCCCCDDDD"));
    }

    #[test]
    fn refusing_an_empty_pattern_set_rather_than_erasing_the_file() {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let p = write_transcript(tmp.path());
        let opts = RedactOpts::default();
        assert!(matches!(
            plan_redact(&p, &opts),
            Err(RedactError::NoPatterns)
        ));
        assert!(matches!(
            execute_redact(&data, &p, &opts, &NoopSink),
            Err(RedactError::NoPatterns)
        ));
    }

    #[test]
    fn a_no_match_redact_is_a_faithful_noop() {
        let tmp = TempDir::new().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let p = write_transcript(tmp.path());
        let before = fs::read_to_string(&p).unwrap();

        let opts = RedactOpts {
            patterns: vec!["nothing-matches-this".into()],
            purge: true,
            ..Default::default()
        };
        let report = execute_redact(&data, &p, &opts, &NoopSink).unwrap();
        assert_eq!(report.matched_values, 0);
        // Byte-identical: unmatched lines take the fast path and are
        // never reserialized.
        assert_eq!(fs::read_to_string(&p).unwrap(), before);
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(rd) = fs::read_dir(dir) else {
            return out;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }
}
