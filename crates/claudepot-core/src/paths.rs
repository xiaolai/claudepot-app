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

/// Claudepot's own private data root.
pub fn claudepot_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".local/share")
        })
        .join("Claudepot")
}

/// Per-account Desktop profile snapshot directory.
pub fn desktop_profile_dir(account_id: uuid::Uuid) -> PathBuf {
    claudepot_data_dir()
        .join("desktop")
        .join(account_id.to_string())
}
