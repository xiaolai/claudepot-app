//! Config section Tauri commands — P0.
//!
//! P0 ships:
//! - `config_scan` — empty tree stub (discovery lands in P1).
//! - `config_list_editors` — detected editors, cached 5 min.
//! - `config_open_in_editor` — fire-and-forget launch.
//! - `config_open_in_editor_path` — raw-path variant; used when the
//!   section is showing a stub and has no node_ids yet, and by the
//!   "Other…" file picker which receives a user-supplied binary.
//! - `config_get_editor_defaults` / `config_set_editor_default` —
//!   per-kind preferences, persisted in `preferences.json`.
//!
//! Per `.claude/rules/architecture.md`: all logic lives in
//! `claudepot_core::config_view`; these commands are DTO adapters only.
//!
//! # Threading policy
//!
//! All handlers in this file are `async fn` so Tauri dispatches them on a
//! Tokio worker instead of the main thread. See the threading-policy
//! comment in `commands.rs` for the full rationale — any blocking I/O on
//! the main thread freezes the webview.

use crate::commands_config_types::{
    auto_reason_label, render_path, ConfigTreeDto, EditorCandidateDto, EditorDefaultsDto,
    EffectiveMcpDto, EffectiveMcpServerDto, EffectiveSettingsDto, McpSimulationModeDto,
    PolicyErrorDto, ProvenanceLeafDto, ScopeNodeDto, SearchHitDto, SearchQueryDto,
    SearchSummaryDto,
};
use crate::config_dto::{kind_from_str, policy_origin_label, scope_label_with_origin, FileNodeDto};
use serde::Serialize;
use crate::preferences::PreferencesState;
use claudepot_core::config_view::{
    discover,
    effective_io,
    effective_mcp::{self, ApprovalState, BlockReason, McpSimulationMode},
    effective_settings,
    launcher::{self, LaunchError},
    model::{ConfigTree, EditorCandidate},
    scan,
    search::{self, CancelToken},
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, State};

/// Cached last-scanned tree, used by `config_preview` to resolve node IDs
/// back to file paths. Rebuilt on every `config_scan`. Shared between the
/// command and the filesystem watcher so snapshots land in one cache.
///
/// `commit` is last-writer-wins by generation, and the generation check
/// lives INSIDE the mutex — the previous "load → lock → write" shape
/// was a non-atomic compare-and-write that let an older writer race
/// past a newer one (audit 2026-04-24 round 2).
#[derive(Default)]
pub(crate) struct ConfigTreeCell {
    pub(crate) tree: Option<ConfigTree>,
    /// Generation of the committed tree; 0 before anything is written.
    pub(crate) last_committed_gen: u64,
}

