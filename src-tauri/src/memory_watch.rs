//! Long-running filesystem watcher for memory file changes.
//!
//! Watches `~/.claude/` recursively, filtering events down to:
//! - `~/.claude/CLAUDE.md` (the global CLAUDE.md)
//! - `~/.claude/projects/<slug>/memory/**/*.md` (auto-memory)
//!
//! On every debounced event, fetches the previous content (last
//! recorded snapshot held in process memory; bootstrapped from
//! `memory_changes.db`'s most recent row on first sight) and the
//! current content, computes the diff, and writes a new
//! `memory_changes` row. Emits `memory:changed` on the Tauri event
//! bus so the MemoryPane can refresh.
//!
//! Project `<repo>/CLAUDE.md` and `<repo>/.claude/CLAUDE.md` are NOT
//! watched here in v1 — those files change infrequently and would
//! require maintaining a list of registered project roots that the
//! watcher tracks dynamically. The MemoryPane refreshes on open and
//! on `memory:changed`, so users see current content; only the
//! change-log entry for project-CLAUDE.md edits is missed in v1.

use claudepot_core::memory_log::{ChangeType, MemoryLog, RecordInput};
use claudepot_core::memory_view::{classify_path_for_watcher, discover_project_roots_from_slugs};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, Debouncer};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(250);

/// How often to rescan `~/.claude/projects/` for newly added projects
/// (audit 2026-05 #5 follow-up). Projects added by CC during a
/// running Claudepot session get picked up here without requiring a
/// restart. 30 s is a balance between latency and rescan cost; the
/// existing recursive `~/.claude/` watch already covers auto-memory
/// edits in real time, so this is only catching project CLAUDE.md
/// candidates that need explicit per-file watches.
const PROJECT_RESCAN_INTERVAL: Duration = Duration::from_secs(30);

/// Maximum cached file content in the watcher fingerprint map. Mirrors
/// `MemoryLog::MAX_DIFF_FILE_BYTES` so a file that's too large to log
/// a diff for is also too large to keep in the fingerprint cache —
/// we still track its hash, just not the bytes (audit 2026-05 #8).
const MAX_FINGERPRINT_BYTES: usize = 256 * 1024;

/// Spawn the watcher. Builds a `notify-debouncer-mini` `Debouncer`
/// inline, attaches the recursive `~/.claude/` watch + initial
/// per-project CLAUDE.md watches, then hands the `Debouncer` to a
/// long-running async task that owns it for the process lifetime.
///
/// The async task drives a `tokio::select!` between filesystem events
/// (via the unbounded mpsc channel) and a periodic project rescan, so
/// projects added at runtime get picked up without a restart (audit
/// 2026-05 #5 follow-up).
pub fn spawn(app: AppHandle, log: Arc<MemoryLog>) {
    let claude_dir = claudepot_core::paths::claude_config_dir();
    if !claude_dir.exists() {
        tracing::info!(
            "memory_watch: {} does not exist yet; skipping watcher startup. \
             Open Settings → Cleanup to set up CC, then restart Claudepot to enable.",
            claude_dir.display()
        );
        return;
    }

    let (tx, rx) = mpsc::unbounded_channel::<WatcherEvent>();

    let mut debouncer: Debouncer<RecommendedWatcher> =
        match new_debouncer(DEBOUNCE_WINDOW, move |res| {
            // The notify callback runs on notify's internal worker
            // thread; sending into a tokio mpsc is fine from any
            // thread (the channel itself is sync-safe).
            let _ = tx.send(WatcherEvent::Debounced(res));
        }) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("memory_watch: failed to build debouncer: {e}");
                return;
            }
        };

    if let Err(e) = debouncer
        .watcher()
        .watch(&claude_dir, RecursiveMode::Recursive)
    {
        tracing::warn!(
            "memory_watch: failed to watch {}: {e}",
            claude_dir.display()
        );
        return;
    }

    // Initial set of project roots (matches what we watch right now).
    let initial_roots = discover_project_roots_from_slugs();
    let mut watched_files: HashSet<PathBuf> = HashSet::new();
    for root in &initial_roots {
        attach_project_watches(&mut debouncer, root, &mut watched_files);
    }

    tauri::async_runtime::spawn(async move {
        run_event_loop(app, log, rx, debouncer, initial_roots, watched_files).await;
    });
}

