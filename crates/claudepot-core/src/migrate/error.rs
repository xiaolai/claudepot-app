//! Errors for the migrate subsystem.
//!
//! `MigrateError` is the surface error returned from every public
//! migrate API. CLI / Tauri adapters should map it to their own
//! presentation layer.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MigrateError {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("bundle integrity violation: {0}")]
    IntegrityViolation(String),

    #[error("serialization: {0}")]
    Serialize(String),

    #[error(
        "unsupported bundle schema_version {found} (expected {expected})"
    )]
    UnsupportedSchemaVersion { found: u32, expected: u32 },

    /// Configuration error — user / adapter supplied a missing or
    /// inconsistent flag. Distinguished from `NotImplemented` so the
    /// CLI can format the message as a user-facing usage error rather
    /// than a feature-gap message.
    #[error("configuration error: {0}")]
    Configuration(String),

    #[error("project not found in bundle: {0}")]
    ProjectNotInBundle(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("trust gate {gate}: {reason}")]
    TrustGate { gate: String, reason: String },

    #[error("live session detected on {0} — refusing to import")]
    LiveSession(String),

    #[error("{0}")]
    Project(#[from] crate::error::ProjectError),

    #[error("not yet implemented: {0}")]
    NotImplemented(String),
}
