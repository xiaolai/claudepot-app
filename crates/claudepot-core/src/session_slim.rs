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
//! Optionally, `strip_images` / `strip_documents` replace base64
//! `image` / `document` blocks with text stubs, mirroring CC's own
//! `stripImagesFromMessages` transform (compact.ts:145). Each image
//! is roughly 2000 tokens on resume, so stripping them from closed
//! sessions removes the resume-time re-upload cost.
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
    #[error("empty filter — at least one criterion must be set for --all")]
    EmptyFilter,
    #[error("listing sessions failed: {0}")]
    Session(#[from] crate::session::SessionError),
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
    /// Replace `image` blocks with `{"type":"text","text":"[image]"}`.
    /// Mirrors CC's own `stripImagesFromMessages` transform; the stub
    /// keeps the enclosing message's UUID chain intact so `--resume`
    /// loads cleanly without re-uploading ~2000 tokens per image.
    pub strip_images: bool,
    /// Replace `document` blocks with `[document]` stubs, analogous
    /// to `strip_images`.
    pub strip_documents: bool,
}

impl Default for SlimOpts {
    fn default() -> Self {
        Self {
            drop_tool_results_over_bytes: 1 << 20, // 1 MiB
            exclude_tools: Vec::new(),
            strip_images: false,
            strip_documents: false,
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
    /// Number of image blocks that will be replaced with text stubs.
    #[serde(default)]
    pub image_redact_count: u32,
    /// Number of document blocks that will be replaced with text stubs.
    #[serde(default)]
    pub document_redact_count: u32,
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
    #[serde(default)]
    pub image_redact_count: u32,
    #[serde(default)]
    pub document_redact_count: u32,
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
    let mut image_count = 0u32;
    let mut document_count = 0u32;
    let mut tools: Vec<String> = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| SlimError::io(path, e))?;
        if line.is_empty() {
            projected += 1; // newline
            continue;
        }
        let mut v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| SlimError::Json { line: i, source: e })?;
        let (new_line, line_stats) = rewrite_line(&mut v, opts);
        redact_count += line_stats.redacted_here;
        image_count += line_stats.images_here;
        document_count += line_stats.documents_here;
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
        image_redact_count: image_count,
        document_redact_count: document_count,
        tools_affected: tools,
    })
}

/// RAII-style cleanup for throwaway files we need to remove on any
/// error path. `disarm()` cancels the cleanup once the file has been
/// successfully renamed or moved away.
struct FileGuard {
    path: Option<PathBuf>,
}
impl FileGuard {
    fn new(p: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(p.into()),
        }
    }
    fn disarm(&mut self) {
        self.path = None;
    }
}
impl Drop for FileGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = fs::remove_file(&p);
        }
    }
}

