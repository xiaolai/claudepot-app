//! Frontend DTOs — what crosses the Tauri command boundary.
//!
//! We deliberately do NOT expose credential blobs, access tokens, or refresh
//! tokens to the webview. Only non-sensitive metadata leaves Rust.

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Serialize)]
pub struct AccountSummary {
    pub uuid: String,
    pub email: String,
    pub org_name: Option<String>,
    pub subscription_type: Option<String>,
    pub is_cli_active: bool,
    pub is_desktop_active: bool,
    pub has_cli_credentials: bool,
    pub has_desktop_profile: bool,
    pub last_cli_switch: Option<DateTime<Utc>>,
    pub last_desktop_switch: Option<DateTime<Utc>>,
    /// "valid", "expired", "no credentials", "missing", "corrupt blob"
    pub token_status: String,
    pub token_remaining_mins: Option<i64>,
    /// True iff the stored blob actually exists and parses. Mirrors reality,
    /// not the DB flag. Used by the UI to gate the "Use CLI" button — the
    /// DB's has_cli_credentials can lie after external state changes.
    pub credentials_healthy: bool,
}

impl From<&claudepot_core::account::Account> for AccountSummary {
    fn from(a: &claudepot_core::account::Account) -> Self {
        let health =
            claudepot_core::services::account_service::token_health(a.uuid, a.has_cli_credentials);
        // A stored blob is "healthy" if it exists and parses. Any other
        // status ("missing", "corrupt blob", "no credentials") means the
        // swap can't succeed — the UI should gate on this, not the DB flag.
        let credentials_healthy = health.status.starts_with("valid") || health.status == "expired";
        Self {
            uuid: a.uuid.to_string(),
            email: a.email.clone(),
            org_name: a.org_name.clone(),
            subscription_type: a.subscription_type.clone(),
            is_cli_active: a.is_cli_active,
            is_desktop_active: a.is_desktop_active,
            has_cli_credentials: a.has_cli_credentials,
            has_desktop_profile: a.has_desktop_profile,
            last_cli_switch: a.last_cli_switch,
            last_desktop_switch: a.last_desktop_switch,
            token_status: health.status,
            token_remaining_mins: health.remaining_mins,
            credentials_healthy,
        }
    }
}

#[derive(Serialize)]
pub struct AppStatus {
    pub platform: String,
    pub arch: String,
    pub cli_active_email: Option<String>,
    pub desktop_active_email: Option<String>,
    pub desktop_installed: bool,
    pub data_dir: String,
    pub account_count: usize,
}

#[derive(Serialize)]
pub struct RegisterOutcome {
    pub email: String,
    pub org_name: String,
    pub subscription_type: String,
}

#[derive(Serialize)]
pub struct RemoveOutcome {
    pub email: String,
    pub was_cli_active: bool,
    pub was_desktop_active: bool,
    pub had_desktop_profile: bool,
    pub warnings: Vec<String>,
}
