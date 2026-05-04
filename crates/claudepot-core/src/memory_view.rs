//! Per-project memory file enumeration.
//!
//! Surfaces the four kinds of memory-shaped artifacts that CC loads
//! for a given project:
//!
//! 1. `<project_root>/CLAUDE.md` — the project-committed instructions.
//! 2. `<project_root>/.claude/CLAUDE.md` — alternate project location.
//! 3. `~/.claude/projects/<slug>/memory/**/*.md` — auto-memory written
//!    by CC's background `extractMemories` fork (and the main agent
//!    when it writes inline). The slug is `sanitize_path` over the
//!    project's canonical git root, matching CC's `getAutoMemPath()`
//!    in `memdir/paths.ts:223`.
//! 4. `~/.claude/CLAUDE.md` — global; affects every project.
//!
//! This module is read-only metadata. Live content reads go through
//! [`read_memory_content`] which performs a containment check against
//! the four allowlisted scopes. `memory_log` (sibling module) layers
//! the persisted change history on top.
//!
//! What this module deliberately does NOT do:
//! - Resolve `@include` chains. `config_view::memory_include` already
//!   does that for the Config browser; the Projects → Memory pane
//!   shows files as they exist on disk.
//! - Walk ancestor `CLAUDE.md` files past the project root. That's a
//!   v2 enhancement; CC walks ancestors but the per-project pane
//!   stays focused on the project anchor.
//! - Open a `config_view::scan` (heavy: plugins, MCP, agents,
//!   skills, …). The four sources above are enumerable directly with
//!   one `dir_entry` call each plus one bounded walk of the
//!   auto-memory dir.

use crate::path_utils::simplify_windows_path;
use crate::paths::claude_config_dir;
use crate::project_memory::find_canonical_git_root;
use crate::project_sanitize::sanitize_path;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Roles of files surfaced in the Projects → Memory pane. Roles drive
/// sort order, scope label, and whether the line-cutoff signal applies.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryFileRole {
    /// `<project_root>/CLAUDE.md`.
    ClaudeMdProject,
    /// `<project_root>/.claude/CLAUDE.md`.
    ClaudeMdProjectLocal,
    /// `~/.claude/projects/<slug>/memory/MEMORY.md` — the index file
    /// that's always loaded into CC's context.
    AutoMemoryIndex,
    /// `~/.claude/projects/<slug>/memory/<topic>.md` — topic memory
    /// files referenced from MEMORY.md.
    AutoMemoryTopic,
    /// `~/.claude/projects/<slug>/memory/logs/YYYY/MM/YYYY-MM-DD.md` —
    /// KAIROS daily log file (gated feature; treated as a regular
    /// memory file for display).
    KairosLog,
    /// `~/.claude/CLAUDE.md` — global; same content shown for every
    /// project, badged accordingly.
    ClaudeMdGlobal,
}

impl MemoryFileRole {
    /// Sort key for the file list. Lower = nearer the top.
    fn sort_key(self) -> u8 {
        match self {
            Self::ClaudeMdProject => 0,
            Self::ClaudeMdProjectLocal => 1,
            Self::AutoMemoryIndex => 2,
            Self::AutoMemoryTopic => 3,
            Self::KairosLog => 4,
            Self::ClaudeMdGlobal => 5,
        }
    }

    /// Human-readable scope label for the UI badge.
    pub fn scope_label(self) -> &'static str {
        match self {
            Self::ClaudeMdProject | Self::ClaudeMdProjectLocal => "Project",
            Self::AutoMemoryIndex => "Auto-memory · index",
            Self::AutoMemoryTopic => "Auto-memory",
            Self::KairosLog => "Auto-memory · log",
            Self::ClaudeMdGlobal => "Global",
        }
    }

    /// MEMORY.md is the only file CC's prompt loader truncates at line
    /// 200 (verified against `memdir/memdir.ts::MAX_ENTRYPOINT_LINES`).
    /// Other files have no per-file cutoff and the signal is irrelevant.
    pub fn has_index_cutoff(self) -> bool {
        matches!(self, Self::AutoMemoryIndex)
    }
}

/// MEMORY.md's hard line cutoff. Verified against CC source
/// `memdir/memdir.ts:35` (2026-01).
pub const MEMORY_INDEX_LINE_CUTOFF: usize = 200;

