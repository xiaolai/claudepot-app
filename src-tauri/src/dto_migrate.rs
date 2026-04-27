//! DTOs for the migrate Tauri commands.
//!
//! No DTO carries credentials. `accounts.export.json` deserializes
//! to `AccountStubDto { email, org, verification_shape }` — never a
//! token field. See `dev-docs/project-migrate-spec.md` §12.3.

use claudepot_core::migrate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportArgsDto {
    pub output_path: String,
    pub project_prefixes: Vec<String>,
    #[serde(default)]
    pub include_global: bool,
    #[serde(default)]
    pub include_worktree: bool,
    #[serde(default)]
    pub include_live: bool,
    #[serde(default)]
    pub include_claudepot_state: bool,
    #[serde(default)]
    pub no_file_history: bool,
    #[serde(default)]
    pub encrypt: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypt_passphrase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_keyfile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_password: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReceiptDto {
    pub bundle_path: String,
    pub bundle_sha256_sidecar: String,
    pub project_count: usize,
    pub file_count: usize,
}

impl From<migrate::ExportReceipt> for ExportReceiptDto {
    fn from(r: migrate::ExportReceipt) -> Self {
        Self {
            bundle_path: r.bundle_path.to_string_lossy().to_string(),
            bundle_sha256_sidecar: r.bundle_sha256_sidecar.to_string_lossy().to_string(),
            project_count: r.project_count,
            file_count: r.file_count,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectArgsDto {
    pub bundle_path: String,
    /// Required when the bundle ends in `.age`. Stored briefly in JS
    /// memory; UI clears it after the inspect finishes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlanDto {
    pub schema_version: u32,
    pub claudepot_version: String,
    pub created_at: String,
    pub source_os: String,
    pub source_arch: String,
    pub flags: ExportFlagsDto,
    pub projects: Vec<ProjectManifestRefDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFlagsDto {
    pub include_global: bool,
    pub include_worktree: bool,
    pub include_live: bool,
    pub include_claudepot_state: bool,
    pub include_file_history: bool,
    pub encrypted: bool,
    pub signed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectManifestRefDto {
    pub id: String,
    pub source_cwd: String,
    pub source_slug: String,
    pub session_count: u32,
}

impl From<migrate::manifest::BundleManifest> for ImportPlanDto {
    fn from(m: migrate::manifest::BundleManifest) -> Self {
        Self {
            schema_version: m.schema_version,
            claudepot_version: m.claudepot_version,
            created_at: m.created_at,
            source_os: m.source_os,
            source_arch: m.source_arch,
            flags: ExportFlagsDto {
                include_global: m.flags.include_global,
                include_worktree: m.flags.include_worktree,
                include_live: m.flags.include_live,
                include_claudepot_state: m.flags.include_claudepot_state,
                include_file_history: m.flags.include_file_history,
                encrypted: m.flags.encrypted,
                signed: m.flags.signed,
            },
            projects: m
                .projects
                .into_iter()
                .map(|p| ProjectManifestRefDto {
                    id: p.id,
                    source_cwd: p.source_cwd,
                    source_slug: p.source_slug,
                    session_count: p.session_count,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportArgsDto {
    pub bundle_path: String,
    /// One of "skip", "merge", "replace".
    pub mode: String,
    /// One of "imported", "target", or null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer: Option<String>,
    #[serde(default)]
    pub accept_hooks: bool,
    #[serde(default)]
    pub accept_mcp: bool,
    #[serde(default)]
    pub remap: Vec<RemapPairDto>,
    #[serde(default)]
    pub no_file_history: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_key_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemapPairDto {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReceiptDto {
    pub bundle_id: String,
    pub projects_imported: Vec<String>,
    /// `(cwd, reason)` per refused project.
    pub projects_refused: Vec<(String, String)>,
    pub journal_path: String,
    pub dry_run: bool,
    pub accounts_listed: Vec<AccountStubDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStubDto {
    pub uuid: String,
    pub email: String,
    pub org_uuid: Option<String>,
    pub org_name: Option<String>,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub verify_status: String,
}

impl From<migrate::ImportReceipt> for ImportReceiptDto {
    fn from(r: migrate::ImportReceipt) -> Self {
        Self {
            bundle_id: r.bundle_id,
            projects_imported: r.projects_imported,
            projects_refused: r.projects_refused,
            journal_path: r.journal_path.to_string_lossy().to_string(),
            dry_run: r.dry_run,
            accounts_listed: r
                .accounts_listed
                .into_iter()
                .map(|s| AccountStubDto {
                    uuid: s.uuid,
                    email: s.email,
                    org_uuid: s.org_uuid,
                    org_name: s.org_name,
                    subscription_type: s.subscription_type,
                    rate_limit_tier: s.rate_limit_tier,
                    verify_status: s.verify_status,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReceiptDto {
    pub bundle_id: String,
    pub steps_reversed: usize,
    pub steps_tampered: Vec<String>,
    pub steps_errored: Vec<String>,
    pub journal_path: String,
    pub counter_journal_path: String,
}

impl From<migrate::UndoReceipt> for UndoReceiptDto {
    fn from(r: migrate::UndoReceipt) -> Self {
        Self {
            bundle_id: r.bundle_id,
            steps_reversed: r.steps_reversed,
            steps_tampered: r.steps_tampered,
            steps_errored: r.steps_errored,
            journal_path: r.journal_path.to_string_lossy().to_string(),
            counter_journal_path: r.counter_journal_path.to_string_lossy().to_string(),
        }
    }
}
