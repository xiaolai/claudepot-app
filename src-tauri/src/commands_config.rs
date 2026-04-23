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

use crate::preferences::PreferencesState;
use claudepot_core::config_view::{
    discover,
    effective_io,
    effective_mcp::{
        self, ApprovalState, AutoApprovalReason, BlockReason, McpSimulationMode,
    },
    effective_settings,
    launcher::{self, LaunchError},
    model::{
        ConfigTree, DetectSource, EditorCandidate, EditorDefaults, FileNode,
        Kind, LaunchKind, Node, ParseIssue, PolicyOrigin, Scope, ScopeNode,
    },
    scan,
    search::{self, CancelToken},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, State};

/// Cached last-scanned tree, used by `config_preview` to resolve node IDs
/// back to file paths. Rebuilt on every `config_scan`.
#[derive(Default)]
pub struct ConfigTreeState(pub Mutex<Option<ConfigTree>>);

/// Active search cancel tokens, keyed by client-supplied search_id.
#[derive(Default)]
pub struct SearchRegistry(pub Mutex<HashMap<String, CancelToken>>);

// ---------- DTOs ------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct ConfigTreeDto {
    pub scopes: Vec<ScopeNodeDto>,
    pub cwd: String,
    pub project_root: String,
    pub memory_slug: String,
    pub memory_slug_lossy: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScopeNodeDto {
    pub id: String,
    pub label: String,
    pub scope_type: String,
    pub recursive_count: usize,
    pub files: Vec<FileNodeDto>,
}

#[derive(Serialize, Clone, Debug)]
pub struct FileNodeDto {
    pub id: String,
    pub kind: String,
    pub abs_path: String,
    pub display_path: String,
    pub size_bytes: u64,
    pub mtime_unix_ns: i64,
    pub summary_title: Option<String>,
    pub summary_description: Option<String>,
    pub issues: Vec<String>,
}

impl From<&FileNode> for FileNodeDto {
    fn from(f: &FileNode) -> Self {
        Self {
            id: f.id.clone(),
            kind: kind_to_str(&f.kind).to_string(),
            abs_path: f.abs_path.display().to_string(),
            display_path: f.display_path.clone(),
            size_bytes: f.size_bytes,
            mtime_unix_ns: f.mtime_unix_ns,
            summary_title: f.summary.as_ref().and_then(|s| s.title.clone()),
            summary_description: f.summary.as_ref().and_then(|s| s.description.clone()),
            issues: f.issues.iter().map(issue_to_str).collect(),
        }
    }
}

fn issue_to_str(i: &ParseIssue) -> String {
    match i {
        ParseIssue::MalformedJson { message } => format!("malformed_json: {message}"),
        ParseIssue::NotASkill => "not_a_skill".to_string(),
        ParseIssue::SymlinkLoop => "symlink_loop".to_string(),
        ParseIssue::PermissionDenied => "permission_denied".to_string(),
        ParseIssue::Other { message } => format!("other: {message}"),
    }
}

fn scope_to_str(s: &Scope) -> String {
    match s {
        Scope::PluginBase => "plugin_base".to_string(),
        Scope::User => "user".to_string(),
        Scope::Project => "project".to_string(),
        Scope::Local => "local".to_string(),
        Scope::Flag => "flag".to_string(),
        Scope::Policy { .. } => "policy".to_string(),
        Scope::ClaudeMdDir { .. } => "claude_md_dir".to_string(),
        Scope::Plugin { .. } => "plugin".to_string(),
        Scope::MemoryCurrent => "memory_current".to_string(),
        Scope::MemoryOther { .. } => "memory_other".to_string(),
        Scope::Effective => "effective".to_string(),
        Scope::RedactedUserConfig => "redacted_user_config".to_string(),
        Scope::Other => "other".to_string(),
    }
}

fn flatten_files(nodes: &[Node]) -> Vec<FileNodeDto> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::File(f) => out.push(FileNodeDto::from(f)),
            Node::Dir(d) => out.extend(flatten_files(&d.children)),
        }
    }
    out
}

