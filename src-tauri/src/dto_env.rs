//! DTOs for the `.env` secret-movement surface — the local vault and
//! the per-project `.env` file view.
//!
//! Plaintext secret values never appear in these DTOs. A vault entry
//! carries only a non-reversible preview; a `.env` entry carries the
//! key, its state, and a preview of the value — never the value
//! itself. The value crosses the bridge only via the Rust-side
//! clipboard copy path (`KeyCopyReceiptDto`).

use claudepot_core::env_vault::env_file::EnvLine;
use claudepot_core::env_vault::store::{secret_preview, VaultSecret};
use serde::Serialize;

/// One named secret in the local vault.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSecretDto {
    pub name: String,
    /// Non-reversible preview, e.g. `sk-a…cdef` (long secrets) or
    /// `••••` (anything under 16 chars is fully masked).
    pub secret_preview: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl From<&VaultSecret> for VaultSecretDto {
    fn from(s: &VaultSecret) -> Self {
        Self {
            name: s.name.clone(),
            secret_preview: s.secret_preview.clone(),
            created_at_ms: s.created_at.timestamp_millis(),
            updated_at_ms: s.updated_at.timestamp_millis(),
        }
    }
}

/// One key inside a project `.env*` file, with its tri-state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvFileEntryDto {
    pub key: String,
    /// `"active"` or `"commented"`. (The third state — *absent* — is
    /// simply the key not appearing in this list.)
    pub state: String,
    /// Non-reversible preview of the value; never the value itself.
    pub value_preview: String,
}

/// One `.env*` file in a project root, with its parsed key entries.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvFileDto {
    /// Bare filename, e.g. `.env` or `.env.local`. This is the handle
    /// the mutation commands take — never a full path from the
    /// renderer.
    pub file_name: String,
    /// Absolute path, for display + copy (per `rules/path-display.md`).
    pub path: String,
    pub entries: Vec<EnvFileEntryDto>,
}

/// Every `.env*` file found in a project root.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEnvDto {
    pub project_path: String,
    pub files: Vec<EnvFileDto>,
}

/// Classify parsed `.env` lines into the renderer's entry list.
/// `Other` lines (blanks, prose comments) are dropped — the UI shows
/// keys, not raw file content.
pub fn entries_from_lines(lines: &[EnvLine]) -> Vec<EnvFileEntryDto> {
    lines
        .iter()
        .filter_map(|l| match l {
            EnvLine::Active { key, value } => Some(EnvFileEntryDto {
                key: key.clone(),
                state: "active".to_string(),
                value_preview: secret_preview(value),
            }),
            EnvLine::Commented { key, value } => Some(EnvFileEntryDto {
                key: key.clone(),
                state: "commented".to_string(),
                value_preview: secret_preview(value),
            }),
            EnvLine::Other(_) => None,
        })
        .collect()
}