/// One memory file, summarized for the file list. Read-only. Construct
/// via [`enumerate_project_memory`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryFileSummary {
    pub abs_path: PathBuf,
    pub role: MemoryFileRole,
    pub size_bytes: u64,
    pub mtime_unix_ns: i64,
    pub line_count: usize,
    /// Lines past CC's MEMORY.md cutoff. `None` for any role where the
    /// cutoff doesn't apply; `Some(0)` when the file is at or under
    /// the cutoff.
    pub lines_past_cutoff: Option<usize>,
}

/// Where the auto-memory dir lives for a given project. Captured so
/// callers (CLI, IPC) can render "memory dir: <path>" without
/// re-deriving the slug.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectMemoryAnchor {
    pub project_root: PathBuf,
    /// Canonical git root if the project is inside a repo; otherwise
    /// equal to `project_root`. This is what CC keys auto-memory on.
    pub auto_memory_anchor: PathBuf,
    /// `sanitize_path(auto_memory_anchor)`. The directory under
    /// `~/.claude/projects/` is named after this slug.
    pub slug: String,
    pub auto_memory_dir: PathBuf,
}

impl ProjectMemoryAnchor {
    /// Build the anchor for `project_root`. The `project_root` is used
    /// as-is — callers that need OS-canonicalization pass an already-
    /// canonicalized path (e.g. via `project_helpers::resolve_path`).
    pub fn for_project(project_root: &Path) -> Self {
        let canon_anchor =
            find_canonical_git_root(project_root).unwrap_or_else(|| project_root.to_path_buf());
        let slug = sanitize_path(&simplify_windows_path(&canon_anchor.to_string_lossy()));
        let auto_memory_dir = claude_config_dir()
            .join("projects")
            .join(&slug)
            .join("memory");
        Self {
            project_root: project_root.to_path_buf(),
            auto_memory_anchor: canon_anchor,
            slug,
            auto_memory_dir,
        }
    }
}

/// Entry / depth caps mirror `config_view::memory_other::count_md_files`
/// — same upper bound on a single memory dir scan so a pathological
/// nested tree (symlink loop, accidentally-checked-in submodule)
/// can't dominate CPU on every refresh.
const MAX_AUTO_MEMORY_ENTRIES: usize = 2048;
const MAX_AUTO_MEMORY_DEPTH: usize = 6;

/// List the memory files associated with `project_root`, in display
/// order (project CLAUDE.md → auto-memory index → topics → logs →
/// global CLAUDE.md). Missing files are silently skipped.
///
/// `include_global` controls whether `~/.claude/CLAUDE.md` is
/// appended. Callers showing a per-project pane normally want it
/// `true` (global affects every project); CLI verbs that filter to
/// "this project only" can set it `false`.
pub fn enumerate_project_memory(
    project_root: &Path,
    include_global: bool,
) -> std::io::Result<EnumerateResult> {
    let anchor = ProjectMemoryAnchor::for_project(project_root);

    let mut files: Vec<MemoryFileSummary> = Vec::new();

    push_if_present(
        &mut files,
        &project_root.join("CLAUDE.md"),
        MemoryFileRole::ClaudeMdProject,
    )?;
    push_if_present(
        &mut files,
        &project_root.join(".claude").join("CLAUDE.md"),
        MemoryFileRole::ClaudeMdProjectLocal,
    )?;

    walk_auto_memory(&anchor.auto_memory_dir, &mut files)?;

    if include_global {
        push_if_present(
            &mut files,
            &claude_config_dir().join("CLAUDE.md"),
            MemoryFileRole::ClaudeMdGlobal,
        )?;
    }

    files.sort_by(|a, b| {
        a.role
            .sort_key()
            .cmp(&b.role.sort_key())
            .then_with(|| a.abs_path.cmp(&b.abs_path))
    });

    Ok(EnumerateResult { anchor, files })
}

/// Bundle returned by [`enumerate_project_memory`]. Carries the anchor
/// alongside the file list so the UI can render the memory-dir path
/// (or "no memory yet — would write to: …") without a second call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnumerateResult {
    pub anchor: ProjectMemoryAnchor,
    pub files: Vec<MemoryFileSummary>,
}

