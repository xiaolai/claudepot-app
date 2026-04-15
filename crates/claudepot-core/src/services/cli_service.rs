//! CLI slot operations — clear credentials via core.

use crate::account::AccountStore;
use crate::cli_backend;

/// Clear CC credentials: save outgoing to Claudepot storage, then remove from CC.
pub async fn clear_credentials(store: &AccountStore) -> Result<(), ClearError> {
    let platform = cli_backend::create_platform();
    clear_credentials_with_platform(store, platform.as_ref()).await?;

    // Also clear CC's keychain entry on macOS
    #[cfg(target_os = "macos")]
    {
        cli_backend::keychain::delete(cli_backend::keychain::DEFAULT_SERVICE)
            .await
            .map_err(|e| ClearError::DeleteFailed(e.to_string()))?;
    }

    Ok(())
}

/// Clear CC credentials using a provided platform (testable variant).
pub async fn clear_credentials_with_platform(
    store: &AccountStore,
    platform: &dyn cli_backend::CliPlatform,
) -> Result<(), ClearError> {
    // Save current credentials before clearing — fail if backup fails
    if let Some(active_uuid_str) = store
        .active_cli_uuid()
        .map_err(|e| ClearError::Store(e.to_string()))?
    {
        let uuid: uuid::Uuid = active_uuid_str
            .parse()
            .map_err(|e| ClearError::Store(format!("corrupt active UUID: {e}")))?;
        if let Some(blob) = platform.read_default().await.map_err(|e| {
            ClearError::SaveFailed(format!("failed to read current credentials: {e}"))
        })? {
            cli_backend::swap::save_private(uuid, &blob)
                .map_err(|e| ClearError::SaveFailed(e.to_string()))?;
        }
    }

    let cred_path = crate::paths::claude_credentials_file();
    if cred_path.exists() {
        std::fs::remove_file(&cred_path).map_err(|e| ClearError::DeleteFailed(e.to_string()))?;
    }

    store
        .clear_active_cli()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SwapError;
    use crate::testing::{lock_data_dir, make_account, setup_test_data_dir, test_store};
    use std::sync::Mutex as StdMutex;

    struct MockPlatform {
        storage: StdMutex<Option<String>>,
    }

    impl MockPlatform {
        fn new(blob: Option<&str>) -> Self {
            Self {
                storage: StdMutex::new(blob.map(String::from)),
            }
        }
    }

    #[async_trait::async_trait]
    impl cli_backend::CliPlatform for MockPlatform {
        async fn read_default(&self) -> Result<Option<String>, SwapError> {
            Ok(self.storage.lock().unwrap().clone())
        }
        async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
            *self.storage.lock().unwrap() = Some(blob.to_string());
            Ok(())
        }
        async fn touch_credfile(&self) -> Result<(), SwapError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_clear_credentials_clears_active_pointer() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let account = make_account("clear@example.com");
        store.insert(&account).unwrap();
        store.set_active_cli(account.uuid).unwrap();

        let platform = MockPlatform::new(None);
        clear_credentials_with_platform(&store, &platform)
            .await
            .unwrap();

        assert!(store.active_cli_uuid().unwrap().is_none());
    }

    #[tokio::test]
    async fn test_clear_credentials_saves_outgoing_blob() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let account = make_account("save@example.com");
        store.insert(&account).unwrap();
        store.set_active_cli(account.uuid).unwrap();

        let platform = MockPlatform::new(Some("current-cc-blob"));
        clear_credentials_with_platform(&store, &platform)
            .await
            .unwrap();

        // Outgoing blob saved to private storage
        let saved = cli_backend::swap::load_private(account.uuid).unwrap();
        assert_eq!(saved, "current-cc-blob");

        cli_backend::swap::delete_private(account.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_clear_credentials_no_active_account() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        // No active CLI account
        let platform = MockPlatform::new(None);
        clear_credentials_with_platform(&store, &platform)
            .await
            .unwrap();

        assert!(store.active_cli_uuid().unwrap().is_none());
    }

    #[tokio::test]
    async fn test_clear_credentials_removes_cred_file() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        // Set up CLAUDE_CONFIG_DIR with a credential file
        let config_dir = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", config_dir.path());
        let cred_file = config_dir.path().join(".credentials.json");
        std::fs::write(&cred_file, "old-creds").unwrap();

        let platform = MockPlatform::new(None);
        clear_credentials_with_platform(&store, &platform)
            .await
            .unwrap();

        assert!(!cred_file.exists());
    }
}
