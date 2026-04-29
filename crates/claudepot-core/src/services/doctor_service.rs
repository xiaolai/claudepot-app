//! Doctor health checks — core business logic.

use crate::account::AccountStore;
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
    /// None = not macOS, Some(Ok(true)) = credential found,
    /// Some(Ok(false)) = no credential, Some(Err) = access error
    pub keychain_status: Option<Result<bool, String>>,
    pub beta_header: String,
    pub api_status: ApiStatus,
    pub account_health: Vec<AccountHealth>,
    pub desktop_profiles: Vec<ProfileInfo>,
    pub db_error: Option<String>,
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
    /// Last persisted verify_status on the account row: "never", "ok",
    /// "drift", "rejected", "network_error". Populated from the DB — if
    /// it says "drift", the doctor reports an error and exits non-zero.
    /// Run `claudepot account verify` to refresh before trusting this
    /// field as current.
    pub verify_status: String,
    pub verified_email: Option<String>,
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
    let keychain_status = check_keychain().await;

    // Beta header
    let beta_header = crate::oauth::beta_header::get_or_default().to_string();

    // API reachability
    let api_status = check_api(&beta_header).await;

    // Account health
    let (accounts, db_error) = match store.list() {
        Ok(a) => (a, None),
        Err(e) => (vec![], Some(format!("failed to list accounts: {e}"))),
    };
    let account_health = build_account_health(&accounts);
    let desktop_profiles = build_profile_info(&accounts);

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
        keychain_status,
        beta_header,
        api_status,
        account_health,
        desktop_profiles,
        db_error,
    }
}

fn detect_cli() -> (Option<PathBuf>, Option<String>) {
    match crate::fs_utils::find_claude_binary() {
        Some(path) => {
            let version = crate::fs_utils::claude_version(&path);
            (Some(path), version)
        }
        None => (None, None),
    }
}

fn detect_desktop() -> (bool, Option<String>) {
    #[cfg(target_os = "macos")]
    {
        let path = std::path::Path::new("/Applications/Claude.app");
        if path.exists() {
            let version = std::process::Command::new("defaults")
                .args([
                    "read",
                    "/Applications/Claude.app/Contents/Info.plist",
                    "CFBundleShortVersionString",
                ])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string());
            return (true, version);
        }
        (false, None)
    }
    #[cfg(target_os = "windows")]
    {
        // Check MSIX package via data dir existence
        let data_dir = crate::paths::claude_desktop_data_dir();
        if let Some(ref dir) = data_dir {
            if dir.exists() {
                // Get version from powershell
                let version = std::process::Command::new("powershell")
                    .args(["-Command", "(Get-AppxPackage Claude).Version"])
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                return (true, version);
            }
        }
        (false, None)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        (false, None)
    }
}

async fn check_keychain() -> Option<Result<bool, String>> {
    #[cfg(target_os = "macos")]
    {
        match crate::cli_backend::keychain::read_default().await {
            Ok(Some(_)) => Some(Ok(true)),
            Ok(None) => Some(Ok(false)),
            Err(e) => Some(Err(format!("keychain access failed: {e}"))),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Build account health vector from a list of accounts (testable extraction).
fn build_account_health(accounts: &[crate::account::Account]) -> Vec<AccountHealth> {
    accounts
        .iter()
        .map(|a| {
            let health =
                crate::services::account_service::token_health(a.uuid, a.has_cli_credentials);
            AccountHealth {
                email: a.email.clone(),
                token_status: health.status,
                remaining_mins: health.remaining_mins,
                verify_status: a.verify_status.clone(),
                verified_email: a.verified_email.clone(),
            }
        })
        .collect()
}

/// Build desktop profile info from a list of accounts (testable extraction).
fn build_profile_info(accounts: &[crate::account::Account]) -> Vec<ProfileInfo> {
    accounts
        .iter()
        .map(|a| {
            let p = crate::paths::desktop_profile_dir(a.uuid);
            ProfileInfo {
                email: a.email.clone(),
                item_count: if p.exists() {
                    std::fs::read_dir(&p).map(|d| d.count()).ok()
                } else {
                    None
                },
            }
        })
        .collect()
}

async fn check_api(beta_header: &str) -> ApiStatus {
    match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Err(_) => ApiStatus::Unreachable("failed to build HTTP client".into()),
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
                    match status {
                        401 => ApiStatus::Reachable, // expected for invalid token probe
                        403 => ApiStatus::GeoBlocked,
                        429 => ApiStatus::Unreachable("rate limited".into()),
                        s if s >= 500 => ApiStatus::Unreachable(format!("server error {s}")),
                        _ => ApiStatus::Reachable,
                    }
                }
                Err(e) => ApiStatus::Unreachable(e.to_string()),
            }
        }
    }
}

