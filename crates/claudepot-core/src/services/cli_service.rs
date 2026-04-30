//! CLI slot operations — clear credentials via core.

use crate::account::AccountStore;
use crate::cli_backend;
use crate::cli_backend::swap::ProfileFetcher;

/// Clear CC credentials: save outgoing to Claudepot storage, then remove from CC.
///
/// Audit fix for services/cli_service.rs:77 — takes a `ProfileFetcher`
/// so the live CC blob's identity can be verified before saving it
/// under the active-CLI uuid. Without this, a stale/manually-swapped
/// CC keychain item would be misfiled into the wrong account's
/// private slot. Callers usually pass
/// `cli_backend::swap::DefaultProfileFetcher`.
pub async fn clear_credentials(
    store: &AccountStore,
    fetcher: &dyn ProfileFetcher,
) -> Result<(), ClearError> {
    clear_credentials_inner(store, fetcher, false).await
}

/// Clear CC credentials, overriding the safety refusal that fires when
/// the active-CLI pointer doesn't match a known account but CC still
/// holds a live blob, OR when the live blob's identity disagrees with
/// the active-CLI account. The caller takes responsibility for the
/// loss of that blob.
pub async fn clear_credentials_force(
    store: &AccountStore,
    fetcher: &dyn ProfileFetcher,
) -> Result<(), ClearError> {
    clear_credentials_inner(store, fetcher, true).await
}

async fn clear_credentials_inner(
    store: &AccountStore,
    fetcher: &dyn ProfileFetcher,
    force: bool,
) -> Result<(), ClearError> {
    let platform = cli_backend::create_platform();
    clear_credentials_with_platform_inner(store, platform.as_ref(), fetcher, force).await?;

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
    fetcher: &dyn ProfileFetcher,
) -> Result<(), ClearError> {
    clear_credentials_with_platform_inner(store, platform, fetcher, false).await
}

/// Force-clear variant for the testable path.
pub async fn clear_credentials_with_platform_force(
    store: &AccountStore,
    platform: &dyn cli_backend::CliPlatform,
    fetcher: &dyn ProfileFetcher,
) -> Result<(), ClearError> {
    clear_credentials_with_platform_inner(store, platform, fetcher, true).await
}

async fn clear_credentials_with_platform_inner(
    store: &AccountStore,
    platform: &dyn cli_backend::CliPlatform,
    fetcher: &dyn ProfileFetcher,
    force: bool,
) -> Result<(), ClearError> {
    // Read CC's live credential blob FIRST. The active-CLI pointer in
    // our store can drift (stale, never set, manually edited). Treating
    // it as the sole source of truth means we'd happily wipe a live
    // unknown blob without saving it. Resolve from the live keychain.
    let live_blob = platform
        .read_default()
        .await
        .map_err(|e| ClearError::SaveFailed(format!("failed to read current credentials: {e}")))?;

    let active_uuid_opt = store
        .active_cli_uuid()
        .map_err(|e| ClearError::Store(e.to_string()))?
        .map(|s| {
            s.parse::<uuid::Uuid>()
                .map_err(|e| ClearError::Store(format!("corrupt active UUID: {e}")))
        })
        .transpose()?;

    // Decide whether to back up the live blob, refuse the clear, or
    // proceed with no backup.
    if let Some(blob_str) = live_blob.as_deref() {
        match active_uuid_opt {
            Some(uuid) => {
                // Audit fix: verify the live blob's identity actually
                // matches the account we're about to save it under.
                // Without this check, a manually-swapped CC keychain
                // item or a stale active pointer would land the wrong
                // account's credentials into `uuid`'s private slot —
                // a corruption we'd never recover from automatically.
                let stored_email = store
                    .find_by_uuid(uuid)
                    .map_err(|e| ClearError::Store(e.to_string()))?
                    .ok_or_else(|| {
                        ClearError::Store(format!("active uuid {uuid} not in account store"))
                    })?
                    .email;
                match cli_backend::swap::verify_blob_identity(blob_str, &stored_email, fetcher)
                    .await
                {
                    Ok(()) => {
                        cli_backend::swap::save_private(uuid, blob_str)
                            .map_err(|e| ClearError::SaveFailed(e.to_string()))?;
                    }
                    Err(e) if force => {
                        tracing::warn!(
                            "clear_credentials force: live blob fails identity check ({e}); \
                             dropping it rather than misfiling"
                        );
                    }
                    Err(e) => {
                        return Err(ClearError::IdentityMismatch(e.to_string()));
                    }
                }
            }
            None if force => {
                // Caller explicitly accepted blob loss. Proceed.
                tracing::warn!(
                    "clear_credentials force: dropping live CC blob with no active-CLI pointer"
                );
            }
            None => {
                // Live CC blob with no claim to it. Refuse the
                // destructive clear. Without a uuid we can't address
                // a private slot to back up to; without a backup the
                // clear is irreversible. Force flag is required.
                //
                // Best-effort identification (read the blob, parse
                // email) is intentionally NOT done here — even if we
                // can name the account, it isn't registered with us
                // and we have no slot to write to. The right path is
                // for the caller to register first or pass force.
                let _ = blob_str;
                return Err(ClearError::UnknownLiveBlob);
            }
        }
    }
    // No live blob → nothing to back up. Active pointer (if any) gets
    // cleared below regardless.

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
    /// CC holds live credentials but Claudepot has no active-CLI
    /// pointer to attribute them to. Refusing the destructive clear
    /// because we can't safely back the blob up.
    #[error(
        "CC holds credentials with no Claudepot account claim — register the account or pass force"
    )]
    UnknownLiveBlob,
    /// CC's live credential blob disagrees with the active-CLI
    /// account's stored email. Refusing to save it under that uuid
    /// because doing so would corrupt the private slot. Pass force
    /// to drop the live blob without saving.
    #[error("CC blob does not belong to the active account: {0}")]
    IdentityMismatch(String),
}

