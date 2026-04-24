//! Config section data model.
//!
//! Types surfaced through the Config section: `Scope`, `Kind`,
//! `ConfigTree`, editor-detection helpers, and the `@include`
//! breadcrumb fields on `FileNode` (`included_by`, `include_depth`).
//! `Kind` tracks every artifact CC loads: memory files, the six
//! `CLAUDE_CONFIG_DIRECTORIES` (commands, agents, output-styles,
//! skills, workflows, rules), MCP sources (regular + managed), policy
//! settings, keybindings, and the redacted global config.
//!
//! All names mirror `dev-docs/config-section-plan.md`. Divergences
//! from CC's own types are called out at the field level.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

// ---------- Scope -----------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Scope {
    PluginBase,
    User,
    Project,
    Local,
    Flag,
    Policy { origin: PolicyOrigin },
    ClaudeMdDir { dir: PathBuf, role: ClaudeMdRole },
    Plugin { id: String, source: PluginSource },
    MemoryCurrent,
    MemoryOther { slug: String, lossy: bool },
    Effective,
    RedactedUserConfig,
    Other,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PolicyOrigin {
    Remote,
    MdmAdmin,
    ManagedFileComposite,
    HkcuUser,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeMdRole {
    Ancestor,
    Cwd,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PluginSource {
    Marketplace { spec: String },
    Builtin,
    Inline,
}

// ---------- Kind ------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    ClaudeMd,
    Settings,
    SettingsLocal,
    ManagedSettings,
    RedactedUserConfig,
    McpJson,
    ManagedMcpJson,
    Agent,
    Skill,
    Command,
    OutputStyle,
    Workflow,
    Rule,
    Hook,
    Memory,
    MemoryIndex,
    Plugin,
    Keybindings,
    Statusline,
    EffectiveSettings,
    EffectiveMcp,
    Other,
}

// ---------- Tree ------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigTree {
    pub scopes: Vec<ScopeNode>,
    pub scanned_at_unix_ns: i64,
    pub cwd: PathBuf,
    pub project_root: PathBuf,
    pub memory_slug: String,
    pub memory_slug_lossy: bool,
    pub cc_version_hint: Option<String>,
    pub enterprise_mcp_lockout: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScopeNode {
    pub id: String,
    pub scope: Scope,
    pub label: String,
    pub children: Vec<Node>,
    pub recursive_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    File(FileNode),
    Dir(DirNode),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileNode {
    pub id: String,
    pub kind: Kind,
    pub abs_path: PathBuf,
    pub display_path: String,
    pub scope_badges: Vec<Scope>,
    pub size_bytes: u64,
    pub mtime_unix_ns: i64,
    pub summary: Option<FileSummary>,
    pub issues: Vec<ParseIssue>,
    pub symlink_origin: Option<PathBuf>,
    /// When this file was reached via a `@include` chain, the absolute
    /// path of the memory file that pulled it in. `None` for the root
    /// memory file or any file not surfaced through the include
    /// resolver. See `memory_include::resolve_all`.
    #[serde(default)]
    pub included_by: Option<PathBuf>,
    /// Depth of this node in the `@include` chain (0 = root file).
    /// Lets the UI render a "depth: 2 ↑" breadcrumb without walking
    /// the parent chain.
    #[serde(default)]
    pub include_depth: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DirNode {
    pub id: String,
    pub abs_path: PathBuf,
    pub display_path: String,
    pub children: Vec<Node>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileSummary {
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ParseIssue {
    MalformedJson { message: String },
    NotASkill,
    SymlinkLoop,
    PermissionDenied,
    Other { message: String },
}

// ---------- Provenance ------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum JsonPathSeg {
    Key(String),
    Index(usize),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProvenanceEntry {
    pub key_path: Vec<JsonPathSeg>,
    pub winner: Scope,
    pub contributors: Vec<Scope>,
    /// True when a higher-precedence null/scalar clobbered a lower
    /// container — the leaf value "won" but there's hidden data below.
    pub suppressed: bool,
}

// ---------- Editor detection ------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct EditorCandidate {
    /// Stable id — e.g. "vscode", "cursor", "system", "env".
    pub id: String,
    pub label: String,
    pub binary_path: Option<PathBuf>,
    pub bundle_id: Option<String>,
    pub launch: LaunchKind,
    pub detected_via: DetectSource,
    pub supports_kinds: Option<Vec<Kind>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LaunchKind {
    Direct { args_template: Vec<String> },
    MacosOpenA { app_name: String },
    EnvEditor,
    SystemHandler,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DetectSource {
    PathBinary { which: PathBuf },
    MacosAppBundle { path: PathBuf },
    WindowsRegistry { key: String },
    LinuxDesktopFile { path: PathBuf },
    EnvVar { name: String },
    SystemDefault,
    UserPicked { path: PathBuf },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EditorDefaults {
    pub by_kind: BTreeMap<Kind, String>,
    /// Global fallback; `"system"` means the OS default handler.
    #[serde(default = "default_fallback")]
    pub fallback: String,
}

fn default_fallback() -> String {
    "system".to_string()
}

impl Default for EditorDefaults {
    fn default() -> Self {
        Self {
            by_kind: BTreeMap::new(),
            fallback: default_fallback(),
        }
    }
}

impl EditorDefaults {
    pub fn new() -> Self {
        Self::default()
    }
}
