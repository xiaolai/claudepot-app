//! ConfigTree diff — produce a `ConfigTreePatch` between two tree
//! snapshots. Used by the watcher to deliver incremental updates to
//! the UI without re-serializing the whole tree.
//!
//! Plan §11.5 / §14.7 rules:
//! - Root scope ordering is immutable; added/removed scopes trigger a
//!   `full_snapshot` rather than `reordered`.
//! - Child `reordered` entries fire only when the canonical order
//!   within a ScopeNode actually changes.
//! - `added`, `updated`, `removed` operate on `FileNode.id`.

use crate::config_view::model::{ConfigTree, FileNode, Node, ScopeNode};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct ConfigTreePatch {
    /// (parent_scope_id, FileNode) — file newly added to that scope.
    pub added: Vec<(String, FileNode)>,
    /// FileNodes whose content/metadata changed (same id).
    pub updated: Vec<FileNode>,
    /// FileNode.id that disappeared.
    pub removed: Vec<String>,
    /// (parent_scope_id, ordered child ids) — only when canonical
    /// order changed.
    pub reordered: Vec<(String, Vec<String>)>,
    /// Set when root scopes changed shape (added/removed) — the UI
    /// should replace its whole tree with this snapshot.
    pub full_snapshot: Option<ConfigTree>,
    /// True when watcher emitted this patch mid-settling (plan §11.2
    /// MAX_CONVERGE_ATTEMPTS hit). UI shows a small "updating…"
    /// indicator until the next converged patch.
    pub dirty_during_emit: bool,
}

/// Compute a patch from `prev` → `next`. When the set of scope ids
/// differs, returns a patch whose `full_snapshot = Some(next.clone())`
/// and leaves other fields empty.
pub fn diff(prev: &ConfigTree, next: &ConfigTree) -> ConfigTreePatch {
    let prev_ids: Vec<&str> = prev.scopes.iter().map(|s| s.id.as_str()).collect();
    let next_ids: Vec<&str> = next.scopes.iter().map(|s| s.id.as_str()).collect();
    if prev_ids != next_ids {
        return ConfigTreePatch {
            full_snapshot: Some(next.clone()),
            ..Default::default()
        };
    }

    let mut patch = ConfigTreePatch::default();
    for (p, n) in prev.scopes.iter().zip(next.scopes.iter()) {
        diff_scope(p, n, &mut patch);
    }
    patch
}

fn diff_scope(prev: &ScopeNode, next: &ScopeNode, patch: &mut ConfigTreePatch) {
    let prev_files = flatten_files(&prev.children);
    let next_files = flatten_files(&next.children);

    let prev_by_id: HashMap<&str, &FileNode> =
        prev_files.iter().map(|f| (f.id.as_str(), *f)).collect();
    let next_by_id: HashMap<&str, &FileNode> =
        next_files.iter().map(|f| (f.id.as_str(), *f)).collect();

    for f in &next_files {
        match prev_by_id.get(f.id.as_str()) {
            None => patch.added.push((prev.id.clone(), (*f).clone())),
            Some(prev_f) if files_differ(prev_f, f) => {
                patch.updated.push((*f).clone());
            }
            _ => {}
        }
    }
    for f in &prev_files {
        if !next_by_id.contains_key(f.id.as_str()) {
            patch.removed.push(f.id.clone());
        }
    }

    let prev_order: Vec<&str> = prev_files.iter().map(|f| f.id.as_str()).collect();
    let next_order: Vec<&str> = next_files.iter().map(|f| f.id.as_str()).collect();
    if prev_order != next_order {
        patch
            .reordered
            .push((next.id.clone(), next_order.into_iter().map(String::from).collect()));
    }
}

fn flatten_files(nodes: &[Node]) -> Vec<&FileNode> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::File(f) => out.push(f),
            Node::Dir(d) => out.extend(flatten_files(&d.children)),
        }
    }
    out
}

fn files_differ(a: &FileNode, b: &FileNode) -> bool {
    a.size_bytes != b.size_bytes
        || a.mtime_unix_ns != b.mtime_unix_ns
        || a.summary_title() != b.summary_title()
        || a.issues != b.issues
}