#[derive(Default, Clone)]
pub struct ConfigTreeState {
    cell: std::sync::Arc<Mutex<ConfigTreeCell>>,
    /// Monotonic counter used to hand out generations to writers. Bumped
    /// via `next_gen()` before a writer starts its scan.
    gen_counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ConfigTreeState {
    /// Claim the next generation for a writer. The writer calls
    /// `commit` with this token on success.
    pub fn next_gen(&self) -> u64 {
        self.gen_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1
    }

    /// Atomically commit the tree iff `my_gen` beats the last committed
    /// generation. The comparison and the write happen under the same
    /// mutex so no older writer can slip past a newer one. Returns
    /// `true` when the commit took effect.
    pub fn commit(&self, my_gen: u64, tree: ConfigTree) -> Result<bool, String> {
        let mut g = self
            .cell
            .lock()
            .map_err(|e| format!("tree state lock: {e}"))?;
        if my_gen <= g.last_committed_gen {
            return Ok(false);
        }
        g.tree = Some(tree);
        g.last_committed_gen = my_gen;
        Ok(true)
    }

    /// Read-only accessor — lets `config_preview` / `config_search_start`
    /// grab a snapshot of the cached tree. Clones under the lock so the
    /// reader leaves with a detached value and never holds the mutex
    /// across I/O.
    pub fn snapshot(&self) -> Result<Option<ConfigTree>, String> {
        let g = self
            .cell
            .lock()
            .map_err(|e| format!("tree state lock: {e}"))?;
        Ok(g.tree.clone())
    }

    /// Run `f` with a reference to the cached tree while holding the
    /// mutex. Used by `config_preview` to locate a `FileNode` and clone
    /// only that node out, without cloning the whole tree.
    pub fn with_tree<T>(
        &self,
        f: impl FnOnce(&ConfigTree) -> T,
    ) -> Result<Option<T>, String> {
        let g = self
            .cell
            .lock()
            .map_err(|e| format!("tree state lock: {e}"))?;
        Ok(g.tree.as_ref().map(f))
    }
}

/// Active search cancel tokens, keyed by client-supplied search_id.
#[derive(Default)]
pub struct SearchRegistry(pub Mutex<HashMap<String, CancelToken>>);

// DTOs + enum converters live in `commands_config_types.rs`.

// ---------- Commands --------------------------------------------------

/// Walk the CC-mandated roots anchored at `cwd` and return the tree.
/// Caches the full tree in `ConfigTreeState` so `config_preview` can
/// resolve node ids without rescanning.
#[tauri::command]
pub async fn config_scan(
    cwd: Option<String>,
    tree_state: State<'_, ConfigTreeState>,
) -> Result<ConfigTreeDto, String> {
    // `cwd = None` means "no project anchor selected" — scan global
    // scopes only. We deliberately do NOT fall back to
    // `std::env::current_dir()`, which was non-deterministic between
    // dev and packaged builds.
    let anchor: Option<PathBuf> = cwd.map(PathBuf::from);

    // Claim a generation BEFORE starting work so the commit check at
    // the end can drop a stale result (two scans for different anchors
    // in flight concurrently — the later-generation wins, the earlier
    // is silently discarded).
    let my_gen = tree_state.next_gen();

    let tree = tauri::async_runtime::spawn_blocking(move || match anchor {
        Some(p) => scan(&p),
        None => claudepot_core::config_view::scan_global(),
    })
    .await
    .map_err(|e| format!("scan join: {e}"))?;

    let dto = ConfigTreeDto {
        scopes: tree.scopes.iter().map(ScopeNodeDto::from).collect(),
        cwd: tree.cwd.display().to_string(),
        project_root: tree.project_root.display().to_string(),
        config_home_dir: tree.cwd.join(".claude").display().to_string(),
        memory_slug: tree.memory_slug.clone(),
        memory_slug_lossy: tree.memory_slug_lossy,
    };

    // Commit iff no newer generation landed while we were scanning.
    let _ = tree_state.commit(my_gen, tree)?;
    Ok(dto)
}

/// Return raw file bytes (head-only) + metadata for a node id.
/// P2 layers secret masking on top of this — for now it reads the file
/// head straight from disk.
#[derive(Serialize, Clone, Debug)]
pub struct PreviewDto {
    pub file: FileNodeDto,
    pub body_utf8: String,
    pub truncated: bool,
}

#[tauri::command]
pub async fn config_preview(
    node_id: String,
    tree_state: State<'_, ConfigTreeState>,
) -> Result<PreviewDto, String> {
    const HEAD_LIMIT: u64 = 256 * 1024;
    // Clone the FileNode out of the locked tree so we can drop the
    // guard before any I/O. `FileNode` is small (paths + a handful of
    // metadata fields); the clone is cheap relative to the 256 KiB
    // read that follows.
    let file = tree_state
        .with_tree(|tree| {
            discover::find_file(tree, &node_id)
                .ok_or_else(|| format!("node not found: {node_id}"))
                .map(Clone::clone)
        })?
        .ok_or_else(|| "tree not scanned yet".to_string())??;

    // File open + read up to 256 KiB. Fast on a local SSD but can stall
    // on a slow mount or a huge file we're about to truncate — push it
    // onto the blocking pool rather than the Tokio IPC worker.
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Read;
        let f = std::fs::File::open(&file.abs_path)
            .map_err(|e| format!("open {}: {}", file.abs_path.display(), e))?;
        let meta = f.metadata().map_err(|e| format!("stat: {e}"))?;
        let truncated = meta.len() > HEAD_LIMIT;
        let mut buf = Vec::with_capacity(std::cmp::min(HEAD_LIMIT, meta.len()) as usize);
        let _ = f.take(HEAD_LIMIT).read_to_end(&mut buf);
        // Mask before the bytes leave the core boundary — no raw secret
        // can reach the IPC frame (plan §7.3).
        let body = claudepot_core::config_view::mask::mask_bytes(&buf);
        Ok(PreviewDto {
            file: FileNodeDto::from(&file),
            body_utf8: body,
            truncated,
        })
    })
    .await
    .map_err(|e| format!("preview join: {e}"))?
}