fn push_if_present(
    out: &mut Vec<MemoryFileSummary>,
    path: &Path,
    role: MemoryFileRole,
) -> std::io::Result<()> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(()); // missing is fine
    };
    if !meta.is_file() {
        return Ok(());
    }
    out.push(summarize(path, role, &meta)?);
    Ok(())
}

fn walk_auto_memory(root: &Path, out: &mut Vec<MemoryFileSummary>) -> std::io::Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    let mut visited = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            visited += 1;
            if visited >= MAX_AUTO_MEMORY_ENTRIES {
                return Ok(());
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if depth + 1 < MAX_AUTO_MEMORY_DEPTH {
                    stack.push((entry.path(), depth + 1));
                }
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let path = entry.path();
            let Some(role) = classify_auto_memory_file(root, &path) else {
                continue;
            };
            let Ok(meta) = entry.metadata() else { continue };
            if let Ok(summary) = summarize(&path, role, &meta) {
                out.push(summary);
            }
        }
    }
    Ok(())
}

/// Classify a path inside the auto-memory dir. Returns `None` for files
/// that aren't markdown.
fn classify_auto_memory_file(root: &Path, path: &Path) -> Option<MemoryFileRole> {
    let ext_md = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false);
    if !ext_md {
        return None;
    }
    let rel = path.strip_prefix(root).ok()?;
    let mut comps = rel.components();
    let first = comps.next()?;
    if first.as_os_str() == "logs" {
        return Some(MemoryFileRole::KairosLog);
    }
    // Anything else inside the memory dir at any depth: distinguish the
    // index from topic files by basename. CC writes the index as
    // `MEMORY.md` (case-sensitive — verified against `memdir/memdir.ts:34`),
    // but match case-insensitively to handle macOS/Windows filesystem
    // case-folding quirks the user might introduce manually.
    let basename_is_index = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("MEMORY.md"))
        .unwrap_or(false);
    if basename_is_index && rel.components().count() == 1 {
        Some(MemoryFileRole::AutoMemoryIndex)
    } else {
        Some(MemoryFileRole::AutoMemoryTopic)
    }
}

fn summarize(
    path: &Path,
    role: MemoryFileRole,
    meta: &std::fs::Metadata,
) -> std::io::Result<MemoryFileSummary> {
    let bytes = std::fs::read(path)?;
    // Lossy decode so a BOM / mojibake file doesn't crash the pane —
    // the line-counting math only cares about `\n`s.
    let text = String::from_utf8_lossy(&bytes);
    let line_count = text.lines().count();
    let lines_past_cutoff = if role.has_index_cutoff() {
        Some(line_count.saturating_sub(MEMORY_INDEX_LINE_CUTOFF))
    } else {
        None
    };
    Ok(MemoryFileSummary {
        abs_path: path.to_path_buf(),
        role,
        size_bytes: meta.len(),
        mtime_unix_ns: mtime_ns(meta),
        line_count,
        lines_past_cutoff,
    })
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    // SystemTime → nanoseconds-since-epoch. Out-of-range or
    // pre-epoch mtimes (which shouldn't occur for files we just stat'd)
    // collapse to 0 so the column never carries a negative ns count.
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or(0)
}

