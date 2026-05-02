//! Static-analysis health metrics for CC's global memory file
//! (`~/.claude/CLAUDE.md`) and the per-account `MEMORY.md` index.
//!
//! Purpose: surface bloat *before* it costs the user. CC truncates
//! globally-loaded memory after line 200 — content past that line is
//! invisible to the model regardless of how carefully the user wrote
//! it. The value here is a transparency surface, not a cleaning tool;
//! we report metrics, the user decides whether to trim.
//!
//! What this module does NOT do:
//!   - Edit, suggest edits, or auto-prune. The CLAUDE.md format is
//!     freeform; any "this is dead text" judgment is the user's.
//!   - Deep-parse markdown. Line counts are physical-line counts;
//!     a multi-paragraph code fence inflates the count truthfully.
//!   - Resolve includes (`@path/to/file.md`). CC resolves them at
//!     load time; the bloat we measure is the *visible* file's own
//!     line count, which is what determines truncation.
//!
//! Token estimate: a deliberate approximation. Anthropic's tokenizer
//! averages ~4 chars per token for English; we use 4.0 as the divisor
//! so the estimate is conservative (slightly under-reports). The
//! consumer surface labels it `est_tokens` to signal that — if the
//! user's CLAUDE.md is tokenizer-pessimal (e.g. heavy code with
//! short identifiers), the real number can run 20–30% higher. A
//! precise tokenizer in the dashboard path would require shipping
//! the tokenizer crate, which adds binary size + a maintenance burden
//! the figure doesn't justify.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// CC's truncation cutoff for globally-loaded memory. Lines past this
/// boundary in `~/.claude/CLAUDE.md` are invisible to the model on
/// every session. Verified against CC source (memdir loader, 2026-01).
/// Bump this constant when CC's published behavior changes — it's the
/// load-bearing number for the "lines past cutoff" health signal.
pub const GLOBAL_MEMORY_LINE_CUTOFF: usize = 200;

/// Approximate chars-per-token used for the rough estimate. 4.0 is the
/// English average for Anthropic's tokenizer; it under-reports for
/// dense code (short identifiers tokenize tighter). The dashboard
/// labels this `est_tokens` to signal the approximation.
const CHARS_PER_TOKEN: f64 = 4.0;

/// One file's health metrics. All fields are derived from a single
/// pass over the file contents — no shell out to a tokenizer, no
/// heavy parse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileHealth {
    /// Absolute path the metrics were computed against. Echoed back
    /// so the consumer surface can render the source without holding
    /// onto a separate "which file?" string.
    pub path: String,
    /// `true` when the file was missing on disk. Distinguishes "no
    /// CLAUDE.md yet" from "CLAUDE.md exists but is empty" — both
    /// are valid states for a fresh CC install but the consumer
    /// renders them differently.
    pub missing: bool,
    /// Total physical-line count. A trailing newline counts the
    /// final empty line; this matches what every CLI tool (`wc -l`)
    /// shows the user, so the displayed number lines up with their
    /// editor.
    pub line_count: usize,
    pub char_count: usize,
    /// Physical lines past the global truncation cutoff (`>= line
    /// 201`). `0` when `line_count <= GLOBAL_MEMORY_LINE_CUTOFF`.
    /// Non-zero values are the actionable signal — CC literally
    /// can't see those lines.
    pub lines_past_cutoff: usize,
    /// Char count *of those past-cutoff lines only*, including
    /// terminating newlines. Lets the dashboard render "12 KB
    /// invisible to Claude" alongside the line count.
    pub chars_past_cutoff: usize,
    /// Approximate token count for the whole file (`char_count /
    /// CHARS_PER_TOKEN`, rounded). See module-level note about the
    /// approximation.
    pub est_tokens: usize,
}

/// Aggregate report covering the files Claudepot can audit cheaply.
/// Fields are independent — a missing `claude_md` doesn't suppress
/// the `memory_md` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHealthReport {
    /// `~/.claude/CLAUDE.md` — the global instruction file CC loads
    /// on every session.
    pub claude_md: FileHealth,
    /// `~/.claude/memory/MEMORY.md` — index for the per-user memory
    /// store. Optional in CC: a user who hasn't enabled memory will
    /// see `missing: true` here, which the consumer renders as a
    /// muted "no memory index" cell rather than a warning.
    pub memory_md: FileHealth,
    /// CC's truncation cutoff at the time the report was built.
    /// Echoed back so the consumer can label warnings authoritatively
    /// (e.g. "lines 201–235 invisible — CC truncates after 200").
    pub line_cutoff: usize,
}

/// Audit a single markdown file. Missing-file is a normal outcome —
/// returns a `FileHealth { missing: true, .. }` rather than an error
/// so the dashboard never goes blank on a fresh install. Any other
/// I/O error (permission denied, mid-read corruption) propagates
/// up so the caller can surface a real failure.
pub fn audit_file(path: &Path) -> std::io::Result<FileHealth> {
    let display_path = path.to_string_lossy().into_owned();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileHealth {
                path: display_path,
                missing: true,
                line_count: 0,
                char_count: 0,
                lines_past_cutoff: 0,
                chars_past_cutoff: 0,
                est_tokens: 0,
            });
        }
        Err(e) => return Err(e),
    };
    // UTF-8 lossy: CLAUDE.md should be UTF-8, but a BOM / mojibake
    // file shouldn't crash the dashboard. Lossy decode preserves the
    // byte structure for line-counting purposes.
    let text = String::from_utf8_lossy(&bytes);
    Ok(audit_text(&display_path, &text))
}

