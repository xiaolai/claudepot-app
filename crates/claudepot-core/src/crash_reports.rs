//! Harvest macOS crash reports (`.ips`) into the app's own log dir.
//!
//! When the app aborts from foreign code — an AppKit assertion, an
//! Obj-C exception, a SIGSEGV in an FFI dependency — the Rust panic
//! hook never fires (it only sees `panic!`), so `panic.log` stays
//! empty. The one place those crashes ARE recorded is the OS:
//! `~/Library/Logs/DiagnosticReports/<proc>-<ts>.ips`, a fully
//! symbolicated, every-thread dump. We never looked at it; the
//! v0.1.4x tray self-quits were only diagnosable by digging there by
//! hand.
//!
//! [`harvest`] reads that directory on startup, summarizes any report
//! newer than the last one it saw, appends a one-line human record to
//! `<log_dir>/crashes.log`, and returns the new summaries so the
//! caller can also push them through `tracing`. The "Reveal logs"
//! button and `claudepot logs` then surface prior crashes in-app
//! without anyone opening `DiagnosticReports`.
//!
//! This is macOS-only at the data-source level (the `.ips` format and
//! directory are Apple's). The parsing/formatting is
//! platform-agnostic and unit-tested on every host; only the wiring in
//! the Tauri shell is gated to macOS. Linux/Windows have no `.ips`
//! equivalent — the synchronous [`crate::diagnostic_logging`] signal
//! handler covers crash capture there.

use std::path::Path;

use serde_json::Value;

/// One distilled crash, ready to log. Every field past `file_name` is
/// best-effort — a malformed or schema-drifted `.ips` yields `None`
/// rather than dropping the whole record. Knowing "a crash happened,
/// here's the file" beats silence even when the body won't parse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrashSummary {
    /// The `.ips` file name (sortable: the embedded timestamp is
    /// fixed-width, so lexical order == chronological order).
    pub file_name: String,
    /// App version from the report header (`app_version`).
    pub app_version: Option<String>,
    /// Human timestamp from the header (`timestamp`).
    pub timestamp: Option<String>,
    /// Mach exception type, e.g. `EXC_BREAKPOINT`.
    pub exc_type: Option<String>,
    /// Unix signal, e.g. `SIGTRAP`.
    pub signal: Option<String>,
    /// Faulting thread identity — its `name` (e.g. `tokio-rt-worker`)
    /// or its dispatch `queue` (e.g. `com.apple.main-thread`). This is
    /// the single most useful field for off-main-thread crashes: it
    /// says which thread died.
    pub faulting_thread: Option<String>,
    /// Top frame of the faulting thread: `"<image> <symbol>"` (symbol
    /// omitted when the report carries only bare offsets).
    pub top_frame: Option<String>,
}

/// Parse one `.ips` file's text into a [`CrashSummary`].
///
/// `.ips` is two concatenated JSON documents: a single-line header,
/// then a multi-line body. Both are parsed defensively — any missing
/// or type-mismatched field degrades to `None`.
pub fn parse_ips(file_name: &str, contents: &str) -> CrashSummary {
    let mut summary = CrashSummary {
        file_name: file_name.to_string(),
        ..Default::default()
    };

    let (header_str, body_str) = match contents.split_once('\n') {
        Some((h, b)) => (h, b),
        // No newline → can't be a valid two-part report; keep just the
        // file name.
        None => return summary,
    };

    if let Ok(header) = serde_json::from_str::<Value>(header_str) {
        summary.app_version = string_field(&header, "app_version");
        summary.timestamp = string_field(&header, "timestamp");
    }

    let body: Value = match serde_json::from_str(body_str) {
        Ok(v) => v,
        Err(_) => return summary,
    };

    summary.exc_type = body.get("exception").and_then(|e| string_field(e, "type"));
    summary.signal = body
        .get("exception")
        .and_then(|e| string_field(e, "signal"));

    if let Some(ft_idx) = body.get("faultingThread").and_then(Value::as_u64) {
        if let Some(thread) = body
            .get("threads")
            .and_then(Value::as_array)
            .and_then(|t| t.get(ft_idx as usize))
        {
            // Prefer the explicit thread name; fall back to the
            // dispatch queue (the main thread is identified by queue,
            // not name).
            summary.faulting_thread =
                string_field(thread, "name").or_else(|| string_field(thread, "queue"));
            summary.top_frame = top_frame(&body, thread);
        }
    }

    summary
}

