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
use zeroize::Zeroize;

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
    // Wrapped in `spawn_blocking` — JSONL parse + redaction render
    // (audit B8 commands_session_share.rs:110).
    tokio::task::spawn_blocking(move || {
        let detail = resolve_session_detail(&target)?;
        let fmt = format_from_dto(format);
        let pol = policy_from_dto(policy);
        Ok::<_, String>(claudepot_core::session_export::export_preview(
            &detail, fmt, &pol,
        ))
    })
    .await
    .map_err(|e| format!("blocking task failed: {e}"))?
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
    // Pre-flighting `resolve_session_detail` + `export_with` + the
    // keychain read on the IPC worker delayed `op_id` return by the
    // full duration of the JSONL parse + redaction pass — for big
    // sessions that's seconds the UI couldn't show progress for
    // (audit B8 commands_session_share.rs:130). Allocate the op id
    // up front so the modal can subscribe immediately, then move the
    // sync prep work into the worker thread.
    let op_id = new_op_id();
    let display = match target.rsplit_once(['/', '\\']) {
        Some((_, base)) => base.to_string(),
        None => target.clone(),
    };
    ops.insert(new_running_op(
        &op_id,
        OpKind::SessionShare,
        display,
        "",
    ));
    let app_c = app.clone();
    let ops_c = ops.inner().clone();
    let op_id_c = op_id.clone();
    let fmt = format_from_dto(format);
    let pol = policy_from_dto(policy);
    // Gist upload runs on its own thread because `deliver` is async
    // (HTTP calls). A current-thread runtime keeps `block_on` legal
    // outside of the global tauri runtime, since the IPC worker
    // already returned the op id.
    std::thread::spawn(move || {
        // All sync prep (JSONL parse, redaction render, keychain
        // read) now lives inside the worker so the IPC `await` above
        // returned immediately with the op id.
        let detail = match resolve_session_detail(&target) {
            Ok(d) => d,
            Err(e) => {
                emit_terminal(&app_c, &ops_c, &op_id_c, Some(e));
                return;
            }
        };
        let body =
            claudepot_core::session_export::export_with(&detail, fmt.clone(), &pol);
        let dest = claudepot_core::session_export_delivery::ExportDestination::Gist {
            filename: claudepot_core::session_export_delivery::default_export_filename(
                &detail.row.session_id,
                claudepot_core::session_export_delivery::extension_for(&fmt),
            ),
            description: format!(
                "Claudepot session export: {}",
                detail.row.session_id
            ),
            public,
        };
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
        // The GUI doesn't need a Rust-side clipboard writer for gist
        // uploads. `deliver` only consults `clipboard` for the
        // `Clipboard` arm, so `None` is correct here.
        let res = rt.block_on(claudepot_core::session_export_delivery::deliver(
            &body, dest, None, &sink,
        ));
        let err = res.err().map(|e| e.to_string());
        emit_terminal(&app_c, &ops_c, &op_id_c, err);
    });
    Ok(op_id)
}

const GH_TOKEN_SERVICE: &str = "claudepot";
const GH_TOKEN_ENTRY: &str = "github-token";

// Token resolution for gist uploads (env var wins over keychain) lives
// in `claudepot_core::session_export_delivery::github_token_resolve`.
// The settings UI commands below operate only on the keychain slot —
// Save/Clear must not be silent no-ops when the env var is also set,
// so they bypass the resolver.

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
    // `s.len()` is byte length; slicing by `s.len() - 4` panics when
    // the trailing chars are multi-byte (audit B8 commands_session_share.rs:225).
    // Walk Unicode scalars from the end so non-ASCII tokens render
    // their last four code points instead of crashing the command.
    if s.is_empty() {
        return None;
    }
    let mut tail: Vec<char> = s.chars().rev().take(4).collect();
    tail.reverse();
    Some(tail.into_iter().collect())
}

#[cfg(test)]
mod last4_of_tests {
    use super::last4_of;

    #[test]
    fn last4_of_ascii_returns_last_four_chars() {
        assert_eq!(last4_of("ghp_abcdefghij"), Some("ghij".to_string()));
    }

    #[test]
    fn last4_of_short_returns_full_string() {
        assert_eq!(last4_of("abc"), Some("abc".to_string()));
    }

    #[test]
    fn last4_of_empty_returns_none() {
        assert_eq!(last4_of(""), None);
    }

    #[test]
    fn last4_of_non_ascii_does_not_panic() {
        // Each emoji is 4 bytes in UTF-8; old code panicked on
        // mid-codepoint slicing. New code walks chars from the end.
        let s = "ab\u{1F600}\u{1F601}\u{1F602}\u{1F603}";
        assert_eq!(
            last4_of(s),
            Some("\u{1F600}\u{1F601}\u{1F602}\u{1F603}".to_string())
        );
    }

    #[test]
    fn last4_of_mixed_bytes_returns_last_four_scalars() {
        // 3 ASCII + 1 emoji → last 4 chars are "bc?<emoji>".
        let s = "abc\u{1F600}";
        assert_eq!(last4_of(s), Some("abc\u{1F600}".to_string()));
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
pub async fn settings_github_token_set(
    mut value: String,
) -> Result<GithubTokenStatus, String> {
    // Trim into a fresh owned `String`; the original IPC arg `value`
    // gets zeroized at the end regardless of outcome (D-5/6/7).
    let mut trimmed = value.trim().to_string();
    // Wipe the IPC bridge buffer up-front — `trimmed` already owns its
    // own copy so the original doesn't need to live any longer.
    value.zeroize();
    if trimmed.is_empty() {
        trimmed.zeroize();
        return Err("token is empty".to_string());
    }

    let result: Result<GithubTokenStatus, String> = (|| {
        let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
            .map_err(|e| format!("keychain init: {e}"))?;
        entry
            .set_password(&trimmed)
            .map_err(|e| format!("keychain set: {e}"))?;
        let env_override = std::env::var("GITHUB_TOKEN")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        Ok(GithubTokenStatus {
            present: true,
            last4: last4_of(&trimmed),
            env_override,
        })
    })();

    trimmed.zeroize();
    result
}

#[tauri::command]
pub async fn settings_github_token_clear() -> Result<(), String> {
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| format!("keychain init: {e}"))?;
    // Delete is a best-effort; not-found is fine.
    let _ = entry.delete_credential();
    Ok(())
}

#[cfg(test)]
mod settings_github_token_set_tests {
    //! Empty-rejection tests for D-5/6/7 settings hardening. We can't
    //! easily exercise the success path in a unit test without a live
    //! keychain entry, but the empty / whitespace-only rejection
    //! paths are pure validation that runs before any keychain I/O.

    use super::*;

    #[tokio::test]
    async fn settings_github_token_set_rejects_empty() {
        // `GithubTokenStatus` doesn't derive `Debug` (it's the wire
        // shape and never gets printed in production), so we match
        // the Err arm explicitly instead of using `unwrap_err`.
        let res = settings_github_token_set("".to_string()).await;
        match res {
            Err(msg) => assert_eq!(msg, "token is empty"),
            Ok(_) => panic!("expected empty input to be rejected"),
        }
    }

    #[tokio::test]
    async fn settings_github_token_set_rejects_whitespace_only() {
        // Trim runs before the empty check; a whitespace-only input
        // must collapse to empty rather than reach the keychain
        // write with a useless payload.
        let res = settings_github_token_set("   \t\n".to_string()).await;
        match res {
            Err(msg) => assert_eq!(msg, "token is empty"),
            Ok(_) => panic!("expected whitespace-only input to be rejected"),
        }
    }
}
