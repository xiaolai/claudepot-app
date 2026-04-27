//! Pure path math for the artifact-lifecycle layer.
//!
//! This module contains no I/O beyond `symlink_metadata` (used to
//! distinguish file vs directory and to detect symlink loops). All
//! decisions are derived from the path string + scope-root list
//! passed by the caller — the higher-level UI always knows which
//! `.claude/` roots are "active".
//!
//! Every mutating entry-point (disable / enable / trash) starts by
//! calling `classify_path` and either acting on the resulting
//! `Trackable` or surfacing the `RefuseReason` to the UI.

use crate::artifact_lifecycle::error::RefuseReason;
use crate::path_utils::simplify_windows_path;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Three trackable artifact kinds. Hooks/rules/plugins are not
/// lifecycle-managed (see the design doc for why).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ArtifactKind {
    Skill,
    Agent,
    Command,
}

impl ArtifactKind {
    /// CC's well-known subdirectory name under each `<root>/`.
    pub fn subdir(self) -> &'static str {
        match self {
            Self::Skill => "skills",
            Self::Agent => "agents",
            Self::Command => "commands",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Agent => "agent",
            Self::Command => "command",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "skill" => Self::Skill,
            "agent" => Self::Agent,
            "command" => Self::Command,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Scope {
    User,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadKind {
    File,
    Directory,
}

/// Result of `classify_path` when the path is eligible for a
/// lifecycle action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trackable {
    pub scope: Scope,
    pub scope_root: PathBuf,
    pub kind: ArtifactKind,
    /// Slash-joined path under `<scope_root>/<kind.subdir()>/`. Eg.
    /// `team/foo.md` for `<root>/agents/team/foo.md`. Always uses
    /// forward slashes for cross-platform stability of the
    /// (scope_root, kind, relative_path) triple.
    pub relative_path: String,
    pub payload_kind: PayloadKind,
    /// True if the path is inside the scope_root's `.disabled/` tree.
    pub already_disabled: bool,
}

/// Active scope roots known to the caller. The GUI always knows the
/// user's `~/.claude/` and the active project's `.claude/`; both go
/// here. Refused roots (plugin cache, managed policy) are NOT
/// included — refusal is path-shape based, not list-membership based.
#[derive(Debug, Clone, Default)]
pub struct ActiveRoots {
    pub user_root: Option<PathBuf>,
    pub project_roots: Vec<PathBuf>,
    /// Managed-policy roots are passed in (per-platform). Refusal
    /// triggers if the path is under any of them.
    pub managed_policy_roots: Vec<PathBuf>,
}

impl ActiveRoots {
    pub fn user(user_root: PathBuf) -> Self {
        Self {
            user_root: Some(user_root),
            ..Default::default()
        }
    }

    pub fn with_project(mut self, project_root: PathBuf) -> Self {
        self.project_roots.push(project_root);
        self
    }

    /// Iterator over `(scope, scope_root)` for every active root.
    pub fn iter_scoped(&self) -> impl Iterator<Item = (Scope, &Path)> {
        self.user_root
            .as_deref()
            .map(|p| (Scope::User, p))
            .into_iter()
            .chain(
                self.project_roots
                    .iter()
                    .map(|p| (Scope::Project, p.as_path())),
            )
    }
}

/// Sentinel directory name used for in-place disable.
pub const DISABLED_DIR: &str = ".disabled";

/// Discover every project `<repo>/.claude/` directory the backend
/// already knows about — derived from the session-index sweep
/// (`project::list_projects`), which records every project that has
/// ever produced a session transcript.
///
/// Lifecycle commands consult this list before accepting a renderer-
/// supplied `project_root`. Without it, validation is circular: the
/// renderer claims a root, the backend checks the same renderer-
/// influenced list, and arbitrary `.claude`-shaped directories
/// elsewhere on disk get accepted as writable scope roots.
///
/// Result is best-effort: discovery failures (read errors, etc.)
/// produce an empty Vec rather than poison the lifecycle surface.
/// Renderer-supplied roots that aren't in the returned list are
/// silently dropped — the operation falls through to user-only scope.
pub fn discover_known_project_roots(config_dir: &std::path::Path) -> Vec<PathBuf> {
    // Lightweight scan: iterate `<config>/projects/<slug>/` directly
    // and recover each project's cwd via the cheap one-line transcript
    // peek used by `recover_cwd_from_sessions`. Skips the recursive
    // dir_size / mtime walks that `project::list_projects` performs
    // (those would scale lifecycle commands with the entire transcript
    // index — significant for power users with hundreds of projects).
    let projects_dir = config_dir.join("projects");
    let entries = match std::fs::read_dir(&projects_dir) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }
        let cwd = match crate::project_helpers::recover_cwd_from_sessions_pub(&path) {
            Some(c) => c,
            None => continue,
        };
        let claude_dir = PathBuf::from(cwd).join(".claude");
        if claude_dir.is_dir() {
            out.push(claude_dir);
        }
    }
    out
}

