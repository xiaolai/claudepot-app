//! CLI slot operations — clear credentials via core.

use crate::account::AccountStore;
use crate::cli_backend;
use crate::error::SwapError;

/// Clear CC credentials: save outgoing to Claudepot storage, then remove from CC.
pub async fn clear_credentials(store: &AccountStore) -> Result<(), ClearError> {
    let platform = cli_backend::create_platform();

    // Save current credentials before clearing
    if let Some(active_uuid_str) = store.active_cli_uuid()
        .map_err(|e| ClearError::Store(e.to_string()))? {
        if let Ok(uuid) = active_uuid_str.parse::<uuid::Uuid>() {
            if let Ok(Some(blob)) = platform.read_default().await {
                cli_backend::swap::save_private(uuid, &blob)
                    .map_err(|e| ClearError::SaveFailed(e.to_string()))?;
            }
        }
    }

    // Clear CC's credentials
    #[cfg(target_os = "macos")]
    {
        cli_backend::keychain::delete(cli_backend::keychain::DEFAULT_SERVICE)
            .await
            .map_err(|e| ClearError::DeleteFailed(e.to_string()))?;
    }

    let cred_path = crate::paths::claude_credentials_file();
    if cred_path.exists() {
        std::fs::remove_file(&cred_path)
            .map_err(|e| ClearError::DeleteFailed(e.to_string()))?;
    }

    store.clear_active_cli()
        .map_err(|e| ClearError::Store(e.to_string()))?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ClearError {
    #[error("failed to save credentials before clearing: {0}")]
    SaveFailed(String),
    #[error("failed to delete credentials: {0}")]
    DeleteFailed(String),
    #[error("store error: {0}")]
    Store(String),
}
