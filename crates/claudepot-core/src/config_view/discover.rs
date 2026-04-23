//! Filesystem discovery for the Config section.
//!
//! Walks the CC-mandated roots (`dev-docs/config-section-plan.md` §6.1)
//! and returns a scope-first tree of `FileNode`s. Hard deny-list per
//! §6.3 keeps noisy caches out.
//!
//! Strategy:
//! - Each scope has its own `collect_*` function returning `Vec<FileNode>`.
//! - `assemble_tree` groups them into `ScopeNode`s with fixed root rank
//!   (plan §11.5).
//! - Parse results from `parse::*` are attached per file.
//! - **No** I/O beyond filesystem reads — no subprocesses, no network.

use crate::config_view::model::{
    ClaudeMdRole, ConfigTree, FileNode, FileSummary, Kind, Node,
    ParseIssue, PolicyOrigin, Scope, ScopeNode,
};
use crate::config_view::parse;
use crate::path_utils::simplify_windows_path;
use crate::paths::claude_config_dir;
use crate::project_sanitize::sanitize_path;
use std::path::{Path, PathBuf};

// ---------- Deny-list (§6.3) ------------------------------------------

const DENY_NAMES: &[&str] = &[
    "file-history",
    "paste-cache",
    "image-cache",
    "previews",
    "debug",
    "cache",
    "downloads",
    "backups",
    ".stfolder",
    ".stignore",
    ".DS_Store",
    ".claude-global-index.db",
    "ide",
    "chrome",
    "statsig",
    ".cometapi-count",
    "projects",
    "todos",
    "shell-snapshots",
];

const DENY_PREFIXES: &[&str] = &[
    "history.jsonl",
    "security_warnings_state_",
];

fn is_denied(name: &str) -> bool {
    if DENY_NAMES.contains(&name) {
        return true;
    }
    if DENY_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return true;
    }
    if name.contains(".sync-conflict-") {
        return true;
    }
    if name.ends_with(".bak") || name.contains(".bak.") {
        return true;
    }
    // Skip `.claude.json` raw — the `RedactedUserConfig` scope renders it.
    if name == ".claude.json" {
        return true;
    }
    false
}

// ---------- Low-level file-node construction -------------------------