/// Rewrite the file. Caller must pass a `data_dir` for trash placement
/// of the pre-slim snapshot. Live-write guard aborts if the source
/// changed between the initial scan and the atomic rename; a second
/// re-stat runs immediately before the rename as defense-in-depth
/// against the TOCTOU window.
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
    // Cleanup guard for the tmp rewrite. Disarmed right before rename.
    let mut tmp_guard = FileGuard::new(tmp_path.clone());

    sink.phase("rewriting", PhaseStatus::Running);
    let f = fs::File::open(path).map_err(|e| SlimError::io(path, e))?;
    let reader = BufReader::new(f);
    let mut tmp = fs::File::create(&tmp_path).map_err(|e| SlimError::io(&tmp_path, e))?;

    let mut redact_count = 0u32;
    let mut image_count = 0u32;
    let mut document_count = 0u32;
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| SlimError::io(path, e))?;
        if line.is_empty() {
            writeln!(tmp).map_err(|e| SlimError::io(&tmp_path, e))?;
            continue;
        }
        let mut v: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| SlimError::Json { line: i, source: e })?;
        let (new_line, stats) = rewrite_line(&mut v, opts);
        redact_count += stats.redacted_here;
        image_count += stats.images_here;
        document_count += stats.documents_here;
        writeln!(tmp, "{new_line}").map_err(|e| SlimError::io(&tmp_path, e))?;
    }
    tmp.sync_all().map_err(|e| SlimError::io(&tmp_path, e))?;
    drop(tmp);

    sink.phase("guarding", PhaseStatus::Running);
    let after = fs::metadata(path).map_err(|e| SlimError::io(path, e))?;
    if after.len() != before_size
        || !same_mtime(
            before_mtime,
            after.modified().map_err(|e| SlimError::io(path, e))?,
        )
    {
        return Err(SlimError::LiveWriteDetected);
    }

    sink.phase("trashing-original", PhaseStatus::Running);
    // Snapshot the unmodified original to a sibling file, then hand it
    // to trash::write. `restore_path` is set to the real session
    // path so `trash::restore` puts bytes back where they came from —
    // without this, restore would recreate the `.pre-slim.jsonl`
    // temp name instead of the real session.
    let snapshot = tmp_path.with_extension("pre-slim.jsonl");
    let mut snap_guard = FileGuard::new(snapshot.clone());
    fs::copy(path, &snapshot).map_err(|e| SlimError::io(&snapshot, e))?;
    let entry = trash::write(
        data_dir,
        TrashPut {
            orig_path: &snapshot,
            restore_path: Some(path),
            kind: TrashKind::Slim,
            cwd: path.parent(),
            reason: Some(format!(
                "pre-slim snapshot of {}",
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            )),
        },
    )?;
    // trash::write moved the snapshot into the batch dir, so the
    // sibling file no longer exists — disarm the guard.
    snap_guard.disarm();

    sink.phase("swapping", PhaseStatus::Running);
    // Second re-stat immediately before rename. Narrows the TOCTOU
    // window between the first guard and the atomic swap. If CC
    // snuck in an append, bail — the snapshot in trash is still the
    // correct pre-slim state and can be restored via `trash restore`.
    let after2 = fs::metadata(path).map_err(|e| SlimError::io(path, e))?;
    if after2.len() != before_size
        || !same_mtime(
            before_mtime,
            after2.modified().map_err(|e| SlimError::io(path, e))?,
        )
    {
        return Err(SlimError::LiveWriteDetected);
    }
    fs::rename(&tmp_path, path).map_err(|e| SlimError::io(path, e))?;
    // Original path now has the new content; tmp is gone.
    tmp_guard.disarm();

    let after_final = fs::metadata(path).map_err(|e| SlimError::io(path, e))?;
    sink.phase("complete", PhaseStatus::Complete);
    Ok(SlimReport {
        original_bytes: before_size,
        final_bytes: after_final.len(),
        redact_count,
        image_redact_count: image_count,
        document_redact_count: document_count,
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
    images_here: u32,
    documents_here: u32,
    tools_here: Vec<String>,
}

/// Replace the given block in place with a `{type:"text",text:<stub>}`
/// and return `true` if the block was a media block of the named kind
/// and stripping was requested. `kind` is `"image"` or `"document"`.
fn maybe_strip_media_block(
    block: &mut serde_json::Value,
    strip: bool,
    kind: &str,
    stub: &str,
) -> bool {
    if !strip {
        return false;
    }
    if block.get("type").and_then(|t| t.as_str()) != Some(kind) {
        return false;
    }
    *block = serde_json::json!({ "type": "text", "text": stub });
    true
}

/// Rewrite a single parsed line in place. Returns the serialized
/// replacement plus per-line statistics.
fn rewrite_line(v: &mut serde_json::Value, opts: &SlimOpts) -> (String, LineStats) {
    let mut stats = LineStats {
        redacted_here: 0,
        images_here: 0,
        documents_here: 0,
        tools_here: Vec::new(),
    };
    // Only user messages carry tool_result parts, images, or documents
    // in CC's format.
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
        // Top-level image / document blocks in message.content[*].
        if maybe_strip_media_block(part, opts.strip_images, "image", "[image]") {
            stats.images_here += 1;
            continue;
        }
        if maybe_strip_media_block(part, opts.strip_documents, "document", "[document]") {
            stats.documents_here += 1;
            continue;
        }

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
            // Excluded tool: the whole tool_result is preserved
            // verbatim, including any nested images/documents.
            continue;
        }
        // Raw size of the part serialized.
        let raw = serde_json::to_string(part).unwrap_or_default();
        let raw_len = raw.len() as u64;
        if raw_len > opts.drop_tool_results_over_bytes {
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
            continue;
        }
        // Tool result stays, but its nested content may hold images
        // or documents we were asked to strip.
        if let Some(inner) = part.get_mut("content").and_then(|c| c.as_array_mut()) {
            for item in inner.iter_mut() {
                if maybe_strip_media_block(item, opts.strip_images, "image", "[image]") {
                    stats.images_here += 1;
                } else if maybe_strip_media_block(
                    item,
                    opts.strip_documents,
                    "document",
                    "[document]",
                ) {
                    stats.documents_here += 1;
                }
            }
        }
    }
    (serde_json::to_string(v).unwrap_or_default(), stats)
}