/// Detected editors, cached for 5 minutes. Pass `force = true` to
/// bypass the cache (e.g. after a user installs a new editor).
#[tauri::command]
pub async fn config_list_editors(
    force: Option<bool>,
) -> Result<Vec<EditorCandidateDto>, String> {
    let force = force.unwrap_or(false);
    let cands = launcher::detect_cached(force);
    Ok(cands.iter().map(EditorCandidateDto::from).collect())
}

/// Read persisted editor defaults.
#[tauri::command]
pub async fn config_get_editor_defaults(
    prefs: State<'_, PreferencesState>,
) -> Result<EditorDefaultsDto, String> {
    let g = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
    Ok(EditorDefaultsDto::from(&g.editor_defaults))
}

/// Set the default editor for a given `kind`. When `kind` is `None`,
/// update the global `fallback` instead.
#[tauri::command]
pub async fn config_set_editor_default(
    kind: Option<String>,
    editor_id: String,
    prefs: State<'_, PreferencesState>,
) -> Result<(), String> {
    if editor_id.trim().is_empty() {
        return Err("editor_id is empty".to_string());
    }
    let mut g = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
    match kind {
        None => {
            g.editor_defaults.fallback = editor_id;
        }
        Some(k) => {
            let parsed = kind_from_str(&k).ok_or_else(|| format!("unknown kind: {k}"))?;
            g.editor_defaults.by_kind.insert(parsed, editor_id);
        }
    }
    g.save()?;
    Ok(())
}

/// Launch an arbitrary path in the user's chosen editor. If
/// `editor_id` is omitted, resolves the per-kind default from
/// `preferences.editor_defaults`.
///
/// `kind_hint` steers per-kind default resolution — callers that
/// don't know the kind pass `None` and we use the global fallback.
#[tauri::command]
pub async fn config_open_in_editor_path(
    path: String,
    editor_id: Option<String>,
    kind_hint: Option<String>,
    prefs: State<'_, PreferencesState>,
) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("path is empty".to_string());
    }
    let target = PathBuf::from(&path);
    let candidates = launcher::detect_cached(false);
    let defaults = {
        let g = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
        g.editor_defaults.clone()
    };
    let chosen: &EditorCandidate = if let Some(id) = editor_id.as_ref() {
        candidates
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| format!("editor not detected: {id}"))?
    } else {
        let kind = kind_hint.as_deref().and_then(kind_from_str);
        let resolved = match kind {
            Some(k) => launcher::resolve_editor_for(&k, &defaults, &candidates),
            None => candidates
                .iter()
                .find(|c| c.id == defaults.fallback)
                .or_else(|| candidates.iter().find(|c| c.id == "system")),
        };
        resolved.ok_or_else(|| "no editor available".to_string())?
    };
    launch_into(chosen, &target)
}

fn launch_into(chosen: &EditorCandidate, target: &Path) -> Result<(), String> {
    launcher::invoke(chosen, target).map_err(|e| match e {
        LaunchError::Spawn(s) => format!("launch failed: {s}"),
        LaunchError::NoEnvEditor => "$EDITOR is not set".to_string(),
        LaunchError::NoBinary => "editor binary missing".to_string(),
        LaunchError::EmptyPath => "path is empty".to_string(),
        LaunchError::UnknownEditor(id) => format!("unknown editor: {id}"),
    })
}

// ---------- Content search (P2) --------------------------------------

/// Start a streaming content search. Hits fire via
/// `config-search-hit::{search_id}`; summary via
/// `config-search-done::{search_id}`.
#[tauri::command]
pub async fn config_search_start(
    search_id: String,
    query: SearchQueryDto,
    app: tauri::AppHandle,
    tree_state: State<'_, ConfigTreeState>,
    registry: State<'_, SearchRegistry>,
) -> Result<(), String> {
    if search_id.trim().is_empty() {
        return Err("search_id is empty".to_string());
    }
    let tree = tree_state
        .snapshot()?
        .ok_or_else(|| "tree not scanned yet".to_string())?;

    let cancel = CancelToken::new();
    {
        let mut g = registry.0.lock().map_err(|e| format!("reg lock: {e}"))?;
        g.insert(search_id.clone(), cancel.clone());
    }

    let query_core = search::SearchQuery {
        text: query.text,
        regex: query.regex,
        case_sensitive: query.case_sensitive,
        scope_filter: query.scope_filter,
        kind_filter: None,
    };

    let search_id_for_task = search_id.clone();
    let app_for_task = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let sid_hit = search_id_for_task.clone();
        let app_hit = app_for_task.clone();
        let summary = search::search(&tree, query_core, &cancel, |hit| {
            let _ = app_hit.emit(
                &format!("config-search-hit::{sid_hit}"),
                SearchHitDto {
                    search_id: sid_hit.clone(),
                    node_id: hit.node_id,
                    line_number: hit.line_number,
                    snippet: hit.snippet,
                    match_count_in_file: hit.match_count_in_file,
                },
            );
        });
        let dto = match summary {
            Ok(s) => SearchSummaryDto {
                search_id: search_id_for_task.clone(),
                total_hits: s.total_hits,
                capped: s.capped,
                skipped_large: s.skipped_large,
                cancelled: s.cancelled,
            },
            Err(msg) => SearchSummaryDto {
                search_id: search_id_for_task.clone(),
                total_hits: 0,
                capped: false,
                skipped_large: 0,
                cancelled: true,
            }
            .with_error(&msg),
        };
        let _ = app_for_task.emit(
            &format!("config-search-done::{search_id_for_task}"),
            &dto,
        );
    });

    Ok(())
}

