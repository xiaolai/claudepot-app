use thiserror::Error;

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),

    #[error("keychain: {0}")]
    Keychain(String),

    #[error("key not found: {0}")]
    NotFound(String),
}
