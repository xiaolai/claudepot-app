//! Session export / share (gist upload) + GitHub token management.
//!
//! `session_export_preview` renders the transcript in the selected
//! format with a redaction policy applied. `session_share_gist_start`
//! pushes the rendered body to a GitHub gist through the op-progress
//! pipeline.
//!
//! The token used by the uploader is environment-first, keychain
//! second — matching the CLI. The settings UI reads / writes only the
//! keychain slot.

use crate::ops::{
    emit_terminal, new_op_id, new_running_op, OpKind, RunningOps, TauriProgressSink,
};
use claudepot_core::paths;
use tauri::{AppHandle, State};

#[derive(serde::Deserialize)]
pub struct RedactionPolicyDto {
    #[serde(default = "default_true")]
    pub anthropic_keys: bool,
    #[serde(default)]
    pub paths: Option<PathStrategyDto>,
    #[serde(default)]
    pub emails: bool,
    #[serde(default)]
    pub env_assignments: bool,
    #[serde(default)]
    pub custom_regex: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PathStrategyDto {
    Off,
    Relative { root: String },
    Hash,
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportFormatDto {
    Markdown,
    MarkdownSlim,
    Json,
    Html {
        #[serde(default)]
        no_js: bool,
    },
}

fn policy_from_dto(p: Option<RedactionPolicyDto>) -> claudepot_core::redaction::RedactionPolicy {
    use claudepot_core::redaction::{PathStrategy, RedactionPolicy};
    match p {
        None => RedactionPolicy::default(),
        Some(dto) => RedactionPolicy {
            anthropic_keys: dto.anthropic_keys,
            paths: match dto.paths {
                Some(PathStrategyDto::Off) | None => PathStrategy::Off,
                Some(PathStrategyDto::Relative { root }) => PathStrategy::Relative {
                    root: std::path::PathBuf::from(root),
                },
                Some(PathStrategyDto::Hash) => PathStrategy::Hash,
            },
            emails: dto.emails,
            env_assignments: dto.env_assignments,
            custom_regex: dto.custom_regex,
        },
    }
}

fn format_from_dto(f: ExportFormatDto) -> claudepot_core::session_export::ExportFormat {
    use claudepot_core::session_export::ExportFormat;
    match f {
        ExportFormatDto::Markdown => ExportFormat::Markdown,
        ExportFormatDto::MarkdownSlim => ExportFormat::MarkdownSlim,
        ExportFormatDto::Json => ExportFormat::Json,
        ExportFormatDto::Html { no_js } => ExportFormat::Html { no_js },
    }
}

/// File extension matching the requested export format. Used by gist
/// uploads so the uploaded file is named with the right suffix.
fn export_extension(f: &ExportFormatDto) -> &'static str {
    match f {
        ExportFormatDto::Markdown | ExportFormatDto::MarkdownSlim => "md",
        ExportFormatDto::Json => "json",
        ExportFormatDto::Html { .. } => "html",
    }
}

fn resolve_session_detail(
    target: &str,
) -> Result<claudepot_core::session::SessionDetail, String> {
    let cfg = paths::claude_config_dir();
    if target.ends_with(".jsonl") {
        let p = std::path::PathBuf::from(target);
        return claudepot_core::session::read_session_detail_at_path(&cfg, &p)
            .map_err(|e| format!("read session: {e}"));
    }
    claudepot_core::session::read_session_detail(&cfg, target)
        .map_err(|e| format!("read session: {e}"))
}

#[tauri::command]
pub async fn session_export_preview(
    target: String,
    format: ExportFormatDto,
    policy: Option<RedactionPolicyDto>,
) -> Result<String, String> {
    let detail = resolve_session_detail(&target)?;
    let fmt = format_from_dto(format);
    let pol = policy_from_dto(policy);
    Ok(claudepot_core::session_export::export_preview(&detail, fmt, &pol))
}

#[tauri::command]
pub async fn session_share_gist_start(
    target: String,
    format: ExportFormatDto,
    policy: Option<RedactionPolicyDto>,
    public: bool,
    app: AppHandle,
    ops: State<'_, RunningOps>,
) -> Result<String, String> {
    let detail = resolve_session_detail(&target)?;
    let ext = export_extension(&format);
    let fmt = format_from_dto(format);
    let pol = policy_from_dto(policy);
    let body = claudepot_core::session_export::export_with(&detail, fmt, &pol);
    let filename = format!("session-{}.{}", detail.row.session_id, ext);
    let description = format!("Claudepot session export: {}", detail.row.session_id);
    let token = github_token_for_upload()?;
    let op_id = new_op_id();
    ops.insert(new_running_op(
        &op_id,
        OpKind::SessionShare,
        detail.row.session_id.clone(),
        "",
    ));
    // Gist upload runs on its own thread because `share_gist` is
    // async (HTTP calls). We set up a single-thread current-thread
    // runtime so the block_on doesn't require a global tokio handle.
    let app_c = app.clone();
    let ops_c = ops.inner().clone();
    let op_id_c = op_id.clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                emit_terminal(&app_c, &ops_c, &op_id_c, Some(e.to_string()));
                return;
            }
        };
        let sink = TauriProgressSink {
            app: app_c.clone(),
            op_id: op_id_c.clone(),
            ops: ops_c.clone(),
        };
        let res = rt.block_on(claudepot_core::session_share::share_gist(
            &body,
            &filename,
            &description,
            public,
            &token,
            &sink,
        ));
        // `ShareError::Display` is already token-scrubbed.
        let err = res.err().map(|e| e.to_string());
        emit_terminal(&app_c, &ops_c, &op_id_c, err);
    });
    Ok(op_id)
}

