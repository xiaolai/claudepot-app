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
use crate::preferences::PreferencesState;
use claudepot_core::config_view::{
    discover, effective_io,
    effective_mcp::{self, ApprovalState, BlockReason, McpSimulationMode},
    effective_settings,
    launcher::{self, LaunchError},
    model::{EditorCandidate, Kind},
    search::{self, CancelToken},
    ConfigScanService,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

/// Active search cancel tokens, keyed by client-supplied search_id.
#[derive(Default)]
pub struct SearchRegistry(pub Mutex<HashMap<String, CancelToken>>);

// DTOs + enum converters live in `commands_config_types.rs`.

// ---------- Commands --------------------------------------------------

/// Walk the CC-mandated roots anchored at `cwd` and return the tree.
/// Caches the full tree in `ConfigScanService` so `config_preview` can
/// resolve node ids without rescanning.
#[tauri::command]
pub async fn config_scan(
    cwd: Option<String>,
    svc: State<'_, Arc<ConfigScanService>>,
) -> Result<ConfigTreeDto, String> {
    // `cwd = None` means "no project anchor selected" — scan global
    // scopes only. We deliberately do NOT fall back to
    // `std::env::current_dir()`, which was non-deterministic between
    // dev and packaged builds.
    let anchor: Option<PathBuf> = cwd.map(PathBuf::from);

    // Service owns the generation handle + the commit race. Run the
    // full scan on the blocking pool so the IPC worker isn't pinned
    // by filesystem stalls.
    let svc = Arc::clone(svc.inner());
    let tree = tauri::async_runtime::spawn_blocking(move || svc.scan_and_commit(anchor.as_deref()))
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
    svc: State<'_, Arc<ConfigScanService>>,
) -> Result<PreviewDto, String> {
    // Markdown / hook scripts: 256 KiB head is plenty.
    const HEAD_LIMIT: u64 = 256 * 1024;
    // JSON files: must read the whole document for the JSON-aware mask
    // path to produce a parseable result. 8 MiB is well above any
    // legitimate ~/.claude.json or settings.json — past that we accept
    // truncation + byte-level masking and the renderer's tree view
    // gracefully degrades to its <pre> fallback.
    const JSON_LIMIT: u64 = 8 * 1024 * 1024;
    // Clone the FileNode out of the locked tree so we can drop the
    // guard before any I/O. `FileNode` is small (paths + a handful of
    // metadata fields); the clone is cheap relative to the read that
    // follows.
    let file = svc
        .with_tree(|tree| {
            discover::find_file(tree, &node_id)
                .ok_or_else(|| format!("node not found: {node_id}"))
                .cloned()
        })
        .ok_or_else(|| "tree not scanned yet".to_string())??;

    // File open + read. Fast on a local SSD but can stall on a slow
    // mount or a huge file — push onto the blocking pool rather than
    // the Tokio IPC worker.
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Read;
        let is_json = is_json_kind(&file.kind);
        let limit: u64 = if is_json { JSON_LIMIT } else { HEAD_LIMIT };
        let f = std::fs::File::open(&file.abs_path)
            .map_err(|e| format!("open {}: {}", file.abs_path.display(), e))?;
        let meta = f.metadata().map_err(|e| format!("stat: {e}"))?;
        let truncated = meta.len() > limit;
        let mut buf = Vec::with_capacity(std::cmp::min(limit, meta.len()) as usize);
        let _ = f.take(limit).read_to_end(&mut buf);
        // Mask before the bytes leave the core boundary — no raw secret
        // can reach the IPC frame (plan §7.3). For JSON kinds, prefer
        // the structure-aware masker so a regex rule with `/`+`=` in
        // its character class can't eat a JSON string delimiter on long
        // path values (regression test:
        // `mask_preview_body_does_not_corrupt_paths` in mask.rs).
        let body = if is_json {
            claudepot_core::config_view::mask::mask_preview_body(&buf)
        } else {
            claudepot_core::config_view::mask::mask_bytes(&buf)
        };
        Ok(PreviewDto {
            file: FileNodeDto::from(&file),
            body_utf8: body,
            truncated,
        })
    })
    .await
    .map_err(|e| format!("preview join: {e}"))?
}

/// File kinds whose on-disk representation is a single JSON document.
/// Used by `config_preview` to switch from byte-level regex masking
/// (unsafe on JSON, see `mask_preview_body`) to structure-aware
/// masking. Markdown-shaped kinds (ClaudeMd, Agent, Skill, Command,
/// OutputStyle, Workflow, Rule, Memory, MemoryIndex, Statusline) and
/// freeform kinds (Hook, Other) keep the byte-level path.
fn is_json_kind(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Settings
            | Kind::SettingsLocal
            | Kind::ManagedSettings
            | Kind::RedactedUserConfig
            | Kind::McpJson
            | Kind::ManagedMcpJson
            | Kind::Plugin
            | Kind::Keybindings
            | Kind::EffectiveSettings
            | Kind::EffectiveMcp
    )
}

/// Detected editors, cached for 5 minutes. Pass `force = true` to
/// bypass the cache (e.g. after a user installs a new editor).
#[tauri::command]
pub async fn config_list_editors(force: Option<bool>) -> Result<Vec<EditorCandidateDto>, String> {
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
    svc: State<'_, Arc<ConfigScanService>>,
    registry: State<'_, SearchRegistry>,
) -> Result<(), String> {
    if search_id.trim().is_empty() {
        return Err("search_id is empty".to_string());
    }
    // `current_tree` returns an `Arc<ConfigTree>`; cheap refcount bump
    // and the search runs against an immutable snapshot, so concurrent
    // commits don't disturb in-flight searches.
    let tree = svc
        .current_tree()
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
        let _ = app_for_task.emit(&format!("config-search-done::{search_id_for_task}"), &dto);
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
                    contributors: p.contributors.iter().map(scope_label_with_origin).collect(),
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
                    contributors: s.contributors.iter().map(scope_label_with_origin).collect(),
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
//
// The tree-state race tests that used to live here moved to
// `claudepot_core::config_view::service::tests` when the cell
// migrated out of the IPC layer (D-1, audit-fix wave 3).