/// Build `"<image> <symbol>"` for the faulting thread's first frame,
/// resolving the image name through the report's `usedImages` table.
fn top_frame(body: &Value, thread: &Value) -> Option<String> {
    let frame = thread
        .get("frames")
        .and_then(Value::as_array)
        .and_then(|f| f.first())?;

    let image_name = frame
        .get("imageIndex")
        .and_then(Value::as_u64)
        .and_then(|i| {
            body.get("usedImages")
                .and_then(Value::as_array)
                .and_then(|imgs| imgs.get(i as usize))
        })
        .and_then(|img| string_field(img, "name"))
        .unwrap_or_else(|| "<unknown image>".to_string());

    match string_field(frame, "symbol") {
        Some(sym) => Some(format!("{image_name} {sym}")),
        None => {
            // No symbol (bare offset) — still useful to name the image.
            let off = frame
                .get("imageOffset")
                .and_then(Value::as_u64)
                .map(|o| format!(" +{o}"))
                .unwrap_or_default();
            Some(format!("{image_name}{off}"))
        }
    }
}

fn string_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(std::string::ToString::to_string)
}

/// Render a one-line, greppable record for `crashes.log`.
///
/// Render-if-present: empty fields are omitted rather than printed as
/// `None`, so the line stays readable whether the body parsed fully or
/// not.
pub fn summary_line(s: &CrashSummary) -> String {
    let mut parts: Vec<String> = vec![s.file_name.clone()];
    if let Some(v) = &s.timestamp {
        parts.push(v.clone());
    }
    if let Some(v) = &s.app_version {
        parts.push(format!("v{v}"));
    }
    match (&s.exc_type, &s.signal) {
        (Some(e), Some(sig)) => parts.push(format!("{e}/{sig}")),
        (Some(e), None) => parts.push(e.clone()),
        (None, Some(sig)) => parts.push(sig.clone()),
        (None, None) => {}
    }
    if let Some(v) = &s.faulting_thread {
        parts.push(format!("thread={v}"));
    }
    if let Some(v) = &s.top_frame {
        parts.push(format!("top=[{v}]"));
    }
    // Fields come from arbitrary `.ips` JSON; a stray newline/CR in any
    // value would split the record across lines and corrupt the
    // append-only log. Collapse every control char to a space so the
    // record is genuinely one line.
    collapse_control_chars(&parts.join(" | "))
}

/// Replace every control character (newline, CR, tab, …) with a space
/// so a value can't break the one-line log format. Named for its
/// contract — `agent::scheduler::systemd::collapse_newlines` is a
/// different, narrower rule (\n and \r only); don't copy-paste one
/// where the other is meant.
fn collapse_control_chars(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Scan `reports_dir` for `<prefix>-*.ips` files newer than the last
/// harvest, summarize each, append a human line per crash to
/// `crashes_log`, advance the watermark in `state_path`, and return the
/// new summaries.
///
/// "Newer" is a lexical comparison of file names against the watermark
/// — sound because the `.ips` timestamp suffix is fixed-width. The
/// first run (no `state_path`) reports every existing report once, then
/// advances the watermark so later runs are quiet.
///
/// Best-effort throughout: a `reports_dir` that doesn't exist returns
/// an empty list (not an error); an unreadable individual report is
/// skipped; only a failure to *record* progress (writing `crashes_log`
/// / `state_path`) surfaces as `Err`, so the caller can warn but still
/// boot.
pub fn harvest(
    reports_dir: &Path,
    prefix: &str,
    crashes_log: &Path,
    state_path: &Path,
) -> std::io::Result<Vec<CrashSummary>> {
    let read_dir = match std::fs::read_dir(reports_dir) {
        Ok(rd) => rd,
        // No DiagnosticReports dir yet (fresh machine, or the user
        // disabled crash reporting) — nothing to harvest.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let watermark = std::fs::read_to_string(state_path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let wanted_prefix = format!("{prefix}-");
    let mut names: Vec<String> = read_dir
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(&wanted_prefix) && n.ends_with(".ips"))
        .filter(|n| n.as_str() > watermark.as_str())
        .collect();
    names.sort();

    if names.is_empty() {
        return Ok(Vec::new());
    }

    let mut summaries = Vec::with_capacity(names.len());
    for name in &names {
        let path = reports_dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(contents) => summaries.push(parse_ips(name, &contents)),
            // Unreadable report: record the bare fact so the crash
            // isn't lost, and keep going.
            Err(_) => summaries.push(CrashSummary {
                file_name: name.clone(),
                ..Default::default()
            }),
        }
    }

    // Durably append the records, THEN advance the watermark. The
    // ordering plus the fsync inside `append` is the durability
    // contract: the watermark can never become durable while the record
    // it represents is still in a buffer. Worst case after a crash
    // between the two steps is a record re-appended next run (a harmless
    // duplicate), never a dropped crash.
    let mut block = String::new();
    for s in &summaries {
        block.push_str(&summary_line(s));
        block.push('\n');
    }
    append(crashes_log, &block)?;

    // The last name is the lexical max (vec is sorted) → new watermark,
    // written atomically so a crash mid-write leaves either the old or
    // the new value, never a torn file.
    if let Some(latest) = names.last() {
        write_state_atomic(state_path, latest)?;
    }

    Ok(summaries)
}

