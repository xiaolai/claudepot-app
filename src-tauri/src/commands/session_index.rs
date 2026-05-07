//! Tauri commands for the Sessions tab and the session debugger.
//!
//! Read-only surface: list, transcript, chunks, context, export,
//! search, worktree grouping. No state; each call hits disk via
//! `claudepot_core::session*`. Handlers are `async fn` so Tauri
//! dispatches them off the main thread — a sync handler that scans
//! thousands of JSONL files would freeze the webview for seconds.

use claudepot_core::paths;

fn join_blocking_err(e: tokio::task::JoinError) -> String {
    format!("blocking task failed: {e}")
}

// ---------------------------------------------------------------------------
// Session index — Sessions tab list + per-session detail (transcript).
// ---------------------------------------------------------------------------

/// Walk `<config>/projects/*/*.jsonl` and produce rich list rows with
/// token totals, first-prompt previews, and model sets. Returned
/// newest-first.
///
/// The body runs in `tokio::task::spawn_blocking` so the SQLite read
/// (and full JSONL fallback when the cache is cold) can't starve the
/// Tauri IPC worker pool. Async-fn alone keeps the call off Tauri's
/// main thread, but the worker pool itself is shared across every
/// async command — letting a multi-second scan park there serializes
/// everything else (audit B8 commands_session_index.rs:28).
#[tauri::command]
pub async fn session_list_all() -> Result<Vec<crate::dto::SessionRowDto>, String> {
    tokio::task::spawn_blocking(|| {
        let cfg = paths::claude_config_dir();
        let rows = claudepot_core::session::list_all_sessions(&cfg)
            .map_err(|e| format!("session list failed: {e}"))?;
        Ok::<_, String>(rows.iter().map(crate::dto::SessionRowDto::from).collect())
    })
    .await
    .map_err(join_blocking_err)?
}

/// Full JSONL parse for a single session, keyed by its UUID. Returns
/// the same row metadata as `session_list_all` plus the normalized
/// event stream for transcript rendering.
///
/// Wrapped in `spawn_blocking` for the same reason as `session_list_all`.
#[tauri::command]
pub async fn session_read(session_id: String) -> Result<crate::dto::SessionDetailDto, String> {
    tokio::task::spawn_blocking(move || {
        let cfg = paths::claude_config_dir();
        let detail = claudepot_core::session::read_session_detail(&cfg, &session_id)
            .map_err(|e| format!("session read failed: {e}"))?;
        Ok::<_, String>(crate::dto::SessionDetailDto::from(&detail))
    })
    .await
    .map_err(join_blocking_err)?
}

/// Full JSONL parse keyed by the transcript's on-disk path. Preferred
/// over `session_read` from the GUI because list rows point at a
/// specific file and two rows can legitimately share a session_id
/// (interrupted rescue or adopt). Path must live under
/// `<config>/projects/` and must end in `.jsonl`.
///
/// Wrapped in `spawn_blocking` for the same reason as `session_list_all`.
#[tauri::command]
pub async fn session_read_path(file_path: String) -> Result<crate::dto::SessionDetailDto, String> {
    tokio::task::spawn_blocking(move || {
        let cfg = paths::claude_config_dir();
        let detail = claudepot_core::session::read_session_detail_at_path(
            &cfg,
            std::path::Path::new(&file_path),
        )
        .map_err(|e| format!("session read failed: {e}"))?;
        Ok::<_, String>(crate::dto::SessionDetailDto::from(&detail))
    })
    .await
    .map_err(join_blocking_err)?
}

/// Drop every cached row in `sessions.db` and repopulate from disk.
/// The (size, mtime_ns) guard handles ~every realistic transcript
/// edit; this is the escape hatch for filesystems with coarse mtime
/// resolution, clock skew, or anything that defeats the guard. The
/// next `session_list_all` call re-scans everything from cold.
///
/// Wrapped in `spawn_blocking` — full rebuild scans every JSONL.
#[tauri::command]
pub async fn session_index_rebuild() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        let data_dir = paths::claudepot_data_dir();
        let db_path = data_dir.join("sessions.db");
        let idx = claudepot_core::session_index::SessionIndex::open(&db_path)
            .map_err(|e| format!("open session index: {e}"))?;
        idx.rebuild()
            .map_err(|e| format!("rebuild session index: {e}"))
    })
    .await
    .map_err(join_blocking_err)?
}

