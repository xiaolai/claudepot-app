use crate::error_code::ErrorCode;
use serde_json::{json, Value};
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

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
impl ErrorCode for KeyError {
    fn code(&self) -> &'static str {
        match self {
            KeyError::Sql(_) => "keys.sql",
            KeyError::Io(_) => "keys.io",
            KeyError::Keychain(_) => "keys.keychain",
            KeyError::NotFound(_) => "keys.not_found",
        }
    }

    fn params(&self) -> Value {
        match self {
            // `detail` is the underlying failure text the English
            // message interpolates — a localized sentence appends it
            // rather than re-deriving it from `message`.
            KeyError::Sql(e) => json!({ "detail": e.to_string() }),
            KeyError::Io(e) => json!({ "detail": e.to_string() }),
            KeyError::Keychain(detail) => json!({ "detail": detail }),
            // Always a row uuid (`store.rs` is the only constructor),
            // never a secret.
            KeyError::NotFound(uuid) => json!({ "uuid": uuid }),
        }
    }
}