// See launcher.rs for the rationale: tests hold `lock_data_dir()`
// across `.await` deliberately to serialize the shared data-dir
// env-var across the test binary.
#[cfg(test)]
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use crate::cli_backend::swap;
    use crate::testing::{
        expired_blob_json, fresh_blob_json, lock_data_dir, make_account, setup_test_data_dir,
        test_store,
    };

    #[test]
    fn test_build_account_health_empty() {
        let health = build_account_health(&[]);
        assert!(health.is_empty());
    }

    #[test]
    fn test_build_account_health_with_credentials() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();

        let account = make_account("health@example.com");
        let id = account.uuid;
        swap::save_private(id, &fresh_blob_json()).unwrap();

        let health = build_account_health(&[account]);
        assert_eq!(health.len(), 1);
        assert_eq!(health[0].email, "health@example.com");
        assert!(health[0].token_status.contains("valid"));
        assert!(health[0].remaining_mins.unwrap() > 0);

        swap::delete_private(id).unwrap();
    }

    #[test]
    fn test_build_account_health_expired() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();

        let account = make_account("expired@example.com");
        let id = account.uuid;
        swap::save_private(id, &expired_blob_json()).unwrap();

        let health = build_account_health(&[account]);
        assert_eq!(health[0].token_status, "expired");

        swap::delete_private(id).unwrap();
    }

    #[test]
    fn test_build_account_health_no_credentials() {
        let account = {
            let mut a = make_account("nocred@example.com");
            a.has_cli_credentials = false;
            a
        };
        let health = build_account_health(&[account]);
        assert_eq!(health[0].token_status, "no credentials");
    }

    #[test]
    fn test_build_profile_info_no_profile() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();

        let account = make_account("noprofile@example.com");
        let info = build_profile_info(&[account]);
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].email, "noprofile@example.com");
        assert!(info[0].item_count.is_none());
    }

    #[test]
    fn test_build_profile_info_with_profile() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();

        let account = make_account("profile@example.com");
        let profile_dir = crate::paths::desktop_profile_dir(account.uuid);
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(profile_dir.join("config.json"), "{}").unwrap();
        std::fs::write(profile_dir.join("Cookies"), "cookies").unwrap();

        let info = build_profile_info(&[account]);
        assert_eq!(info[0].item_count, Some(2));
    }

    #[tokio::test]
    async fn test_check_health_with_empty_store() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let report = check_health(&store).await;
        assert_eq!(report.account_count, 0);
        assert!(report.account_health.is_empty());
        assert!(report.desktop_profiles.is_empty());
        assert!(!report.platform.is_empty());
        assert!(!report.arch.is_empty());
    }

    #[tokio::test]
    async fn test_check_health_with_accounts() {
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        let account = make_account("doctor@example.com");
        store.insert(&account).unwrap();

        let report = check_health(&store).await;
        assert_eq!(report.account_count, 1);
        assert_eq!(report.account_health.len(), 1);
        assert_eq!(report.account_health[0].email, "doctor@example.com");
    }

    // -- Group 8: doctor accuracy --

    #[test]
    fn test_doctor_expired_accounts_counted_as_warnings() {
        // Build account_health for a store with two accounts (1 fresh, 1 expired).
        // The expired one must have status == "expired"; the fresh one "valid (...)".
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();

        let fresh = make_account("fresh@example.com");
        swap::save_private(fresh.uuid, &fresh_blob_json()).unwrap();
        let expired = make_account("expired@example.com");
        swap::save_private(expired.uuid, &expired_blob_json()).unwrap();

        let health = build_account_health(&[fresh.clone(), expired.clone()]);
        assert_eq!(health.len(), 2);

        let fresh_entry = health
            .iter()
            .find(|h| h.email == "fresh@example.com")
            .unwrap();
        assert!(
            fresh_entry.token_status.contains("valid"),
            "fresh account shows as valid"
        );
        let expired_entry = health
            .iter()
            .find(|h| h.email == "expired@example.com")
            .unwrap();
        assert_eq!(
            expired_entry.token_status, "expired",
            "expired account surfaces as 'expired' (counted as warning in summary)"
        );

        swap::delete_private(fresh.uuid).unwrap();
        swap::delete_private(expired.uuid).unwrap();
    }

    #[tokio::test]
    async fn test_doctor_db_error_surfaces() {
        // Corrupt the store (drop accounts table), then run check_health.
        // Expected: report.db_error is Some(...) and account_count is 0.
        let _lock = lock_data_dir();
        let _env = setup_test_data_dir();
        let (store, _db) = test_store();

        store.corrupt_for_test();

        let report = check_health(&store).await;
        assert!(
            report.db_error.is_some(),
            "db_error should surface when store.list() fails"
        );
        assert_eq!(report.account_count, 0, "no accounts listed on failure");
        assert!(report.account_health.is_empty());
    }
}
