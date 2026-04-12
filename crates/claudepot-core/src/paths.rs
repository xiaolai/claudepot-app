use std::path::PathBuf;

/// CC CLI config directory. Honors `$CLAUDE_CONFIG_DIR`, defaults to `~/.claude`.
pub fn claude_config_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".claude")
        })
}

/// CC CLI credentials file path.
pub fn claude_credentials_file() -> PathBuf {
    claude_config_dir().join(".credentials.json")
}

/// Claude Desktop data directory (macOS / Windows). Returns None on Linux.
pub fn claude_desktop_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::data_dir().map(|d| d.join("Claude"))
    }
    #[cfg(target_os = "windows")]
    {
        dirs::data_local_dir().map(|d| {
            d.join("Packages")
                .join("Claude_pzs8sxrjxfjjc")
                .join("LocalCache")
                .join("Roaming")
                .join("Claude")
        })
    }
    #[cfg(target_os = "linux")]
    {
        None
    }
}

/// Claudepot's own private data root. Honors `$CLAUDEPOT_DATA_DIR`.
pub fn claudepot_data_dir() -> PathBuf {
    std::env::var_os("CLAUDEPOT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .unwrap_or_else(|| PathBuf::from("/tmp"))
                        .join(".local/share")
                })
                .join("Claudepot")
        })
}

/// Per-account Desktop profile snapshot directory.
pub fn desktop_profile_dir(account_id: uuid::Uuid) -> PathBuf {
    claudepot_data_dir()
        .join("desktop")
        .join(account_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::lock_data_dir;

    #[test]
    fn test_claude_config_dir_honors_env() {
        let _lock = lock_data_dir();
        std::env::set_var("CLAUDE_CONFIG_DIR", "/custom/config");
        let result = claude_config_dir();
        assert_eq!(result, PathBuf::from("/custom/config"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn test_claude_config_dir_default_fallback() {
        let _lock = lock_data_dir();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let result = claude_config_dir();
        // Should end with .claude (either ~/.claude or /tmp/.claude)
        assert!(result.ends_with(".claude"), "got: {}", result.display());
    }

    #[test]
    fn test_claude_credentials_file_is_under_config() {
        let _lock = lock_data_dir();
        std::env::set_var("CLAUDE_CONFIG_DIR", "/test/config");
        let result = claude_credentials_file();
        assert_eq!(result, PathBuf::from("/test/config/.credentials.json"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn test_claudepot_data_dir_honors_env() {
        let _lock = lock_data_dir();
        std::env::set_var("CLAUDEPOT_DATA_DIR", "/custom/data");
        let result = claudepot_data_dir();
        assert_eq!(result, PathBuf::from("/custom/data"));
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn test_claudepot_data_dir_default_contains_claudepot() {
        let _lock = lock_data_dir();
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
        let result = claudepot_data_dir();
        assert!(
            result.to_string_lossy().contains("Claudepot"),
            "got: {}",
            result.display()
        );
    }

    #[test]
    fn test_desktop_profile_dir_includes_uuid() {
        let _lock = lock_data_dir();
        std::env::set_var("CLAUDEPOT_DATA_DIR", "/data");
        let id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let result = desktop_profile_dir(id);
        assert_eq!(
            result,
            PathBuf::from("/data/desktop/550e8400-e29b-41d4-a716-446655440000")
        );
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }
}