// ---------------------------------------------------------------------------
// Bulk slim — filter-driven over the whole session index.
// ---------------------------------------------------------------------------

/// One row of a bulk plan. Carries the filter-row identity plus the
/// per-file `SlimPlan` so the CLI/GUI can show per-session projections
/// before execute.
#[derive(Debug, Clone, Serialize)]
pub struct BulkSlimEntry {
    pub session_id: String,
    pub file_path: PathBuf,
    pub project_path: String,
    pub plan: SlimPlan,
}

/// Plan for a bulk `session slim --all`. Rows sorted by projected
/// bytes saved (descending). Entries are only the matched files where
/// slim would actually change bytes or redact content — matched rows
/// whose slim would be a pure no-op are dropped here, not at execute
/// time, so the dry-run preview accurately reflects what Execute will
/// touch.
#[derive(Debug, Clone, Serialize)]
pub struct BulkSlimPlan {
    pub entries: Vec<BulkSlimEntry>,
    /// Matched files whose `plan_slim()` call itself errored (not
    /// found, I/O, JSON parse at scan). Surfaced so one unreadable
    /// row does not disappear from the report.
    pub failed_to_plan: Vec<(PathBuf, String)>,
    pub total_bytes_saved: u64,
    pub total_image_redacts: u32,
    pub total_document_redacts: u32,
    pub total_tool_result_redacts: u32,
}

/// Bulk execute outcome. `skipped_live` is its own bucket: those are
/// not failures — the session was being written to and slim bailed
/// cleanly. A retry after the session goes idle will pick them up.
#[derive(Debug, Clone, Serialize)]
pub struct BulkSlimReport {
    pub succeeded: Vec<(PathBuf, SlimReport)>,
    pub skipped_live: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub total_bytes_saved: u64,
    pub total_image_redacts: u32,
    pub total_document_redacts: u32,
    pub total_tool_result_redacts: u32,
}

