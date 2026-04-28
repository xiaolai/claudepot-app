//! Session + subagent JSONL path rewriting for project rename (Phase 6).
//!
//! Rewrites `cwd` fields in session jsonl files and absolute-path strings in
//! `.meta.json` sidecars when a project is renamed. Scope per spec
//! §4.2 P6: prefix-match with `path + MAIN_SEPARATOR` boundary; no case
//! folding. Parallel via rayon; mandatory progress callback; collect-all
//! errors then report.

use crate::error::ProjectError;
use rayon::prelude::*;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Aggregate statistics for a P6 rewrite pass.
#[derive(Debug, Default, Clone)]
pub struct RewriteStats {
    pub files_scanned: usize,
    pub files_modified: usize,
    pub lines_rewritten: usize,
}

/// Re-export for callers that used the old name. New code should
/// import `ProgressSink` from `project_progress` directly.
pub use crate::project_progress::{NoopSink, PhaseStatus, ProgressSink};

/// Rewrite `cwd` fields inside all jsonl + `.meta.json` files under
/// `project_dir` that reference `old_path` (exact-match or prefix with
/// path-separator boundary).
///
/// Returns an aggregate stats struct plus a per-file error list. If any
/// file failed, callers should consider the phase failed — successful
/// rewrites are still durable (atomic replace) so rollback can reverse
/// them by calling this function again with `old_path` / `new_path`
/// swapped.
///
/// The `sink.sub_progress("P6", done, total)` hook fires after each
/// file completes. `done` is a monotonically increasing count of files
/// processed (not modified); `total` is the upfront file count.
pub fn rewrite_project_paths(
    project_dir: &Path,
    old_path: &str,
    new_path: &str,
    sink: &dyn ProgressSink,
) -> Result<(RewriteStats, Vec<(PathBuf, ProjectError)>), ProjectError> {
    let files = collect_rewrite_targets(project_dir)?;
    let total = files.len();

    let stats = Mutex::new(RewriteStats::default());
    let errors: Mutex<Vec<(PathBuf, ProjectError)>> = Mutex::new(Vec::new());
    let done = std::sync::atomic::AtomicUsize::new(0);

    // Cap concurrency at num_cpus — for I/O-heavy rewrites we don't
    // benefit from more threads, and unbounded rayon can exhaust open
    // FDs on large projects.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_cpus::get())
        .build()
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    pool.install(|| {
        files.par_iter().for_each(|path| {
            let result = if is_jsonl(path) {
                rewrite_jsonl(path, old_path, new_path)
            } else {
                rewrite_meta_json(path, old_path, new_path)
            };

            // PoisonError recovery: another worker panicking shouldn't
            // crash the remaining rewrites. Recover the inner guard and
            // keep going — the panicked worker's file stays unmodified
            // (atomic-replace ensures no half-write on disk).
            {
                let mut s = stats.lock().unwrap_or_else(|e| e.into_inner());
                s.files_scanned += 1;
                match &result {
                    Ok(lines) if *lines > 0 => {
                        s.files_modified += 1;
                        s.lines_rewritten += lines;
                    }
                    Ok(_) => {}
                    Err(_) => {}
                }
            }
            if let Err(e) = result {
                errors
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .push((path.clone(), e));
            }
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            sink.sub_progress("P6", n, total);
        })
    });

    let stats = stats.into_inner().unwrap_or_else(|e| e.into_inner());
    let errors = errors.into_inner().unwrap_or_else(|e| e.into_inner());
    tracing::info!(
        scanned = stats.files_scanned,
        modified = stats.files_modified,
        lines = stats.lines_rewritten,
        errors = errors.len(),
        "P6 jsonl rewrite complete"
    );
    Ok((stats, errors))
}

fn collect_rewrite_targets(project_dir: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    let mut out = Vec::new();
    walk_collect(project_dir, &mut out)?;
    Ok(out)
}