/// Why a containment check failed. Lets the IPC layer return a typed
/// error and still log the offending path for debugging.
#[derive(Debug, thiserror::Error)]
pub enum ReadMemoryError {
    #[error("path is outside the allowed memory scopes: {0}")]
    PathOutsideScope(PathBuf),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Read a memory file's contents after verifying the path is one we're
/// allowed to surface. Allowed scopes:
///
/// - The `auto_memory_dir` for any registered project (caller passes
///   the list — we don't enumerate `~/.claude/projects/` here so a
///   freshly-created project's memory dir is read-allowed only after
///   it's been registered).
/// - The four CLAUDE.md candidate paths derivable from each registered
///   project root (`<R>/CLAUDE.md`, `<R>/.claude/CLAUDE.md`).
/// - `~/.claude/CLAUDE.md` (global).
///
/// Path canonicalization is performed before the allowlist check so a
/// symlink that lands in the auto-memory dir but points outside cannot
/// escape the scope. Without this, the previous `starts_with` check
/// matched the symlink path lexically while `std::fs::read` followed
/// the link to read arbitrary files (audit 2026-05, #1 high).
pub fn read_memory_content(
    target: &Path,
    allowed_project_roots: &[PathBuf],
) -> Result<String, ReadMemoryError> {
    let canonical = canonical_for_check(target);
    if !is_allowed(&canonical, allowed_project_roots) {
        return Err(ReadMemoryError::PathOutsideScope(target.to_path_buf()));
    }
    // Lossy: a binary file in the memory dir (oddly enough, but possible)
    // would crash a strict UTF-8 read. Replacement chars in the rendered
    // output are the right behavior — the file IS broken, not us.
    let bytes = std::fs::read(&canonical)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Resolve a path to its canonical form for containment checks. Falls
/// back to the original path when canonicalization fails (file
/// missing, permission denied) — `is_allowed` will then evaluate the
/// declared shape, which is the correct behavior for paths that don't
/// exist yet (e.g. a memory file the user just deleted).
fn canonical_for_check(target: &Path) -> PathBuf {
    crate::path_utils::canonicalize_simplified(target).unwrap_or_else(|_| target.to_path_buf())
}

/// Public predicate for the same check `read_memory_content` runs. Used
/// by the watcher to decide whether to record an event for a given path.
///
/// Compares the canonicalized form of `target` against canonicalized
/// allowed roots so symlinks don't escape the scope. The anchor (and
/// thus the slug-derived auto_memory_dir) is built from the
/// caller-supplied path verbatim — canonicalizing the root before
/// `sanitize_path` would change the slug and miss the directory CC
/// actually wrote (since CC bases its slug on the same input the
/// caller hands us).
pub fn is_allowed(target: &Path, allowed_project_roots: &[PathBuf]) -> bool {
    let target_canon = canonical_for_check(target);
    let global_claude_md = canonical_for_check(&claude_config_dir().join("CLAUDE.md"));
    if path_eq(&target_canon, &global_claude_md) {
        return true;
    }
    for root in allowed_project_roots {
        if path_eq(&target_canon, &canonical_for_check(&root.join("CLAUDE.md")))
            || path_eq(
                &target_canon,
                &canonical_for_check(&root.join(".claude").join("CLAUDE.md")),
            )
        {
            return true;
        }
        let anchor = ProjectMemoryAnchor::for_project(root);
        let auto_canon = canonical_for_check(&anchor.auto_memory_dir);
        if target_canon.starts_with(&auto_canon) {
            return true;
        }
    }
    false
}

fn path_eq(a: &Path, b: &Path) -> bool {
    // Plain string compare on the simplified form. This is intentionally
    // exact — no canonicalization — because the watcher emits paths as
    // notify reports them and we want the same key shape on both sides.
    let na = simplify_windows_path(&a.to_string_lossy());
    let nb = simplify_windows_path(&b.to_string_lossy());
    na == nb
}

/// Classify a path emitted by the watcher. Returns:
/// - `Some((ClaudeMdGlobal, None))` for `~/.claude/CLAUDE.md`.
/// - `Some((AutoMemoryIndex|Topic|KairosLog, Some(slug)))` for files
///   inside `~/.claude/projects/<slug>/memory/`.
/// - `Some((ClaudeMdProject*, Some(slug)))` for `<R>/CLAUDE.md` or
///   `<R>/.claude/CLAUDE.md` where R is one of `known_project_roots`
///   (slug = `sanitize_path(R)`, matching the auto-memory dir slug).
/// - `None` for any path the change-log shouldn't track.
///
/// Audit 2026-05 #5: project CLAUDE.md files now classify, so the
/// change log captures their edits when the watcher subscribes.
pub fn classify_path_for_watcher(
    path: &Path,
    known_project_roots: &[PathBuf],
) -> Option<(MemoryFileRole, Option<String>)> {
    let cfg = claude_config_dir();
    if path_eq(path, &cfg.join("CLAUDE.md")) {
        return Some((MemoryFileRole::ClaudeMdGlobal, None));
    }
    // Project CLAUDE.md candidates — match before auto-memory check
    // so a CLAUDE.md inside a registered root doesn't accidentally
    // route through the auto-memory classifier.
    for root in known_project_roots {
        if path_eq(path, &root.join("CLAUDE.md")) {
            let slug = sanitize_path(&simplify_windows_path(&root.to_string_lossy()));
            return Some((MemoryFileRole::ClaudeMdProject, Some(slug)));
        }
        if path_eq(path, &root.join(".claude").join("CLAUDE.md")) {
            let slug = sanitize_path(&simplify_windows_path(&root.to_string_lossy()));
            return Some((MemoryFileRole::ClaudeMdProjectLocal, Some(slug)));
        }
    }
    let projects_dir = cfg.join("projects");
    let rel = path.strip_prefix(&projects_dir).ok()?;
    let mut comps = rel.components();
    let slug = comps.next()?.as_os_str().to_str()?.to_string();
    let memory_seg = comps.next()?.as_os_str();
    if memory_seg != "memory" {
        return None;
    }
    let inside = rel.strip_prefix(&slug).ok()?.strip_prefix("memory").ok()?;
    let role = classify_auto_memory_file(&projects_dir.join(&slug).join("memory"), path)?;
    if !path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
    {
        return None;
    }
    // `inside` exists only to enforce that the path lies underneath
    // the `memory` segment; we don't need to inspect it further.
    let _ = inside;
    Some((role, Some(slug)))
}

/// Best-effort enumeration of project roots from `~/.claude/projects/`.
/// Recovers the original (unsanitized) project path for each slug dir.
///
/// Strategy per slug:
///   1. If the slug is short enough to be losslessly invertible
///      (< 200 chars per `MAX_SANITIZED_LENGTH`), use `unsanitize_path`.
///   2. Otherwise it carries a hash tail and the inversion is
///      ambiguous — fall back to scanning session transcripts in the
///      slug dir for an authoritative `cwd` field
///      (`recover_cwd_from_sessions_pub`).
///   3. If neither yields a usable path that exists on disk, skip
///      the entry. Better to miss a project than to fabricate one.
///
/// Audit 2026-05 #5: previously skipped lossy slugs entirely, so
/// projects with very deep paths lost project-CLAUDE.md tracking. The
/// session-cwd fallback closes that gap whenever any session file
/// exists for the project (which is the case for every project that
/// has ever been used by CC).
pub fn discover_project_roots_from_slugs() -> Vec<PathBuf> {
    let projects_dir = claude_config_dir().join("projects");
    let Ok(rd) = std::fs::read_dir(&projects_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        if let Some(candidate) = recover_project_root_from_slug_dir(&entry.path()) {
            if candidate.exists() {
                out.push(candidate);
            }
        }
    }
    out
}

/// Recover the original project root from one `~/.claude/projects/<slug>/`
/// directory. See [`discover_project_roots_from_slugs`] for the full
/// strategy and rationale.
fn recover_project_root_from_slug_dir(slug_dir: &Path) -> Option<PathBuf> {
    use crate::project_sanitize::{unsanitize_path, MAX_SANITIZED_LENGTH};
    let name = slug_dir.file_name()?.to_str()?;
    if name.len() < MAX_SANITIZED_LENGTH {
        return Some(PathBuf::from(unsanitize_path(name)));
    }
    // Lossy slug: walk the slug dir's session.jsonl files for an
    // authoritative `cwd`. If the project has ever been opened in CC,
    // at least one session file should carry it.
    crate::project_helpers::recover_cwd_from_sessions_pub(slug_dir)
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn role_sort_orders_project_first_global_last() {
        let mut roles = vec![
            MemoryFileRole::ClaudeMdGlobal,
            MemoryFileRole::AutoMemoryTopic,
            MemoryFileRole::ClaudeMdProject,
            MemoryFileRole::AutoMemoryIndex,
            MemoryFileRole::KairosLog,
            MemoryFileRole::ClaudeMdProjectLocal,
        ];
        roles.sort_by_key(|r| r.sort_key());
        assert_eq!(
            roles,
            vec![
                MemoryFileRole::ClaudeMdProject,
                MemoryFileRole::ClaudeMdProjectLocal,
                MemoryFileRole::AutoMemoryIndex,
                MemoryFileRole::AutoMemoryTopic,
                MemoryFileRole::KairosLog,
                MemoryFileRole::ClaudeMdGlobal,
            ]
        );
    }

    #[test]
    fn classify_picks_index_only_at_top_level() {
        let root = Path::new("/m");
        assert_eq!(
            classify_auto_memory_file(root, Path::new("/m/MEMORY.md")),
            Some(MemoryFileRole::AutoMemoryIndex)
        );
        // A file named MEMORY.md inside a subdir is NOT the index — CC
        // only loads the top-level one.
        assert_eq!(
            classify_auto_memory_file(root, Path::new("/m/sub/MEMORY.md")),
            Some(MemoryFileRole::AutoMemoryTopic)
        );
    }

    #[test]
    fn classify_picks_topic_for_other_md() {
        assert_eq!(
            classify_auto_memory_file(Path::new("/m"), Path::new("/m/user.md")),
            Some(MemoryFileRole::AutoMemoryTopic)
        );
    }

    #[test]
    fn classify_picks_kairos_for_logs() {
        assert_eq!(
            classify_auto_memory_file(
                Path::new("/m"),
                Path::new("/m/logs/2026/05/2026-05-04.md")
            ),
            Some(MemoryFileRole::KairosLog)
        );
    }

    #[test]
    fn classify_skips_non_md() {
        assert!(classify_auto_memory_file(Path::new("/m"), Path::new("/m/notes.txt")).is_none());
        assert!(classify_auto_memory_file(Path::new("/m"), Path::new("/m/.DS_Store")).is_none());
    }

    #[test]
    fn classify_handles_case_insensitive_index() {
        assert_eq!(
            classify_auto_memory_file(Path::new("/m"), Path::new("/m/memory.md")),
            Some(MemoryFileRole::AutoMemoryIndex)
        );
    }

    #[test]
    fn summarize_reports_lines_past_cutoff_for_index_only() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("MEMORY.md");
        let body: String = (0..250).map(|_| "x\n").collect();
        fs::write(&f, &body).unwrap();
        let meta = fs::metadata(&f).unwrap();
        let s = summarize(&f, MemoryFileRole::AutoMemoryIndex, &meta).unwrap();
        assert_eq!(s.lines_past_cutoff, Some(50));

        let g = tmp.path().join("user.md");
        fs::write(&g, &body).unwrap();
        let meta = fs::metadata(&g).unwrap();
        let s = summarize(&g, MemoryFileRole::AutoMemoryTopic, &meta).unwrap();
        assert_eq!(s.lines_past_cutoff, None);
    }

    #[test]
    fn summarize_handles_missing_trailing_newline() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("MEMORY.md");
        fs::write(&f, "one\ntwo").unwrap();
        let meta = fs::metadata(&f).unwrap();
        let s = summarize(&f, MemoryFileRole::AutoMemoryIndex, &meta).unwrap();
        assert_eq!(s.line_count, 2);
        assert_eq!(s.lines_past_cutoff, Some(0));
    }