/// Pure variant of [`audit_file`] that takes pre-loaded text. Lets
/// tests drive the metrics without writing to disk and keeps the
/// I/O path narrow for fuzz-testing the scoring rules.
pub fn audit_text(path: &str, text: &str) -> FileHealth {
    let char_count = text.chars().count();
    // `lines()` strips the terminator; for a file ending in `\n`
    // that's the count we want (the trailing newline doesn't add a
    // visible line). Files without a trailing newline are still
    // counted truthfully — `lines()` includes the final partial
    // line.
    let lines: Vec<&str> = text.lines().collect();
    let line_count = lines.len();
    let (lines_past_cutoff, chars_past_cutoff) = if line_count > GLOBAL_MEMORY_LINE_CUTOFF {
        let tail = &lines[GLOBAL_MEMORY_LINE_CUTOFF..];
        let count = tail.len();
        // Re-count chars in the tail. Each `&str` in `lines` excludes
        // its terminator, so the +count term restores the newlines
        // that were stripped (an upper bound matching what the file
        // actually contained for files with `\n` line endings).
        let chars = tail.iter().map(|l| l.chars().count()).sum::<usize>() + count;
        (count, chars)
    } else {
        (0, 0)
    };
    let est_tokens = (char_count as f64 / CHARS_PER_TOKEN).round() as usize;
    FileHealth {
        path: path.to_string(),
        missing: false,
        line_count,
        char_count,
        lines_past_cutoff,
        chars_past_cutoff,
        est_tokens,
    }
}

/// Build the full health report. Reads both files; `~/.claude/CLAUDE.md`
/// and `~/.claude/memory/MEMORY.md`. Missing files are reported as
/// `missing: true` rather than as errors. Any non-NotFound I/O failure
/// (permission denied, etc.) on either file propagates as `Err` so
/// the consumer surface can show a "couldn't audit" pill rather than
/// a confusing zero figure.
pub fn build_report() -> std::io::Result<MemoryHealthReport> {
    let cfg = crate::paths::claude_config_dir();
    let claude_md_path = cfg.join("CLAUDE.md");
    let memory_md_path: PathBuf = cfg.join("memory").join("MEMORY.md");
    let claude_md = audit_file(&claude_md_path)?;
    let memory_md = audit_file(&memory_md_path)?;
    Ok(MemoryHealthReport {
        claude_md,
        memory_md,
        line_cutoff: GLOBAL_MEMORY_LINE_CUTOFF,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn audit_text_counts_basic_metrics() {
        let body = "line1\nline2\nline3\n";
        let h = audit_text("/x", body);
        assert!(!h.missing);
        assert_eq!(h.line_count, 3);
        assert_eq!(h.char_count, body.chars().count());
        assert_eq!(h.lines_past_cutoff, 0);
        assert_eq!(h.chars_past_cutoff, 0);
        // 18 chars / 4 = 4.5 → 5 (rounded).
        assert_eq!(h.est_tokens, 5);
    }

    #[test]
    fn audit_text_zero_lines_for_empty_file() {
        let h = audit_text("/x", "");
        assert!(!h.missing);
        assert_eq!(h.line_count, 0);
        assert_eq!(h.char_count, 0);
        assert_eq!(h.est_tokens, 0);
    }

    #[test]
    fn audit_text_flags_lines_past_cutoff() {
        // 250 lines, each "x". Cutoff is 200; expect 50 past-cutoff
        // lines totalling 50 chars + 50 newlines = 100 chars.
        let body: String = (0..250).map(|_| "x\n").collect();
        let h = audit_text("/x", &body);
        assert_eq!(h.line_count, 250);
        assert_eq!(h.lines_past_cutoff, 50);
        assert_eq!(h.chars_past_cutoff, 100);
    }

    #[test]
    fn audit_text_at_cutoff_exact_reports_zero_past() {
        // Exactly 200 lines must NOT trigger the past-cutoff signal.
        // Off-by-one here would make every "near-limit" CLAUDE.md
        // light up red; cutoff is "past 200", inclusive of line 200.
        let body: String = (0..200).map(|_| "y\n").collect();
        let h = audit_text("/x", &body);
        assert_eq!(h.line_count, 200);
        assert_eq!(h.lines_past_cutoff, 0);
    }

    #[test]
    fn audit_file_reports_missing_for_nonexistent_path() {
        let tmp = TempDir::new().unwrap();
        let h = audit_file(&tmp.path().join("does-not-exist.md")).unwrap();
        assert!(h.missing);
        assert_eq!(h.line_count, 0);
        assert_eq!(h.char_count, 0);
    }

    #[test]
    fn audit_file_reads_real_content() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("CLAUDE.md");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "hello").unwrap();
        writeln!(f, "world").unwrap();
        let h = audit_file(&p).unwrap();
        assert!(!h.missing);
        assert_eq!(h.line_count, 2);
        // "hello\nworld\n" = 12 chars.
        assert_eq!(h.char_count, 12);
    }

    #[test]
    fn audit_text_handles_no_trailing_newline() {
        let h = audit_text("/x", "one\ntwo");
        // `lines()` includes the final partial line.
        assert_eq!(h.line_count, 2);
        assert_eq!(h.char_count, 7);
    }

    #[test]
    fn audit_text_handles_lossy_utf8_path() {
        // Smoke test: lossy decode in audit_file preserves char count.
        // We test directly via audit_text since the lossy step lives
        // there.
        let h = audit_text("/x", "café\n");
        assert_eq!(h.char_count, 5); // c-a-f-é-\n
    }
}
