//! Mode A atomic swap primitive for CLI credentials.
//! See reference.md §I.7.

use crate::account::AccountStore;
use crate::error::SwapError;
use super::CliPlatform;
use uuid::Uuid;

/// Swap the active CLI account from `current_id` to `target_id`.
///
/// 1. Read the current blob from CC storage (may have been refreshed).
/// 2. Save outgoing blob to Claudepot private storage.
/// 3. Load target blob from Claudepot private storage.
/// 4. Write target blob to CC storage + touch credfile.
/// 5. On failure at step 4, rollback Claudepot private storage.
pub async fn switch(
    store: &AccountStore,
    current_id: Option<Uuid>,
    target_id: Uuid,
    platform: &dyn CliPlatform,
) -> Result<(), SwapError> {
    // Load target blob from Claudepot private storage first.
    // If it doesn't exist, fail before touching anything.
    let target_blob = load_private(target_id)?;

    // Save outgoing (current CC blob may have been refreshed by the CLI).
    if let Some(cur) = current_id {
        if let Some(current_blob) = platform.read_default().await? {
            let previous_private = load_private_opt(cur);
            save_private(cur, &current_blob)?;

            // Write target to CC storage.
            if let Err(e) = platform.write_default(&target_blob).await {
                // Rollback: restore previous Claudepot blob for outgoing account.
                match previous_private {
                    Some(prev) => { let _ = save_private(cur, &prev); }
                    None => { let _ = delete_private(cur); }
                }
                return Err(e);
            }
        } else {
            // No current blob in CC — just write target directly.
            platform.write_default(&target_blob).await?;
        }
    } else {
        platform.write_default(&target_blob).await?;
    }

    // Bump mtime for cross-process invalidation (best-effort).
    let _ = platform.touch_credfile().await;

    // Update active pointer in account store.
    store
        .set_active_cli(target_id)
        .map_err(|e| SwapError::WriteFailed(format!("db update failed: {e}")))?;

    Ok(())
}

// --- Private storage: file-based, 0600 perms ---
// Using files instead of the `keyring` crate because unsigned CLI
// binaries over SSH can't reliably write to macOS keychains.
// Blobs are stored at: <claudepot_data_dir>/credentials/<uuid>.json

fn private_path(account_id: Uuid) -> std::path::PathBuf {
    crate::paths::claudepot_data_dir()
        .join("credentials")
        .join(format!("{}.json", account_id))
}

pub fn load_private(account_id: Uuid) -> Result<String, SwapError> {
    let path = private_path(account_id);
    std::fs::read_to_string(&path)
        .map_err(|_| SwapError::NoStoredCredentials(account_id))
}

fn load_private_opt(account_id: Uuid) -> Option<String> {
    std::fs::read_to_string(private_path(account_id)).ok()
}

pub fn save_private(account_id: Uuid, blob: &str) -> Result<(), SwapError> {
    let path = private_path(account_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = tempfile::NamedTempFile::new_in(
        path.parent().unwrap_or(std::path::Path::new(".")),
    )?;
    std::io::Write::write_all(&mut tmp, blob.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    tmp.persist(&path)
        .map_err(|e| SwapError::WriteFailed(format!("persist failed: {e}")))?;
    Ok(())
}

pub fn delete_private(account_id: Uuid) -> Result<(), SwapError> {
    let path = private_path(account_id);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}
