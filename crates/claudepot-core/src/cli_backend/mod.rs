pub mod credfile;
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
