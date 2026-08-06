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

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// **The vault's secret rule, restated because `params` crosses the IPC
/// bridge into the JS heap** (`rules/architecture.md`, "IPC trust +
/// secret direction"): every value below is a secret's *name* — the
/// `env_secrets.name` column, an env key like `OPENAI_API_KEY`. The
/// secret itself lives in the 0600 column and leaves Rust only through
/// the clipboard write in `commands/env_secret.rs`. A variant that
/// carried a value would have to be redacted before it reached here,
/// and there is deliberately no such variant.
impl crate::error_code::ErrorCode for VaultError {
    fn code(&self) -> &'static str {
        match self {
            VaultError::Sql(_) => "env_vault.sql",
            VaultError::Io(_) => "env_vault.io",
            VaultError::NotFound(_) => "env_vault.not_found",
            VaultError::DuplicateName(_) => "env_vault.duplicate_name",
            VaultError::InvalidName(_) => "env_vault.invalid_name",
        }
    }

    fn params(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            VaultError::Sql(e) => json!({ "detail": e.to_string() }),
            VaultError::Io(e) => json!({ "detail": e.to_string() }),
            VaultError::NotFound(name) => json!({ "name": name }),
            VaultError::DuplicateName(name) => json!({ "name": name }),
            VaultError::InvalidName(name) => json!({ "name": name }),
        }
    }
}