impl From<&ScopeNode> for ScopeNodeDto {
    fn from(s: &ScopeNode) -> Self {
        Self {
            id: s.id.clone(),
            label: s.label.clone(),
            scope_type: scope_to_str(&s.scope),
            recursive_count: s.recursive_count,
            files: flatten_files(&s.children),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct EditorCandidateDto {
    pub id: String,
    pub label: String,
    pub binary_path: Option<String>,
    pub bundle_id: Option<String>,
    pub launch_kind: String,
    pub detected_via: String,
    pub supports_kinds: Option<Vec<Kind>>,
}

impl From<&EditorCandidate> for EditorCandidateDto {
    fn from(c: &EditorCandidate) -> Self {
        let launch_kind = match &c.launch {
            LaunchKind::Direct { .. } => "direct",
            LaunchKind::MacosOpenA { .. } => "macos-open-a",
            LaunchKind::EnvEditor => "env-editor",
            LaunchKind::SystemHandler => "system-handler",
        }
        .to_string();
        let detected_via = match &c.detected_via {
            DetectSource::PathBinary { .. } => "path-binary",
            DetectSource::MacosAppBundle { .. } => "macos-app",
            DetectSource::WindowsRegistry { .. } => "windows-registry",
            DetectSource::LinuxDesktopFile { .. } => "linux-desktop-file",
            DetectSource::EnvVar { .. } => "env-var",
            DetectSource::SystemDefault => "system-default",
            DetectSource::UserPicked { .. } => "user-picked",
        }
        .to_string();
        Self {
            id: c.id.clone(),
            label: c.label.clone(),
            binary_path: c
                .binary_path
                .as_ref()
                .map(|p| p.display().to_string()),
            bundle_id: c.bundle_id.clone(),
            launch_kind,
            detected_via,
            supports_kinds: c.supports_kinds.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EditorDefaultsDto {
    pub by_kind: std::collections::BTreeMap<String, String>,
    pub fallback: String,
}

impl From<&EditorDefaults> for EditorDefaultsDto {
    fn from(d: &EditorDefaults) -> Self {
        Self {
            by_kind: d
                .by_kind
                .iter()
                .map(|(k, v)| (kind_to_str(k).to_string(), v.clone()))
                .collect(),
            fallback: d.fallback.clone(),
        }
    }
}

fn kind_to_str(k: &Kind) -> &'static str {
    match k {
        Kind::ClaudeMd => "claude_md",
        Kind::Settings => "settings",
        Kind::SettingsLocal => "settings_local",
        Kind::ManagedSettings => "managed_settings",
        Kind::RedactedUserConfig => "redacted_user_config",
        Kind::McpJson => "mcp_json",
        Kind::ManagedMcpJson => "managed_mcp_json",
        Kind::Agent => "agent",
        Kind::Skill => "skill",
        Kind::Command => "command",
        Kind::Rule => "rule",
        Kind::Hook => "hook",
        Kind::Memory => "memory",
        Kind::MemoryIndex => "memory_index",
        Kind::Plugin => "plugin",
        Kind::Keybindings => "keybindings",
        Kind::Statusline => "statusline",
        Kind::EffectiveSettings => "effective_settings",
        Kind::EffectiveMcp => "effective_mcp",
        Kind::Other => "other",
    }
}

fn kind_from_str(s: &str) -> Option<Kind> {
    Some(match s {
        "claude_md" => Kind::ClaudeMd,
        "settings" => Kind::Settings,
        "settings_local" => Kind::SettingsLocal,
        "managed_settings" => Kind::ManagedSettings,
        "redacted_user_config" => Kind::RedactedUserConfig,
        "mcp_json" => Kind::McpJson,
        "managed_mcp_json" => Kind::ManagedMcpJson,
        "agent" => Kind::Agent,
        "skill" => Kind::Skill,
        "command" => Kind::Command,
        "rule" => Kind::Rule,
        "hook" => Kind::Hook,
        "memory" => Kind::Memory,
        "memory_index" => Kind::MemoryIndex,
        "plugin" => Kind::Plugin,
        "keybindings" => Kind::Keybindings,
        "statusline" => Kind::Statusline,
        "effective_settings" => Kind::EffectiveSettings,
        "effective_mcp" => Kind::EffectiveMcp,
        "other" => Kind::Other,
        _ => return None,
    })
}

// ---------- Commands --------------------------------------------------

/// Walk the CC-mandated roots anchored at `cwd` and return the tree.
/// Caches the full tree in `ConfigTreeState` so `config_preview` can
/// resolve node ids without rescanning.
#[tauri::command]
pub async fn config_scan(
    cwd: Option<String>,
    tree_state: State<'_, ConfigTreeState>,
) -> Result<ConfigTreeDto, String> {
    let cwd_path = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let tree = tauri::async_runtime::spawn_blocking(move || scan(&cwd_path))
        .await
        .map_err(|e| format!("scan join: {e}"))?;

    let dto = ConfigTreeDto {
        scopes: tree.scopes.iter().map(ScopeNodeDto::from).collect(),
        cwd: tree.cwd.display().to_string(),
        project_root: tree.project_root.display().to_string(),
        memory_slug: tree.memory_slug.clone(),
        memory_slug_lossy: tree.memory_slug_lossy,
    };

    {
        let mut g = tree_state
            .0
            .lock()
            .map_err(|e| format!("tree state lock: {e}"))?;
        *g = Some(tree);
    }
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
pub fn config_preview(
    node_id: String,
    tree_state: State<'_, ConfigTreeState>,
) -> Result<PreviewDto, String> {
    const HEAD_LIMIT: u64 = 256 * 1024;
    let guard = tree_state
        .0
        .lock()
        .map_err(|e| format!("tree state lock: {e}"))?;
    let tree = guard.as_ref().ok_or_else(|| "tree not scanned yet".to_string())?;
    let file = discover::find_file(tree, &node_id)
        .ok_or_else(|| format!("node not found: {node_id}"))?;

    use std::io::Read;
    let f = std::fs::File::open(&file.abs_path)
        .map_err(|e| format!("open {}: {}", file.abs_path.display(), e))?;
    let meta = f.metadata().map_err(|e| format!("stat: {e}"))?;
    let truncated = meta.len() > HEAD_LIMIT;
    let mut buf = Vec::with_capacity(std::cmp::min(HEAD_LIMIT, meta.len()) as usize);
    let _ = f.take(HEAD_LIMIT).read_to_end(&mut buf);
    // Mask before the bytes leave the core boundary — no raw secret can
    // reach the IPC frame (plan §7.3).
    let body = claudepot_core::config_view::mask::mask_bytes(&buf);

    Ok(PreviewDto {
        file: FileNodeDto::from(file),
        body_utf8: body,
        truncated,
    })
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
pub fn config_get_editor_defaults(
    prefs: State<'_, PreferencesState>,
) -> Result<EditorDefaultsDto, String> {
    let g = prefs.0.lock().map_err(|e| format!("prefs lock: {e}"))?;
    Ok(EditorDefaultsDto::from(&g.editor_defaults))
}

/// Set the default editor for a given `kind`. When `kind` is `None`,
/// update the global `fallback` instead.
#[tauri::command]
pub fn config_set_editor_default(
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
pub fn config_open_in_editor_path(
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

#[derive(Deserialize, Clone, Debug)]
pub struct SearchQueryDto {
    pub text: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub scope_filter: Option<Vec<String>>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SearchHitDto {
    pub search_id: String,
    pub node_id: String,
    pub line_number: u32,
    pub snippet: String,
    pub match_count_in_file: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct SearchSummaryDto {
    pub search_id: String,
    pub total_hits: u32,
    pub capped: bool,
    pub skipped_large: u32,
    pub cancelled: bool,
}

/// Start a streaming content search. Hits fire via
/// `config-search-hit::{search_id}`; summary via
/// `config-search-done::{search_id}`.
#[tauri::command]
pub fn config_search_start(
    search_id: String,
    query: SearchQueryDto,
    app: tauri::AppHandle,
    tree_state: State<'_, ConfigTreeState>,
    registry: State<'_, SearchRegistry>,
) -> Result<(), String> {
    if search_id.trim().is_empty() {
        return Err("search_id is empty".to_string());
    }
    let tree = {
        let g = tree_state.0.lock().map_err(|e| format!("tree lock: {e}"))?;
        g.clone().ok_or_else(|| "tree not scanned yet".to_string())?
    };

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

impl SearchSummaryDto {
    fn with_error(self, _msg: &str) -> Self {
        // Error details land in a trace log; the client sees `cancelled`.
        self
    }
}

#[tauri::command]
pub fn config_search_cancel(
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

#[derive(Serialize, Clone, Debug)]
pub struct ProvenanceLeafDto {
    /// Dotted JSON path — `"a.b[2].c"`.
    pub path: String,
    pub winner: String,
    pub contributors: Vec<String>,
    pub suppressed: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct EffectiveSettingsDto {
    /// Fully merged JSON, with secrets masked.
    pub merged: serde_json::Value,
    /// One entry per primitive leaf.
    pub provenance: Vec<ProvenanceLeafDto>,
    /// Winning policy origin ("remote" | "mdm_admin" | …) or None.
    pub policy_winner: Option<String>,
    pub policy_errors: Vec<PolicyErrorDto>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PolicyErrorDto {
    pub origin: String,
    pub message: String,
}

#[tauri::command]
pub async fn config_effective_settings(
    cwd: Option<String>,
) -> Result<EffectiveSettingsDto, String> {
    let cwd_path = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

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
                    winner: scope_label(&p.winner),
                    contributors: p.contributors.iter().map(scope_label).collect(),
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

fn render_path(segs: &[claudepot_core::config_view::model::JsonPathSeg]) -> String {
    use claudepot_core::config_view::model::JsonPathSeg;
    let mut out = String::new();
    for (i, seg) in segs.iter().enumerate() {
        match seg {
            JsonPathSeg::Key(k) => {
                if i > 0 {
                    out.push('.');
                }
                out.push_str(k);
            }
            JsonPathSeg::Index(idx) => {
                out.push_str(&format!("[{idx}]"));
            }
        }
    }
    out
}

fn scope_label(s: &Scope) -> String {
    match s {
        Scope::PluginBase => "plugin_base".into(),
        Scope::User => "user".into(),
        Scope::Project => "project".into(),
        Scope::Local => "local".into(),
        Scope::Flag => "flag".into(),
        Scope::Policy { origin } => format!("policy:{}", policy_origin_label(origin)),
        Scope::ClaudeMdDir { .. } => "claude_md_dir".into(),
        Scope::Plugin { id, .. } => format!("plugin:{id}"),
        Scope::MemoryCurrent => "memory_current".into(),
        Scope::MemoryOther { .. } => "memory_other".into(),
        Scope::Effective => "effective".into(),
        Scope::RedactedUserConfig => "redacted_user_config".into(),
        Scope::Other => "other".into(),
    }
}

fn policy_origin_label(o: &PolicyOrigin) -> String {
    match o {
        PolicyOrigin::Remote => "remote".into(),
        PolicyOrigin::MdmAdmin => "mdm_admin".into(),
        PolicyOrigin::ManagedFileComposite => "managed_file_composite".into(),
        PolicyOrigin::HkcuUser => "hkcu_user".into(),
    }
}

// ---------- Effective MCP (P5 UI) ------------------------------------

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum McpSimulationModeDto {
    Interactive,
    NonInteractive,
    SkipPermissions,
}

impl From<McpSimulationModeDto> for McpSimulationMode {
    fn from(d: McpSimulationModeDto) -> Self {
        match d {
            McpSimulationModeDto::Interactive => McpSimulationMode::Interactive,
            McpSimulationModeDto::NonInteractive => McpSimulationMode::NonInteractive,
            McpSimulationModeDto::SkipPermissions => McpSimulationMode::SkipPermissions,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct EffectiveMcpDto {
    pub enterprise_lockout: bool,
    pub servers: Vec<EffectiveMcpServerDto>,
}

#[derive(Serialize, Clone, Debug)]
pub struct EffectiveMcpServerDto {
    pub name: String,
    pub source_scope: String,
    pub contributors: Vec<String>,
    pub approval: String,
    /// E.g. "enable_all_project_mcp", "non_interactive_with_project_source_enabled"
    pub approval_reason: Option<String>,
    pub blocked_by: Option<String>,
    /// Server config JSON with secrets masked.
    pub masked: serde_json::Value,
}

#[tauri::command]
pub async fn config_effective_mcp(
    cwd: Option<String>,
    mode: Option<McpSimulationModeDto>,
) -> Result<EffectiveMcpDto, String> {
    let cwd_path = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
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
                    source_scope: scope_label(&s.source_scope),
                    contributors: s.contributors.iter().map(scope_label).collect(),
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

fn auto_reason_label(r: &AutoApprovalReason) -> String {
    match r {
        AutoApprovalReason::EnableAllProjectMcp => "enable_all_project_mcp".into(),
        AutoApprovalReason::NonInteractiveWithProjectSourceEnabled => {
            "non_interactive_with_project_source_enabled".into()
        }
        AutoApprovalReason::SkipPermissionsWithProjectSourceEnabled => {
            "skip_permissions_with_project_source_enabled".into()
        }
    }
}
