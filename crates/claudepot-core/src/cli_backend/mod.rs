pub mod claude_json;
pub mod credfile;
pub mod secret_file;
pub mod storage;
pub mod swap;

#[cfg(target_os = "macos")]
pub mod keychain;

use crate::error::SwapError;

/// Platform-agnostic interface for reading/writing CC CLI credentials.
/// macOS: Keychain via `/usr/bin/security` subprocess.
/// Linux/Windows: `.credentials.json` file.
#[async_trait::async_trait]
pub trait CliPlatform: Send + Sync {
    async fn read_default(&self) -> Result<Option<String>, SwapError>;
    async fn write_default(&self, blob: &str) -> Result<(), SwapError>;
    async fn touch_credfile(&self) -> Result<(), SwapError>;
    /// Remove CC's default credentials completely. Used by `swap::switch`'s
    /// post-switch rollback path when there was no prior blob and the
    /// new target blob failed verification — leaving the slot empty is
    /// the correct "undo" for "slot was empty before".
    ///
    /// Implementations should succeed on "slot was already empty". The
    /// default here is no-op: platforms that can't cleanly clear fall
    /// back to warning-logged inaction rather than a hard failure.
    async fn clear_default(&self) -> Result<(), SwapError> {
        tracing::warn!(
            "clear_default: default no-op implementation — platform has no clear operation"
        );
        Ok(())
    }
}

/// Create the platform-appropriate CLI backend.
pub fn create_platform() -> Box<dyn CliPlatform> {
    #[cfg(target_os = "macos")]
    {
        Box::new(keychain::MacosKeychain)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(credfile::CredentialFile)
    }
}
