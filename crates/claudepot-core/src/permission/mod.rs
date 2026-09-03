//! Per-project Claude Code permission mode + time-boxed grants.
//!
//! See `dev-docs/permission-and-env-secrets.md` for the design. This
//! module is pure-Rust, no Tauri dependency. It provides:
//!
//! - [`mode`] — `PermissionMode`, a typed view of CC's
//!   `permissions.defaultMode` setting value.
//! - [`settings`] — resolve / read / write the nested
//!   `permissions.defaultMode` key across CC's settings layers,
//!   including the rule that CC ignores `bypassPermissions` / `auto`
//!   from project-scope files since 2.1.257.
//! - [`grants`] — schema for grants
//!   (`~/.claudepot/permission-grants.json`), plus the schema-1
//!   records still awaiting a settings revert.
//! - [`store`] — atomic load/save of the grants file.
//! - [`eval`] — pure expiration logic (`partition`, `expired_grants`,
//!   `active_grant`).
//! - [`hook`] — what the `PermissionRequest` hook decides from a
//!   grant, and the read-only load it uses.
//!
//! A grant is answered by Claude Code's `PermissionRequest` hook
//! (`claudepot hook permission-request`, shared with
//! `remote::approval`), not by a settings write: CC ≥ 2.1.257 refuses
//! `bypassPermissions` from the project-scope files a per-project
//! grant could write to. The orchestrator
//! (`src-tauri/src/permission_orchestrator.rs`) loads grants each
//! `usage_snapshot::run_tick`, drops the expired ones, reverts any
//! schema-1 settings writes, and keeps the hook entry in step. Nothing
//! here performs Tauri I/O — that bridge lives in `src-tauri`.

pub mod eval;
pub mod grants;
pub mod hook;
pub mod mode;
pub mod settings;
pub mod store;

pub use eval::{active_grant, expired_grants, partition, GrantPartition};
pub use grants::{Grant, GrantsFile, LegacySettingsGrant, ValidationError, SCHEMA_VERSION};
pub use mode::PermissionMode;
pub use settings::{
    clear_default_mode, read_default_mode, resolve_default_mode, write_default_mode, IgnoredValue,
    PermissionDecisionSource, PermissionSettingsError, PermissionState,
    PROJECT_SCOPE_IGNORES_SINCE,
};
pub use store::{load, save, PermissionStoreError};