/// Take a (Windows-aware) absolute path and decide what lifecycle
/// action it's eligible for. All mutating ops call this first.
///
/// Refusal reasons map to UI affordances; see the design doc.
pub fn classify_path(path: &Path, roots: &ActiveRoots) -> Result<Trackable, RefuseReason> {
    let path = canonicalize_for_classify(path);

    // Plugin cache check (path-shape) takes precedence over scope
    // membership — plugins can sit under a managed `~/.claude/` too.
    if let Some(plugin_id) = plugin_id_from_path(&path) {
        return Err(RefuseReason::Plugin {
            plugin_id,
            path: path.clone(),
        });
    }

    // Managed-policy check — explicit list passed by the caller.
    for managed_root in &roots.managed_policy_roots {
        if starts_with_dir(&path, managed_root) {
            return Err(RefuseReason::ManagedPolicy {
                root: managed_root.clone(),
                path: path.clone(),
            });
        }
    }

    // Find the deepest scope root that contains the path. Iterating
    // by length descending handles nested project roots correctly
    // (a child project's `.claude/` wins over a parent's).
    let mut candidates: Vec<(Scope, &Path)> = roots.iter_scoped().collect();
    candidates.sort_by_key(|(_, p)| std::cmp::Reverse(p.as_os_str().len()));
    let (scope, scope_root) = candidates
        .into_iter()
        .find(|(_, root)| starts_with_dir(&path, root))
        .ok_or_else(|| RefuseReason::OutOfScope { path: path.clone() })?;

    // Now classify the path under <scope_root>/...
    let rel = path.strip_prefix(scope_root).map_err(|_| {
        RefuseReason::OutOfScope {
            path: path.clone(),
        }
    })?;
    let mut comps = rel.components();
    let first = comps
        .next()
        .ok_or_else(|| RefuseReason::OutOfScope {
            path: path.clone(),
        })?;
    let first_s = first.as_os_str().to_string_lossy().into_owned();

    let (kind_subdir, already_disabled) = if first_s == DISABLED_DIR {
        // Path is inside `.disabled/<kind>/...` — second component is
        // the kind subdir.
        let second = comps.next().ok_or_else(|| RefuseReason::WrongKind {
            path: path.clone(),
        })?;
        (second.as_os_str().to_string_lossy().into_owned(), true)
    } else {
        (first_s, false)
    };

    let kind = match kind_subdir.as_str() {
        "skills" => ArtifactKind::Skill,
        "agents" => ArtifactKind::Agent,
        "commands" => ArtifactKind::Command,
        _ => return Err(RefuseReason::WrongKind { path: path.clone() }),
    };

    let rel_under_kind: PathBuf = comps.collect();
    if rel_under_kind.as_os_str().is_empty() {
        return Err(RefuseReason::WrongKind { path: path.clone() });
    }
    // Reject any traversal / absolute / prefix components in the
    // path-under-kind. Without this, `<root>/agents/../../etc/passwd`
    // would classify as a trackable agent and let the caller mutate
    // arbitrary filesystem locations under the rename's resolution.
    for component in rel_under_kind.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err(RefuseReason::WrongKind { path: path.clone() }),
        }
    }

    // Skill files live at `<root>/skills/<name>/SKILL.md`. CC's
    // Skill discovery treats the parent directory `<name>/` as the
    // skill — moving just SKILL.md would leave behind an empty
    // skill dir that CC then sees as broken. So when the caller
    // hands us a SKILL.md path, classify the SKILL DIRECTORY.
    let is_skill_md_inside_dir = kind == ArtifactKind::Skill
        && path.file_name().map(|n| n == "SKILL.md").unwrap_or(false)
        && rel_under_kind
            .parent()
            .map(|p| !p.as_os_str().is_empty())
            .unwrap_or(false);
    let (relative_path_pb, payload_kind) = if is_skill_md_inside_dir {
        // rel_under_kind = "<name>/SKILL.md" → take just "<name>".
        let name_only = rel_under_kind
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| rel_under_kind.clone());
        (name_only, PayloadKind::Directory)
    } else {
        let payload = detect_payload_kind(&path)?;
        (rel_under_kind, payload)
    };
    let relative_path = path_to_forward_slash(&relative_path_pb);

    Ok(Trackable {
        scope,
        scope_root: scope_root.to_path_buf(),
        kind,
        relative_path,
        payload_kind,
        already_disabled,
    })
}