/// Attach per-file watches for `<R>/CLAUDE.md` and `<R>/.claude/CLAUDE.md`
/// when they exist, recording each successful watch in `watched_files`
/// so the rescan loop never re-watches the same file.
fn attach_project_watches(
    debouncer: &mut Debouncer<RecommendedWatcher>,
    root: &Path,
    watched_files: &mut HashSet<PathBuf>,
) {
    for candidate in [
        root.join("CLAUDE.md"),
        root.join(".claude").join("CLAUDE.md"),
    ] {
        if watched_files.contains(&candidate) {
            continue;
        }
        if !candidate.exists() {
            continue;
        }
        match debouncer
            .watcher()
            .watch(&candidate, RecursiveMode::NonRecursive)
        {
            Ok(()) => {
                watched_files.insert(candidate);
            }
            Err(e) => {
                tracing::debug!("memory_watch: skip watch on {}: {e}", candidate.display());
            }
        }
    }
}

enum WatcherEvent {
    Debounced(Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>),
}

#[derive(Clone, Debug)]
struct Fingerprint {
    size: u64,
    sha256: String,
    bytes: Vec<u8>,
}

async fn run_event_loop(
    app: AppHandle,
    log: Arc<MemoryLog>,
    mut rx: mpsc::UnboundedReceiver<WatcherEvent>,
    mut debouncer: Debouncer<RecommendedWatcher>,
    initial_roots: Vec<PathBuf>,
    mut watched_files: HashSet<PathBuf>,
) {
    let mut prints: HashMap<PathBuf, Fingerprint> = HashMap::new();
    let mut project_roots: Vec<PathBuf> = initial_roots;

    // Periodic rescan ticker. The first tick fires immediately by
    // default; skip it so we don't re-do the work `spawn` already did.
    let mut rescan = tokio::time::interval(PROJECT_RESCAN_INTERVAL);
    rescan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    rescan.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            ev = rx.recv() => {
                let Some(ev) = ev else {
                    // Channel closed — debouncer dropped or sender
                    // gone. Exit the loop; the task ends cleanly.
                    break;
                };
                let WatcherEvent::Debounced(result) = ev;
                let events = match result {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("memory_watch: notify error: {e}");
                        continue;
                    }
                };
                for ev in events {
                    handle_one(&app, &log, &mut prints, &project_roots, &ev.path, ev.kind).await;
                }
            }
            _ = rescan.tick() => {
                // Re-discover projects so newly-created ones get
                // their CLAUDE.md candidates watched. Diff against
                // the existing set and only attach new watches.
                let next_roots = discover_project_roots_from_slugs();
                let known: HashSet<&PathBuf> = project_roots.iter().collect();
                for root in &next_roots {
                    if known.contains(root) {
                        continue;
                    }
                    attach_project_watches(&mut debouncer, root, &mut watched_files);
                }
                project_roots = next_roots;
            }
        }
    }
}

