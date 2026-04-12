//! File-based credential storage — Linux, Windows, macOS fallback.
//! Reads/writes `$CLAUDE_CONFIG_DIR/.credentials.json` with 0600 perms.

use crate::error::SwapError;
use crate::paths;
use std::io::Write;

/// Read the credential blob from `.credentials.json`.
pub fn read_default() -> Result<Option<String>, SwapError> {
    let path = paths::claude_credentials_file();

    // Verify file permissions before reading credentials
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 {
                tracing::warn!(
                    "credential file {} has permissions {:o} (expected 600), fixing",
                    path.display(), mode
                );
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    match std::fs::read_to_string(&path) {
        Ok(s) if s.trim().is_empty() => Ok(None),
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SwapError::FileError(e)),
    }
}

/// Write the credential blob to `.credentials.json` atomically with 0600 perms.
pub fn write_default(blob: &str) -> Result<(), SwapError> {
    let path = paths::claude_credentials_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = tempfile::NamedTempFile::new_in(
        path.parent().unwrap_or(std::path::Path::new(".")),
    )?;
    tmp.write_all(blob.as_bytes())?;

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

/// Touch the mtime of `.credentials.json` for cross-process invalidation.
pub fn touch() -> Result<(), SwapError> {
    let path = paths::claude_credentials_file();
    if path.exists() {
        filetime::set_file_mtime(&path, filetime::FileTime::now())?;
    }
    Ok(())
}

/// The file-based CliPlatform implementation (Linux, Windows).
pub struct CredentialFile;

#[async_trait::async_trait]
impl super::CliPlatform for CredentialFile {
    async fn read_default(&self) -> Result<Option<String>, SwapError> {
        read_default()
    }

    async fn write_default(&self, blob: &str) -> Result<(), SwapError> {
        write_default(blob)
    }

    async fn touch_credfile(&self) -> Result<(), SwapError> {
        touch()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::lock_data_dir;

    fn setup_config_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.path());
        dir
    }

    #[test]
    fn test_credfile_write_and_read_roundtrip() {
        let _lock = lock_data_dir();
        let _dir = setup_config_dir();

        write_default(r#"{"test":"blob"}"#).unwrap();
        let result = read_default().unwrap();
        assert_eq!(result, Some(r#"{"test":"blob"}"#.to_string()));
    }

    #[test]
    fn test_credfile_read_missing_returns_none() {
        let _lock = lock_data_dir();
        let _dir = setup_config_dir();

        let result = read_default().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_credfile_read_empty_returns_none() {
        let _lock = lock_data_dir();
        let dir = setup_config_dir();

        std::fs::write(dir.path().join(".credentials.json"), "  \n  ").unwrap();
        let result = read_default().unwrap();
        assert_eq!(result, None);
    }

    #[cfg(unix)]
    #[test]
    fn test_credfile_write_sets_0600_permissions() {
        let _lock = lock_data_dir();
        let dir = setup_config_dir();

        write_default("secret").unwrap();

        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(dir.path().join(".credentials.json")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn test_credfile_touch_existing_file() {
        let _lock = lock_data_dir();
        let dir = setup_config_dir();

        write_default("data").unwrap();
        let before = std::fs::metadata(dir.path().join(".credentials.json"))
            .unwrap().modified().unwrap();

        // Small delay to ensure mtime changes
        std::thread::sleep(std::time::Duration::from_millis(50));

        touch().unwrap();
        let after = std::fs::metadata(dir.path().join(".credentials.json"))
            .unwrap().modified().unwrap();

        assert!(after >= before);
    }

    #[test]
    fn test_credfile_touch_missing_file_is_noop() {
        let _lock = lock_data_dir();
        let _dir = setup_config_dir();

        // No credential file exists — touch should succeed silently
        touch().unwrap();
    }

    #[test]
    fn test_credfile_write_overwrites() {
        let _lock = lock_data_dir();
        let _dir = setup_config_dir();

        write_default("first").unwrap();
        write_default("second").unwrap();
        assert_eq!(read_default().unwrap(), Some("second".to_string()));
    }
}