trait SummaryTitleAccess {
    fn summary_title(&self) -> Option<&str>;
}
impl SummaryTitleAccess for FileNode {
    fn summary_title(&self) -> Option<&str> {
        self.summary.as_ref().and_then(|s| s.title.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_view::model::{FileSummary, Kind, Scope};
    use std::path::PathBuf;

    fn file(id: &str, name: &str, size: u64, mtime: i64) -> FileNode {
        FileNode {
            id: id.to_string(),
            kind: Kind::ClaudeMd,
            abs_path: PathBuf::from(format!("/{name}")),
            display_path: format!("/{name}"),
            scope_badges: vec![Scope::User],
            size_bytes: size,
            mtime_unix_ns: mtime,
            summary: Some(FileSummary {
                title: Some(name.to_string()),
                description: None,
            }),
            issues: vec![],
            symlink_origin: None,
            included_by: None,
            include_depth: 0,
        }
    }

    fn tree(scope_id: &str, files: Vec<FileNode>) -> ConfigTree {
        let recursive = files.len();
        ConfigTree {
            scopes: vec![ScopeNode {
                id: scope_id.to_string(),
                scope: Scope::User,
                label: scope_id.to_string(),
                recursive_count: recursive,
                children: files.into_iter().map(Node::File).collect(),
            }],
            scanned_at_unix_ns: 0,
            cwd: PathBuf::from("/"),
            project_root: PathBuf::from("/"),
            memory_slug: String::new(),
            memory_slug_lossy: false,
            cc_version_hint: None,
            enterprise_mcp_lockout: false,
        }
    }

    #[test]
    fn identical_trees_emit_empty_patch() {
        let a = tree("s", vec![file("1", "a", 10, 1)]);
        let b = tree("s", vec![file("1", "a", 10, 1)]);
        let p = diff(&a, &b);
        assert!(p.added.is_empty());
        assert!(p.updated.is_empty());
        assert!(p.removed.is_empty());
        assert!(p.reordered.is_empty());
        assert!(p.full_snapshot.is_none());
    }

    #[test]
    fn added_file_surfaces_in_patch() {
        let a = tree("s", vec![file("1", "a", 10, 1)]);
        let b = tree("s", vec![file("1", "a", 10, 1), file("2", "b", 20, 2)]);
        let p = diff(&a, &b);
        assert_eq!(p.added.len(), 1);
        assert_eq!(p.added[0].1.id, "2");
    }

    #[test]
    fn removed_file_surfaces_in_patch() {
        let a = tree("s", vec![file("1", "a", 10, 1), file("2", "b", 20, 2)]);
        let b = tree("s", vec![file("1", "a", 10, 1)]);
        let p = diff(&a, &b);
        assert_eq!(p.removed, vec!["2"]);
    }

    #[test]
    fn mtime_change_emits_update() {
        let a = tree("s", vec![file("1", "a", 10, 100)]);
        let b = tree("s", vec![file("1", "a", 10, 200)]);
        let p = diff(&a, &b);
        assert_eq!(p.updated.len(), 1);
        assert_eq!(p.updated[0].id, "1");
    }

    #[test]
    fn size_change_emits_update() {
        let a = tree("s", vec![file("1", "a", 10, 100)]);
        let b = tree("s", vec![file("1", "a", 20, 100)]);
        let p = diff(&a, &b);
        assert_eq!(p.updated.len(), 1);
    }

    #[test]
    fn scope_set_change_triggers_full_snapshot() {
        let a = tree("s-old", vec![file("1", "a", 10, 1)]);
        let b = tree("s-new", vec![file("1", "a", 10, 1)]);
        let p = diff(&a, &b);
        assert!(p.full_snapshot.is_some());
        assert!(p.added.is_empty());
        assert!(p.updated.is_empty());
    }

    #[test]
    fn reorder_within_scope_emits_reorder_only() {
        let a = tree("s", vec![file("1", "a", 10, 1), file("2", "b", 20, 2)]);
        let b = tree("s", vec![file("2", "b", 20, 2), file("1", "a", 10, 1)]);
        let p = diff(&a, &b);
        assert_eq!(p.reordered.len(), 1);
        assert_eq!(p.reordered[0].0, "s");
        assert_eq!(p.reordered[0].1, vec!["2", "1"]);
        assert!(p.added.is_empty());
        assert!(p.updated.is_empty());
    }

    #[test]
    fn unchanged_ordering_does_not_emit_reorder() {
        let a = tree("s", vec![file("1", "a", 10, 1), file("2", "b", 20, 2)]);
        let b = tree("s", vec![file("1", "a", 10, 1), file("2", "b", 20, 2)]);
        let p = diff(&a, &b);
        assert!(p.reordered.is_empty());
    }
}
