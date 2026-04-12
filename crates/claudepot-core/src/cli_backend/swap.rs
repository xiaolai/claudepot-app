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

// --- Private storage helpers using `keyring` crate ---

fn private_entry(account_id: Uuid) -> keyring::Entry {
    keyring::Entry::new("com.claudepot.credentials", &account_id.to_string())
        .expect("keyring entry creation failed")
}

fn load_private(account_id: Uuid) -> Result<String, SwapError> {
    private_entry(account_id)
        .get_password()
        .map_err(|_| SwapError::NoStoredCredentials(account_id))
}

fn load_private_opt(account_id: Uuid) -> Option<String> {
    private_entry(account_id).get_password().ok()
}

fn save_private(account_id: Uuid, blob: &str) -> Result<(), SwapError> {
    private_entry(account_id)
        .set_password(blob)
        .map_err(|e| SwapError::KeychainError(format!("keyring set failed: {e}")))
}

fn delete_private(account_id: Uuid) -> Result<(), SwapError> {
    let _ = private_entry(account_id).delete_credential();
    Ok(())
}
