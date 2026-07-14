//! Project confinement for the MCP memory server.
//!
//! # Why this exists
//!
//! `sessions.db` is a **cross-project** index. On a working machine it
//! holds transcripts from every project the user has ever opened —
//! side projects, client work, personal finances. The MCP memory
//! server exposes that index to an LLM.
//!
//! Before this module, the server's reads were unbounded:
//! `claudepot_search_memory` with no `project_path` searched *every*
//! project, `claudepot_read_conversation` would read *any* indexed
//! transcript, and `claudepot_list_projects` enumerated the lot. A
//! coding agent in project A could read project B's transcripts, and
//! the instruction snippet Claudepot itself installs actively told it
//! to search without a scope.
//!
//! # The boundary
//!
//! Confinement is decided by the **human at registration time** — it
//! lives in `~/.claude.json` as a CLI flag, where the user can audit
//! it — and never by the agent at call time. That placement is the
//! whole point: an agent can be prompt-injected into *asking* for
//! more, but it cannot grant itself more. Default is deny:
//! `claudepot mcp memory-server` with no flags confines to `$PWD`.
//!
//! # Exact match, not substring
//!
//! [`super::search::SearchQuery::project_path`] is a `LIKE '%v%'`
//! substring filter — a convenience for humans narrowing a search. It
//! is **not** usable as a security boundary: confining to
//! `/x/claudepot-app` by substring would also match
//! `/x/claudepot-app-old`, a different project. Every check here
//! compares normalized paths for **equality**, and
//! [`super::search::SearchQuery::project_path_exact`] exists so the
//! server can express that in SQL.

use std::path::Path;

use crate::path_utils::simplify_windows_path;

/// What one memory-server process is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpScope {
    /// Confined to a single project. Reads outside it are refused.
    Project(String),
    /// Explicitly unconfined — the user passed `--all-projects`,
    /// accepting that any agent talking to this server can read
    /// every indexed project.
    AllProjects,
}

/// Refusal to cross the project boundary.
///
/// The message names only the root the caller is *already* confined
/// to (which the caller necessarily knows) — never the path it tried
/// to reach, and never any other project's path. An error is an
/// emission surface too.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cross-project access denied: this memory server is confined to {root}. Re-register it with --all-projects to widen the scope.")]
pub struct ScopeDenied {
    pub root: String,
}

impl McpScope {
    /// Confine to `path`, normalizing it the same way stored
    /// `sessions.project_path` values are compared.
    pub fn project(path: &Path) -> Self {
        McpScope::Project(normalize(path))
    }

    /// The confinement root, or `None` when unconfined.
    pub fn root(&self) -> Option<&str> {
        match self {
            McpScope::Project(p) => Some(p.as_str()),
            McpScope::AllProjects => None,
        }
    }

    /// Resolve the **exact** `project_path` filter a search must run
    /// under.
    ///
    /// * unconfined → `None` (no exact filter; the caller's own
    ///   substring filter still applies)
    /// * confined, no request → the root
    /// * confined, request == root → the root
    /// * confined, request != root → refused
    ///
    /// Refusing (rather than silently rewriting the request to the
    /// root) is deliberate: a caller that asked for another project
    /// should learn it cannot have it, not receive this project's
    /// rows as if they were the answer.
    pub fn confine_search(&self, requested: Option<&str>) -> Result<Option<String>, ScopeDenied> {
        match self {
            McpScope::AllProjects => Ok(None),
            McpScope::Project(root) => match requested {
                None => Ok(Some(root.clone())),
                Some(r) if normalize_str(r) == *root => Ok(Some(root.clone())),
                Some(_) => Err(self.denied()),
            },
        }
    }

    /// May the caller read a transcript belonging to
    /// `session_project`?
    pub fn check_read(&self, session_project: &str) -> Result<(), ScopeDenied> {
        match self {
            McpScope::AllProjects => Ok(()),
            McpScope::Project(root) if normalize_str(session_project) == *root => Ok(()),
            McpScope::Project(_) => Err(self.denied()),
        }
    }

    /// May the caller write a durable row scoped to `target`?
    ///
    /// `None` means a **global** memory. Those are allowed even from
    /// a confined server: a global memory reveals nothing about
    /// another project — it only records something the user told
    /// *this* agent. The asymmetry with [`Self::check_read`] is
    /// intentional; the risk here is exfiltration, not authorship.
    pub fn check_write(&self, target: Option<&str>) -> Result<(), ScopeDenied> {
        match (self, target) {
            (McpScope::AllProjects, _) => Ok(()),
            (McpScope::Project(_), None) => Ok(()),
            (McpScope::Project(root), Some(t)) if normalize_str(t) == *root => Ok(()),
            (McpScope::Project(_), Some(_)) => Err(self.denied()),
        }
    }

    fn denied(&self) -> ScopeDenied {
        ScopeDenied {
            root: self.root().unwrap_or_default().to_string(),
        }
    }
}

/// Normalize a project path for comparison.
///
/// Strips Windows verbatim prefixes (`\\?\C:\…` → `C:\…`, and the
/// `\\?\UNC\server\share` → `\\server\share` rewrite) per
/// `rules/paths.md`, then drops trailing separators so `/a/b` and
/// `/a/b/` compare equal.
///
/// Case is **preserved**. macOS and Windows filesystems are usually
/// case-insensitive, but `sessions.project_path` stores CC's `cwd`
/// verbatim and [`super::search::list_sessions`] already matches it
/// exactly — folding case here would make the guard disagree with the
/// query it guards.
pub fn normalize(path: &Path) -> String {
    normalize_str(&path.to_string_lossy())
}