fn blake3_id(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    // We don't need blake3 at all; a 16-char sha256 hex is stable and
    // consistent. Rename the helper name to match the plan's intent but
    // keep it dependency-free.
    let mut h = Sha256::new();
    h.update(p.display().to_string().as_bytes());
    let out = h.finalize();
    hex::encode(out)[..16].to_string()
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn make_file_node(path: &Path, kind: Kind, scope: Scope, parsed: parse::Parsed) -> FileNode {
    let simplified = simplify_windows_path(&path.display().to_string());
    let display = simplified.clone();
    let abs = PathBuf::from(simplified);
    let meta = std::fs::metadata(&abs).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta.as_ref().map(mtime_ns).unwrap_or(0);
    let id = blake3_id(&abs);

    FileNode {
        id,
        kind,
        abs_path: abs,
        display_path: display,
        scope_badges: vec![scope],
        size_bytes: size,
        mtime_unix_ns: mtime,
        summary: parsed.summary,
        issues: parsed.issues,
        symlink_origin: None,
    }
}

fn read_head(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(limit as usize);
    let _ = f.take(limit).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn parse_file(path: &Path, kind: &Kind) -> parse::Parsed {
    let Some(bytes) = read_head(path, 64 * 1024) else {
        return parse::Parsed {
            summary: None,
            issues: vec![ParseIssue::PermissionDenied],
        };
    };
    match kind {
        Kind::Settings | Kind::SettingsLocal | Kind::ManagedSettings | Kind::Keybindings => {
            parse::parse_settings_json(&bytes)
        }
        Kind::McpJson | Kind::ManagedMcpJson => parse::parse_settings_json(&bytes),
        Kind::ClaudeMd => parse::parse_claude_md(&bytes),
        Kind::Agent | Kind::Rule | Kind::Command => parse::parse_frontmatter_markdown(&bytes),
        Kind::Skill => parse::parse_frontmatter_markdown(&bytes),
        Kind::Memory => parse::parse_memory_head(&bytes),
        Kind::MemoryIndex => parse::parse_memory_index(&bytes).1,
        _ => parse::Parsed::empty(),
    }
}

fn maybe_file(path: PathBuf, kind: Kind, scope: Scope) -> Option<FileNode> {
    if !path.is_file() {
        return None;
    }
    let parsed = parse_file(&path, &kind);
    Some(make_file_node(&path, kind, scope, parsed))
}

// ---------- Per-scope collectors --------------------------------------

pub fn collect_user() -> Vec<FileNode> {
    let home = claude_config_dir();
    let mut out = Vec::new();

    out.extend(maybe_file(home.join("settings.json"), Kind::Settings, Scope::User));
    out.extend(maybe_file(
        home.join("keybindings.json"),
        Kind::Keybindings,
        Scope::User,
    ));
    out.extend(maybe_file(home.join("CLAUDE.md"), Kind::ClaudeMd, Scope::User));

    out.extend(collect_dir_of_kind(
        &home.join("agents"),
        Kind::Agent,
        Scope::User,
        true,
    ));
    out.extend(collect_skills_dir(&home.join("skills"), Scope::User));
    out.extend(collect_dir_of_kind(
        &home.join("commands"),
        Kind::Command,
        Scope::User,
        true,
    ));
    out.extend(collect_dir_of_kind(
        &home.join("rules"),
        Kind::Rule,
        Scope::User,
        true,
    ));

    out
}

pub fn collect_project(cwd: &Path) -> Vec<FileNode> {
    let dotclaude = cwd.join(".claude");
    let mut out = Vec::new();

    out.extend(maybe_file(
        dotclaude.join("settings.json"),
        Kind::Settings,
        Scope::Project,
    ));
    out.extend(collect_dir_of_kind(
        &dotclaude.join("agents"),
        Kind::Agent,
        Scope::Project,
        true,
    ));
    out.extend(collect_skills_dir(&dotclaude.join("skills"), Scope::Project));
    out.extend(collect_dir_of_kind(
        &dotclaude.join("commands"),
        Kind::Command,
        Scope::Project,
        true,
    ));
    out.extend(collect_dir_of_kind(
        &dotclaude.join("rules"),
        Kind::Rule,
        Scope::Project,
        true,
    ));
    // CLAUDE.md here is handled by claudemd_walk.
    out
}

pub fn collect_local(cwd: &Path) -> Vec<FileNode> {
    let mut out = Vec::new();
    out.extend(maybe_file(
        cwd.join(".claude").join("settings.local.json"),
        Kind::SettingsLocal,
        Scope::Local,
    ));
    out
}

/// Walk from cwd up to canonical git-root (or filesystem root as fallback)
/// collecting `CLAUDE.md` + `.claude/CLAUDE.md` at each level. Per
/// `dev-docs/config-section-plan.md` §6.4.
pub fn collect_claudemd_walk(cwd: &Path) -> Vec<(PathBuf, ClaudeMdRole, FileNode)> {
    let mut out = Vec::new();
    let stop = find_stop_boundary(cwd);
    let dirs = ancestors_up_to(cwd, stop.as_deref());

    for (i, dir) in dirs.iter().enumerate() {
        let role = if i + 1 == dirs.len() {
            ClaudeMdRole::Cwd
        } else {
            ClaudeMdRole::Ancestor
        };
        for candidate in [dir.join("CLAUDE.md"), dir.join(".claude").join("CLAUDE.md")] {
            if let Some(f) = maybe_file(
                candidate,
                Kind::ClaudeMd,
                Scope::ClaudeMdDir {
                    dir: dir.clone(),
                    role: role.clone(),
                },
            ) {
                out.push((dir.clone(), role.clone(), f));
            }
        }
    }
    out
}

fn find_stop_boundary(cwd: &Path) -> Option<PathBuf> {
    // Stop at `.git` dir or home — whichever is nearer.
    let home = dirs::home_dir();
    let mut cur = Some(cwd.to_path_buf());
    while let Some(d) = cur {
        if d.join(".git").exists() {
            return Some(d);
        }
        if Some(&d) == home.as_ref() {
            return Some(d);
        }
        cur = d.parent().map(|p| p.to_path_buf());
    }
    None
}

fn ancestors_up_to(cwd: &Path, stop: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = Some(cwd.to_path_buf());
    while let Some(d) = cur {
        out.push(d.clone());
        if let Some(s) = stop {
            if s == d {
                break;
            }
        }
        cur = d.parent().map(|p| p.to_path_buf());
    }
    out.reverse(); // shallow → deep
    out
}

/// Memory dir for current project (per `getAutoMemPath`, simplified).
pub fn collect_memory_current(project_root: &Path) -> (Vec<FileNode>, String, bool) {
    let base = claude_config_dir().join("projects");
    let slug = sanitize_path(&project_root.display().to_string());
    let dir = base.join(&slug).join("memory");
    let lossy = false;
    if !dir.is_dir() {
        return (Vec::new(), slug, lossy);
    }
    let mut files = Vec::new();
    walk_memory_dir(&dir, &mut files);
    (files, slug, lossy)
}

fn walk_memory_dir(dir: &Path, out: &mut Vec<FileNode>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_denied(&name) {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            walk_memory_dir(&path, out);
        } else if ft.is_file() {
            let kind = if name.eq_ignore_ascii_case("MEMORY.md") {
                Kind::MemoryIndex
            } else if name.ends_with(".md") {
                Kind::Memory
            } else {
                continue;
            };
            if let Some(f) = maybe_file(path, kind, Scope::MemoryCurrent) {
                out.push(f);
            }
        }
    }
}

/// Managed settings composite — `managed-settings.json` + any drop-ins
/// under `managed-settings.d/`. Surfaces one `FileNode` per file so the
/// user can inspect each contributor; the effective composite is resolved
/// by `policy::policy_resolve`.
pub fn collect_policy_managed_files() -> Vec<FileNode> {
    let home = claude_config_dir();
    let mut out = Vec::new();
    if let Some(f) = maybe_file(
        home.join("managed-settings.json"),
        Kind::ManagedSettings,
        Scope::Policy {
            origin: PolicyOrigin::ManagedFileComposite,
        },
    ) {
        out.push(f);
    }
    if let Ok(rd) = std::fs::read_dir(home.join("managed-settings.d")) {
        let mut entries: Vec<PathBuf> = rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.is_file() && p.extension().is_some_and(|ext| ext == "json") {
                    Some(p)
                } else {
                    None
                }
            })
            .collect();
        entries.sort();
        for p in entries {
            if let Some(f) = maybe_file(
                p,
                Kind::ManagedSettings,
                Scope::Policy {
                    origin: PolicyOrigin::ManagedFileComposite,
                },
            ) {
                out.push(f);
            }
        }
    }
    out
}

