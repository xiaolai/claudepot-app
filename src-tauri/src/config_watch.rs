//! Filesystem watcher bridge for the Config section.
//!
//! Core exposes `DirtyGen` + `scan_until_stable` + deterministic
//! ingest (`config_view::watch`). This module wires those primitives
//! to a real `notify-debouncer-mini` watcher, then emits
//! `config-tree-patch` events to the webview so the React tree can
//! incrementally apply diffs (plan §11.5 / §13.6).
//!
//! Design decisions:
//! - One watcher task per `config_watch_start` call. Calling start
//!   again with a different cwd stops the previous task first.
//! - The debouncer coalesces bursts of events (250 ms, matching plan
//!   §11.1). After a batch lands, we run `scan_until_stable` to reach
//!   a converged snapshot, diff against the previous snapshot, and
//!   emit the patch. `dirty_during_emit` tags any patch that bailed
//!   with in-flight events.
//! - A 5-minute keepalive forces a fresh scan even if no events
//!   arrived (plan §11.4) to cover FS event drops.

use crate::config_dto::{file_to_dto, flatten_file_refs, scope_kind_label};
use crate::config_watch_types::{
    AddedFileDto, ConfigTreePatchEvent, ConfigTreeSnapshotDto, ReorderedDto, ScopeSnapshotDto,
};
use claudepot_core::config_view::{
    diff::ConfigTreePatch as CorePatch,
    discover,
    model::ConfigTree,
    watch::{ingest_event, scan_until_stable, DirtyGen, FsEvent, FsEventKind},
};
use claudepot_core::paths::claude_config_dir;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_mini::new_debouncer;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Handle for a running watcher. Dropping it stops the task.
pub struct WatcherHandle {
    /// Set to true to request a clean shutdown on the next tick.
    stop: Arc<std::sync::atomic::AtomicBool>,
    /// Joinable worker thread. `None` after `stop()`.
    worker: Option<std::thread::JoinHandle<()>>,
}

impl WatcherHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Default)]
pub struct ConfigWatchState(pub Mutex<Option<WatcherHandle>>);

const DEBOUNCE: Duration = Duration::from_millis(250);
const KEEPALIVE: Duration = Duration::from_secs(300);

// DTOs for the `config-tree-patch` event live in
// `config_watch_types.rs` so this file can focus on the watcher state
// machine.

/// Kick off a watcher.
///
/// `anchor = Some(cwd)` watches the anchored project: cwd subtree,
/// every ancestor up to git-root/home, and `~/.claude`.
/// `anchor = None` is the global-only mode used when the Config page
/// has no project selected — only `~/.claude` is watched and rescans
/// use `assemble_tree(_, global_only=true)`.
pub fn start(
    app: AppHandle,
    anchor: Option<PathBuf>,
    tree_state: crate::commands_config::ConfigTreeState,
) -> Result<WatcherHandle, String> {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_inner = stop.clone();
    let app_inner = app.clone();
    let tree_state_inner = tree_state.clone();
    let anchor_inner = anchor.clone();

    // Seed runs inside the worker — the caller (the Tauri IPC worker)
    // returns immediately instead of sitting on a full-tree scan.
    //
    // We seed unconditionally. A previous version tried to skip the
    // seed when `tree_state` was already populated (reusing what
    // `config_scan` left behind), but that let a new watcher for
    // anchor B diff against anchor A's cached tree — `diff()` doesn't
    // always emit `full_snapshot` on similar-shaped trees, so bogus
    // incremental patches could slip through (audit 2026-04-24, H3).
    // Overwriting with a fresh seed is cheap on a background thread
    // and is always correct.
    //
    // The seed write uses the same generation counter as `config_scan`
    // so a slow watcher seed can't clobber a newer `config_scan` result
    // (audit 2026-04-24, H2).
    let worker = std::thread::Builder::new()
        .name("config-watch".to_string())
        .spawn(move || {
            let my_gen = tree_state_inner.next_gen();
            let seed = match &anchor_inner {
                Some(cwd) => discover::assemble_tree(cwd, false),
                None => {
                    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                    discover::assemble_tree(&home, true)
                }
            };
            let _ = tree_state_inner.commit(my_gen, seed);
            if let Err(e) = run_loop(app_inner, anchor_inner, tree_state_inner, stop_inner) {
                tracing::error!("config-watch loop exited: {e}");
            }
        })
        .map_err(|e| format!("spawn config-watch: {e}"))?;

    Ok(WatcherHandle {
        stop,
        worker: Some(worker),
    })
}