/// Pure plan builder given a pre-scanned row list. `filter.validate()`
/// still applies — bulk slim refuses to run on an empty filter for
/// the same reason bulk prune does: a user almost certainly did not
/// mean "every session".
pub fn plan_slim_all_from_rows(
    rows: &[crate::session::SessionRow],
    filter: &crate::session_prune::PruneFilter,
    opts: &SlimOpts,
    now_ms: i64,
) -> Result<BulkSlimPlan, SlimError> {
    filter.validate().map_err(|_| SlimError::EmptyFilter)?;
    let mut entries: Vec<BulkSlimEntry> = Vec::new();
    let mut failed_to_plan: Vec<(PathBuf, String)> = Vec::new();
    for row in rows.iter().filter(|r| filter.matches(r, now_ms)) {
        match plan_slim(&row.file_path, opts) {
            Ok(plan) => {
                // Skip matched rows where slim would be a no-op —
                // no bytes to save, no blocks to redact. Execute
                // would rewrite them anyway (churning mtime and
                // producing empty trash entries), and the user sees
                // a noisy preview listing files that didn't need to
                // be there.
                let has_effect = plan.bytes_saved() > 0
                    || plan.redact_count > 0
                    || plan.image_redact_count > 0
                    || plan.document_redact_count > 0;
                if !has_effect {
                    continue;
                }
                entries.push(BulkSlimEntry {
                    session_id: row.session_id.clone(),
                    file_path: row.file_path.clone(),
                    project_path: row.project_path.clone(),
                    plan,
                });
            }
            Err(e) => {
                // Don't disappear unreadable rows — the caller can
                // still see them in the report and decide.
                failed_to_plan.push((row.file_path.clone(), e.to_string()));
            }
        }
    }
    // Sort by projected bytes-saved descending. Biggest wins first.
    entries.sort_by(|a, b| b.plan.bytes_saved().cmp(&a.plan.bytes_saved()));
    let total_bytes_saved = entries.iter().map(|e| e.plan.bytes_saved()).sum();
    let total_image_redacts = entries.iter().map(|e| e.plan.image_redact_count).sum();
    let total_document_redacts = entries.iter().map(|e| e.plan.document_redact_count).sum();
    let total_tool_result_redacts = entries.iter().map(|e| e.plan.redact_count).sum();
    Ok(BulkSlimPlan {
        entries,
        failed_to_plan,
        total_bytes_saved,
        total_image_redacts,
        total_document_redacts,
        total_tool_result_redacts,
    })
}

/// Scan the session index and build a bulk plan. Touches disk only
/// to list sessions and to size each candidate file.
pub fn plan_slim_all(
    config_dir: &Path,
    filter: &crate::session_prune::PruneFilter,
    opts: &SlimOpts,
) -> Result<BulkSlimPlan, SlimError> {
    let rows = crate::session::list_all_sessions(config_dir).map_err(SlimError::Session)?;
    plan_slim_all_from_rows(&rows, filter, opts, chrono::Utc::now().timestamp_millis())
}

/// Execute a bulk plan. One file at a time, failures collected
/// per-file so one bad (or still-live) session does not abort the
/// batch.
pub fn execute_slim_all(
    data_dir: &Path,
    plan: &BulkSlimPlan,
    opts: &SlimOpts,
    sink: &dyn ProgressSink,
) -> BulkSlimReport {
    sink.phase("plan-validated", PhaseStatus::Complete);
    let total = plan.entries.len();
    let mut succeeded: Vec<(PathBuf, SlimReport)> = Vec::new();
    let mut skipped_live: Vec<PathBuf> = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = Vec::new();
    let mut bytes_saved: u64 = 0;
    let mut images: u32 = 0;
    let mut documents: u32 = 0;
    let mut tool_result_redacts: u32 = 0;
    for (i, e) in plan.entries.iter().enumerate() {
        sink.sub_progress("slimming", i, total);
        match execute_slim(
            data_dir,
            &e.file_path,
            opts,
            &crate::project_progress::NoopSink,
        ) {
            Ok(r) => {
                bytes_saved = bytes_saved.saturating_add(r.bytes_saved());
                images = images.saturating_add(r.image_redact_count);
                documents = documents.saturating_add(r.document_redact_count);
                tool_result_redacts = tool_result_redacts.saturating_add(r.redact_count);
                succeeded.push((e.file_path.clone(), r));
            }
            Err(SlimError::LiveWriteDetected) => {
                skipped_live.push(e.file_path.clone());
            }
            Err(err) => {
                failed.push((e.file_path.clone(), err.to_string()));
            }
        }
    }
    sink.sub_progress("slimming", total, total);
    sink.phase("complete", PhaseStatus::Complete);
    BulkSlimReport {
        succeeded,
        skipped_live,
        failed,
        total_bytes_saved: bytes_saved,
        total_image_redacts: images,
        total_document_redacts: documents,
        total_tool_result_redacts: tool_result_redacts,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "session_slim_tests.rs"]
mod tests;