/// RedactedUserConfig — `~/.claude.json` IF present. Parser is a no-op
/// at the file level; actual redaction lives in P2's
/// `redacted_claude_json.rs`. For P1 we just surface the file node.
pub fn collect_redacted_user_config() -> Option<FileNode> {
    let p = claude_config_dir().join(".claude.json");
    if !p.is_file() {
        return None;
    }
    let meta = std::fs::metadata(&p).ok()?;
    let size = meta.len();
    let mtime = mtime_ns(&meta);
    let display = p.display().to_string();
    let id = blake3_id(&p);
    Some(FileNode {
        id,
        kind: Kind::RedactedUserConfig,
        abs_path: p,
        display_path: display,
        scope_badges: vec![Scope::RedactedUserConfig],
        size_bytes: size,
        mtime_unix_ns: mtime,
        summary: Some(FileSummary {
            title: Some("Global config".to_string()),
            description: Some("Redacted view of ~/.claude.json".to_string()),
        }),
        issues: Vec::new(),
        symlink_origin: None,
    })
}

// ---------- Internals -------------------------------------------------

fn collect_dir_of_kind(
    dir: &Path,
    kind: Kind,
    scope: Scope,
    recurse: bool,
) -> Vec<FileNode> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_denied(&name) {
            continue;
        }
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            if recurse {
                out.extend(collect_dir_of_kind(&path, kind.clone(), scope.clone(), true));
            }
        } else if ft.is_file() && name.ends_with(".md") {
            if let Some(f) = maybe_file(path, kind.clone(), scope.clone()) {
                out.push(f);
            }
        }
    }
    out
}

