//! Per-project Claude Code permission mode + time-boxed grants.
//!
//! See `dev-docs/permission-and-env-secrets.md` for the design. This
//! module is pure-Rust, no Tauri dependency. It provides:
//!
//! - [`mode`] — `PermissionMode`, a typed view of CC's
//!   `permissions.defaultMode` setting value.
//! - [`settings`] — resolve / read / write the nested
//!   `permissions.defaultMode` key across CC's settings layers.
//! - [`grants`] — schema for time-boxed grants
//!   (`~/.claudepot/permission-grants.json`).
//! - [`store`] — atomic load/save of the grants file.
//! - [`eval`] — pure expiration logic (`partition`, `expired_grants`,
//!   `active_grant`).
//!
//! The orchestrator (`src-tauri/src/permission_orchestrator.rs`)
//! loads grants each `usage_snapshot::run_tick`, reverts the expired
//! ones via [`settings`], and surfaces the rest to the UI. Nothing
//! here performs Tauri I/O — that bridge lives in `src-tauri`.

pub mod eval;
pub mod grants;
pub mod mode;
pub mod settings;
pub mod store;

pub use eval::{active_grant, expired_grants, partition, GrantPartition};
pub use grants::{Grant, GrantsFile, ValidationError, SCHEMA_VERSION};
pub use mode::PermissionMode;
pub use settings::{
    clear_default_mode, read_default_mode, resolve_default_mode, write_default_mode,
    PermissionDecisionSource, PermissionSettingsError, PermissionState,
};
pub use store::{load, save, PermissionStoreError};
