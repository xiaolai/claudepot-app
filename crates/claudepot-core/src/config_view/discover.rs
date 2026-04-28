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

use crate::config_view::memory_include;
use crate::config_view::model::{
    ClaudeMdRole, ConfigTree, FileNode, FileSummary, Kind, Node, ParseIssue, PolicyOrigin, Scope,
    ScopeNode,
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
    // Lifecycle stash — `artifact_lifecycle` moves disabled skills/
    // agents/commands into `<scope_root>/.disabled/`. The active
    // Config tree must NOT walk into it (otherwise disabled
    // artifacts would re-appear under their kind groups). The
    // lifecycle module's own discover walks `.disabled/` directly
    // and bypasses this list intentionally.
    crate::artifact_lifecycle::DISABLED_DIR,
];

const DENY_PREFIXES: &[&str] = &["history.jsonl", "security_warnings_state_"];

/// True when `name` matches the shared Config-section deny-list
/// (`dev-docs/config-section-plan.md` §6.3). This function is the
/// single source of truth — `config_view::watch::is_in_scope` and the
/// Tauri-side watcher BOTH call it so the watcher never wakes up for
/// files discovery would ignore.
pub fn is_denied(name: &str) -> bool {
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
        included_by: None,
        include_depth: 0,
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
        Kind::Agent | Kind::Rule | Kind::Command | Kind::OutputStyle | Kind::Workflow => {
            parse::parse_frontmatter_markdown(&bytes)
        }
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

    out.extend(maybe_file(
        home.join("settings.json"),
        Kind::Settings,
        Scope::User,
    ));
    out.extend(maybe_file(
        home.join("keybindings.json"),
        Kind::Keybindings,
        Scope::User,
    ));
    out.extend(maybe_file(
        home.join("CLAUDE.md"),
        Kind::ClaudeMd,
        Scope::User,
    ));

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
    // `output-styles` and `workflows` are two more
    // `CLAUDE_CONFIG_DIRECTORIES` per `markdownConfigLoader.ts:29-36`.
    out.extend(collect_dir_of_kind(
        &home.join("output-styles"),
        Kind::OutputStyle,
        Scope::User,
        true,
    ));
    out.extend(collect_dir_of_kind(
        &home.join("workflows"),
        Kind::Workflow,
        Scope::User,
        true,
    ));

    out
}

pub fn collect_project(cwd: &Path) -> Vec<FileNode> {
    let dotclaude = cwd.join(".claude");
    let mut out = Vec::new();

    // Single `settings.json` at cwd — CC does not walk for settings.
    out.extend(maybe_file(
        dotclaude.join("settings.json"),
        Kind::Settings,
        Scope::Project,
    ));

    // Agents / skills / commands / output-styles / workflows: CC walks
    // cwd → git-root OR home via `getProjectDirsUpToHome`
    // (`markdownConfigLoader.ts:234`). Dedup by abs_path so symlinked
    // repos don't produce duplicate entries — stricter dedup by inode
    // lands later with E1.
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for dir in ancestors_for(cwd, WalkBoundary::GitRootOrHome) {
        let dc = dir.join(".claude");
        for f in collect_dir_of_kind(&dc.join("agents"), Kind::Agent, Scope::Project, true)
            .into_iter()
            .chain(collect_skills_dir(&dc.join("skills"), Scope::Project))
            .chain(collect_dir_of_kind(
                &dc.join("commands"),
                Kind::Command,
                Scope::Project,
                true,
            ))
            .chain(collect_dir_of_kind(
                &dc.join("output-styles"),
                Kind::OutputStyle,
                Scope::Project,
                true,
            ))
            .chain(collect_dir_of_kind(
                &dc.join("workflows"),
                Kind::Workflow,
                Scope::Project,
                true,
            ))
        {
            if seen.insert(f.abs_path.clone()) {
                out.push(f);
            }
        }
    }
    // Rules walk cwd-upward — see `collect_rules_walk`. CLAUDE.md walks
    // via `collect_claudemd_walk`.
    out
}

