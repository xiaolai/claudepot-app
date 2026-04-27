//! Typed errors surfaced from the artifact-lifecycle layer.
//!
//! Every variant maps to an actionable UI affordance in the GUI
//! (toast text, recovery offer, refusal explanation). New variants
//! land here behind `#[non_exhaustive]` so adding refusal categories
//! doesn't break exhaustive matches downstream.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LifecycleError {
    /// The path is not eligible for any lifecycle action — explained
    /// by the inner `RefuseReason`.
    #[error("{0}")]
    Refused(#[from] RefuseReason),

    /// Trying to disable / trash a path that no longer exists.
    /// Trash returns this; disable treats already-disabled as a no-op
    /// success (idempotent).
    #[error("source missing: {0}")]
    SourceMissing(PathBuf),

    /// Destination already exists and the caller passed
    /// `OnConflict::Refuse`. Caller can retry with `Suffix`.
    #[error("destination already exists: {0}")]
    Conflict(PathBuf),

    /// Restore was attempted but the original `scope_root` no longer
    /// exists (project deleted). Caller can recreate the project or
    /// `forget` the trash entry.
    #[error("scope root missing: {0}")]
    ScopeRootMissing(PathBuf),

    /// Trash entry isn't in the `Healthy` state and the caller invoked
    /// the wrong action — e.g., `restore` on a `MissingManifest` entry
    /// which requires `recover` instead.
    #[error("trash entry state {state:?} cannot {action}")]
    WrongTrashState {
        state: &'static str,
        action: &'static str,
    },

    /// Trash entry id wasn't found.
    #[error("trash entry not found: {0}")]
    TrashEntryNotFound(String),

    /// IO failure during a rename / copy / read. The `op` field
    /// names the high-level operation for diagnostics.
    #[error("{op} failed: {source}")]
    Io {
        op: &'static str,
        #[source]
        source: std::io::Error,
    },

    /// Manifest JSON couldn't be parsed.
    #[error("manifest parse failed: {0}")]
    ManifestParse(#[source] serde_json::Error),

    /// Recovery couldn't infer the artifact kind from a payload —
    /// e.g., orphan-payload entries with multiple children.
    #[error("recovery ambiguous: {0}")]
    RecoveryAmbiguous(String),
}

impl LifecycleError {
    /// Helper for IO call sites — wraps an `io::Error` with the
    /// op label so error toasts read clearly ("disable failed: …",
    /// not just "I/O error").
    pub fn io(op: &'static str) -> impl FnOnce(std::io::Error) -> Self {
        move |source| LifecycleError::Io { op, source }
    }
}

/// Refusal categories surfaced on every entry-point. Each variant
/// maps to a unique UI message — the GUI hides Disable/Trash actions
/// entirely when classify returns one of these (rather than greying
/// out a button).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RefuseReason {
    #[error("plugin-owned ({plugin_id}) — manage from the plugin manager")]
    Plugin { plugin_id: String, path: PathBuf },

    #[error("managed by your organization's policy — read-only in Claudepot")]
    ManagedPolicy { root: PathBuf, path: PathBuf },

    #[error("outside Claudepot's managed scope ({path})")]
    OutOfScope { path: PathBuf },

    #[error("symlink loop at {path}")]
    SymlinkLoop { path: PathBuf },

    #[error("not a Skill, Agent, or Slash command ({path})")]
    WrongKind { path: PathBuf },

    #[error("path is already disabled ({path}) — use enable instead")]
    AlreadyDisabled { path: PathBuf },
}

pub type Result<T, E = LifecycleError> = std::result::Result<T, E>;
