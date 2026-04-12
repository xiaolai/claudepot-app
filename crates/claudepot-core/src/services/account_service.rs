//! Account registration, removal, and inspection — core business logic.
//! CLI and Tauri both call these functions.

use crate::account::{Account, AccountStore};
use crate::blob::CredentialBlob;
use crate::cli_backend;
use crate::cli_backend::swap;
use crate::oauth::{profile, usage};
use crate::paths;
use chrono::Utc;
use uuid::Uuid;

/// Result of registering a new account.
#[derive(Debug)]
pub struct RegisterResult {
    pub uuid: Uuid,
    pub email: String,
    pub org_name: String,
    pub subscription_type: String,
    pub rate_limit_tier: Option<String>,
}

/// Register an account by importing the current CC credentials.
pub async fn register_from_current(store: &AccountStore) -> Result<RegisterResult, RegisterError> {
    let platform = cli_backend::create_platform();
    let blob_str = platform.read_default().await
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?
        .ok_or(RegisterError::NoCredentials)?;

    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?;

    let prof = profile::fetch(&blob.claude_ai_oauth.access_token).await
        .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?;

    if let Some(existing) = store.find_by_email(&prof.email)
        .map_err(|e| RegisterError::Store(e.to_string()))? {
        return Err(RegisterError::AlreadyRegistered(existing.email, existing.uuid));
    }

    let account_id = Uuid::new_v4();
    swap::save_private(account_id, &blob_str)
        .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid.clone()),
        org_name: Some(prof.org_name.clone()),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    };
    store.insert(&account).map_err(|e| RegisterError::Store(e.to_string()))?;

    Ok(RegisterResult {
        uuid: account_id,
        email: prof.email,
        org_name: prof.org_name,
        subscription_type: prof.subscription_type,
        rate_limit_tier: prof.rate_limit_tier,
    })
}

/// Register an account from a refresh token (headless).
pub async fn register_from_token(
    store: &AccountStore,
    refresh_token: &str,
) -> Result<RegisterResult, RegisterError> {
    use crate::oauth::refresh;

    let token_resp = refresh::refresh(refresh_token).await
        .map_err(|e| RegisterError::ProfileFetch(format!("token exchange failed: {e}")))?;

    let prof = profile::fetch(&token_resp.access_token).await
        .map_err(|e| RegisterError::ProfileFetch(e.to_string()))?;

    if let Some(existing) = store.find_by_email(&prof.email)
        .map_err(|e| RegisterError::Store(e.to_string()))? {
        return Err(RegisterError::AlreadyRegistered(existing.email, existing.uuid));
    }

    let account_id = Uuid::new_v4();
    let blob_str = refresh::build_blob(&token_resp);
    swap::save_private(account_id, &blob_str)
        .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid.clone()),
        org_name: Some(prof.org_name.clone()),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    };
    store.insert(&account).map_err(|e| RegisterError::Store(e.to_string()))?;

    Ok(RegisterResult {
        uuid: account_id,
        email: prof.email,
        org_name: prof.org_name,
        subscription_type: prof.subscription_type,
        rate_limit_tier: prof.rate_limit_tier,
    })
}

/// Register an account via browser-based OAuth login.
/// Runs `claude auth login` in a temp config dir, reads credentials,
/// fetches profile, and registers the account.
pub async fn register_from_browser(store: &AccountStore) -> Result<RegisterResult, RegisterError> {
    use crate::onboard;

    let config_dir = onboard::run_auth_login().await
        .map_err(|e| RegisterError::CredentialRead(e.to_string()))?;

    let blob_str = match onboard::read_credentials_from_dir(&config_dir).await {
        Ok(b) => b,
        Err(e) => {
            onboard::cleanup(&config_dir).await;
            return Err(RegisterError::CredentialRead(e.to_string()));
        }
    };

    let blob = CredentialBlob::from_json(&blob_str)
        .map_err(|e| {
            // Fire-and-forget cleanup — don't propagate cleanup errors
            let config_dir = config_dir.clone();
            tokio::spawn(async move { onboard::cleanup(&config_dir).await });
            RegisterError::CredentialRead(e.to_string())
        })?;

    let prof = match profile::fetch(&blob.claude_ai_oauth.access_token).await {
        Ok(p) => p,
        Err(e) => {
            onboard::cleanup(&config_dir).await;
            return Err(RegisterError::ProfileFetch(e.to_string()));
        }
    };

    if let Some(existing) = store.find_by_email(&prof.email)
        .map_err(|e| RegisterError::Store(e.to_string()))? {
        onboard::cleanup(&config_dir).await;
        return Err(RegisterError::AlreadyRegistered(existing.email, existing.uuid));
    }

    let account_id = Uuid::new_v4();
    swap::save_private(account_id, &blob_str)
        .map_err(|e| RegisterError::CredentialWrite(e.to_string()))?;

    let account = Account {
        uuid: account_id,
        email: prof.email.clone(),
        org_uuid: Some(prof.org_uuid.clone()),
        org_name: Some(prof.org_name.clone()),
        subscription_type: Some(prof.subscription_type.clone()),
        rate_limit_tier: prof.rate_limit_tier.clone(),
        created_at: Utc::now(),
        last_cli_switch: None,
        last_desktop_switch: None,
        has_cli_credentials: true,
        has_desktop_profile: false,
        is_cli_active: false,
        is_desktop_active: false,
    };
    store.insert(&account).map_err(|e| RegisterError::Store(e.to_string()))?;
    onboard::cleanup(&config_dir).await;

    Ok(RegisterResult {
        uuid: account_id,
        email: prof.email,
        org_name: prof.org_name,
        subscription_type: prof.subscription_type,
        rate_limit_tier: prof.rate_limit_tier,
    })
}

