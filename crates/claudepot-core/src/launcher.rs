//! Mode D — env-var inject launcher.
//!
//! Spawns a child process with `CLAUDE_CODE_OAUTH_TOKEN` set from the
//! account's stored credential. Zero disk state mutation.

use crate::blob::CredentialBlob;
use crate::cli_backend::swap;
use crate::error::LauncherError;
use crate::oauth::refresh;

use uuid::Uuid;

/// Get a fresh access token for an account, refreshing if expired.
pub async fn get_access_token(account_id: Uuid) -> Result<String, LauncherError> {
    let blob_str = swap::load_private(account_id)
        .map_err(|_| LauncherError::NoStoredCredentials(account_id))?;
    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| LauncherError::CorruptBlob(e.to_string()))?;

    // If token has >5 minutes remaining, use it directly
    if !blob.is_expired(300) {
        return Ok(blob.claude_ai_oauth.access_token.clone());
    }

    // Refresh needed
    tracing::debug!("access token expired/expiring, refreshing...");
    let token_resp = refresh::refresh(&blob.claude_ai_oauth.refresh_token)
        .await
        .map_err(|e| LauncherError::RefreshFailed(e.to_string()))?;

    // Save the rotated credentials, preserving original subscription metadata
    let new_blob_str = refresh::build_blob(&token_resp, Some(&blob));
    swap::save_private(account_id, &new_blob_str)
        .map_err(|e| LauncherError::SaveFailed(e.to_string()))?;

    Ok(token_resp.access_token)
}

/// Spawn a child process with CLAUDE_CODE_OAUTH_TOKEN injected.
/// Returns the child's exit code.
pub async fn run(
    account_id: Uuid,
    args: &[String],
) -> Result<i32, LauncherError> {
    let access_token = get_access_token(account_id).await?;

    if args.is_empty() {
        return Err(LauncherError::NoCommand);
    }

    let (cmd, cmd_args) = args.split_first()
        .ok_or(LauncherError::NoCommand)?;

    let status = tokio::process::Command::new(cmd)
        .args(cmd_args)
        .env("CLAUDE_CODE_OAUTH_TOKEN", &access_token)
        .env("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB", "1")
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await
        .map_err(|e| LauncherError::SpawnFailed(e.to_string()))?;

    Ok(status.code().unwrap_or(1))
}