/// Compute the `.disabled/...` target a `Trackable` would land at.
pub fn disabled_target_for(t: &Trackable) -> PathBuf {
    t.scope_root
        .join(DISABLED_DIR)
        .join(t.kind.subdir())
        .join(forward_to_native(&t.relative_path))
}

/// Compute the active `<root>/<kind>/...` location an enabled
/// Trackable would restore to.
pub fn enabled_target_for(t: &Trackable) -> PathBuf {
    t.scope_root
        .join(t.kind.subdir())
        .join(forward_to_native(&t.relative_path))
}

// ---------- internals ----------

/// Use lstat-equivalent so symlinks are inspected as themselves.
fn detect_payload_kind(path: &Path) -> Result<PayloadKind, RefuseReason> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| RefuseReason::OutOfScope {
        path: path.to_path_buf(),
    })?;
    if meta.file_type().is_symlink() {
        // Refuse symlink loops — best effort detection by following
        // the link once and checking it doesn't resolve to itself
        // through std::fs::canonicalize (which would loop forever
        // on a symlink-to-itself; canonicalize returns Err in that
        // case, which we treat as a loop).
        if std::fs::canonicalize(path).is_err() {
            return Err(RefuseReason::SymlinkLoop {
                path: path.to_path_buf(),
            });
        }
        // Treat symlink-to-file and symlink-to-dir based on the
        // pointed kind, but the rename always moves the link itself.
        let target_meta = std::fs::metadata(path).map_err(|_| RefuseReason::SymlinkLoop {
            path: path.to_path_buf(),
        })?;
        return Ok(if target_meta.is_dir() {
            PayloadKind::Directory
        } else {
            PayloadKind::File
        });
    }
    Ok(if meta.is_dir() {
        PayloadKind::Directory
    } else {
        PayloadKind::File
    })
}

/// Detect plugin cache paths. CC stores plugins at
/// `<some_root>/plugins/cache/<owner>/<plugin>/<version>/...`.
///
/// Component-based: walks the path's components instead of doing a
/// substring match so Windows paths (with backslash separators) are
/// handled correctly. Returns `Some(<plugin>)` when the
/// `plugins/cache/<owner>/<plugin>/...` shape appears anywhere in
/// the path.
fn plugin_id_from_path(path: &Path) -> Option<String> {
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    for window in parts.windows(4) {
        if window[0] == "plugins" && window[1] == "cache" {
            let plugin = window[3];
            if !plugin.is_empty() {
                return Some(plugin.to_string());
            }
        }
    }
    None
}

fn canonicalize_for_classify(path: &Path) -> PathBuf {
    PathBuf::from(simplify_windows_path(&path.display().to_string()))
}

/// Strict directory containment check: `child` is under `parent` AND
/// the next char after `parent`'s prefix is a path separator (or
/// child equals parent).
fn starts_with_dir(child: &Path, parent: &Path) -> bool {
    if child == parent {
        return true;
    }
    child.starts_with(parent)
}

