//! Key management — Anthropic API keys and Claude Code OAuth tokens.
//!
//! Keys are a separate noun from accounts: an account has CLI + Desktop
//! credential slots that represent "who CC is logged in as"; a key is a
//! freestanding credential the user created out-of-band (via the
//! Anthropic console for API keys, or `claude setup-token` for OAuth
//! tokens). They may be tagged with an account for context, but the
//! relationship is informational — no enforcement.
//!
//! Storage:
//! * Metadata lives in `keys.db` (SQLite) under the Claudepot data dir.
//! * Secrets live in the OS keychain via the `keyring` crate. This is
//!   Claudepot's OWN secret namespace — NEVER use `/usr/bin/security`
//!   here; that is reserved for CC's shared `Claude Code-credentials`
//!   slot (see `cli_backend::keychain`).

mod error;
mod format;
mod keychain;
mod store;
mod types;

pub use error::KeyError;
pub use format::{classify_token, token_preview, KeyPrefix};
pub use keychain::{
    delete_api_secret, delete_oauth_secret, read_api_secret, read_oauth_secret,
    write_api_secret, write_oauth_secret,
};
pub use store::KeyStore;
pub use types::{ApiKey, OauthToken};

/// OAuth tokens created by `claude setup-token` have a one-year
/// validity window. The token blob is opaque, so we record our own
/// `created_at` at add-time and derive the expiry + days-remaining
/// at read-time. Anthropic may change this upstream without notice —
/// treat it as a proxy, not ground truth. A `401` from the usage
/// endpoint remains the authoritative "expired" signal.
pub const OAUTH_TOKEN_VALIDITY_DAYS: i64 = 365;