fn run_loop(
    app: AppHandle,
    anchor: Option<PathBuf>,
    tree_state: crate::commands_config::ConfigTreeState,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    use std::sync::mpsc::channel;

    let (tx, rx) = channel::<Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>>();

    let mut debouncer = new_debouncer(DEBOUNCE, move |res| {
        // Forward to the run loop. Ignore channel-closed errors — the
        // worker is shutting down.
        let _ = tx.send(res);
    })
    .map_err(|e| format!("debouncer init: {e}"))?;

    // Roots depend on anchor mode.
    // Anchored: watch ~/.claude, the cwd subtree, AND every ancestor
    // directory up to git-root/home — discovery walks CLAUDE.md +
    // .claude/rules at each ancestor (plan §6.4).
    // Global: only ~/.claude — there's no project subtree to watch.
    // Recursive watches on ancestors are fine: core's `is_in_scope`
    // deny-list filters the noise.
    let home = claude_config_dir();
    let mut roots: Vec<PathBuf> = vec![home.clone()];
    if let Some(cwd) = anchor.as_ref() {
        roots.push(cwd.clone());
        roots.extend(watch_ancestor_dirs(cwd, &home));
    }
    roots.sort();
    roots.dedup();
    for root in &roots {
        if !root.exists() {
            continue;
        }
        debouncer
            .watcher()
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| format!("watch {}: {}", root.display(), e))?;
    }

    let gen = DirtyGen::new();
    let mut generation_counter: u64 = 0;
    let mut last_keepalive = std::time::Instant::now();

    loop {
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }

        match rx.recv_timeout(KEEPALIVE) {
            Ok(Ok(batch)) => {
                let any_relevant = batch
                    .iter()
                    .map(notify_to_fs_event)
                    .any(|e| ingest_event(&gen, &e));
                if !any_relevant {
                    continue;
                }
                generation_counter = generation_counter.wrapping_add(1);
                emit_patch(
                    &app,
                    anchor.as_deref(),
                    &tree_state,
                    &gen,
                    generation_counter,
                );
                last_keepalive = std::time::Instant::now();
            }
            Ok(Err(e)) => {
                tracing::warn!("watcher event error: {e}");
                // Keep looping — one bad event doesn't kill the watcher.
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 5-min keepalive rescan — catches dropped events.
                if last_keepalive.elapsed() >= KEEPALIVE {
                    gen.bump();
                    generation_counter = generation_counter.wrapping_add(1);
                    emit_patch(
                        &app,
                        anchor.as_deref(),
                        &tree_state,
                        &gen,
                        generation_counter,
                    );
                    last_keepalive = std::time::Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("watcher channel disconnected".to_string());
            }
        }
    }
}