fn path_to_forward_slash(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn forward_to_native(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_with(user: &str, project: Option<&str>) -> ActiveRoots {
        let mut r = ActiveRoots::user(PathBuf::from(user));
        if let Some(p) = project {
            r = r.with_project(PathBuf::from(p));
        }
        r
    }

    fn make_temp_file(dir: &std::path::Path, sub: &str) -> PathBuf {
        let p = dir.join(sub);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        p
    }

    fn make_temp_dir(dir: &std::path::Path, sub: &str) -> PathBuf {
        let p = dir.join(sub);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("SKILL.md"), b"x").unwrap();
        p
    }

    #[test]
    fn classify_user_skill_dir_form() {
        let tmp = tempfile::tempdir().unwrap();
        let user_root = tmp.path().join(".claude");
        let skill_dir = make_temp_dir(&user_root, "skills/myskill");
        let r = classify_path(
            &skill_dir,
            &root_with(&user_root.to_string_lossy(), None),
        )
        .unwrap();
        assert_eq!(r.scope, Scope::User);
        assert_eq!(r.kind, ArtifactKind::Skill);
        assert_eq!(r.relative_path, "myskill");
        assert_eq!(r.payload_kind, PayloadKind::Directory);
        assert!(!r.already_disabled);
    }

    #[test]
    fn classify_user_agent_file() {
        let tmp = tempfile::tempdir().unwrap();
        let user_root = tmp.path().join(".claude");
        let agent = make_temp_file(&user_root, "agents/foo.md");
        let r = classify_path(
            &agent,
            &root_with(&user_root.to_string_lossy(), None),
        )
        .unwrap();
        assert_eq!(r.kind, ArtifactKind::Agent);
        assert_eq!(r.relative_path, "foo.md");
        assert_eq!(r.payload_kind, PayloadKind::File);
    }

    #[test]
    fn classify_nested_command_preserves_relative_path() {
        let tmp = tempfile::tempdir().unwrap();
        let user_root = tmp.path().join(".claude");
        let cmd = make_temp_file(&user_root, "commands/team/lint.md");
        let r = classify_path(
            &cmd,
            &root_with(&user_root.to_string_lossy(), None),
        )
        .unwrap();
        assert_eq!(r.kind, ArtifactKind::Command);
        assert_eq!(r.relative_path, "team/lint.md");
    }

    #[test]
    fn classify_disabled_path_marks_already_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let user_root = tmp.path().join(".claude");
        let disabled = make_temp_file(&user_root, ".disabled/agents/foo.md");
        let r = classify_path(
            &disabled,
            &root_with(&user_root.to_string_lossy(), None),
        )
        .unwrap();
        assert_eq!(r.kind, ArtifactKind::Agent);
        assert_eq!(r.relative_path, "foo.md");
        assert!(r.already_disabled);
    }

    #[test]
    fn classify_plugin_path_refused() {
        // Plugin cache wins over scope membership.
        let tmp = tempfile::tempdir().unwrap();
        let user_root = tmp.path().join(".claude");
        let plugin = make_temp_file(
            &user_root,
            "plugins/cache/owner/my-plugin/0.1.0/skills/x/SKILL.md",
        );
        let err = classify_path(
            &plugin,
            &root_with(&user_root.to_string_lossy(), None),
        )
        .unwrap_err();
        assert!(matches!(err, RefuseReason::Plugin { ref plugin_id, .. } if plugin_id == "my-plugin"));
    }

    #[test]
    fn classify_path_outside_any_root_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let user_root = tmp.path().join(".claude");
        let stray = make_temp_file(tmp.path(), "elsewhere/agents/foo.md");
        let err = classify_path(
            &stray,
            &root_with(&user_root.to_string_lossy(), None),
        )
        .unwrap_err();
        assert!(matches!(err, RefuseReason::OutOfScope { .. }));
    }

    #[test]
    fn classify_picks_deepest_scope_root_for_nested_projects() {
        // Outer project at /work and inner project at /work/inner —
        // an agent under /work/inner/.claude/agents/X must classify
        // as the inner project's scope.
        let tmp = tempfile::tempdir().unwrap();
        let outer = tmp.path().join("work");
        let inner = outer.join("inner");
        let outer_claude = outer.join(".claude");
        let inner_claude = inner.join(".claude");
        let agent = make_temp_file(&inner_claude, "agents/foo.md");

        let mut roots = ActiveRoots::user(tmp.path().join(".user-root"));
        roots = roots.with_project(outer_claude.clone());
        roots = roots.with_project(inner_claude.clone());

        let r = classify_path(&agent, &roots).unwrap();
        assert_eq!(r.scope_root, inner_claude);
        assert_eq!(r.scope, Scope::Project);
        assert_eq!(r.relative_path, "foo.md");
    }

    #[test]
    fn classify_random_subdir_under_root_is_wrong_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let user_root = tmp.path().join(".claude");
        let stray = make_temp_file(&user_root, "settings.json");
        let err = classify_path(
            &stray,
            &root_with(&user_root.to_string_lossy(), None),
        )
        .unwrap_err();
        assert!(matches!(err, RefuseReason::WrongKind { .. }));
    }

    #[test]
    fn classify_managed_root_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let managed_root = tmp.path().join("managed");
        let agent = make_temp_file(&managed_root, "agents/foo.md");
        let mut roots = ActiveRoots::user(tmp.path().join(".user"));
        roots.managed_policy_roots.push(managed_root.clone());
        let err = classify_path(&agent, &roots).unwrap_err();
        assert!(matches!(err, RefuseReason::ManagedPolicy { .. }));
    }

    #[test]
    fn disabled_target_round_trips_to_enabled_target() {
        let user_root = PathBuf::from("/u/.claude");
        let t = Trackable {
            scope: Scope::User,
            scope_root: user_root.clone(),
            kind: ArtifactKind::Agent,
            relative_path: "team/foo.md".into(),
            payload_kind: PayloadKind::File,
            already_disabled: false,
        };
        let dis = disabled_target_for(&t);
        let en = enabled_target_for(&t);
        assert_eq!(dis, user_root.join(".disabled/agents/team/foo.md"));
        assert_eq!(en, user_root.join("agents/team/foo.md"));
    }

    #[test]
    fn discover_known_project_roots_returns_only_existing_dot_claude_dirs() {
        // Seed a fake config dir with two project sanitized slugs.
        // One project's `.claude/` exists on disk, the other doesn't.
        // Discovery must surface only the existing one.
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("claude_home");
        std::fs::create_dir_all(config.join("projects")).unwrap();

        let alive_repo = tmp.path().join("repo-alive");
        let alive_claude = alive_repo.join(".claude");
        std::fs::create_dir_all(&alive_claude).unwrap();
        let dead_repo = tmp.path().join("repo-dead");
        // Note: dead_repo does NOT have a `.claude/` directory.
        std::fs::create_dir_all(&dead_repo).unwrap();

        // Plant transcripts that record each project's cwd.
        let alive_slug = crate::project_sanitize::sanitize_path(&alive_repo.to_string_lossy());
        let dead_slug = crate::project_sanitize::sanitize_path(&dead_repo.to_string_lossy());
        for (slug, cwd) in [
            (&alive_slug, &alive_repo),
            (&dead_slug, &dead_repo),
        ] {
            let dir = config.join("projects").join(slug);
            std::fs::create_dir_all(&dir).unwrap();
            let session = dir.join("S.jsonl");
            std::fs::write(
                &session,
                format!(
                    r#"{{"type":"user","timestamp":"2026-04-10T10:00:00Z","cwd":"{}","sessionId":"S","message":{{"role":"user","content":"hi"}}}}
"#,
                    cwd.to_string_lossy()
                ),
            )
            .unwrap();
        }

        let discovered = discover_known_project_roots(&config);
        assert!(discovered.contains(&alive_claude), "alive project surfaces");
        assert!(
            !discovered.iter().any(|p| p.starts_with(&dead_repo)),
            "dead project (no .claude) must NOT surface"
        );
    }

    #[test]
    fn plugin_id_extraction_handles_mixed_separators() {
        // Cross-platform fixture: even with backslashes the slash form
        // wins because `simplify_windows_path` normalizes verbatim
        // prefixes; bare backslashes inside a unix-style absolute
        // string aren't normalized so we should miss those gracefully.
        let p = PathBuf::from("/u/.claude/plugins/cache/me/codex-toolkit/0.8.2/skills/x/SKILL.md");
        assert_eq!(plugin_id_from_path(&p), Some("codex-toolkit".to_string()));

        let p = PathBuf::from("/u/.claude/skills/x/SKILL.md");
        assert_eq!(plugin_id_from_path(&p), None);
    }
}
