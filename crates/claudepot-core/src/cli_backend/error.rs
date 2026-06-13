//! Boundary error for the `cli_backend` module (the CLI credential
//! slot). Historically lived in the crate-root `error.rs`; relocated
//! next to its boundary per rust-conventions ("one enum per module
//! boundary"). `crate::error::SwapError` remains a re-export.

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