    #[test]
    fn enumerate_skips_missing_files() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path().join("config-dir"));
        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();
        // No CLAUDE.md, no auto-mem dir, no global. Result is empty
        // but Ok — no I/O error.
        let r = enumerate_project_memory(&project, true).unwrap();
        assert!(r.files.is_empty());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn enumerate_finds_all_four_kinds() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();

        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();
        fs::write(project.join("CLAUDE.md"), "project").unwrap();
        let dot_claude = project.join(".claude");
        fs::create_dir(&dot_claude).unwrap();
        fs::write(dot_claude.join("CLAUDE.md"), "project-local").unwrap();
        fs::write(cfg.join("CLAUDE.md"), "global").unwrap();

        // Auto-memory dir for this project — same slug logic as CC.
        let anchor = ProjectMemoryAnchor::for_project(&project);
        fs::create_dir_all(&anchor.auto_memory_dir).unwrap();
        fs::write(anchor.auto_memory_dir.join("MEMORY.md"), "index").unwrap();
        fs::write(anchor.auto_memory_dir.join("user.md"), "topic").unwrap();
        fs::create_dir_all(anchor.auto_memory_dir.join("logs/2026/05")).unwrap();
        fs::write(
            anchor
                .auto_memory_dir
                .join("logs/2026/05/2026-05-04.md"),
            "log",
        )
        .unwrap();

        let r = enumerate_project_memory(&project, true).unwrap();
        let roles: Vec<_> = r.files.iter().map(|f| f.role).collect();
        assert!(roles.contains(&MemoryFileRole::ClaudeMdProject));
        assert!(roles.contains(&MemoryFileRole::ClaudeMdProjectLocal));
        assert!(roles.contains(&MemoryFileRole::AutoMemoryIndex));
        assert!(roles.contains(&MemoryFileRole::AutoMemoryTopic));
        assert!(roles.contains(&MemoryFileRole::KairosLog));
        assert!(roles.contains(&MemoryFileRole::ClaudeMdGlobal));
        // Sort: project first, global last.
        assert_eq!(roles.first(), Some(&MemoryFileRole::ClaudeMdProject));
        assert_eq!(roles.last(), Some(&MemoryFileRole::ClaudeMdGlobal));

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn enumerate_omits_global_when_flag_false() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join("CLAUDE.md"), "global").unwrap();
        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();

        let r = enumerate_project_memory(&project, false).unwrap();
        assert!(!r
            .files
            .iter()
            .any(|f| f.role == MemoryFileRole::ClaudeMdGlobal));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn read_memory_content_rejects_paths_outside_allowed_scope() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();

        let outside = tmp.path().join("evil.md");
        fs::write(&outside, "secret").unwrap();
        let err = read_memory_content(&outside, std::slice::from_ref(&project)).unwrap_err();
        match err {
            ReadMemoryError::PathOutsideScope(_) => {}
            other => panic!("expected PathOutsideScope, got {:?}", other),
        }
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn read_memory_content_allows_global_claude_md() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        let global = cfg.join("CLAUDE.md");
        fs::write(&global, "hello world").unwrap();
        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();

        let content = read_memory_content(&global, &[project]).unwrap();
        assert_eq!(content, "hello world");
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn read_memory_content_allows_files_inside_auto_memory_dir() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();
        let anchor = ProjectMemoryAnchor::for_project(&project);
        fs::create_dir_all(&anchor.auto_memory_dir).unwrap();
        let topic = anchor.auto_memory_dir.join("user.md");
        fs::write(&topic, "topic content").unwrap();

        let content = read_memory_content(&topic, &[project]).unwrap();
        assert_eq!(content, "topic content");
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn classify_path_for_watcher_picks_global_claude_md() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        let path = cfg.join("CLAUDE.md");
        let (role, slug) = classify_path_for_watcher(&path, &[]).expect("classify");
        assert_eq!(role, MemoryFileRole::ClaudeMdGlobal);
        assert!(slug.is_none());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn classify_path_for_watcher_picks_auto_memory_index_with_slug() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        let path = cfg
            .join("projects")
            .join("-Users-joker-foo")
            .join("memory")
            .join("MEMORY.md");
        let (role, slug) = classify_path_for_watcher(&path, &[]).expect("classify");
        assert_eq!(role, MemoryFileRole::AutoMemoryIndex);
        assert_eq!(slug.as_deref(), Some("-Users-joker-foo"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn classify_path_for_watcher_picks_topic_for_other_md() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        let path = cfg
            .join("projects")
            .join("-Users-joker-foo")
            .join("memory")
            .join("user.md");
        let (role, slug) = classify_path_for_watcher(&path, &[]).expect("classify");
        assert_eq!(role, MemoryFileRole::AutoMemoryTopic);
        assert_eq!(slug.as_deref(), Some("-Users-joker-foo"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn classify_path_for_watcher_picks_kairos_log() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        let path = cfg
            .join("projects")
            .join("-Users-joker-foo")
            .join("memory")
            .join("logs/2026/05/2026-05-04.md");
        let (role, _slug) = classify_path_for_watcher(&path, &[]).expect("classify");
        assert_eq!(role, MemoryFileRole::KairosLog);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn classify_path_for_watcher_skips_unrelated_paths() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        // settings.json — must NOT classify as memory.
        assert!(classify_path_for_watcher(&cfg.join("settings.json"), &[]).is_none());
        // project root with no memory dir.
        assert!(classify_path_for_watcher(
            &cfg.join("projects").join("foo").join("config.json"),
            &[]
        )
        .is_none());
        // .md file outside the memory dir.
        assert!(classify_path_for_watcher(
            &cfg.join("projects").join("foo").join("notes.md"),
            &[]
        )
        .is_none());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn discover_project_roots_recovers_lossy_slugs_from_session_cwd() {
        // Audit 2026-05 #5 follow-up: lossy slugs (>= 200 chars) used
        // to be skipped entirely. Now they recover via session.jsonl
        // `cwd`. Build a fixture matching that shape.
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        let projects = cfg.join("projects");
        fs::create_dir_all(&projects).unwrap();

        // Real project root the test will recover from session cwd.
        let project = tmp.path().join("deep").join("project");
        fs::create_dir_all(&project).unwrap();

        // Build a slug name at the lossy boundary (>= 200 chars).
        let lossy_slug: String = "x".repeat(220);
        let slug_dir = projects.join(&lossy_slug);
        fs::create_dir_all(&slug_dir).unwrap();

        // One session.jsonl with the cwd field — the recover helper
        // walks files in the dir looking for `"cwd"`.
        let session = slug_dir.join("session1.jsonl");
        let line = format!(
            r#"{{"type":"user","cwd":{},"timestamp":"2026-01-01T00:00:00Z"}}"#,
            serde_json::to_string(&project.to_string_lossy().into_owned()).unwrap(),
        );
        fs::write(&session, line + "\n").unwrap();

        let roots = discover_project_roots_from_slugs();
        assert!(
            roots
                .iter()
                .any(|r| r == &project || r == &project.canonicalize().unwrap_or(project.clone())),
            "expected recovered root in {:?}",
            roots
        );

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn classify_path_for_watcher_picks_project_claude_md() {
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();
        let project = tmp.path().join("my-project");
        fs::create_dir(&project).unwrap();
        let claude_md = project.join("CLAUDE.md");
        let dot_claude_md = project.join(".claude").join("CLAUDE.md");

        let (role, slug) =
            classify_path_for_watcher(&claude_md, std::slice::from_ref(&project))
                .expect("classify project CLAUDE.md");
        assert_eq!(role, MemoryFileRole::ClaudeMdProject);
        assert!(slug.is_some());

        let (role, _) =
            classify_path_for_watcher(&dot_claude_md, std::slice::from_ref(&project))
                .expect("classify .claude/CLAUDE.md");
        assert_eq!(role, MemoryFileRole::ClaudeMdProjectLocal);

        // Without project_roots, the project CLAUDE.md is invisible —
        // proves the new arg is what enables tracking.
        assert!(classify_path_for_watcher(&claude_md, &[]).is_none());

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    #[cfg(unix)]
    fn read_memory_content_rejects_symlink_pointing_outside_scope() {
        // Audit 2026-05, finding #1: a symlink inside auto-memory dir
        // whose target is outside must be rejected before std::fs::read
        // follows the link. Pre-fix, the lexical `starts_with` allowed
        // the read to traverse out.
        let _lock = crate::testing::lock_data_dir();
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("config-dir");
        std::env::set_var("CLAUDE_CONFIG_DIR", &cfg);
        fs::create_dir_all(&cfg).unwrap();

        let project = tmp.path().join("project");
        fs::create_dir(&project).unwrap();
        let anchor = ProjectMemoryAnchor::for_project(&project);
        fs::create_dir_all(&anchor.auto_memory_dir).unwrap();

        // Secret outside any allowed scope.
        let secret = tmp.path().join("secret.txt");
        fs::write(&secret, "TOP SECRET").unwrap();

        // Symlink lives inside the auto-memory dir but points outward.
        let escape_link = anchor.auto_memory_dir.join("escape.md");
        std::os::unix::fs::symlink(&secret, &escape_link).unwrap();

        let err = read_memory_content(&escape_link, &[project]).unwrap_err();
        match err {
            ReadMemoryError::PathOutsideScope(_) => {}
            other => panic!("expected PathOutsideScope, got {other:?}"),
        }

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn anchor_uses_git_root_when_present() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let nested = repo.join("nested/deep");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();

        let anchor = ProjectMemoryAnchor::for_project(&nested);
        // Canonicalize for comparison since `find_canonical_git_root`
        // canonicalizes; tempdir itself may carry symlinks (e.g.
        // /var/folders → /private/var/folders on macOS).
        assert_eq!(
            anchor.auto_memory_anchor,
            repo.canonicalize().unwrap_or(repo.clone())
        );
    }
}