// See launcher.rs for the rationale: tests hold `lock_data_dir()`
// across `.await` deliberately to serialize the shared data-dir
// env-var across the test binary.
#[cfg(test)]
#[allow(clippy::await_holding_lock)]
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

    /// A fetcher that always reports the given email regardless of the
    /// access token. Lets the cli-service tests skip real OAuth calls
    /// while still exercising the identity-check path. A second
    /// instance with a different email simulates a stranger blob.
    struct ConstFetcher(&'static str);

    #[async_trait::async_trait]
    impl cli_backend::swap::ProfileFetcher for ConstFetcher {
        async fn fetch_email(
            &self,
            _access_token: &str,
        ) -> Result<String, crate::error::OAuthError> {
            Ok(self.0.to_string())
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
        let fetcher = ConstFetcher("save@example.com");
        clear_credentials_with_platform(&store, &platform, &fetcher)
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

        // Live CC blob is now identity-verified before being saved
        // under the active uuid (audit fix). Use a real CredentialBlob
        // shape so the parse step succeeds and the fetcher's "save@…"
        // matches the stored email.
        let blob_json =
            crate::testing::sample_blob_json(chrono::Utc::now().timestamp_millis() + 3_600_000);
        let platform = MockPlatform::new(Some(&blob_json));
        let fetcher = ConstFetcher("save@example.com");
        clear_credentials_with_platform(&store, &platform, &fetcher)
            .await
            .unwrap();

        // Outgoing blob saved to private storage.
        let saved = cli_backend::swap::load_private(account.uuid).unwrap();
        assert_eq!(saved, blob_json);

        cli_backend::swap::delete_private(account.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_clear_credentials_refuses_identity_mismatch() {
        // Audit fix: if the live CC blob's identity disagrees with
        // the active-CLI account's stored email, refuse the save
        // (otherwise we'd misfile someone else's credentials).
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let account = make_account("active@example.com");
        store.insert(&account).unwrap();
        store.set_active_cli(account.uuid).unwrap();

        let blob_json =
            crate::testing::sample_blob_json(chrono::Utc::now().timestamp_millis() + 3_600_000);
        let platform = MockPlatform::new(Some(&blob_json));
        // Fetcher claims the blob belongs to a DIFFERENT email — i.e.
        // CC was manually swapped or our active pointer is stale.
        let fetcher = ConstFetcher("someone-else@example.com");

        let err = clear_credentials_with_platform(&store, &platform, &fetcher)
            .await
            .expect_err("must refuse to misfile blob under wrong account");
        assert!(
            matches!(err, ClearError::IdentityMismatch(_)),
            "expected IdentityMismatch, got {err:?}"
        );

        // Active pointer must still be set — we refused before any cleanup.
        assert!(store.active_cli_uuid().unwrap().is_some());
        // The wrong blob did NOT land under active uuid's slot.
        assert!(cli_backend::swap::load_private(account.uuid).is_err());
    }

    #[tokio::test]
    async fn test_clear_credentials_no_active_account() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        // No active CLI account
        let platform = MockPlatform::new(None);
        let fetcher = ConstFetcher("save@example.com");
        clear_credentials_with_platform(&store, &platform, &fetcher)
            .await
            .unwrap();

        assert!(store.active_cli_uuid().unwrap().is_none());
    }

    #[tokio::test]
    async fn test_clear_credentials_refuses_when_active_unknown_but_cc_live() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        // No active CLI pointer in store, but CC keychain has a live blob.
        // Without an explicit force, this must refuse rather than wipe
        // the blob with no backup.
        let platform = MockPlatform::new(Some("stranger-cc-blob"));
        let fetcher = ConstFetcher("stranger@example.com");
        let err = clear_credentials_with_platform(&store, &platform, &fetcher)
            .await
            .expect_err("must refuse to clear unknown live blob");
        assert!(
            matches!(err, ClearError::UnknownLiveBlob),
            "expected UnknownLiveBlob, got {err:?}"
        );

        // CC blob and active pointer untouched (we refused before either delete).
        assert_eq!(
            platform.storage.lock().unwrap().as_deref(),
            Some("stranger-cc-blob")
        );
        assert!(store.active_cli_uuid().unwrap().is_none());
    }

    #[tokio::test]
    async fn test_clear_credentials_force_drops_unknown_blob() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        // Force variant accepts the loss when no active pointer exists.
        let platform = MockPlatform::new(Some("stranger-cc-blob"));
        let fetcher = ConstFetcher("stranger@example.com");
        clear_credentials_with_platform_force(&store, &platform, &fetcher)
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
        let fetcher = ConstFetcher("save@example.com");
        clear_credentials_with_platform(&store, &platform, &fetcher)
            .await
            .unwrap();

        assert!(!cred_file.exists());
    }
}