async fn handle_one(
    app: &AppHandle,
    log: &Arc<MemoryLog>,
    prints: &mut HashMap<PathBuf, Fingerprint>,
    project_roots: &[PathBuf],
    path: &Path,
    kind: notify_debouncer_mini::DebouncedEventKind,
) {
    let Some((role, slug)) = classify_path_for_watcher(path, project_roots) else {
        return;
    };

    let exists = path.is_file();
    let new_bytes = if exists {
        std::fs::read(path).ok()
    } else {
        None
    };
    let prev = prints.get(path).cloned();

    // Bootstrap: if we don't have a fingerprint for this path, try
    // pulling the most recent log entry's hash AND read the current
    // file content so the very next event has a baseline to diff
    // against (audit 2026-05 #6: pre-fix, the first post-restart
    // change always logged with diff_omit_reason=Endpoint because
    // bytes were empty).
    let prev = match prev {
        Some(p) => Some(p),
        None => log.latest_for_path(path).ok().flatten().map(|c| {
            // Re-read current bytes so we have a real baseline. If the
            // file is missing or too large, fall back to hash-only.
            let bytes = if path.is_file() {
                match std::fs::read(path) {
                    Ok(b) if b.len() <= MAX_FINGERPRINT_BYTES => b,
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            Fingerprint {
                size: c.size_after.unwrap_or(0).max(0) as u64,
                sha256: c.hash_after.unwrap_or_default(),
                bytes,
            }
        }),
    };

    let change_type = match (prev.is_some(), exists) {
        (false, true) => ChangeType::Created,
        (true, true) => ChangeType::Modified,
        (true, false) => ChangeType::Deleted,
        (false, false) => return, // ghost event — never seen, doesn't exist
    };

    // Skip if size + sha match what we already stored. Cheap
    // dedup against notify firing on metadata-only writes.
    if let (Some(prev_fp), Some(bytes)) = (&prev, &new_bytes) {
        let current_sha = sha256_hex(bytes);
        if prev_fp.size == bytes.len() as u64 && prev_fp.sha256 == current_sha {
            // Refresh fingerprint cache. Audit 2026-05 #8: bound the
            // bytes payload so the cache can't grow unbounded across a
            // long-running session with many large memory files.
            prints.insert(path.to_path_buf(), make_fingerprint(bytes, current_sha));
            return;
        }
    }

    let mtime_ns_v = mtime_ns(path);
    let before_bytes = prev
        .as_ref()
        .map(|p| p.bytes.as_slice())
        .filter(|b| !b.is_empty());
    let after_bytes = new_bytes.as_deref();

    if let Err(e) = log.record(&RecordInput {
        project_slug: slug.as_deref(),
        abs_path: path,
        role,
        change_type,
        mtime_ns: mtime_ns_v,
        before: before_bytes,
        after: after_bytes,
    }) {
        tracing::warn!(
            "memory_watch: log record failed for {}: {e}",
            path.display()
        );
        return;
    }

    // Update fingerprint cache. Audit 2026-05 #8: bound bytes payload.
    match (exists, new_bytes) {
        (true, Some(bytes)) => {
            let sha = sha256_hex(&bytes);
            prints.insert(path.to_path_buf(), make_fingerprint(&bytes, sha));
        }
        (false, _) => {
            prints.remove(path);
        }
        _ => {}
    }
    let _ = mtime_ns_v; // mtime is recorded via memory_log; only used as input above

    // Notify the renderer. Cheap — payload is just enough to drive
    // a refresh; the renderer fetches the actual rows via IPC.
    let _ = app.emit(
        "memory:changed",
        serde_json::json!({
            "abs_path": path.to_string_lossy(),
            "role": role,
            "change_type": match change_type {
                ChangeType::Created => "created",
                ChangeType::Modified => "modified",
                ChangeType::Deleted => "deleted",
            },
            "project_slug": slug,
            "kind_hint": match kind {
                notify_debouncer_mini::DebouncedEventKind::Any => "any",
                notify_debouncer_mini::DebouncedEventKind::AnyContinuous => "continuous",
                _ => "other",
            },
        }),
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Build a fingerprint, bounding the cached byte payload so the
/// in-memory cache stays linear in path count rather than total
/// memory-file size. Files at or above the cap drop their bytes; the
/// next diff for them will be flagged `too_large` by `memory_log`,
/// which already handles the same threshold.
fn make_fingerprint(bytes: &[u8], sha256: String) -> Fingerprint {
    let size = bytes.len() as u64;
    let cached = if bytes.len() <= MAX_FINGERPRINT_BYTES {
        bytes.to_vec()
    } else {
        Vec::new()
    };
    Fingerprint {
        size,
        sha256,
        bytes: cached,
    }
}

fn mtime_ns(path: &Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or(0)
}
