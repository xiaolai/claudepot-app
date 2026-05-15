//! Local named-secret vault + line-oriented `.env` file editing.
//!
//! See `dev-docs/permission-and-env-secrets.md`. Pure-Rust, no Tauri
//! dependency. Two independent pieces:
//!
//! - [`env_file`] — format-preserving line editor for `.env` files
//!   (`parse` / `set` / `comment` / `uncomment` / `delete`). Touches
//!   only the target key's line.
//! - [`store`] — `VaultStore`, a SQLite-backed registry of named
//!   secrets at `~/.claudepot/env-vault.db` (0600). Mirrors
//!   `keys::store`'s at-rest pattern.
//!
//! The Tauri layer (`src-tauri/src/commands/env_secret.rs`) is the
//! only egress point for a plaintext value — it writes to the OS
//! clipboard Rust-side or into a `.env` file, and zeroizes its copy.

pub mod env_file;
pub mod error;
pub mod store;

pub use env_file::{
    comment, delete, parse, set, uncomment, is_valid_key, EnvEditError, EnvLine,
};
pub use error::VaultError;
pub use store::{secret_preview, VaultSecret, VaultStore};

/// Standard filename for the vault DB inside `claudepot_data_dir()`.
pub const VAULT_DB_FILENAME: &str = "env-vault.db";

/// `~/.claudepot/env-vault.db` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn vault_db_path() -> std::path::PathBuf {
    crate::paths::claudepot_data_dir().join(VAULT_DB_FILENAME)
}
