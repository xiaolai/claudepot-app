//! `claudepot project export | import | migrate inspect | migrate undo`
//!
//! Thin CLI adapter over `claudepot_core::migrate`. Per
//! `.claude/rules/commands.md`, this is a verb-group sibling of the
//! existing `project.rs` because the migrate verbs share helpers
//! (substitution-rule parsing, JSON output shape) and would otherwise
//! fragment the project-noun namespace.
//!
//! Per-verb gates (live-session detection, journal nags) reuse the
//! same machinery as `project move` — see spec §9.
//!
//! Per the commands.md rule for nouns with ≥3 verbs, each verb lives
//! one-per-file under `commands/project_migrate/<verb>.rs`. This
//! entry point holds the shared imports, the helpers consumed by
//! multiple verbs, the submodule declarations, and the `pub use`
//! re-exports `main.rs` depends on.

use crate::AppContext;
use anyhow::{anyhow, Result};
use claudepot_core::account::AccountStore;
use claudepot_core::migrate::{
    self, conflicts, state as migrate_state, ExportOptions, ImportOptions, MigrateError,
};
use claudepot_core::paths;
use claudepot_core::project_helpers::resolve_path;
use std::path::PathBuf;

/// Parse `--remap source=target` repeatedly into pairs. Empty value
/// passes through cleanly so the flag is optional.
pub fn parse_remap(values: &[String]) -> Result<Vec<(String, String)>> {
    values
        .iter()
        .map(|s| {
            s.split_once('=')
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .ok_or_else(|| anyhow!("invalid --remap value (need source=target): {s}"))
        })
        .collect()
}

fn map_migrate_err(e: MigrateError) -> anyhow::Error {
    anyhow!("{e}")
}

// Submodule declarations. Verb implementations live one-per-file
// under `commands/project_migrate/<verb>.rs`; helpers above are
// visible to each submodule via `use super::*;` (child modules
// reach the parent's private items in Rust).
mod export;
mod import;
mod inspect;
mod undo;

// Re-exports — main.rs's match block and clap variants depend on
// these names.
pub use export::{export, ExportArgs};
pub use import::{import, ImportArgs};
pub use inspect::inspect;
pub use undo::undo;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remap_well_formed() {
        let v = parse_remap(&["/a=/b".to_string(), "C:\\x=/y".to_string()]).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], ("/a".to_string(), "/b".to_string()));
        assert_eq!(v[1], ("C:\\x".to_string(), "/y".to_string()));
    }

    #[test]
    fn parse_remap_rejects_missing_separator() {
        let r = parse_remap(&["bad".to_string()]);
        assert!(r.is_err());
    }
}
