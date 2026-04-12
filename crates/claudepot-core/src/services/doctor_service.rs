//! Doctor health checks — core business logic.

use crate::account::AccountStore;
use crate::blob::CredentialBlob;
use crate::cli_backend::swap::load_private;
use std::path::PathBuf;

#[derive(Debug)]
pub struct HealthReport {
    pub platform: String,
    pub arch: String,
    pub data_dir: PathBuf,
    pub data_dir_exists: bool,
    pub account_count: usize,
    pub cli_path: Option<PathBuf>,
    pub cli_version: Option<String>,
    pub desktop_installed: bool,
    pub desktop_version: Option<String>,
    pub keychain_readable: Option<bool>,
    pub beta_header: String,
    pub api_status: ApiStatus,
    pub account_health: Vec<AccountHealth>,
    pub desktop_profiles: Vec<ProfileInfo>,
}

#[derive(Debug)]
pub enum ApiStatus {
    Reachable,
    GeoBlocked,
    Unreachable(String),
    Unknown,
}

#[derive(Debug)]
pub struct AccountHealth {
    pub email: String,
    pub token_status: String,
    pub remaining_mins: Option<i64>,
}

#[derive(Debug)]
pub struct ProfileInfo {
    pub email: String,
    pub item_count: Option<usize>,
}

/// Run all health checks and return a structured report.
pub async fn check_health(store: &AccountStore) -> HealthReport {
    let data_dir = crate::paths::claudepot_data_dir();

    // CLI detection
    let (cli_path, cli_version) = detect_cli();

    // Desktop detection
    let (desktop_installed, desktop_version) = detect_desktop();

    // Keychain
    let keychain_readable = check_keychain().await;

    // Beta header
    let beta_header = crate::oauth::beta_header::get_or_default().to_string();

    // API reachability
    let api_status = check_api(&beta_header).await;

    // Account health
    let accounts = store.list().unwrap_or_default();
    let account_health: Vec<AccountHealth> = accounts.iter().map(|a| {
        let health = crate::services::account_service::token_health(a.uuid, a.has_cli_credentials);
        AccountHealth {
            email: a.email.clone(),
            token_status: health.status,
            remaining_mins: health.remaining_mins,
        }
    }).collect();

    // Desktop profiles
    let profile_base = data_dir.join("desktop");
    let desktop_profiles: Vec<ProfileInfo> = accounts.iter().map(|a| {
        let p = crate::paths::desktop_profile_dir(a.uuid);
        ProfileInfo {
            email: a.email.clone(),
            item_count: if p.exists() {
                std::fs::read_dir(&p).map(|d| d.count()).ok()
            } else {
                None
            },
        }
    }).collect();

    HealthReport {
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        data_dir_exists: data_dir.exists(),
        data_dir,
        account_count: accounts.len(),
        cli_path,
        cli_version,
        desktop_installed,
        desktop_version,
        keychain_readable,
        beta_header,
        api_status,
        account_health,
        desktop_profiles,
    }
}

fn detect_cli() -> (Option<PathBuf>, Option<String>) {
    let candidates = [
        dirs::home_dir().map(|h| h.join(".local/bin/claude")),
        Some(PathBuf::from("/usr/local/bin/claude")),
        Some(PathBuf::from("/usr/bin/claude")),
    ];
    for path in candidates.iter().flatten() {
        if path.exists() {
            let version = std::process::Command::new(path)
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string());
            return (Some(path.clone()), version);
        }
    }
    (None, None)
}

fn detect_desktop() -> (bool, Option<String>) {
    #[cfg(target_os = "macos")]
    {
        let path = std::path::Path::new("/Applications/Claude.app");
        if path.exists() {
            let version = std::process::Command::new("defaults")
                .args(["read", "/Applications/Claude.app/Contents/Info.plist", "CFBundleShortVersionString"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string());
            return (true, version);
        }
        (false, None)
    }
    #[cfg(not(target_os = "macos"))]
    {
        (false, None)
    }
}

async fn check_keychain() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        match crate::cli_backend::keychain::read_default().await {
            Ok(Some(_)) => Some(true),
            Ok(None) => Some(false),
            Err(_) => Some(false),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

async fn check_api(beta_header: &str) -> ApiStatus {
    match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Err(_) => return ApiStatus::Unreachable("failed to build HTTP client".into()),
        Ok(client) => {
            match client
                .get("https://api.anthropic.com/api/oauth/profile")
                .header("Authorization", "Bearer test")
                .header("anthropic-beta", beta_header)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 401 { ApiStatus::Reachable }
                    else if status == 403 { ApiStatus::GeoBlocked }
                    else { ApiStatus::Reachable }
                }
                Err(e) => ApiStatus::Unreachable(e.to_string()),
            }
        }
    }
}