fn walk_collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), ProjectError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(ProjectError::Io)? {
        let entry = entry.map_err(ProjectError::Io)?;
        let ft = entry.file_type().map_err(ProjectError::Io)?;
        if ft.is_dir() {
            walk_collect(&entry.path(), out)?;
        } else if ft.is_file() {
            let path = entry.path();
            if is_jsonl(&path) || is_meta_json(&path) {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn is_jsonl(p: &Path) -> bool {
    p.extension().map(|e| e == "jsonl").unwrap_or(false)
}

fn is_meta_json(p: &Path) -> bool {
    p.to_string_lossy().ends_with(".meta.json")
}

/// Rewrite a JSONL file line-by-line. Returns the number of lines that
/// were rewritten. Uses atomic tempfile + rename for durability.
///
/// Streaming: reads the source via `BufReader` line-by-line and writes
/// to the tempfile in the same pass. Memory footprint is bounded by
/// the longest single line. Pre-scan for the needle is done on the
/// first chunk only — we take a quick peek at the first 64 KB to
/// decide whether to even open a tempfile; if absent, we still must
/// scan the remainder, so we start writing and abort (discard tempfile)
/// if no rewrites happened by EOF.
fn rewrite_jsonl(path: &Path, old_path: &str, new_path: &str) -> Result<usize, ProjectError> {
    // JSON-escaped needle (handles Windows backslashes and Unicode).
    let old_escaped = serde_json::to_string(old_path).unwrap_or_else(|_| format!("\"{old_path}\""));
    let needle = old_escaped.trim_matches('"');

    let src = fs::File::open(path).map_err(ProjectError::Io)?;
    let reader = BufReader::new(src);

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(ProjectError::Io)?;

    let mut rewritten = 0usize;
    for line in reader.lines() {
        let line = line.map_err(ProjectError::Io)?;
        let (new_line, changed) = rewrite_jsonl_line(&line, old_path, new_path, needle);
        if changed {
            rewritten += 1;
        }
        writeln!(tmp, "{new_line}").map_err(ProjectError::Io)?;
    }

    if rewritten > 0 {
        tmp.persist(path).map_err(|e| ProjectError::Io(e.error))?;
    } else {
        // No-op rewrite; discard the tempfile rather than persist.
        drop(tmp);
    }
    Ok(rewritten)
}

/// Rewrite one JSONL line. Cheap fast path: if the line doesn't contain
/// the needle, return unchanged. Slow path: parse as JSON, mutate `cwd`
/// with prefix-match-plus-boundary, serialize back.
fn rewrite_jsonl_line(line: &str, old_path: &str, new_path: &str, needle: &str) -> (String, bool) {
    if !line.contains(needle) {
        return (line.to_string(), false);
    }
    let mut value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return (line.to_string(), false),
    };
    let changed = rewrite_cwd_in_value(&mut value, old_path, new_path);
    if !changed {
        return (line.to_string(), false);
    }
    match serde_json::to_string(&value) {
        Ok(s) => (s, true),
        Err(_) => (line.to_string(), false),
    }
}

/// Mutate any `cwd` field (at any nesting depth) per the prefix-match
/// boundary rule. Returns true if any rewrite occurred.
fn rewrite_cwd_in_value(v: &mut Value, old_path: &str, new_path: &str) -> bool {
    let mut changed = false;
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if k == "cwd" {
                    if let Value::String(s) = child {
                        if let Some(next) = rewrite_path_string(s, old_path, new_path) {
                            *s = next;
                            changed = true;
                        }
                    }
                } else if rewrite_cwd_in_value(child, old_path, new_path) {
                    changed = true;
                }
            }
        }
        Value::Array(arr) => {
            for child in arr.iter_mut() {
                if rewrite_cwd_in_value(child, old_path, new_path) {
                    changed = true;
                }
            }
        }
        _ => {}
    }
    changed
}

/// Rewrite a single path string per the prefix-match-with-boundary rule
/// (spec §4.2 P6, §8 Q4). Returns `Some(new)` if the string matches;
/// `None` if it doesn't (caller keeps the original).
///
/// Rules:
///   - Exact match: `s == old_path` → `new_path`.
///   - Boundary prefix: `s == old_path + SEP + suffix` →
///     `new_path + NEW_SEP + suffix`. Both `\` and `/` are accepted as
///     boundary separators because session JSONL `cwd` values may carry
///     a Windows path on a Unix host (and vice versa) and the host's
///     `MAIN_SEPARATOR` would miss the foreign form. Audit B3:
///     `project_rewrite.rs:251` was Unix-only on Linux/macOS.
///   - Else: no rewrite.
///
/// Separator preservation: the boundary separator from the source
/// (after `old_path`) is reused when constructing the rewritten suffix,
/// so a `C:\foo\bar` cwd stored in a JSONL on macOS keeps its
/// backslashes. The host `MAIN_SEPARATOR` is only used as a tie-breaker
/// when neither `old_path` nor `s` carries a separator after `old_path`
/// (exact-match path).
pub(crate) fn rewrite_path_string(s: &str, old_path: &str, new_path: &str) -> Option<String> {
    if s == old_path {
        return Some(new_path.to_string());
    }
    // Try both separators. Order matters only when `old_path` happens
    // to have a trailing single-char that combines with `\` and `/`
    // ambiguously — in practice, a cwd string never ends in a separator
    // so the first match wins cleanly.
    for sep in ['\\', '/'] {
        let boundary = format!("{old_path}{sep}");
        if let Some(rest) = s.strip_prefix(&boundary) {
            // Preserve the SOURCE separator inside the suffix as-is
            // (`rest` is already separator-correct for its origin
            // file). The boundary separator we splice between
            // `new_path` and `rest` is the one we just matched, which
            // matches the document's native shape.
            return Some(format!("{new_path}{sep}{rest}"));
        }
    }
    None
}

