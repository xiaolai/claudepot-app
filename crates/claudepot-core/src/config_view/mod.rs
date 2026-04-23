//! Read-only Config browser — parse, mask, merge, watch CC configuration
//! artifacts. P0 ships the module scaffold, the `model::Scope/Kind` enums,
//! a minimal empty [`ConfigTree`], and the editor-detection `launcher`
//! submodule. Parse/merge/watch land in later phases (see
//! `dev-docs/config-section-plan.md` §15).

pub mod discover;
pub mod launcher;
pub mod model;
pub mod parse;

use std::path::Path;

/// Walk the CC-mandated roots at `cwd` and return a populated tree.
pub fn scan(cwd: &Path) -> model::ConfigTree {
    discover::assemble_tree(cwd)
}

/// Empty-tree builder — kept for callers that want an anchored skeleton
/// without scanning. Useful in early UI states and tests.
pub fn empty_tree(cwd: &Path) -> model::ConfigTree {
    let cwd = cwd.to_path_buf();
    model::ConfigTree {
        scopes: Vec::new(),
        scanned_at_unix_ns: current_unix_ns(),
        cwd: cwd.clone(),
        project_root: cwd,
        memory_slug: String::new(),
        memory_slug_lossy: false,
        cc_version_hint: None,
        enterprise_mcp_lockout: false,
    }
}

fn current_unix_ns() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}
