//! Mode A atomic swap primitive for CLI credentials.
//! See reference.md §I.7.

use crate::account::AccountStore;
use crate::error::SwapError;
use super::CliPlatform;
use uuid::Uuid;
use std::fs;

/// Acquire an exclusive file lock for swap operations.
/// Returns the locked file handle — lock is released when dropped.
fn acquire_swap_lock() -> Result<fs::File, SwapError> {
    let lock_path = crate::paths::claudepot_data_dir().join(".swap.lock");
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err(SwapError::WriteFailed(
                    "another claudepot swap is in progress".into(),
                ));
            }
            return Err(SwapError::FileError(err));
        }
    }
    #[cfg(windows)]
    {
        // On Windows, opening with write + no sharing provides exclusion.
        // The OpenOptions above already provide this behavior.
    }

    Ok(file)
}

/// Swap the active CLI account from `current_id` to `target_id`.
///
/// Acquires an exclusive file lock to prevent concurrent swaps.
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
    // Acquire exclusive lock — prevents concurrent swaps.
    let _lock = acquire_swap_lock()?;

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

    // _lock dropped here — releases the file lock.
    Ok(())
}

// --- Private storage: file-based, 0600 perms ---
//
// WORKAROUND: using files instead of the `keyring` crate.
//
// The `keyring` crate calls macOS `SecItem*` APIs, which require the
// calling binary to have a valid code-signing identity. Our debug
// builds (`cargo build`) produce ad-hoc signed binaries that lack
// the entitlements needed for Keychain access — `set_password()`
// returns Ok(()) but silently writes nothing. This affects ALL
// unsigned binaries, not just SSH sessions.
//
// Once the release binary is signed with a Developer ID certificate
// (implementation plan Phase 12), switch back to `keyring`:
//
//   keyring::Entry::new("com.claudepot.credentials", &uuid.to_string())
//
// Tracked as audit finding Critical #2 (2026-04-12).
//
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock CliPlatform for testing swap logic.
    struct MockPlatform {
        storage: Mutex<Option<String>>,
        fail_write: bool,
    }

    impl MockPlatform {
        fn new(initial: Option<&str>) -> Self {
            Self {
                storage: Mutex::new(initial.map(String::from)),
                fail_write: false,
            }
        }
        fn failing() -> Self {
            Self { storage: Mutex::new(None), fail_write: true }
        }
        fn get(&self) -> Option<String> {
            self.storage.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            Ok(self.storage.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
            if self.fail_write {
                return Err(SwapError::WriteFailed("mock write failure".into()));
            }
            *self.storage.lock().unwrap() = Some(blob.to_string());
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    fn test_store() -> (AccountStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = AccountStore::open(&db).unwrap();
        (store, dir)
    }

    #[test]
    fn test_private_storage_roundtrip() {
        let id = Uuid::new_v4();
        let blob = r#"{"test":"data"}"#;

        // Save
        save_private(id, blob).unwrap();

        // Load
        let loaded = load_private(id).unwrap();
        assert_eq!(loaded, blob);

        // Verify 0600 permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(private_path(id)).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }

        // Delete
        delete_private(id).unwrap();
        assert!(load_private(id).is_err());
    }

    #[test]
    fn test_private_storage_missing() {
        let id = Uuid::new_v4();
        assert!(matches!(
            load_private(id),
            Err(SwapError::NoStoredCredentials(_))
        ));
    }

    #[test]
    fn test_private_storage_overwrite() {
        let id = Uuid::new_v4();
        save_private(id, "first").unwrap();
        save_private(id, "second").unwrap();
        assert_eq!(load_private(id).unwrap(), "second");
        delete_private(id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_success() {
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();

        // Pre-store target credentials
        save_private(target_id, "target_blob").unwrap();

        let platform = MockPlatform::new(None);
        switch(&store, None, target_id, &platform).await.unwrap();

        assert_eq!(platform.get(), Some("target_blob".to_string()));
        assert_eq!(store.active_cli_uuid().unwrap(), Some(target_id.to_string()));

        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_saves_outgoing() {
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        // Pre-store target credentials
        save_private(target_id, "target_blob").unwrap();

        // Platform has current credentials (as if CC refreshed them)
        let platform = MockPlatform::new(Some("refreshed_current_blob"));

        switch(&store, Some(current_id), target_id, &platform).await.unwrap();

        // Current's credentials should be saved to private storage
        assert_eq!(load_private(current_id).unwrap(), "refreshed_current_blob");
        // Target should be in platform
        assert_eq!(platform.get(), Some("target_blob".to_string()));

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_rollback_on_write_failure() {
        let (store, _dir) = test_store();
        let current_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();

        // Pre-store both
        save_private(current_id, "original_current").unwrap();
        save_private(target_id, "target_blob").unwrap();

        // Platform will fail on write
        let platform = MockPlatform::failing();
        // Set initial storage to simulate current CC credentials
        *platform.storage.lock().unwrap() = Some("cc_current".to_string());

        let result = switch(&store, Some(current_id), target_id, &platform).await;
        assert!(result.is_err());

        // Current's private storage should be rolled back to original
        assert_eq!(load_private(current_id).unwrap(), "original_current");

        delete_private(current_id).unwrap();
        delete_private(target_id).unwrap();
    }

    #[tokio::test]
    async fn test_swap_no_target_credentials() {
        let (store, _dir) = test_store();
        let target_id = Uuid::new_v4();
        // Don't pre-store — target has no credentials
        let platform = MockPlatform::new(None);

        let result = switch(&store, None, target_id, &platform).await;
        assert!(matches!(result, Err(SwapError::NoStoredCredentials(_))));
    }
}