#[tauri::command]
pub async fn config_search_cancel(
    search_id: String,
    registry: State<'_, SearchRegistry>,
) -> Result<(), String> {
    let mut g = registry.0.lock().map_err(|e| format!("reg lock: {e}"))?;
    if let Some(tok) = g.remove(&search_id) {
        tok.cancel();
    }
    Ok(())
}

// ---------- Effective Settings (P4 UI) --------------------------------

#[tauri::command]
pub async fn config_effective_settings(
    cwd: Option<String>,
) -> Result<EffectiveSettingsDto, String> {
    // Effective settings merges Project + Local + User + Policy — the
    // result is only meaningful with a project anchor. Callers that
    // reach this command in global-only mode have skipped the UI
    // gating; fail loudly rather than fabricate a result.
    let cwd_path = cwd
        .map(PathBuf::from)
        .ok_or_else(|| "no project anchored — effective settings needs a cwd".to_string())?;

    let dto = tauri::async_runtime::spawn_blocking(move || {
        let input = effective_io::load_effective_settings_input(&cwd_path);
        let r = effective_settings::compute(&input);
        EffectiveSettingsDto {
            merged: r.merged,
            provenance: r
                .provenance
                .into_iter()
                .map(|p| ProvenanceLeafDto {
                    path: render_path(&p.key_path),
                    winner: scope_label_with_origin(&p.winner),
                    contributors: p
                        .contributors
                        .iter()
                        .map(scope_label_with_origin)
                        .collect(),
                    suppressed: p.suppressed,
                })
                .collect(),
            policy_winner: r.policy.winner.map(|o| policy_origin_label(&o)),
            policy_errors: r
                .policy
                .errors
                .into_iter()
                .map(|e| PolicyErrorDto {
                    origin: policy_origin_label(&e.origin),
                    message: e.message,
                })
                .collect(),
        }
    })
    .await
    .map_err(|e| format!("effective-settings join: {e}"))?;

    Ok(dto)
}

// ---------- Effective MCP (P5 UI) ------------------------------------

#[tauri::command]
pub async fn config_effective_mcp(
    cwd: Option<String>,
    mode: Option<McpSimulationModeDto>,
) -> Result<EffectiveMcpDto, String> {
    // Same rationale as `config_effective_settings`: MCP merge requires
    // a project anchor.
    let cwd_path = cwd
        .map(PathBuf::from)
        .ok_or_else(|| "no project anchored — effective MCP needs a cwd".to_string())?;
    let mode: McpSimulationMode = mode.unwrap_or(McpSimulationModeDto::Interactive).into();

    let dto = tauri::async_runtime::spawn_blocking(move || {
        let input = effective_io::load_effective_settings_input(&cwd_path);
        let merged_settings = effective_settings::compute(&input).merged;
        let bundle = effective_io::load_mcp_bundle(&cwd_path, merged_settings);
        let lockout = !bundle.enterprise.is_empty();
        let servers = effective_mcp::compute(&bundle, mode);

        let servers_dto = servers
            .into_iter()
            .map(|s| {
                let (approval, reason) = match s.approval {
                    ApprovalState::Approved => ("approved".to_string(), None),
                    ApprovalState::Rejected => ("rejected".to_string(), None),
                    ApprovalState::Pending => ("pending".to_string(), None),
                    ApprovalState::AutoApproved(r) => {
                        ("auto_approved".to_string(), Some(auto_reason_label(&r)))
                    }
                };
                let blocked = s.blocked_by.map(|b| match b {
                    BlockReason::EnterpriseLockout => "enterprise_lockout".to_string(),
                    BlockReason::DisabledByUser => "disabled_by_user".to_string(),
                });
                EffectiveMcpServerDto {
                    name: s.name,
                    source_scope: scope_label_with_origin(&s.source_scope),
                    contributors: s
                        .contributors
                        .iter()
                        .map(scope_label_with_origin)
                        .collect(),
                    approval,
                    approval_reason: reason,
                    blocked_by: blocked,
                    masked: s.masked,
                }
            })
            .collect();

        EffectiveMcpDto {
            enterprise_lockout: lockout,
            servers: servers_dto,
        }
    })
    .await
    .map_err(|e| format!("effective-mcp join: {e}"))?;

    Ok(dto)
}