fn normalize_str(raw: &str) -> String {
    let simplified = simplify_windows_path(raw);
    let trimmed = simplified.trim_end_matches(['/', '\\']);
    // A bare root (`/`, `C:\`) trims to empty or to a bare drive —
    // keep the original rather than emit an empty root.
    if trimmed.is_empty() {
        simplified
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn confined(root: &str) -> McpScope {
        McpScope::Project(normalize_str(root))
    }

    // ─── the attack this module exists to stop ──────────────────

    #[test]
    fn a_sibling_project_sharing_our_prefix_is_denied() {
        // The substring filter in SearchQuery would match this.
        // Equality must not. This is the whole reason the guard
        // cannot be built on `project_path LIKE '%root%'`.
        let scope = confined("/Users/dev/work/claudepot-app");
        assert_eq!(
            scope.check_read("/Users/dev/work/claudepot-app-old"),
            Err(ScopeDenied {
                root: "/Users/dev/work/claudepot-app".into()
            })
        );
    }

    #[test]
    fn a_subdirectory_of_our_project_is_not_our_project() {
        // sessions.project_path is a whole cwd, never a subpath, so a
        // "child" value is a different project row — deny it.
        let scope = confined("/Users/dev/work/app");
        assert!(scope.check_read("/Users/dev/work/app/crates/core").is_err());
    }

    #[test]
    fn requesting_another_project_in_search_is_refused_not_rewritten() {
        let scope = confined("/Users/dev/work/app");
        assert!(scope
            .confine_search(Some("/Users/dev/private/finances"))
            .is_err());
    }

    // ─── the happy paths ────────────────────────────────────────

    #[test]
    fn unscoped_search_is_forced_to_the_root() {
        let scope = confined("/Users/dev/work/app");
        assert_eq!(
            scope.confine_search(None).unwrap(),
            Some("/Users/dev/work/app".to_string())
        );
    }

    #[test]
    fn asking_for_our_own_project_is_allowed() {
        let scope = confined("/Users/dev/work/app");
        assert_eq!(
            scope.confine_search(Some("/Users/dev/work/app")).unwrap(),
            Some("/Users/dev/work/app".to_string())
        );
        assert!(scope.check_read("/Users/dev/work/app").is_ok());
    }

    #[test]
    fn all_projects_applies_no_exact_filter_and_reads_anything() {
        let scope = McpScope::AllProjects;
        assert_eq!(scope.confine_search(None).unwrap(), None);
        assert!(scope.check_read("/anywhere/at/all").is_ok());
        assert!(scope.check_write(Some("/anywhere/at/all")).is_ok());
        assert_eq!(scope.root(), None);
    }

    // ─── writes: global allowed, cross-project denied ────────────

    #[test]
    fn a_global_memory_may_be_written_from_a_confined_server() {
        // Authorship, not exfiltration — a global memory reveals
        // nothing about another project.
        let scope = confined("/Users/dev/work/app");
        assert!(scope.check_write(None).is_ok());
    }

    #[test]
    fn writing_a_memory_onto_another_project_is_denied() {
        let scope = confined("/Users/dev/work/app");
        assert!(scope.check_write(Some("/Users/dev/work/other")).is_err());
    }

    // ─── path shapes (rules/paths.md: cover all four) ────────────

    #[test]
    fn trailing_separators_do_not_change_identity() {
        let scope = confined("/Users/dev/work/app/");
        assert!(scope.check_read("/Users/dev/work/app").is_ok());
        assert!(scope.check_read("/Users/dev/work/app/").is_ok());
    }

    #[test]
    fn windows_drive_paths_compare_exactly() {
        let scope = confined(r"C:\Users\dev\app");
        assert!(scope.check_read(r"C:\Users\dev\app").is_ok());
        assert!(scope.check_read(r"C:\Users\dev\app-old").is_err());
    }

    #[test]
    fn unc_paths_compare_exactly() {
        let scope = confined(r"\\server\share\app");
        assert!(scope.check_read(r"\\server\share\app").is_ok());
        assert!(scope.check_read(r"\\server\share\app-old").is_err());
    }

    #[test]
    fn a_verbatim_prefix_normalizes_to_the_plain_form() {
        // canonicalize() on Windows yields \\?\C:\…; CC never writes
        // that shape into project_path, so the guard must strip it or
        // it would refuse the caller's own project.
        let scope = confined(r"C:\Users\dev\app");
        assert!(scope.check_read(r"\\?\C:\Users\dev\app").is_ok());

        let verbatim_scope = McpScope::project(&PathBuf::from(r"\\?\C:\Users\dev\app"));
        assert_eq!(verbatim_scope.root(), Some(r"C:\Users\dev\app"));
    }

    #[test]
    fn a_verbatim_unc_prefix_normalizes_to_the_plain_unc_form() {
        let scope = confined(r"\\server\share\app");
        assert!(scope.check_read(r"\\?\UNC\server\share\app").is_ok());
    }

    #[test]
    fn the_denial_message_never_names_the_path_that_was_refused() {
        // An error is an emission surface. Naming the project the
        // caller tried to reach would leak the very thing the guard
        // is protecting when the caller is guessing at paths.
        let scope = confined("/Users/dev/work/app");
        let err = scope.check_read("/Users/dev/private/finances").unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("finances"));
        assert!(msg.contains("/Users/dev/work/app"));
    }
}