const GH_TOKEN_SERVICE: &str = "claudepot";
const GH_TOKEN_ENTRY: &str = "github-token";

/// Token used by gist uploads: env var wins over keychain, same as
/// the CLI. Kept private — the settings UI never sees this directly;
/// it operates on the keychain slot only, so Save/Clear aren't
/// silent no-ops when the env var is also set.
fn github_token_for_upload() -> Result<String, String> {
    if let Ok(v) = std::env::var("GITHUB_TOKEN") {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    entry
        .get_password()
        .map_err(|_| "no GitHub token stored".to_string())
}

/// Read only the keychain-backed token. Returns `None` when absent.
fn github_token_keychain_read() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(_) => Ok(None),
    }
}

#[derive(serde::Serialize)]
pub struct GithubTokenStatus {
    /// True iff a value lives in the Claudepot keychain slot.
    pub present: bool,
    /// Last four chars of the keychain value, if present.
    pub last4: Option<String>,
    /// True when `GITHUB_TOKEN` env var is set — the CLI and the
    /// gist uploader both prefer it over the keychain value. The UI
    /// can surface this so users understand why "Clear" didn't take
    /// effect for an upload.
    pub env_override: bool,
}

fn last4_of(s: &str) -> Option<String> {
    if s.len() >= 4 {
        Some(s[s.len() - 4..].to_string())
    } else if !s.is_empty() {
        Some(s.to_string())
    } else {
        None
    }
}

#[tauri::command]
pub async fn settings_github_token_get() -> Result<GithubTokenStatus, String> {
    let env_override = std::env::var("GITHUB_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    match github_token_keychain_read()? {
        Some(t) => Ok(GithubTokenStatus {
            present: true,
            last4: last4_of(&t),
            env_override,
        }),
        None => Ok(GithubTokenStatus {
            present: false,
            last4: None,
            env_override,
        }),
    }
}

#[tauri::command]
pub async fn settings_github_token_set(value: String) -> Result<GithubTokenStatus, String> {
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    entry
        .set_password(&value)
        .map_err(|e| format!("keychain set: {e}"))?;
    let env_override = std::env::var("GITHUB_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    Ok(GithubTokenStatus {
        present: true,
        last4: last4_of(&value),
        env_override,
    })
}

#[tauri::command]
pub async fn settings_github_token_clear() -> Result<(), String> {
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    // Delete is a best-effort; not-found is fine.
    let _ = entry.delete_credential();
    Ok(())
}
