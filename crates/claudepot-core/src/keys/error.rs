use thiserror::Error;

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),

    /// Filesystem failures around the store file itself (create dir,
    /// permission hardening). Previously shoehorned into `Sql` via
    /// `ToSqlConversionFailure`, which misreported a chmod failure as
    /// a SQL conversion error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("keychain: {0}")]
    Keychain(String),

    #[error("key not found: {0}")]
    NotFound(String),
}
