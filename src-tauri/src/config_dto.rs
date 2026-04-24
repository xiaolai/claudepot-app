//! Shared serde DTOs and label helpers for the Config tree.
//!
//! Owned by both `commands_config` (one-shot scan) and `config_watch`
//! (live patches). Both produce the same shape so the React reducer
//! can apply scan + patch with one code path. Defining the shape twice
//! lets the two surfaces silently drift, so this module is the single
//! source of truth for `FileNodeDto` and the kind/issue/scope label
//! converters.

use claudepot_core::config_view::model::{
    FileNode, FileSummary, Kind, Node, ParseIssue, PolicyOrigin, Scope,
};
use serde::Serialize;

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
    /// Absolute path of the memory file that `@include`-pulled this
    /// one. `None` for root files.
    pub included_by: Option<String>,
    /// Depth in the `@include` chain (0 = root, 1 = direct include).
    pub include_depth: usize,
}

impl From<&FileNode> for FileNodeDto {
    fn from(f: &FileNode) -> Self {
        file_to_dto(f)
    }
}

/// Build a `FileNodeDto` from a core `FileNode`. Same as the `From`
/// impl, exposed by name for callers that already have a `&FileNode`
/// and prefer reading code that says what it does.
pub fn file_to_dto(f: &FileNode) -> FileNodeDto {
    FileNodeDto {
        id: f.id.clone(),
        kind: kind_label(&f.kind),
        abs_path: f.abs_path.display().to_string(),
        display_path: f.display_path.clone(),
        size_bytes: f.size_bytes,
        mtime_unix_ns: f.mtime_unix_ns,
        summary_title: f
            .summary
            .as_ref()
            .and_then(|s: &FileSummary| s.title.clone()),
        summary_description: f
            .summary
            .as_ref()
            .and_then(|s: &FileSummary| s.description.clone()),
        issues: f.issues.iter().map(issue_label).collect(),
        included_by: f.included_by.as_ref().map(|p| p.display().to_string()),
        include_depth: f.include_depth,
    }
}

/// Recursively gather every `File` node into a flat DTO list. Dirs
/// disappear — the React tree builds groups from `display_path`.
pub fn flatten_files(nodes: &[Node]) -> Vec<FileNodeDto> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::File(f) => out.push(file_to_dto(f)),
            Node::Dir(d) => out.extend(flatten_files(&d.children)),
        }
    }
    out
}

/// Reference-flavored variant of `flatten_files` for callers that need
/// to inspect the underlying `FileNode` (rather than emit a DTO).
pub fn flatten_file_refs<'a>(nodes: &'a [Node]) -> Vec<&'a FileNode> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::File(f) => out.push(f),
            Node::Dir(d) => out.extend(flatten_file_refs(&d.children)),
        }
    }
    out
}

/// Stringify a `Kind` for the JS boundary. The frontend uses these
/// exact strings as discriminants — never rename without coordinating.
pub fn kind_to_str(k: &Kind) -> &'static str {
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
        Kind::OutputStyle => "output_style",
        Kind::Workflow => "workflow",
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

pub fn kind_label(k: &Kind) -> String {
    kind_to_str(k).to_string()
}

pub fn kind_from_str(s: &str) -> Option<Kind> {
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
        "output_style" => Kind::OutputStyle,
        "workflow" => Kind::Workflow,
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

pub fn issue_label(i: &ParseIssue) -> String {
    match i {
        ParseIssue::MalformedJson { message } => format!("malformed_json: {message}"),
        ParseIssue::NotASkill => "not_a_skill".to_string(),
        ParseIssue::SymlinkLoop => "symlink_loop".to_string(),
        ParseIssue::PermissionDenied => "permission_denied".to_string(),
        ParseIssue::Other { message } => format!("other: {message}"),
    }
}

/// Plain scope kind without payload — used by the tree views and the
/// watcher patch payload. For the variant-rich label that includes
/// policy origin / plugin id, see [`scope_label_with_origin`].
pub fn scope_kind_label(s: &Scope) -> String {
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

/// Variant-rich label — used by provenance reporting where the policy
/// origin (`policy:remote`, `policy:mdm_admin`, etc.) and plugin id
/// (`plugin:my-plugin`) need to round-trip to the UI. Plain
/// kind-only labels go through [`scope_kind_label`].
pub fn scope_label_with_origin(s: &Scope) -> String {
    match s {
        Scope::PluginBase => "plugin_base".to_string(),
        Scope::User => "user".to_string(),
        Scope::Project => "project".to_string(),
        Scope::Local => "local".to_string(),
        Scope::Flag => "flag".to_string(),
        Scope::Policy { origin } => format!("policy:{}", policy_origin_label(origin)),
        Scope::ClaudeMdDir { .. } => "claude_md_dir".to_string(),
        Scope::Plugin { id, .. } => format!("plugin:{id}"),
        Scope::MemoryCurrent => "memory_current".to_string(),
        Scope::MemoryOther { .. } => "memory_other".to_string(),
        Scope::Effective => "effective".to_string(),
        Scope::RedactedUserConfig => "redacted_user_config".to_string(),
        Scope::Other => "other".to_string(),
    }
}

pub fn policy_origin_label(o: &PolicyOrigin) -> String {
    match o {
        PolicyOrigin::Remote => "remote".to_string(),
        PolicyOrigin::MdmAdmin => "mdm_admin".to_string(),
        PolicyOrigin::ManagedFileComposite => "managed_file_composite".to_string(),
        PolicyOrigin::HkcuUser => "hkcu_user".to_string(),
    }
}
