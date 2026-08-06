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

    #[error("unsupported bundle schema_version {found} (expected {expected})")]
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

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl crate::error_code::ErrorCode for MigrateError {
    fn code(&self) -> &'static str {
        match self {
            MigrateError::Io(_) => "migrate.io",
            MigrateError::IntegrityViolation(_) => "migrate.integrity_violation",
            MigrateError::Serialize(_) => "migrate.serialize",
            MigrateError::UnsupportedSchemaVersion { .. } => "migrate.unsupported_schema_version",
            MigrateError::Configuration(_) => "migrate.configuration",
            MigrateError::ProjectNotInBundle(_) => "migrate.project_not_in_bundle",
            MigrateError::Conflict(_) => "migrate.conflict",
            MigrateError::TrustGate { .. } => "migrate.trust_gate",
            MigrateError::LiveSession(_) => "migrate.live_session",
            // Delegates rather than minting `migrate.project`. The
            // variant is `#[error("{0}")]` — its English text *is* the
            // inner error's, so a `migrate.project` code carrying
            // `detail` would freeze an English clause inside a
            // localized sentence. Two enums, one code, one translation.
            MigrateError::Project(e) => crate::error_code::ErrorCode::code(e),
            MigrateError::NotImplemented(_) => "migrate.not_implemented",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            MigrateError::Io(e) => serde_json::json!({ "detail": e.to_string() }),
            MigrateError::IntegrityViolation(detail) => serde_json::json!({ "detail": detail }),
            MigrateError::Serialize(detail) => serde_json::json!({ "detail": detail }),
            MigrateError::UnsupportedSchemaVersion { found, expected } => serde_json::json!({
                "found": found,
                "expected": expected,
            }),
            MigrateError::Configuration(detail) => serde_json::json!({ "detail": detail }),
            // A composed sentence ("no on-disk slug for cwd … (looked
            // for …)"), not a bare path — see `migrate::mod`'s only
            // constructor.
            MigrateError::ProjectNotInBundle(detail) => serde_json::json!({ "detail": detail }),
            MigrateError::Conflict(detail) => serde_json::json!({ "detail": detail }),
            MigrateError::TrustGate { gate, reason } => serde_json::json!({
                "gate": gate,
                "reason": reason,
            }),
            MigrateError::LiveSession(path) => serde_json::json!({ "path": path }),
            MigrateError::Project(e) => crate::error_code::ErrorCode::params(e),
            MigrateError::NotImplemented(detail) => serde_json::json!({ "detail": detail }),
        }
    }
}