// ---------------------------------------------------------------------------
// Session debugger — chunks, linked tools, subagents, phases, context,
// export, search, worktree grouping. All read-only.
// ---------------------------------------------------------------------------

/// Chunked event stream plus per-chunk linked tools — the shape the
/// Sessions transcript renders from.
///
/// Wrapped in `spawn_blocking` — `load_detail_by_path` parses the full
/// JSONL synchronously.
#[tauri::command]
pub async fn session_chunks(file_path: String) -> Result<Vec<crate::dto::SessionChunkDto>, String> {
    tokio::task::spawn_blocking(move || {
        let detail = load_detail_by_path(&file_path)?;
        let chunks = claudepot_core::session_chunks::build_chunks(&detail.events);
        Ok::<_, String>(
            chunks
                .iter()
                .map(crate::dto::SessionChunkDto::from)
                .collect(),
        )
    })
    .await
    .map_err(join_blocking_err)?
}

/// Visible-context token attribution across six categories.
///
/// Wrapped in `spawn_blocking` — same reason as `session_chunks`.
#[tauri::command]
pub async fn session_context_attribution(
    file_path: String,
) -> Result<crate::dto::ContextStatsDto, String> {
    tokio::task::spawn_blocking(move || {
        let detail = load_detail_by_path(&file_path)?;
        let stats = claudepot_core::session_context::attribute_context(&detail.events);
        Ok::<_, String>((&stats).into())
    })
    .await
    .map_err(join_blocking_err)?
}

/// Export transcript to Markdown or JSON (sk-ant-* redacted). Kept as
/// an internal helper for `session_export_to_file` — not exposed
/// separately until the UI has a "copy to clipboard" flow that needs
/// the raw body.
fn session_export_text(file_path: String, format: String) -> Result<String, String> {
    let detail = load_detail_by_path(&file_path)?;
    let fmt = match format.as_str() {
        "md" | "markdown" => claudepot_core::session_export::ExportFormat::Markdown,
        "json" => claudepot_core::session_export::ExportFormat::Json,
        other => return Err(format!("unknown format: {other}")),
    };
    Ok(claudepot_core::session_export::export(&detail, fmt))
}

/// Export transcript directly to disk. The UI hands us an absolute
/// path chosen by the user via the native save dialog; we validate,
/// then create the file atomically with restrictive permissions.
///
/// Boundary checks:
/// * `output_path` must be absolute and may not contain any `..`
///   component (defence against UI-side bugs that would allow a
///   compromised webview to write outside the user's chosen dir).
/// * The file is created with `CREATE | TRUNCATE` and — on Unix —
///   an O_NOFOLLOW-like intent enforced by `OpenOptions.mode(0o600)`
///   *before* any bytes are written, so the window where the file
///   could be world-readable is closed.
/// * A pre-existing symlink at `output_path` is refused; if the user
///   really wants to overwrite a symlink target they can delete it
///   first.
/// * Chmod failure after the fact is treated as fatal (we'd otherwise
///   fail open on a filesystem that silently ignored the mode bits).
#[tauri::command]
pub async fn session_export_to_file(
    file_path: String,
    format: String,
    output_path: String,
) -> Result<usize, String> {
    // Wrapped in `spawn_blocking` — JSONL parse, redaction, atomic
    // write, and chmod are all sync and would otherwise hold the IPC
    // worker for the whole export (audit B8 commands_session_index.rs:146).
    tokio::task::spawn_blocking(move || -> Result<usize, String> {
        session_export_to_file_sync(file_path, format, output_path)
    })
    .await
    .map_err(join_blocking_err)?
}