/// Skills directory: strict shape — `<name>/SKILL.md` only. Flat `.md`
/// under `skills/` is flagged `NotASkill` (plan §6.5).
fn collect_skills_dir(dir: &Path, scope: Scope) -> Vec<FileNode> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_denied(&name) {
            continue;
        }
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.is_file() {
                if let Some(f) = maybe_file(skill_md, Kind::Skill, scope.clone()) {
                    out.push(f);
                }
            }
        } else if ft.is_file() && name.ends_with(".md") {
            // Flat `.md` — invalid per CC's strict rule.
            let path_c = path.clone();
            out.push(FileNode {
                id: blake3_id(&path_c),
                kind: Kind::Skill,
                abs_path: path_c.clone(),
                display_path: path_c.display().to_string(),
                scope_badges: vec![scope.clone()],
                size_bytes: std::fs::metadata(&path_c).map(|m| m.len()).unwrap_or(0),
                mtime_unix_ns: std::fs::metadata(&path_c)
                    .ok()
                    .map(|m| mtime_ns(&m))
                    .unwrap_or(0),
                summary: None,
                issues: vec![ParseIssue::NotASkill],
                symlink_origin: None,
            });
        }
    }
    out
}

// ---------- Tree assembly --------------------------------------------

/// Build the full read-only `ConfigTree` anchored at `cwd`. Produces
/// scope roots in the fixed rank defined by plan §11.5.
pub fn assemble_tree(cwd: &Path) -> ConfigTree {
    let project_root =
        crate::project_memory::find_canonical_git_root(cwd).unwrap_or_else(|| cwd.to_path_buf());

    let (memory_files, memory_slug, memory_slug_lossy) =
        collect_memory_current(&project_root);

    let mut scopes: Vec<ScopeNode> = Vec::new();

    if let Some(rc) = collect_redacted_user_config() {
        scopes.push(scope_node(
            "scope:redacted",
            Scope::RedactedUserConfig,
            "Global config (redacted)",
            vec![Node::File(rc)],
        ));
    }

    let user_files = collect_user();
    if !user_files.is_empty() {
        scopes.push(scope_node(
            "scope:user",
            Scope::User,
            "User (~/.claude)",
            user_files.into_iter().map(Node::File).collect(),
        ));
    }

    let project_files = collect_project(cwd);
    if !project_files.is_empty() {
        scopes.push(scope_node(
            "scope:project",
            Scope::Project,
            "Project (cwd/.claude)",
            project_files.into_iter().map(Node::File).collect(),
        ));
    }

    let local_files = collect_local(cwd);
    if !local_files.is_empty() {
        scopes.push(scope_node(
            "scope:local",
            Scope::Local,
            "Local (settings.local.json)",
            local_files.into_iter().map(Node::File).collect(),
        ));
    }

    let claudemd = collect_claudemd_walk(cwd);
    for (dir, role, f) in claudemd {
        let label = format!(
            "CLAUDE.md — {}{}",
            dir.display(),
            if matches!(role, ClaudeMdRole::Cwd) { " (cwd)" } else { "" },
        );
        scopes.push(scope_node(
            &format!("scope:claudemd:{}", f.id),
            Scope::ClaudeMdDir { dir: dir.clone(), role: role.clone() },
            &label,
            vec![Node::File(f)],
        ));
    }

    let policy_files = collect_policy_managed_files();
    if !policy_files.is_empty() {
        scopes.push(scope_node(
            "scope:policy:managed",
            Scope::Policy {
                origin: PolicyOrigin::ManagedFileComposite,
            },
            "Policy (managed-settings)",
            policy_files.into_iter().map(Node::File).collect(),
        ));
    }

    if !memory_files.is_empty() {
        scopes.push(scope_node(
            "scope:memory-current",
            Scope::MemoryCurrent,
            "Memory (this project)",
            memory_files.into_iter().map(Node::File).collect(),
        ));
    }

    ConfigTree {
        scopes,
        scanned_at_unix_ns: current_unix_ns(),
        cwd: cwd.to_path_buf(),
        project_root,
        memory_slug,
        memory_slug_lossy,
        cc_version_hint: None,
        enterprise_mcp_lockout: false,
    }
}