fn append(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(contents.as_bytes())?;
    // fsync the file so the record outlives the watermark written next;
    // fsync the parent dir so a newly-created crashes.log entry survives
    // power loss too, not just a process crash.
    f.sync_all()?;
    fsync_parent_dir(path)
}

/// Write `contents` to `path` atomically: write a sibling temp file,
/// fsync it, rename over the target, then fsync the parent directory.
/// Rename is atomic on the same filesystem, and the directory fsync
/// makes the rename itself durable, so a reader never sees a
/// half-written watermark and a power loss never resurrects the old one.
fn write_state_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    // Append ".tmp" to the full path (don't use `with_extension`, which
    // would clobber a dotfile name like `.crash-harvest-state`).
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    fsync_parent_dir(path)
}

/// fsync the directory containing `path` so a file creation or rename in
/// it survives power loss — a data fsync persists file *contents*, not
/// the directory entry that points at them.
///
/// Best-effort and a no-op where a directory handle can't be opened
/// (Windows won't hand one out via `File::open`). Production harvest is
/// macOS-only, but the helper stays cross-platform so the Linux test
/// suite exercises the real path.
fn fsync_parent_dir(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    // An empty parent (a bare relative filename) denotes the cwd;
    // opening "" fails, so there's nothing to sync.
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    match std::fs::File::open(parent) {
        Ok(dir) => dir.sync_all(),
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but structurally faithful `.ips`: a header line, then
    /// a body whose faulting thread is a named worker with a
    /// bare-offset top frame — exactly the shape of the v0.1.4x tray
    /// crashes.
    fn sample_ips() -> String {
        let header = r#"{"app_version":"0.1.45","timestamp":"2026-06-07 17:29:03.00 +0800"}"#;
        let body = r#"{
            "exception": {"type": "EXC_BREAKPOINT", "signal": "SIGTRAP"},
            "faultingThread": 1,
            "usedImages": [
                {"name": "AppKit"},
                {"name": "claudepot-tauri"}
            ],
            "threads": [
                {"queue": "com.apple.main-thread", "frames": []},
                {"name": "tokio-rt-worker", "frames": [{"imageIndex": 1, "imageOffset": 14332392}]}
            ]
        }"#;
        format!("{header}\n{body}")
    }

    #[test]
    fn parse_ips_extracts_header_and_faulting_thread() {
        let s = parse_ips("claudepot-tauri-2026-06-07-172903.ips", &sample_ips());
        assert_eq!(s.app_version.as_deref(), Some("0.1.45"));
        assert_eq!(s.exc_type.as_deref(), Some("EXC_BREAKPOINT"));
        assert_eq!(s.signal.as_deref(), Some("SIGTRAP"));
        assert_eq!(s.faulting_thread.as_deref(), Some("tokio-rt-worker"));
        assert_eq!(
            s.top_frame.as_deref(),
            Some("claudepot-tauri +14332392"),
            "bare-offset frame should name the image and offset"
        );
    }

    #[test]
    fn parse_ips_degrades_gracefully_on_garbage_body() {
        let s = parse_ips("x.ips", "not json\nstill not json");
        assert_eq!(s.file_name, "x.ips");
        assert!(s.exc_type.is_none());
        assert!(s.faulting_thread.is_none());
    }

    #[test]
    fn parse_ips_with_no_newline_keeps_only_file_name() {
        let s = parse_ips("solo.ips", "{}");
        assert_eq!(s.file_name, "solo.ips");
        assert!(s.app_version.is_none());
    }

    #[test]
    fn summary_line_omits_absent_fields() {
        let s = CrashSummary {
            file_name: "a.ips".to_string(),
            signal: Some("SIGABRT".to_string()),
            ..Default::default()
        };
        let line = summary_line(&s);
        assert!(line.contains("a.ips"));
        assert!(line.contains("SIGABRT"));
        assert!(!line.contains("None"), "must never print None: {line}");
        assert!(!line.contains("thread="), "absent thread omitted: {line}");
    }

    #[test]
    fn summary_line_collapses_embedded_newlines() {
        // A malformed report whose field carries a newline must not be
        // able to split the append-only log into two lines.
        let s = CrashSummary {
            file_name: "a.ips".to_string(),
            faulting_thread: Some("evil\nthread\r\nname".to_string()),
            ..Default::default()
        };
        let line = summary_line(&s);
        assert!(!line.contains('\n'), "no embedded newline: {line:?}");
        assert!(!line.contains('\r'), "no embedded CR: {line:?}");
        assert!(line.contains("evil thread  name"), "got: {line:?}");
    }

    #[test]
    fn harvest_reports_new_files_then_advances_watermark() {
        let tmp = tempfile::tempdir().unwrap();
        let reports = tmp.path().join("DiagnosticReports");
        std::fs::create_dir_all(&reports).unwrap();
        let crashes_log = tmp.path().join("crashes.log");
        let state = tmp.path().join(".state");

        std::fs::write(
            reports.join("claudepot-tauri-2026-06-05-143156.ips"),
            sample_ips(),
        )
        .unwrap();
        std::fs::write(
            reports.join("claudepot-tauri-2026-06-07-172903.ips"),
            sample_ips(),
        )
        .unwrap();
        // Unrelated process report — must be ignored.
        std::fs::write(reports.join("WindowServer-2026-06-07-000000.ips"), "x\n{}").unwrap();

        let first = harvest(&reports, "claudepot-tauri", &crashes_log, &state).unwrap();
        assert_eq!(first.len(), 2, "both claudepot reports are new on run 1");

        // crashes.log got two lines; watermark advanced to the latest.
        let logged = std::fs::read_to_string(&crashes_log).unwrap();
        assert_eq!(logged.lines().count(), 2);
        assert_eq!(
            std::fs::read_to_string(&state).unwrap(),
            "claudepot-tauri-2026-06-07-172903.ips"
        );

        // Run 2 with no new files → nothing reported, log unchanged.
        let second = harvest(&reports, "claudepot-tauri", &crashes_log, &state).unwrap();
        assert!(second.is_empty());
        assert_eq!(
            std::fs::read_to_string(&crashes_log)
                .unwrap()
                .lines()
                .count(),
            2
        );

        // A newer report shows up → only that one is harvested.
        std::fs::write(
            reports.join("claudepot-tauri-2026-06-08-080000.ips"),
            sample_ips(),
        )
        .unwrap();
        let third = harvest(&reports, "claudepot-tauri", &crashes_log, &state).unwrap();
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].file_name, "claudepot-tauri-2026-06-08-080000.ips");
    }

    #[test]
    fn harvest_missing_reports_dir_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let got = harvest(
            &tmp.path().join("does-not-exist"),
            "claudepot-tauri",
            &tmp.path().join("crashes.log"),
            &tmp.path().join(".state"),
        )
        .unwrap();
        assert!(got.is_empty());
    }
}