fn session_export_to_file_sync(
    file_path: String,
    format: String,
    output_path: String,
) -> Result<usize, String> {
    let output = std::path::Path::new(&output_path);
    if !output.is_absolute() {
        return Err(format!("output path must be absolute: {output_path}"));
    }
    if output
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("output path must not contain `..`: {output_path}"));
    }
    // Refuse to overwrite a symlink — the user's chosen filesystem
    // might resolve to somewhere unexpected under our permissions.
    match std::fs::symlink_metadata(output) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!("refusing to overwrite symlink: {output_path}"));
        }
        _ => {}
    }

    let body = session_export_text(file_path, format)?;

    // Atomic write: render into a sibling temp file, fsync, then
    // rename into place. On Unix `rename(2)` is atomic within the same
    // filesystem. If we crash mid-write the user still sees the
    // previous file (or no file) — never a half-written transcript.
    let parent = output
        .parent()
        .ok_or_else(|| format!("output has no parent directory: {output_path}"))?;
    let final_name = output
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("output has no filename: {output_path}"))?;

    // Unique per-call suffix so concurrent exports don't stomp each other.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(".{final_name}.claudepot-tmp-{nonce}"));

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // create_new + mode == file is born with 0600 on filesystems
        // that honor it. Filesystems that silently ignore mode still
        // benefit from the `unreadable-until-rename` property via umask,
        // and the post-write chmod fallback below catches the rest.
        opts.mode(0o600);
    }
    let mut file = opts
        .open(&tmp_path)
        .map_err(|e| format!("open tmp {}: {e}", tmp_path.display()))?;

    use std::io::Write as _;
    if let Err(e) = (|| -> std::io::Result<()> {
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })() {
        // Best-effort cleanup; ignore secondary errors.
        drop(file);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("write tmp {}: {e}", tmp_path.display()));
    }
    drop(file);

    // Belt-and-braces permission check before the rename. If we can't
    // enforce 0600, delete the tmp file and refuse the export.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&tmp_path).map_err(|e| format!("stat tmp: {e}"))?;
        if meta.permissions().mode() & 0o077 != 0 {
            if let Err(e) =
                std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
            {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(format!("chmod tmp: {e}"));
            }
            let mode2 = std::fs::metadata(&tmp_path)
                .map_err(|e| format!("re-stat tmp: {e}"))?
                .permissions()
                .mode();
            if mode2 & 0o077 != 0 {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(format!(
                    "filesystem does not enforce 0600 permissions at {output_path}"
                ));
            }
        }
    }

    // Rename into place. Atomic on POSIX when src + dst are on the
    // same filesystem; Windows' `rename` is also atomic per MSFT docs
    // on the same volume. We prepared `tmp_path` in `parent`, so this
    // is always same-filesystem.
    if let Err(e) = std::fs::rename(&tmp_path, output) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("rename into {output_path}: {e}"));
    }

    Ok(body.len())
}

/// Cross-session text search. Returns up to `limit` hits.
///
/// Wrapped in `spawn_blocking` — the body opens every `.jsonl` that
/// doesn't match via the row-level fast path and scans line by line.
/// For a multi-thousand-session corpus this is many seconds of pure
/// blocking I/O, which would otherwise pin a Tauri IPC worker.
#[tauri::command]
pub async fn session_search(
    query: String,
    limit: Option<usize>,
) -> Result<Vec<crate::dto::SearchHitDto>, String> {
    tokio::task::spawn_blocking(move || {
        let cfg = paths::claude_config_dir();
        let rows = claudepot_core::session::list_all_sessions(&cfg)
            .map_err(|e| format!("list sessions: {e}"))?;
        let hits = claudepot_core::session_search::search_rows(&rows, &query, limit.unwrap_or(25))
            .map_err(|e| format!("search sessions: {e}"))?;
        Ok::<_, String>(hits.iter().map(crate::dto::SearchHitDto::from).collect())
    })
    .await
    .map_err(join_blocking_err)?
}

/// Group all sessions by git repository (collapses worktrees into a
/// single repository row).
///
/// Wrapped in `spawn_blocking` for the same reason as `session_list_all`.
#[tauri::command]
pub async fn session_worktree_groups() -> Result<Vec<crate::dto::RepositoryGroupDto>, String> {
    tokio::task::spawn_blocking(|| {
        let cfg = paths::claude_config_dir();
        let rows = claudepot_core::session::list_all_sessions(&cfg)
            .map_err(|e| format!("list sessions: {e}"))?;
        let groups = claudepot_core::session_worktree::group_by_repo(rows);
        Ok::<_, String>(
            groups
                .iter()
                .map(crate::dto::RepositoryGroupDto::from)
                .collect(),
        )
    })
    .await
    .map_err(join_blocking_err)?
}

pub(crate) fn load_detail_by_path(
    file_path: &str,
) -> Result<claudepot_core::session::SessionDetail, String> {
    let cfg = paths::claude_config_dir();
    claudepot_core::session::read_session_detail_at_path(&cfg, std::path::Path::new(file_path))
        .map_err(|e| format!("session read failed: {e}"))
}