fn scope_node(id: &str, scope: Scope, label: &str, children: Vec<Node>) -> ScopeNode {
    let n = children.len();
    let mut sn = ScopeNode {
        id: id.to_string(),
        scope,
        label: label.to_string(),
        children,
        recursive_count: 0,
    };
    sn.recursive_count = count_nodes(&sn.children);
    let _ = n;
    // Canonical child order: Dir first, then File, ASCII-case-insensitive by label/name.
    sn.children.sort_by(sort_child);
    sn
}

fn count_nodes(nodes: &[Node]) -> usize {
    let mut n = 0;
    for node in nodes {
        match node {
            Node::File(_) => n += 1,
            Node::Dir(d) => {
                n += 1 + count_nodes(&d.children);
            }
        }
    }
    n
}

fn sort_child(a: &Node, b: &Node) -> std::cmp::Ordering {
    let a_is_dir = matches!(a, Node::Dir(_));
    let b_is_dir = matches!(b, Node::Dir(_));
    match (a_is_dir, b_is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => {
            let (an, bn) = (node_label(a), node_label(b));
            an.to_ascii_lowercase().cmp(&bn.to_ascii_lowercase())
        }
    }
}

fn node_label(n: &Node) -> &str {
    match n {
        Node::File(f) => {
            f.summary
                .as_ref()
                .and_then(|s| s.title.as_deref())
                .unwrap_or(&f.display_path)
        }
        Node::Dir(d) => &d.display_path,
    }
}

fn current_unix_ns() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or(0)
}

/// Lookup a file node in the tree by id. Used for preview / reveal.
pub fn find_file<'a>(tree: &'a ConfigTree, id: &str) -> Option<&'a FileNode> {
    for scope in &tree.scopes {
        if let Some(f) = find_file_in_nodes(&scope.children, id) {
            return Some(f);
        }
    }
    None
}

fn find_file_in_nodes<'a>(nodes: &'a [Node], id: &str) -> Option<&'a FileNode> {
    for n in nodes {
        match n {
            Node::File(f) if f.id == id => return Some(f),
            Node::Dir(d) => {
                if let Some(f) = find_file_in_nodes(&d.children, id) {
                    return Some(f);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn deny_names_catches_known_junk() {
        assert!(is_denied("paste-cache"));
        assert!(is_denied("cache"));
        assert!(is_denied(".DS_Store"));
        assert!(is_denied("history.jsonl"));
        assert!(is_denied("foo.sync-conflict-x.txt"));
        assert!(is_denied(".claude.json"));
        assert!(!is_denied("settings.json"));
    }

    #[test]
    fn skills_strict_dir_shape() {
        let td = TempDir::new().unwrap();
        let skills = td.path().join("skills");
        std::fs::create_dir_all(skills.join("good")).unwrap();
        std::fs::write(skills.join("good").join("SKILL.md"), "# title\nbody").unwrap();
        std::fs::write(skills.join("flat.md"), "# invalid").unwrap();

        let files = collect_skills_dir(&skills, Scope::User);
        // One valid + one NotASkill row.
        assert_eq!(files.len(), 2);
        let good = files.iter().find(|f| f.issues.is_empty()).unwrap();
        assert!(good.display_path.ends_with("SKILL.md"));
        let bad = files.iter().find(|f| !f.issues.is_empty()).unwrap();
        assert!(matches!(bad.issues[0], ParseIssue::NotASkill));
    }

    #[test]
    fn assemble_tree_over_empty_dir_is_fine() {
        let td = TempDir::new().unwrap();
        let tree = assemble_tree(td.path());
        // No panics; ConfigTree built.
        assert_eq!(tree.cwd, td.path());
    }

    #[test]
    fn ancestors_shallow_to_deep() {
        let root = PathBuf::from("/a/b/c/d");
        let stop = PathBuf::from("/a/b");
        let a = ancestors_up_to(&root, Some(&stop));
        assert_eq!(a, vec![
            PathBuf::from("/a/b"),
            PathBuf::from("/a/b/c"),
            PathBuf::from("/a/b/c/d"),
        ]);
    }
}