/// Walk cwd upward collecting every `.claude/rules/**/*.md` at each
/// level. Memory-file walk — bounded by FS root per
/// `claudemd.ts:909-919` (rules are loaded as Project memory by
/// `processMdRules`, which runs inside the `while (currentDir !==
/// parse(currentDir).root)` loop at `claudemd.ts:854`).
pub fn collect_rules_walk(cwd: &Path) -> Vec<FileNode> {
    let mut out = Vec::new();
    for dir in ancestors_for(cwd, WalkBoundary::FsRoot) {
        let rules_dir = dir.join(".claude").join("rules");
        out.extend(collect_dir_of_kind(
            &rules_dir,
            Kind::Rule,
            Scope::Project,
            true,
        ));
    }
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

/// Walk from cwd to filesystem root collecting `CLAUDE.md` +
/// `.claude/CLAUDE.md` at each ancestor. Matches CC's memory walk at
/// `claudemd.ts:854` — goes to `parse(currentDir).root`, NOT bounded
/// by git-root.
pub fn collect_claudemd_walk(cwd: &Path) -> Vec<(PathBuf, ClaudeMdRole, FileNode)> {
    let mut out = Vec::new();
    let dirs = ancestors_for(cwd, WalkBoundary::FsRoot);

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

/// Walk cwd → FS root collecting `CLAUDE.local.md` at each ancestor.
/// CC loads these as Local memory (`claudemd.ts:922-933`). Each file
/// gets its own scope so contributors stay distinct in the tree.
pub fn collect_claudemd_local_walk(cwd: &Path) -> Vec<(PathBuf, ClaudeMdRole, FileNode)> {
    let mut out = Vec::new();
    let dirs = ancestors_for(cwd, WalkBoundary::FsRoot);

    for (i, dir) in dirs.iter().enumerate() {
        let role = if i + 1 == dirs.len() {
            ClaudeMdRole::Cwd
        } else {
            ClaudeMdRole::Ancestor
        };
        if let Some(f) = maybe_file(dir.join("CLAUDE.local.md"), Kind::ClaudeMd, Scope::Local) {
            out.push((dir.clone(), role.clone(), f));
        }
    }
    out
}

/// Walk cwd → FS root collecting `.mcp.json` at each ancestor. CC's
/// MCP project-scope loader walks all the way to
/// `parse(currentDir).root` (`services/mcp/config.ts:914-920`).
pub fn collect_mcp_json_walk(cwd: &Path) -> Vec<FileNode> {
    let mut out = Vec::new();
    for dir in ancestors_for(cwd, WalkBoundary::FsRoot) {
        if let Some(f) = maybe_file(dir.join(".mcp.json"), Kind::McpJson, Scope::Project) {
            out.push(f);
        }
    }
    out
}

/// Boundary strategy used by each walk. CC distinguishes two:
/// - `GitRootOrHome` — agents/skills/commands via `getProjectDirsUpToHome`
///   (`markdownConfigLoader.ts:234-289`). Stops at first `.git` OR home.
/// - `FsRoot` — memory walk (CLAUDE.md, CLAUDE.local.md, .claude/CLAUDE.md,
///   .claude/rules, .mcp.json) via `claudemd.ts:854` and
///   `services/mcp/config.ts:914-920`. Walks all the way to the filesystem
///   root; does NOT stop at .git.
pub enum WalkBoundary {
    GitRootOrHome,
    FsRoot,
}

/// Compute the list of ancestor dirs under the chosen `WalkBoundary`.
/// Output is shallow → deep (root first, cwd last), matching how CC
/// processes memory files so deeper overrides shallower.
///
/// `GitRootOrHome`: includes the nearest `.git` dir (if found) and
/// stops there. If no `.git` is found, stops at — and **excludes** —
/// `$HOME`. CC's `getProjectDirsUpToHome` (`markdownConfigLoader.ts:244-
/// 251`) breaks the loop *before* processing the home directory so
/// `~/.claude/{agents,…}` never leaks into Project scope.
///
/// `FsRoot`: walks cwd up to (but **not including**) the filesystem
/// root. CC's memory loader uses `while (currentDir !==
/// parse(currentDir).root)` at `claudemd.ts:854`, so `/CLAUDE.md` and
/// `/.mcp.json` are never loaded.
fn ancestors_for(cwd: &Path, boundary: WalkBoundary) -> Vec<PathBuf> {
    let home = dirs::home_dir();
    let fs_root = filesystem_root_of(cwd);
    let mut out = Vec::new();
    let mut cur = Some(cwd.to_path_buf());
    while let Some(d) = cur {
        match boundary {
            WalkBoundary::GitRootOrHome => {
                if home.as_ref() == Some(&d) {
                    // Exclude `$HOME` itself — CC stops BEFORE adding it.
                    break;
                }
                out.push(d.clone());
                if d.join(".git").exists() {
                    break;
                }
            }
            WalkBoundary::FsRoot => {
                if Some(&d) == fs_root.as_ref() {
                    // Exclude the filesystem root.
                    break;
                }
                out.push(d.clone());
            }
        }
        cur = d.parent().map(|p| p.to_path_buf());
    }
    out.reverse();
    out
}

/// Return the filesystem root for an arbitrary path (e.g. `/` on Unix,
/// `C:\` on Windows). Matches JS `path.parse(dir).root`.
fn filesystem_root_of(p: &Path) -> Option<PathBuf> {
    let mut cur = p.to_path_buf();
    while let Some(parent) = cur.parent() {
        cur = parent.to_path_buf();
    }
    Some(cur)
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
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
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

/// Enabled plugin scan (marketplaces + builtins). Returns a list of
/// per-plugin file nodes (manifest + settings) for display, plus a
/// `Vec<plugin_base::Plugin>` the merge cascade will consume.
pub fn collect_plugins() -> (Vec<FileNode>, Vec<crate::config_view::plugin_base::Plugin>) {
    use crate::config_view::plugin_base::{
        load_plugin_manifest, load_plugin_settings, Plugin, PluginSourceDisplay,
    };

    let home = claude_config_dir();
    let enabled_specs = load_enabled_plugin_specs(&home);
    // On-disk layout:
    //   ~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/
    //       ├── .claude-plugin/plugin.json  (or plugin.json)
    //       ├── agents/  skills/  commands/  …
    // Older docs referenced a `repos` directory — CC switched to
    // `cache` with per-version subdirs. We iterate three levels
    // and treat each version dir as an independent plugin root so
    // side-by-side version installs stay distinct in the UI.
    let root = home.join("plugins").join("cache");
    let mut files = Vec::new();
    let mut plugins = Vec::new();
    let Ok(rd) = std::fs::read_dir(&root) else {
        return (files, plugins);
    };
    for marketplace in rd.flatten() {
        if !marketplace.path().is_dir() {
            continue;
        }
        let Ok(plug_rd) = std::fs::read_dir(marketplace.path()) else {
            continue;
        };
        for plug in plug_rd.flatten() {
            if !plug.path().is_dir() {
                continue;
            }
            let Ok(version_rd) = std::fs::read_dir(plug.path()) else {
                continue;
            };
            for version in version_rd.flatten() {
                let plug_root = version.path();
                if !plug_root.is_dir() {
                    continue;
                }
                let manifest = match load_plugin_manifest(&plug_root) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let settings = load_plugin_settings(&plug_root);
                let plug_name = plug.file_name().to_string_lossy().into_owned();
                let version_name = version.file_name().to_string_lossy().into_owned();
                let marketplace_name = marketplace.file_name().to_string_lossy().into_owned();
                // Plugin id includes the version so two installs of
                // the same plugin (e.g. upgrade path) don't collide
                // in the id space. Friendly display stays
                // `<plugin>@<marketplace>` — version shows on hover
                // or in details.
                let id = if version_name == "unknown" {
                    plug_name.clone()
                } else {
                    format!("{plug_name}-{version_name}")
                };

                let plugin_scope = Scope::Plugin {
                    id: id.clone(),
                    source: crate::config_view::model::PluginSource::Marketplace {
                        spec: format!("{plug_name}@{marketplace_name}"),
                    },
                };

                // Emit the manifest node for UI.
                let manifest_path = if plug_root.join("plugin.json").is_file() {
                    plug_root.join("plugin.json")
                } else {
                    plug_root.join(".claude-plugin").join("plugin.json")
                };
                if let Some(f) = maybe_file(manifest_path, Kind::Plugin, plugin_scope.clone()) {
                    files.push(f);
                }

                // Walk the plugin's content directories with the same
                // rules CC uses for user / project scopes — agents,
                // skills, commands, output-styles, workflows, rules.
                // Each produces FileNodes tagged with this plugin's
                // scope so the UI can bucket them under the correct
                // bundle, and so the cross-scope Definitions view
                // (which filters plugins out) stays accurate.
                files.extend(collect_dir_of_kind(
                    &plug_root.join("agents"),
                    Kind::Agent,
                    plugin_scope.clone(),
                    true,
                ));
                files.extend(collect_skills_dir(
                    &plug_root.join("skills"),
                    plugin_scope.clone(),
                ));
                files.extend(collect_dir_of_kind(
                    &plug_root.join("commands"),
                    Kind::Command,
                    plugin_scope.clone(),
                    true,
                ));
                files.extend(collect_dir_of_kind(
                    &plug_root.join("output-styles"),
                    Kind::OutputStyle,
                    plugin_scope.clone(),
                    true,
                ));
                files.extend(collect_dir_of_kind(
                    &plug_root.join("workflows"),
                    Kind::Workflow,
                    plugin_scope.clone(),
                    true,
                ));
                files.extend(collect_dir_of_kind(
                    &plug_root.join("rules"),
                    Kind::Rule,
                    plugin_scope.clone(),
                    true,
                ));

                // Plugin-shipped `settings.json` is real and mergeable
                // (see plugin_base.rs); surface it so users can inspect.
                if let Some(f) = maybe_file(
                    plug_root.join("settings.json"),
                    Kind::Settings,
                    plugin_scope.clone(),
                ) {
                    files.push(f);
                }
                // `hooks.json` / `hooks/<name>.json` aren't standard
                // across every plugin yet — honor them when present
                // so packagers using them surface correctly.
                if let Some(f) = maybe_file(
                    plug_root.join("hooks.json"),
                    Kind::Hook,
                    plugin_scope.clone(),
                ) {
                    files.push(f);
                }

                let spec = format!("{plug_name}@{marketplace_name}");
                let enabled = enabled_specs.contains(&spec) || enabled_specs.contains(&plug_name);
                plugins.push(Plugin {
                    id,
                    root: plug_root,
                    manifest,
                    enabled,
                    settings,
                    source: PluginSourceDisplay::Marketplace { spec },
                });
            }
        }
    }
    (files, plugins)
}

/// Read enabled plugin specs from user settings. CC's discovery enables
/// plugins only when the user's `settings.json` has a truthy entry
/// under `plugins.<spec>`; entries absent from the map are disabled.
/// Returns the set of truthy specs; absent / missing files yield an
/// empty set, which correctly disables every discovered marketplace
/// plugin.
fn load_enabled_plugin_specs(home: &Path) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    let mut out = HashSet::new();
    let p = home.join("settings.json");
    let Some(bytes) = std::fs::read(p).ok() else {
        return out;
    };
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_slice(&bytes) else {
        return out;
    };
    let Some(plugins) = v.get("plugins").and_then(|x| x.as_object()) else {
        return out;
    };
    for (spec, val) in plugins {
        let enabled = match val {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::Object(o) => {
                o.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true)
            }
            _ => true,
        };
        if enabled {
            out.insert(spec.clone());
        }
    }
    out
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
    // `managed-mcp.json` — when present and non-empty, CC locks out
    // project + user + local MCP sources
    // (`services/mcp/config.ts`, plan §9.4). Surface as a Policy entry
    // so the user can see the lockout source.
    if let Some(f) = maybe_file(
        home.join("managed-mcp.json"),
        Kind::ManagedMcpJson,
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

/// RedactedUserConfig — global user config. Mirrors CC's
/// `getGlobalClaudeFile` (`env.ts:14-26`): legacy
/// `<claude_config_dir>/.config.json` wins when present; otherwise
/// `$CLAUDE_CONFIG_DIR/.claude.json` (or `~/.claude.json` when the env
/// var is unset). Surface the actual file so the preview points to the
/// authoritative location.
pub fn collect_redacted_user_config() -> Option<FileNode> {
    let legacy = claude_config_dir().join(".config.json");
    let primary_base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let primary = primary_base.join(".claude.json");
    let p = if legacy.is_file() {
        legacy
    } else if primary.is_file() {
        primary
    } else {
        return None;
    };
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
        included_by: None,
        include_depth: 0,
    })
}

// ---------- Internals -------------------------------------------------

fn collect_dir_of_kind(dir: &Path, kind: Kind, scope: Scope, recurse: bool) -> Vec<FileNode> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
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
                out.extend(collect_dir_of_kind(
                    &path,
                    kind.clone(),
                    scope.clone(),
                    true,
                ));
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
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
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
                included_by: None,
                include_depth: 0,
            });
        }
    }
    out
}