/// Rewrite a `.meta.json` file. Parses the whole file as JSON, walks all
/// string values, applies the prefix-match rule, and writes back atomically
/// if any value changed.
fn rewrite_meta_json(path: &Path, old_path: &str, new_path: &str) -> Result<usize, ProjectError> {
    let contents = fs::read_to_string(path).map_err(ProjectError::Io)?;
    // Audit M11: use the JSON-escaped needle for the fast path.
    // `contents` is JSON, so Windows paths appear with backslashes
    // escaped as `\\`. A raw-string `contains(old_path)` check missed
    // every Windows path, causing the sidecar to be silently skipped
    // even when P6 reported a clean pass. JSONL uses the same
    // escaped needle; this aligns both file formats.
    let old_escaped = serde_json::to_string(old_path).unwrap_or_else(|_| format!("\"{old_path}\""));
    let needle = old_escaped.trim_matches('"');
    if !contents.contains(needle) && !contents.contains(old_path) {
        // Also accept the raw form so non-Windows callers keep working
        // when the path has no escape-worthy characters and the two
        // forms happen to be equal. (In practice both branches hit
        // the same needle string on Unix; Windows is the case this
        // fix targets.)
        return Ok(0);
    }
    let mut value: Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return Ok(0), // skip unparseable
    };
    let changed = rewrite_strings_in_value(&mut value, old_path, new_path);
    if changed == 0 {
        return Ok(0);
    }
    let new_json = serde_json::to_string_pretty(&value)
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(ProjectError::Io)?;
    tmp.write_all(new_json.as_bytes())
        .map_err(ProjectError::Io)?;
    tmp.write_all(b"\n").map_err(ProjectError::Io)?;
    tmp.persist(path).map_err(|e| ProjectError::Io(e.error))?;
    Ok(changed)
}

/// Crate-public wrapper for `rewrite_strings_in_value` so the P7 config
/// rewriter can reuse the same path-rewrite logic on `~/.claude.json`
/// values.
pub(crate) fn rewrite_strings_in_value_pub(v: &mut Value, old_path: &str, new_path: &str) -> usize {
    rewrite_strings_in_value(v, old_path, new_path)
}