// `render_path` + `auto_reason_label` live in `commands_config_types`.

#[cfg(test)]
mod tree_state_tests {
    use super::ConfigTreeState;
    use claudepot_core::config_view::model::ConfigTree;
    use std::path::PathBuf;

    fn fixture_tree(tag: &str) -> ConfigTree {
        // Minimal synthetic tree keyed by `cwd` so assertions can tell
        // one writer's tree from another's.
        ConfigTree {
            scopes: Vec::new(),
            scanned_at_unix_ns: 0,
            cwd: PathBuf::from(tag),
            project_root: PathBuf::from(tag),
            memory_slug: String::new(),
            memory_slug_lossy: false,
            cc_version_hint: None,
            enterprise_mcp_lockout: false,
        }
    }

    #[test]
    fn commit_accepts_writes_in_generation_order() {
        let state = ConfigTreeState::default();
        let g1 = state.next_gen();
        let g2 = state.next_gen();
        assert!(state.commit(g1, fixture_tree("a")).unwrap());
        assert!(state.commit(g2, fixture_tree("b")).unwrap());
        assert_eq!(
            state.snapshot().unwrap().unwrap().cwd,
            PathBuf::from("b")
        );
    }

    #[test]
    fn commit_drops_older_writer_when_newer_already_committed() {
        // Classic stale-write race: claim g1, claim g2, commit g2,
        // commit g1. The g1 write MUST be dropped — the atomic
        // check-and-write inside `commit` guarantees this (audit
        // 2026-04-24 round 3, the original check-then-lock shape let
        // g1 slip past).
        let state = ConfigTreeState::default();
        let g1 = state.next_gen();
        let g2 = state.next_gen();
        assert!(state.commit(g2, fixture_tree("new")).unwrap());
        assert!(!state.commit(g1, fixture_tree("stale")).unwrap());
        assert_eq!(
            state.snapshot().unwrap().unwrap().cwd,
            PathBuf::from("new")
        );
    }

    #[test]
    fn concurrent_writers_always_leave_the_newest_tree_committed() {
        use std::sync::Arc;
        use std::thread;
        // Spawn 32 writers; each claims a generation, spins for a
        // variable interval, then commits a tree tagged with its
        // generation. Repeat to hammer the compare-and-write path.
        //
        // Strong assertion (round-3 follow-up audit, L-fix): after
        // every writer finishes, `last_committed_gen` MUST equal the
        // highest claimed generation AND the committed tree's tag
        // must match — the winner is the writer who claimed the
        // highest generation, not merely "some writer with a
        // generation <= max." The round-2 non-atomic commit would
        // frequently leave `last_committed_gen < max_gen` here and
        // fail this assertion.
        for _trial in 0..50 {
            let state = Arc::new(ConfigTreeState::default());
            let mut handles = Vec::new();
            for i in 0..32 {
                let s = state.clone();
                handles.push(thread::spawn(move || {
                    let g = s.next_gen();
                    for _ in 0..(i * 7 % 13) {
                        std::hint::spin_loop();
                    }
                    let _ = s.commit(g, fixture_tree(&format!("w{g}")));
                    g
                }));
            }
            let gens: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            let max_gen = *gens.iter().max().unwrap();
            let cell = state.cell.lock().unwrap();
            assert_eq!(
                cell.last_committed_gen, max_gen,
                "the writer with the highest claimed generation must win"
            );
            let tree = cell.tree.as_ref().expect("at least one writer committed");
            assert_eq!(
                tree.cwd,
                PathBuf::from(format!("w{max_gen}")),
                "committed tree must come from the winning writer"
            );
        }
    }
}
