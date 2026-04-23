//! Config section data model.
//!
//! P0 ships the **stable subset** needed to wire the section in: `Scope`,
//! `Kind`, `ConfigTree`, `EditorCandidate`, `EditorDefaults`, plus a few
//! support enums. The richer `Annotated` / `ProvenanceEntry` /
//! `ConfigTreePatch` types land alongside the scanners that populate
//! them in later phases (see `dev-docs/config-section-plan.md` §5).

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
