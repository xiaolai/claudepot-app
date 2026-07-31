//! Boards — durable, agent-written visual surfaces.
//!
//! See `dev-docs/agent-boards-plan.md` (revision 3) for the full
//! design. A **board** is a named, persistent grid of widgets bound to
//! named data series. An agent pushes structure once, then streams rows
//! into named series; the board outlives the session that created it.
//!
//! Pure Rust, no Tauri dependency. Pieces:
//!
//! - [`spec`] — the board spec: a named widget grid, revisioned.
//! - [`series`] — typed columns and values. Types are fixed at series
//!   creation; a push with a mismatched type is an error, never a
//!   coercion.
//! - [`store`] — [`BoardStore`], SQLite-backed at
//!   `~/.claudepot/boards.db`, with versioned migrations.
//! - [`ingest`] — write-path validation, idempotency, per-writer
//!   ordering, and caps.
//! - [`monitor`] — `PRAGMA data_version` change detection for readers.
//! - [`export`] — the re-importable JSON envelope and CSV-per-series.
//!
//! # There is no write channel, and that is deliberate
//!
//! Every writer — the MCP server subprocess, `claudepot experimental
//! board push`, a scheduled agent run — opens this store **directly**.
//! There is no IPC channel to the GUI and no authentication step. The
//! precedent is `sessions.db`, which `claudepot-cli`'s MCP server, a
//! dozen CLI verbs, and the Tauri app all open with nothing between
//! them.
//!
//! The trust boundary is filesystem permissions on `~/.claudepot/` —
//! the same boundary already protecting `keys.db` and `env-vault.db`,
//! both of which hold secrets while a board holds none.
//!
//! **The cost, paid explicitly:** [`WriterId`] is *self-reported*.
//! Any local process that can open the DB can claim to be any writer.
//! Core cannot prove otherwise, so every surface that renders
//! provenance must present it as a claim ("Reported by …"), never as
//! verified identity. See [`series::Provenance`].
//!
//! This is the right trade for a display surface and the wrong one for
//! approvals — which is why the deferred interaction half must
//! reintroduce an authenticated identity boundary rather than inherit
//! this one.
//!
//! # Classification
//!
//! `boards.db` is **user data, not a cache**. A board's contents exist
//! nowhere else once the writing session ends. Migrations preserve rows
//! (no drop-and-rebuild), corruption is not recoverable by re-indexing,
//! and export exists before any pruning does.

pub mod error;
pub mod export;
pub mod ingest;
pub mod monitor;
pub mod series;
pub mod spec;
pub mod store;
pub mod widget;

pub use error::BoardError;
pub use export::{
    export_csv_dir, export_json, import_json, BoardEnvelope, BoardExport, ExportedRow,
    SeriesExport, EXPORT_SCHEMA_VERSION,
};
pub use ingest::{IngestCaps, PushMode, PushOutcome, PushRequest};
pub use monitor::ChangeMonitor;
pub use series::{Column, ColumnType, Provenance, Row, SeriesDef, Value, WriterId, WriterKind};
pub use spec::{BoardSpec, WidgetKind, WidgetSlot};
pub use store::{
    Board, BoardDetailSnapshot, BoardStore, BoardSummary, SeriesSnapshot, SCHEMA_VERSION,
};
pub use widget::{resolve, AxisRange, AxisScale, Label, Point, RenderPlan, ResolvedWidget};

/// Standard filename for the boards DB inside `claudepot_data_dir()`.
pub const BOARDS_DB_FILENAME: &str = "boards.db";

/// `~/.claudepot/boards.db` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn boards_db_path() -> std::path::PathBuf {
    crate::paths::claudepot_data_dir().join(BOARDS_DB_FILENAME)
}