/// Collect ancestor directories of `cwd` up to the same boundary that
/// discovery uses (first `.git` directory, or the user's home). Callers
/// then `watch(root, Recursive)` on each — CLAUDE.md and .claude/rules
/// live at ancestor levels (plan §6.4), so ignoring them here means
/// edits would require a manual refresh.
fn watch_ancestor_dirs(cwd: &Path, home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur: Option<PathBuf> = cwd.parent().map(|p| p.to_path_buf());
    while let Some(dir) = cur {
        out.push(dir.clone());
        if dir.join(".git").exists() {
            break;
        }
        if dir == home {
            break;
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
    out
}

fn notify_to_fs_event(ev: &notify_debouncer_mini::DebouncedEvent) -> FsEvent {
    // notify-debouncer-mini only reports paths + an Any/AnyContinuous
    // event kind; treat every batch entry as Modified for the in-scope
    // filter. Rename from/to pairs surface as two separate events, so
    // our Modified classification is safe (both sides get checked by
    // `ingest_event`).
    let _ = ev; // `kind` field is intentionally simple in mini.
    FsEvent {
        kind: FsEventKind::Modified,
        path: ev.path.clone(),
        rename_to: None,
    }
}

fn emit_patch(
    app: &AppHandle,
    anchor: Option<&std::path::Path>,
    tree_state: &crate::commands_config::ConfigTreeState,
    gen: &DirtyGen,
    generation: u64,
) {
    let my_gen = tree_state.next_gen();
    let (next, dirty) = scan_until_stable(gen, || match anchor {
        Some(cwd) => discover::assemble_tree(cwd, false),
        None => {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            discover::assemble_tree(&home, true)
        }
    });

    // Snapshot the previous committed tree before building the diff.
    // Cloning leaves us holding a detached value so the mutex is free
    // during the diff + commit that follow.
    let prev_opt = match tree_state.snapshot() {
        Ok(o) => o,
        Err(_) => return,
    };

    let core_patch: CorePatch = match &prev_opt {
        Some(prev) => {
            let mut p = claudepot_core::config_view::diff::diff(prev, &next);
            p.dirty_during_emit = dirty;
            p
        }
        None => CorePatch {
            full_snapshot: Some(next.clone()),
            dirty_during_emit: dirty,
            ..Default::default()
        },
    };

    // Attempt to commit. If we lose the race (a newer generation —
    // typically a concurrent `config_scan` — already committed), DROP
    // the emit too: the client's tree is already at least as fresh as
    // this patch's baseline, and applying our stale diff would roll it
    // back (audit 2026-04-24 round 3).
    let won = tree_state.commit(my_gen, next).unwrap_or(false);
    if !won {
        return;
    }

    let payload = encode_patch(core_patch, generation);
    let _ = app.emit("config-tree-patch", payload);
}

fn encode_patch(p: CorePatch, generation: u64) -> ConfigTreePatchEvent {
    ConfigTreePatchEvent {
        generation,
        added: p
            .added
            .into_iter()
            .map(|(pid, f)| AddedFileDto {
                parent_scope_id: pid,
                file: file_to_dto(&f),
            })
            .collect(),
        updated: p.updated.iter().map(file_to_dto).collect(),
        removed: p.removed,
        reordered: p
            .reordered
            .into_iter()
            .map(|(pid, ids)| ReorderedDto {
                parent_scope_id: pid,
                child_ids: ids,
            })
            .collect(),
        full_snapshot: p.full_snapshot.map(snapshot_to_dto),
        dirty_during_emit: p.dirty_during_emit,
    }
}

fn snapshot_to_dto(t: ConfigTree) -> ConfigTreeSnapshotDto {
    ConfigTreeSnapshotDto {
        scopes: t
            .scopes
            .iter()
            .map(|s| ScopeSnapshotDto {
                id: s.id.clone(),
                label: s.label.clone(),
                scope_type: scope_kind_label(&s.scope),
                recursive_count: s.recursive_count,
                files: flatten_file_refs(&s.children)
                    .into_iter()
                    .map(file_to_dto)
                    .collect(),
            })
            .collect(),
        cwd: t.cwd.display().to_string(),
        project_root: t.project_root.display().to_string(),
        config_home_dir: t.cwd.join(".claude").display().to_string(),
        memory_slug: t.memory_slug,
        memory_slug_lossy: t.memory_slug_lossy,
    }
}

// Silence unused warnings — `EventKind` is referenced through the
// notify re-export graph; keeping the import available documents the
// dependency edge.
#[allow(dead_code)]
fn _type_guard(_: EventKind) {}

/// Turn on the real-FS watcher. Idempotent — calling this while a
/// watcher is already running stops the old one and starts a new one
/// rooted at `cwd`.
///
/// The caller-facing work is cheap: stop the old watcher, clone the
/// shared tree cache, spawn the worker. The worker thread owns the
/// initial `assemble_tree` seed so the IPC worker that dispatched this
/// command doesn't sit holding the full-tree scan (which overlapped
/// with `config_scan`'s own scan on every anchor change).
#[tauri::command]
pub async fn config_watch_start(
    cwd: Option<String>,
    app: AppHandle,
    state: tauri::State<'_, ConfigWatchState>,
    tree_state: tauri::State<'_, crate::commands_config::ConfigTreeState>,
) -> Result<(), String> {
    let mut guard = state
        .0
        .lock()
        .map_err(|e| format!("watch state lock: {e}"))?;
    if let Some(mut h) = guard.take() {
        h.stop();
    }

    // Hand the watcher a clone of the full `ConfigTreeState` (tree +
    // generation counter) — snapshots land in the same cache that
    // `config_scan` writes through, and both use the generation to
    // avoid clobbering each other's results.
    let shared = tree_state.inner().clone();
    let handle = start(app.clone(), cwd.map(PathBuf::from), shared)?;
    *guard = Some(handle);
    Ok(())
}

#[tauri::command]
pub async fn config_watch_stop(
    state: tauri::State<'_, ConfigWatchState>,
) -> Result<(), String> {
    let mut guard = state
        .0
        .lock()
        .map_err(|e| format!("watch state lock: {e}"))?;
    if let Some(mut h) = guard.take() {
        h.stop();
    }
    Ok(())
}