/// Recursively walk a JSON value, rewriting any string that matches the
/// prefix-match rule. Returns the number of rewrites performed.
fn rewrite_strings_in_value(v: &mut Value, old_path: &str, new_path: &str) -> usize {
    let mut count = 0;
    match v {
        Value::String(s) => {
            if let Some(next) = rewrite_path_string(s, old_path, new_path) {
                *s = next;
                count += 1;
            }
        }
        Value::Object(map) => {
            for (_, child) in map.iter_mut() {
                count += rewrite_strings_in_value(child, old_path, new_path);
            }
        }
        Value::Array(arr) => {
            for child in arr.iter_mut() {
                count += rewrite_strings_in_value(child, old_path, new_path);
            }
        }
        _ => {}
    }
    count
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_progress() -> &'static dyn ProgressSink {
        &NoopSink
    }

    #[test]
    fn test_rewrite_path_string_exact_match() {
        let r = rewrite_path_string("/a/b", "/a/b", "/c/d");
        assert_eq!(r, Some("/c/d".to_string()));
    }

    #[test]
    fn test_rewrite_path_string_boundary_prefix() {
        let sep = std::path::MAIN_SEPARATOR;
        let r = rewrite_path_string(&format!("/a/b{sep}src{sep}main.rs"), "/a/b", "/c/d");
        assert_eq!(r, Some(format!("/c/d{sep}src{sep}main.rs")));
    }

    #[test]
    fn test_rewrite_path_string_false_positive_prefix_rejected() {
        // `/a/b-backup` should NOT match old=`/a/b`
        let r = rewrite_path_string("/a/b-backup", "/a/b", "/c/d");
        assert_eq!(r, None);
    }

    #[test]
    fn test_rewrite_path_string_unrelated_path() {
        let r = rewrite_path_string("/elsewhere", "/a/b", "/c/d");
        assert_eq!(r, None);
    }

    // -------------------------------------------------------------------
    // Cross-OS path-shape coverage. The four canonical shapes per
    // .claude/rules/paths.md must rewrite on any host because session
    // JSONL `cwd` strings cross OS boundaries (sync, restore, etc.).
    // -------------------------------------------------------------------

    #[test]
    fn test_rewrite_path_string_windows_drive_boundary_on_any_host() {
        // The audit case: `C:\foo\bar` stored in a JSONL on macOS.
        // MAIN_SEPARATOR on Linux/macOS is `/`, so the legacy code
        // would never match the `\` boundary. Both separators must
        // work.
        let r = rewrite_path_string(
            r"C:\Users\joker\proj\src\main.rs",
            r"C:\Users\joker\proj",
            r"D:\code\proj",
        );
        assert_eq!(r, Some(r"D:\code\proj\src\main.rs".to_string()));
    }

    #[test]
    fn test_rewrite_path_string_unc_boundary_on_any_host() {
        let r = rewrite_path_string(
            r"\\server\share\proj\src\lib.rs",
            r"\\server\share\proj",
            r"\\backup\share\proj",
        );
        assert_eq!(r, Some(r"\\backup\share\proj\src\lib.rs".to_string()));
    }

    #[test]
    fn test_rewrite_path_string_unix_boundary_on_any_host() {
        let r = rewrite_path_string(
            "/Users/joker/proj/src/main.rs",
            "/Users/joker/proj",
            "/Users/joker/code",
        );
        assert_eq!(r, Some("/Users/joker/code/src/main.rs".to_string()));
    }

    #[test]
    fn test_rewrite_path_string_verbatim_drive_boundary() {
        // Verbatim-prefixed paths are rare in CC's writers but defense
        // in depth: if one leaks in, the boundary must still match.
        let r = rewrite_path_string(
            r"\\?\C:\Users\joker\proj\src\main.rs",
            r"\\?\C:\Users\joker\proj",
            r"\\?\D:\code\proj",
        );
        assert_eq!(r, Some(r"\\?\D:\code\proj\src\main.rs".to_string()));
    }

    #[test]
    fn test_rewrite_path_string_windows_false_positive_prefix_rejected() {
        // `C:\Users\joker\proj-backup` must NOT match `C:\Users\joker\proj`.
        let r = rewrite_path_string(
            r"C:\Users\joker\proj-backup",
            r"C:\Users\joker\proj",
            r"D:\code\proj",
        );
        assert_eq!(r, None);
    }

    #[test]
    fn test_rewrite_jsonl_exact_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("session.jsonl");
        fs::write(
            &f,
            r#"{"cwd":"/a/b","msg":"hi"}
{"cwd":"/other","msg":"no"}
"#,
        )
        .unwrap();

        let n = rewrite_jsonl(&f, "/a/b", "/c/d").unwrap();
        assert_eq!(n, 1);
        let after = fs::read_to_string(&f).unwrap();
        assert!(after.contains(r#""cwd":"/c/d""#));
        assert!(after.contains(r#""cwd":"/other""#));
    }

    #[test]
    fn test_rewrite_jsonl_prefix_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("session.jsonl");
        let sep = std::path::MAIN_SEPARATOR;
        fs::write(
            &f,
            format!(
                r#"{{"cwd":"/a/b{sep}src","x":1}}
"#
            ),
        )
        .unwrap();

        let n = rewrite_jsonl(&f, "/a/b", "/c/d").unwrap();
        assert_eq!(n, 1);
        let after = fs::read_to_string(&f).unwrap();
        let expected_cwd = format!("/c/d{sep}src");
        assert!(after.contains(&expected_cwd));
    }

    #[test]
    fn test_rewrite_jsonl_no_match_no_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("session.jsonl");
        let before = r#"{"cwd":"/elsewhere"}
"#;
        fs::write(&f, before).unwrap();
        let mtime_before = fs::metadata(&f).unwrap().modified().unwrap();

        let n = rewrite_jsonl(&f, "/a/b", "/c/d").unwrap();
        assert_eq!(n, 0);
        assert_eq!(fs::read_to_string(&f).unwrap(), before);
        // mtime unchanged since we didn't persist the tempfile
        let mtime_after = fs::metadata(&f).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after);
    }

    #[test]
    fn test_rewrite_jsonl_unparseable_line_passed_through() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("session.jsonl");
        fs::write(
            &f,
            r#"{"cwd":"/a/b","ok":1}
not json /a/b
{"cwd":"/a/b"}
"#,
        )
        .unwrap();

        let n = rewrite_jsonl(&f, "/a/b", "/c/d").unwrap();
        assert_eq!(n, 2); // two parseable lines rewritten
        let after = fs::read_to_string(&f).unwrap();
        assert!(after.contains("not json /a/b")); // unparseable preserved verbatim
        assert_eq!(after.matches(r#""cwd":"/c/d""#).count(), 2);
    }

    #[test]
    fn test_rewrite_project_paths_full_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("project");
        fs::create_dir(&proj).unwrap();

        // Session jsonl at root
        fs::write(
            proj.join("abc.jsonl"),
            r#"{"cwd":"/a/b"}
{"cwd":"/a/b/src"}
"#,
        )
        .unwrap();

        // Subagent jsonl nested
        let subagent_dir = proj
            .join("sessionX")
            .join("subagents")
            .join("workflows")
            .join("runY");
        fs::create_dir_all(&subagent_dir).unwrap();
        fs::write(
            subagent_dir.join("agent-foo.jsonl"),
            r#"{"cwd":"/a/b","agentId":"foo"}
"#,
        )
        .unwrap();

        // Meta json sidecar
        fs::write(
            subagent_dir.join("agent-foo.meta.json"),
            r#"{"workdir":"/a/b","runId":"runY"}"#,
        )
        .unwrap();

        // Irrelevant file
        fs::write(proj.join("readme.txt"), "ignore me /a/b").unwrap();

        let (stats, errors) =
            rewrite_project_paths(&proj, "/a/b", "/c/d", noop_progress()).unwrap();
        assert!(errors.is_empty());
        assert_eq!(stats.files_scanned, 3); // 2 jsonls + 1 meta.json (readme ignored)
        assert_eq!(stats.files_modified, 3);
        assert_eq!(stats.lines_rewritten, 4); // 2 + 1 + 1

        let main = fs::read_to_string(proj.join("abc.jsonl")).unwrap();
        assert!(main.contains(r#""cwd":"/c/d""#));
        assert!(main.contains(r#""cwd":"/c/d/src""#));

        let sub = fs::read_to_string(subagent_dir.join("agent-foo.jsonl")).unwrap();
        assert!(sub.contains(r#""cwd":"/c/d""#));

        let meta = fs::read_to_string(subagent_dir.join("agent-foo.meta.json")).unwrap();
        assert!(meta.contains(r#""workdir": "/c/d""#) || meta.contains(r#""workdir":"/c/d""#));

        // readme.txt untouched
        assert_eq!(
            fs::read_to_string(proj.join("readme.txt")).unwrap(),
            "ignore me /a/b"
        );
    }

    #[test]
    fn test_rewrite_progress_callback_fires() {
        use std::sync::Mutex;
        #[derive(Default)]
        struct CountingSink {
            calls: Mutex<Vec<(String, usize, usize)>>,
        }
        impl ProgressSink for CountingSink {
            fn phase(&self, _p: &str, _s: PhaseStatus) {}
            fn sub_progress(&self, phase: &str, done: usize, total: usize) {
                self.calls
                    .lock()
                    .unwrap()
                    .push((phase.to_string(), done, total));
            }
        }

        let sink = CountingSink::default();

        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("p");
        fs::create_dir(&proj).unwrap();
        fs::write(proj.join("a.jsonl"), r#"{"cwd":"/a"}"#).unwrap();
        fs::write(proj.join("b.jsonl"), r#"{"cwd":"/other"}"#).unwrap();

        let (_, _) = rewrite_project_paths(&proj, "/a", "/z", &sink).unwrap();
        let calls = sink.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        // All calls are for P6 with total=2.
        assert!(calls.iter().all(|c| c.0 == "P6" && c.2 == 2));
    }

    #[test]
    fn test_rewrite_is_idempotent_after_first_run() {
        // After a successful rewrite, re-running with the same args is a
        // no-op because nothing matches `old_path` anymore.
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("p");
        fs::create_dir(&proj).unwrap();
        fs::write(proj.join("s.jsonl"), r#"{"cwd":"/a/b"}"#).unwrap();

        let (s1, _) = rewrite_project_paths(&proj, "/a/b", "/c/d", noop_progress()).unwrap();
        assert_eq!(s1.lines_rewritten, 1);

        let (s2, _) = rewrite_project_paths(&proj, "/a/b", "/c/d", noop_progress()).unwrap();
        assert_eq!(s2.lines_rewritten, 0);
        assert_eq!(s2.files_modified, 0);
    }
}