// ---------- @include expansion ---------------------------------------

/// Read the `hasClaudeMdExternalIncludesApproved` flag from the global
/// user config (`~/.claude.json` or the legacy `.config.json`). A
/// missing file, parse error, or absent key means the flag is off —
/// Project/Local/Managed includes outside cwd are then skipped, matching
/// CC's default-deny behavior (`claudemd.ts:798-802`).
fn external_includes_approved() -> bool {
    let Some(node) = collect_redacted_user_config() else {
        return false;
    };
    let Ok(bytes) = std::fs::read(&node.abs_path) else {
        return false;
    };
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_slice(&bytes) else {
        return false;
    };
    v.get("hasClaudeMdExternalIncludesApproved")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
}

/// True when `~/.claude/managed-mcp.json` exists AND declares at least
/// one MCP server in a valid server-map shape. CC uses this as an
/// enterprise lockout switch — when set, project/user/local MCP sources
/// are ignored entirely (`services/mcp/config.ts` + plan §9.4).
///
/// Validation rules (so a stray top-level field doesn't toggle lockout):
/// * The file must parse as a top-level JSON object.
/// * It must expose a server map either at `mcpServers` or as the bare
///   top-level object (the same shorthand `.mcp.json` accepts).
/// * Every value in the chosen map must itself be a JSON object —
///   stray scalars / arrays mean the file is malformed and lockout
///   stays off.
/// * The map must be non-empty.
///
/// Anything else — empty `{}`, malformed JSON, the wrong shape — is
/// treated as "not provisioned" so admins can drop a placeholder
/// without flipping the lockout.
fn managed_mcp_is_nonempty() -> bool {
    let p = claude_config_dir().join("managed-mcp.json");
    let Ok(bytes) = std::fs::read(&p) else {
        return false;
    };
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_slice(&bytes) else {
        return false;
    };
    managed_mcp_value_is_provisioned(&v)
}