/// Remove an account and all its associated data.
pub fn remove_account(store: &AccountStore, uuid: Uuid) -> Result<RemoveResult, RegisterError> {
    let account = store.find_by_uuid(uuid)
        .map_err(|e| RegisterError::Store(e.to_string()))?
        .ok_or(RegisterError::NotFound)?;

    // Delete credential
    let _ = swap::delete_private(uuid);

    // Delete Desktop profile
    let profile_dir = paths::desktop_profile_dir(uuid);
    let had_profile = profile_dir.exists();
    if had_profile {
        let _ = std::fs::remove_dir_all(&profile_dir);
    }

    // Remove from store
    store.remove(uuid).map_err(|e| RegisterError::Store(e.to_string()))?;

    // Clear active pointers if needed
    if account.is_cli_active {
        let _ = store.clear_active_cli();
    }
    if account.is_desktop_active {
        let _ = store.clear_active_desktop();
    }

    Ok(RemoveResult {
        email: account.email,
        was_cli_active: account.is_cli_active,
        was_desktop_active: account.is_desktop_active,
        had_desktop_profile: had_profile,
    })
}

#[derive(Debug)]
pub struct RemoveResult {
    pub email: String,
    pub was_cli_active: bool,
    pub was_desktop_active: bool,
    pub had_desktop_profile: bool,
}

/// Token health info for an account.
#[derive(Debug)]
pub struct TokenHealth {
    pub status: String,
    pub remaining_mins: Option<i64>,
}

/// Get token health for an account.
pub fn token_health(uuid: Uuid, has_credentials: bool) -> TokenHealth {
    if !has_credentials {
        return TokenHealth { status: "no credentials".into(), remaining_mins: None };
    }
    match swap::load_private(uuid) {
        Ok(blob_str) => match CredentialBlob::from_json(&blob_str) {
            Ok(blob) => {
                let remaining = (blob.claude_ai_oauth.expires_at
                    - Utc::now().timestamp_millis()) / 60_000;
                if remaining > 0 {
                    TokenHealth {
                        status: format!("valid ({}m remaining)", remaining),
                        remaining_mins: Some(remaining),
                    }
                } else {
                    TokenHealth { status: "expired".into(), remaining_mins: Some(remaining) }
                }
            }
            Err(_) => TokenHealth { status: "corrupt blob".into(), remaining_mins: None },
        },
        Err(_) => TokenHealth { status: "missing".into(), remaining_mins: None },
    }
}

/// Fetch live usage for an account (returns None if token expired or missing).
pub async fn fetch_usage(uuid: Uuid) -> Option<usage::UsageResponse> {
    let blob_str = swap::load_private(uuid).ok()?;
    let blob = CredentialBlob::from_json(&blob_str).ok()?;
    if blob.is_expired(0) {
        return None;
    }
    usage::fetch(&blob.claude_ai_oauth.access_token).await.ok()
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("no CC credentials found — log in with `claude auth login` first")]
    NoCredentials,
    #[error("failed to read credentials: {0}")]
    CredentialRead(String),
    #[error("failed to write credentials: {0}")]
    CredentialWrite(String),
    #[error("profile fetch failed: {0}")]
    ProfileFetch(String),
    #[error("already registered: {0} (uuid: {1})")]
    AlreadyRegistered(String, Uuid),
    #[error("store error: {0}")]
    Store(String),
    #[error("account not found")]
    NotFound,
}
