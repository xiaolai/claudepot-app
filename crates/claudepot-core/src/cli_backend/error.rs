//! Boundary error for the `cli_backend` module (the CLI credential
//! slot). Historically lived in the crate-root `error.rs`; relocated
//! next to its boundary per rust-conventions ("one enum per module
//! boundary"). `crate::error::SwapError` remains a re-export.

use crate::error_code::ErrorCode;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SwapError {
    #[error("no stored credentials for account {0}")]
    NoStoredCredentials(uuid::Uuid),

    #[error("no default credentials found in CC storage")]
    NoDefaultCredentials,

    #[error("failed to write credentials: {0}")]
    WriteFailed(String),

    #[error("keychain operation failed: {0}")]
    KeychainError(String),

    #[error("file operation failed: {0}")]
    FileError(#[from] std::io::Error),

    #[error("corrupt credential blob: {0}")]
    CorruptBlob(String),

    #[error("token refresh failed: {0}")]
    RefreshFailed(String),

    #[error(
        "identity mismatch: account {stored_email} holds credentials for {actual_email}. \
         Run `claudepot account remove {stored_email}` and re-add to re-seed."
    )]
    IdentityMismatch {
        stored_email: String,
        actual_email: String,
    },

    /// Surface-agnostic message: callers append their own remediation
    /// copy (CLI says "pass --force"; the GUI/tray surfaces an Override
    /// affordance). Keep "Claude Code process is running" verbatim —
    /// `useActions.useCli` substring-matches on it to route the in-app
    /// switch path into the Override toast.
    #[error("a Claude Code process is running — its next token refresh will overwrite this swap")]
    LiveSessionConflict,

    #[error("identity verification failed: {0}")]
    IdentityVerificationFailed(String),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// Module segment is `cli_backend` — the owning module, which is also
/// the `cli` noun's credential slot.
impl ErrorCode for SwapError {
    fn code(&self) -> &'static str {
        match self {
            SwapError::NoStoredCredentials(_) => "cli_backend.no_stored_credentials",
            SwapError::NoDefaultCredentials => "cli_backend.no_default_credentials",
            SwapError::WriteFailed(_) => "cli_backend.write_failed",
            SwapError::KeychainError(_) => "cli_backend.keychain_error",
            SwapError::FileError(_) => "cli_backend.file_error",
            SwapError::CorruptBlob(_) => "cli_backend.corrupt_blob",
            SwapError::RefreshFailed(_) => "cli_backend.refresh_failed",
            SwapError::IdentityMismatch { .. } => "cli_backend.identity_mismatch",
            SwapError::LiveSessionConflict => "cli_backend.live_session_conflict",
            SwapError::IdentityVerificationFailed(_) => "cli_backend.identity_verification_failed",
        }
    }

    fn params(&self) -> Value {
        match self {
            // The account row's uuid, never a credential.
            SwapError::NoStoredCredentials(uuid) => json!({ "uuid": uuid.to_string() }),
            SwapError::NoDefaultCredentials => json!({}),
            SwapError::WriteFailed(detail) => json!({ "detail": detail }),
            SwapError::KeychainError(detail) => json!({ "detail": detail }),
            SwapError::FileError(e) => json!({ "detail": e.to_string() }),
            // Every constructor passes a serde/JSON parse message or a
            // `.claude.json:`-prefixed one — a shape complaint, never
            // the blob's contents. Re-check that before widening.
            SwapError::CorruptBlob(detail) => json!({ "detail": detail }),
            SwapError::RefreshFailed(detail) => json!({ "detail": detail }),
            // Two emails, both already first-class UI data (the account
            // noun's name is its email). No token material.
            SwapError::IdentityMismatch {
                stored_email,
                actual_email,
            } => json!({ "stored_email": stored_email, "actual_email": actual_email }),
            SwapError::LiveSessionConflict => json!({}),
            SwapError::IdentityVerificationFailed(detail) => json!({ "detail": detail }),
        }
    }
}
