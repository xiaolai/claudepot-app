//! Error type for the env-secret vault store.

/// Failures from the SQLite-backed named-secret vault.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no vault secret named `{0}`")]
    NotFound(String),
    #[error("a vault secret named `{0}` already exists")]
    DuplicateName(String),
    #[error("`{0}` is not a valid env key name")]
    InvalidName(String),
}