/// Pure decision function — split out from `managed_mcp_is_nonempty` so
/// it can be unit-tested without driving `CLAUDE_CONFIG_DIR` and the
/// real filesystem. Encodes the validation rules documented above.
fn managed_mcp_value_is_provisioned(v: &serde_json::Value) -> bool {
    let Some(top) = v.as_object() else {
        return false;
    };

    // Prefer the explicit `mcpServers` key when present; otherwise
    // accept the bare `{ <name>: {…} }` shorthand. Either way the map
    // body must look like a server registry — every value an object.
    let map = match top.get("mcpServers") {
        Some(serde_json::Value::Object(m)) => m,
        Some(_) => return false, // wrong shape under the canonical key
        None => top,
    };

    if map.is_empty() {
        return false;
    }
    map.values().all(|v| v.is_object())
}

/// For every memory-kind `FileNode` in `files`, resolve its `@include`
/// chain and append synthetic nodes describing each reachable target.
/// Cycles and missing files are silently dropped inside the resolver.
///
/// `is_user_memory` → always include external targets
/// (`claudemd.ts:833`). Project/Local/Managed → gate on
/// `hasClaudeMdExternalIncludesApproved`.
fn expand_includes(files: &mut Vec<FileNode>, original_cwd: &Path, is_user_memory: bool) {
    let allow_external = is_user_memory || external_includes_approved();
    let memory_kinds: &[Kind] = &[Kind::ClaudeMd, Kind::Memory, Kind::MemoryIndex, Kind::Rule];
    let roots: Vec<(PathBuf, Scope)> = files
        .iter()
        .filter(|f| memory_kinds.contains(&f.kind))
        .map(|f| {
            (
                f.abs_path.clone(),
                f.scope_badges.first().cloned().unwrap_or(Scope::Other),
            )
        })
        .collect();
    let mut seen: std::collections::HashSet<PathBuf> =
        files.iter().map(|f| f.abs_path.clone()).collect();
    for (root, scope) in roots {
        for inc in memory_include::resolve_all(&root, original_cwd, allow_external) {
            if !seen.insert(inc.abs_path.clone()) {
                continue;
            }
            let kind = if memory_include::is_text_extension(&inc.abs_path)
                && inc
                    .abs_path
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("md"))
                    .unwrap_or(false)
            {
                Kind::Memory
            } else {
                Kind::Other
            };
            let parsed = parse_file(&inc.abs_path, &kind);
            let mut node = make_file_node(&inc.abs_path, kind, scope.clone(), parsed);
            node.included_by = Some(inc.included_by.clone());
            node.include_depth = inc.depth;
            // Title hint so the UI can show "included from …".
            let name = inc
                .abs_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            node.summary = Some(FileSummary {
                title: Some(name),
                description: Some(format!(
                    "@include depth {} (from {})",
                    inc.depth,
                    inc.included_by
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )),
            });
            files.push(node);
        }
    }
}

