//! Test utilities — only compiled when `#[cfg(test)]`.
//!
//! Provides a global mutex for tests that modify `CLAUDEPOT_DATA_DIR`.

use std::sync::Mutex;

/// Global lock for tests that modify the `CLAUDEPOT_DATA_DIR` env var.
/// All tests across all modules that call `setup_test_data_dir()` share
/// this lock to prevent env var races.
///
/// Use `lock_data_dir()` instead of `DATA_DIR_LOCK.lock()` directly
/// to handle mutex poisoning from earlier test panics.
pub static DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the DATA_DIR_LOCK, recovering from poison if a prior test panicked.
pub fn lock_data_dir() -> std::sync::MutexGuard<'static, ()> {
    DATA_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Set `CLAUDEPOT_DATA_DIR` to a fresh temp dir and return it.
/// Caller MUST hold `DATA_DIR_LOCK` for the duration of the test.
pub fn setup_test_data_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());
    dir
}

/// Create a test AccountStore backed by a temp SQLite DB.
pub fn test_store() -> (crate::account::AccountStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let store = crate::account::AccountStore::open(&db).unwrap();
    (store, dir)
}

/// Create a test Account with sensible defaults.
pub fn make_account(email: &str) -> crate::account::Account {
    crate::account::Account {
        uuid: uuid::Uuid::new_v4(),
        email: email.to_string(),
        org_uuid: Some("org-test".to_string()),
        org_name: Some("Test Org".to_string()),
        subscription_type: Some("pro".to_string()),
        rate_limit_tier: None,
        created_at: chrono::Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    }
}