// ---------- Tree assembly --------------------------------------------

/// Build the full read-only `ConfigTree` anchored at `cwd`. Produces
/// scope roots in the fixed rank defined by plan §11.5.
///
/// When `global_only` is true, every cwd-dependent scope is skipped —
/// Project, Local, `.mcp.json` walk, `CLAUDE.md` walk, and
/// Memory-current are omitted. `cwd` is still used for the returned
/// tree's display path (callers typically pass `$HOME`) but nothing
/// is read from it. This is the mode used when the Config page has
/// no project anchor selected.
pub fn assemble_tree(cwd: &Path, global_only: bool) -> ConfigTree {
    let project_root = if global_only {
        cwd.to_path_buf()
    } else {
        crate::project_memory::find_canonical_git_root(cwd).unwrap_or_else(|| cwd.to_path_buf())
    };

    let (memory_files, memory_slug, memory_slug_lossy) = if global_only {
        (Vec::new(), String::new(), false)
    } else {
        collect_memory_current(&project_root)
    };

    let mut scopes: Vec<ScopeNode> = Vec::new();

    if let Some(rc) = collect_redacted_user_config() {
        scopes.push(scope_node(
            "scope:redacted",
            Scope::RedactedUserConfig,
            "Global config (redacted)",
            vec![Node::File(rc)],
        ));
    }

    let mut user_files = collect_user();
    // User memory can always @include external files
    // (`claudemd.ts:833` — `includeExternal: true` regardless of flag).
    // Includes may reference absolute paths; resolve against `cwd` even
    // in global mode (no effect when nothing references cwd-relative
    // paths).
    expand_includes(&mut user_files, cwd, /* is_user_memory */ true);
    if !user_files.is_empty() {
        scopes.push(scope_node(
            "scope:user",
            Scope::User,
            "User (~/.claude)",
            user_files.into_iter().map(Node::File).collect(),
        ));
    }

    if !global_only {
        let mut project_files = collect_project(cwd);
        project_files.extend(collect_rules_walk(cwd));
        expand_includes(&mut project_files, cwd, /* is_user_memory */ false);
        if !project_files.is_empty() {
            scopes.push(scope_node(
                "scope:project",
                Scope::Project,
                "Project (cwd/.claude)",
                project_files.into_iter().map(Node::File).collect(),
            ));
        }

        let mut local_files = collect_local(cwd);
        // CLAUDE.local.md at every ancestor to FS root, collapsed into
        // the single Local scope (`claudemd.ts:922-933`). One entry per
        // file.
        let local_md = collect_claudemd_local_walk(cwd);
        for (_dir, _role, f) in local_md {
            local_files.push(f);
        }
        expand_includes(&mut local_files, cwd, /* is_user_memory */ false);
        if !local_files.is_empty() {
            scopes.push(scope_node(
                "scope:local",
                Scope::Local,
                "Local (settings.local.json + CLAUDE.local.md)",
                local_files.into_iter().map(Node::File).collect(),
            ));
        }

        // `.mcp.json` at every ancestor to FS root
        // (`services/mcp/config.ts:914-920`). One scope for all of them
        // — dedicated label so users can tell it apart from other
        // Project settings sources.
        let mcp_files = collect_mcp_json_walk(cwd);
        if !mcp_files.is_empty() {
            scopes.push(scope_node(
                "scope:mcp-project",
                Scope::Project,
                "MCP (.mcp.json walk)",
                mcp_files.into_iter().map(Node::File).collect(),
            ));
        }

        let claudemd = collect_claudemd_walk(cwd);
        for (dir, role, f) in claudemd {
            let label = format!(
                "CLAUDE.md — {}{}",
                dir.display(),
                if matches!(role, ClaudeMdRole::Cwd) {
                    " (cwd)"
                } else {
                    ""
                },
            );
            // Root file plus its @include chain. CC processes this file
            // as `Project` memory type (`claudemd.ts:887-895`) — use
            // the non-user external-gate.
            let root_id = f.id.clone();
            let mut bucket = vec![f];
            expand_includes(&mut bucket, cwd, /* is_user_memory */ false);
            scopes.push(scope_node(
                &format!("scope:claudemd:{}", root_id),
                Scope::ClaudeMdDir {
                    dir: dir.clone(),
                    role: role.clone(),
                },
                &label,
                bucket.into_iter().map(Node::File).collect::<Vec<_>>(),
            ));
        }
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

    let (plugin_files, _plugins) = collect_plugins();
    if !plugin_files.is_empty() {
        // Group under a single Plugin scope for the tree — per-plugin
        // breakdown lives inside plugin renderers.
        scopes.push(scope_node(
            "scope:plugins",
            Scope::Plugin {
                id: "all".to_string(),
                source: crate::config_view::model::PluginSource::Marketplace {
                    spec: "all".to_string(),
                },
            },
            "Plugins",
            plugin_files.into_iter().map(Node::File).collect(),
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

    // Memory-other walks every *sibling* project under the auto-memory
    // base dir — not cwd-dependent beyond excluding the current slug,
    // so it stays visible in global mode.
    let other_slugs = crate::config_view::memory_other::scan_other_memory_dirs(&memory_slug);
    if !other_slugs.is_empty() {
        let files: Vec<FileNode> = other_slugs
            .iter()
            .map(crate::config_view::memory_other::make_slug_file_node)
            .collect();
        scopes.push(scope_node(
            "scope:memory-other",
            Scope::Other,
            "Memory (other projects)",
            files.into_iter().map(Node::File).collect(),
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
        // Enterprise managed-mcp lockout is a user-wide condition (same
        // `managed-mcp.json` applies to every project), so compute it
        // in both modes.
        enterprise_mcp_lockout: managed_mcp_is_nonempty(),
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
        Node::File(f) => f
            .summary
            .as_ref()
            .and_then(|s| s.title.as_deref())
            .unwrap_or(&f.display_path),
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
        assert!(is_denied(".claude.json"));
        assert!(!is_denied("settings.json"));
    }

    #[test]
    fn deny_names_excludes_lifecycle_disabled_dir() {
        // The lifecycle module stashes disabled artifacts under
        // <root>/.disabled/. Active Config discovery must skip it,
        // otherwise disabled skills/agents would re-appear under
        // their kind groups in the tree.
        assert!(is_denied(".disabled"));
        assert_eq!(crate::artifact_lifecycle::DISABLED_DIR, ".disabled");
    }

    #[test]
    fn deny_names_for_sync_conflict_and_bak() {
        assert!(is_denied("foo.sync-conflict-x.txt"));
        assert!(is_denied("foo.bak"));
        assert!(is_denied("foo.bak.20240101"));
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
        let tree = assemble_tree(td.path(), false);
        // No panics; ConfigTree built.
        assert_eq!(tree.cwd, td.path());
    }

    #[test]
    fn ancestors_fs_root_excludes_filesystem_root() {
        let root = PathBuf::from("/a/b/c/d");
        let a = ancestors_for(&root, WalkBoundary::FsRoot);
        // CC: `while (currentDir !== parse(currentDir).root)` — root
        // itself is NOT in the list.
        assert_eq!(a.last().unwrap(), &PathBuf::from("/a/b/c/d"));
        assert!(!a.contains(&PathBuf::from("/")));
        assert_eq!(a.first().unwrap(), &PathBuf::from("/a"));
    }

    // ----- managed-mcp.json lockout shape validation --------------

    #[test]
    fn managed_mcp_empty_object_is_not_provisioned() {
        // Pre-fix bug: any non-empty object enabled lockout. Empty
        // object must stay unprovisioned.
        let v = serde_json::json!({});
        assert!(!managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn managed_mcp_top_level_array_is_not_provisioned() {
        let v = serde_json::json!([{"command": "x"}]);
        assert!(!managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn managed_mcp_top_level_scalar_is_not_provisioned() {
        let v = serde_json::json!("hi");
        assert!(!managed_mcp_value_is_provisioned(&v));
        let v = serde_json::json!(42);
        assert!(!managed_mcp_value_is_provisioned(&v));
        let v = serde_json::json!(null);
        assert!(!managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn managed_mcp_unrelated_object_is_not_provisioned() {
        // A dict of strings (not server objects) must NOT trigger
        // lockout — this is the regression class the audit flagged.
        let v = serde_json::json!({"note": "placeholder", "schema": "v1"});
        assert!(!managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn managed_mcp_servers_key_with_objects_is_provisioned() {
        let v = serde_json::json!({
            "mcpServers": {
                "tools": {"command": "x", "args": []}
            }
        });
        assert!(managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn managed_mcp_servers_key_empty_is_not_provisioned() {
        let v = serde_json::json!({"mcpServers": {}});
        assert!(!managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn managed_mcp_servers_key_wrong_shape_is_not_provisioned() {
        // `mcpServers` present but not an object → reject without
        // falling back to the bare-shorthand branch.
        let v = serde_json::json!({"mcpServers": []});
        assert!(!managed_mcp_value_is_provisioned(&v));
        let v = serde_json::json!({"mcpServers": "x"});
        assert!(!managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn managed_mcp_shorthand_with_objects_is_provisioned() {
        // `.mcp.json`-style shorthand: top-level keys ARE the servers.
        let v = serde_json::json!({
            "tools": {"command": "x"}
        });
        assert!(managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn managed_mcp_shorthand_with_non_object_values_is_not_provisioned() {
        // A single bad entry disqualifies — shape must be a server map.
        let v = serde_json::json!({
            "tools": {"command": "x"},
            "broken": "string-not-server"
        });
        assert!(!managed_mcp_value_is_provisioned(&v));
    }

    #[test]
    fn ancestors_git_root_or_home_stops_at_git() {
        let td = TempDir::new().unwrap();
        let mid = td.path().join("mid");
        let leaf = mid.join("leaf");
        std::fs::create_dir_all(&leaf).unwrap();
        std::fs::create_dir_all(mid.join(".git")).unwrap();
        let a = ancestors_for(&leaf, WalkBoundary::GitRootOrHome);
        // `.git` parent is INCLUDED (matches CC — see
        // `markdownConfigLoader.ts:261-275` — git-root dir is processed,
        // then the loop breaks).
        assert_eq!(a.first().unwrap(), &mid);
        assert_eq!(a.last().unwrap(), &leaf);
    }
}
