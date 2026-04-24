//! Tauri commands for the Keys section.
//!
//! Metadata and secrets both live in `~/.claudepot/keys.db` (0o600 on
//! Unix). The OS Keychain is NOT used for this feature — CC's shared
//! `Claude Code-credentials` slot is the only Keychain surface touched
//! by the app (via `cli_backend::keychain`). The plaintext token
//! crosses the Tauri bridge ONLY on deliberate `*_copy` / `*_add` —
//! never on list, probe, or usage fetch.
//!
//! Every handler is `async fn` for the same reason the rest of the
//! command surface is — blocking I/O on Tauri's main thread freezes
//! the webview. See `commands.rs` header for the full rationale.

use crate::commands::open_store;
use crate::dto::{ApiKeySummaryDto, OauthTokenSummaryDto};
use claudepot_core::account::AccountStore;
use claudepot_core::keys::{
    classify_token, token_preview, KeyPrefix, KeyStore, OAUTH_TOKEN_VALIDITY_DAYS,
};
use claudepot_core::paths;
use claudepot_core::services::usage_cache::UsageCache;
use std::collections::HashMap;
use uuid::Uuid;

fn open_keys_store() -> Result<KeyStore, String> {
    let db = paths::claudepot_data_dir().join("keys.db");
    KeyStore::open(&db).map_err(|e| format!("keys store open failed: {e}"))
}

/// Build `uuid → email` from a single `accounts.list()` call so N-row
/// key tables don't fire N SELECTs when rendering the Keys section.
fn account_email_map(store: &AccountStore) -> Result<HashMap<Uuid, String>, String> {
    let accounts = store.list().map_err(|e| format!("list failed: {e}"))?;
    Ok(accounts.into_iter().map(|a| (a.uuid, a.email)).collect())
}

fn parse_account_uuid(s: &str, email_map: &HashMap<Uuid, String>) -> Result<Uuid, String> {
    let id = Uuid::parse_str(s).map_err(|_| format!("invalid account uuid: {s}"))?;
    if !email_map.contains_key(&id) {
        return Err(format!("no registered account with uuid {s}"));
    }
    Ok(id)
}

fn oauth_summary(
    t: claudepot_core::keys::OauthToken,
    email_map: &HashMap<Uuid, String>,
) -> OauthTokenSummaryDto {
    let expires_at = t.created_at + chrono::Duration::days(OAUTH_TOKEN_VALIDITY_DAYS);
    // Ceil the remaining time so a token with 12 hours left reads
    // "1d left" instead of collapsing to 0 and tripping the UI's
    // `<= 0` Expired check. Past-expiry deltas stay negative.
    let secs_remaining = (expires_at - chrono::Utc::now()).num_seconds();
    let days_remaining = if secs_remaining > 0 {
        (secs_remaining + 86_399) / 86_400
    } else {
        secs_remaining / 86_400
    };
    OauthTokenSummaryDto {
        account_email: email_map.get(&t.account_uuid).cloned(),
        uuid: t.uuid.to_string(),
        label: t.label,
        token_preview: t.token_preview,
        account_uuid: t.account_uuid.to_string(),
        created_at: t.created_at,
        expires_at,
        days_remaining,
        last_probed_at: t.last_probed_at,
        last_probe_status: t.last_probe_status,
    }
}

fn api_summary(
    k: claudepot_core::keys::ApiKey,
    email_map: &HashMap<Uuid, String>,
) -> ApiKeySummaryDto {
    ApiKeySummaryDto {
        account_email: email_map.get(&k.account_uuid).cloned(),
        uuid: k.uuid.to_string(),
        label: k.label,
        token_preview: k.token_preview,
        account_uuid: k.account_uuid.to_string(),
        created_at: k.created_at,
        last_probed_at: k.last_probed_at,
        last_probe_status: k.last_probe_status,
    }
}

/// `async fn` keeps the two SQLite opens + the email-map list off
/// Tauri's main thread. Called alongside `key_oauth_list` and
/// `account_list` in the KeysSection mount Promise.all — sync
/// handlers there would serialise on the UI thread and freeze the
/// window. See `session_list_all` / commit 4ad707e for the full
/// rationale.
#[tauri::command]
pub async fn key_api_list() -> Result<Vec<ApiKeySummaryDto>, String> {
    let keys = open_keys_store()?;
    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let rows = keys
        .list_api_keys()
        .map_err(|e| format!("list api keys: {e}"))?;
    Ok(rows.into_iter().map(|k| api_summary(k, &email_map)).collect())
}

/// Sibling to `key_api_list` — same `async fn` rationale applies.
#[tauri::command]
pub async fn key_oauth_list() -> Result<Vec<OauthTokenSummaryDto>, String> {
    let keys = open_keys_store()?;
    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let rows = keys
        .list_oauth_tokens()
        .map_err(|e| format!("list oauth tokens: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|t| oauth_summary(t, &email_map))
        .collect())
}

/// Add an `ANTHROPIC_API_KEY`. `account_uuid` is required — every key
/// was created under some account, and recording that makes the row
/// findable by account later. The "no account" case isn't a real state
/// we need to model.
///
/// `async fn` — the body opens two SQLite stores and writes the
/// encrypted secret into `keys.db`. Keeping it off the main thread is
/// consistent with the rest of the `key_*` family; see `key_api_list`.
#[tauri::command]
pub async fn key_api_add(
    label: String,
    token: String,
    account_uuid: String,
) -> Result<ApiKeySummaryDto, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    let token = token.trim();
    if !matches!(classify_token(token), Some(KeyPrefix::ApiKey)) {
        return Err(
            "not an API key — expected a value starting with `sk-ant-api03-`".to_string(),
        );
    }

    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let account_id = parse_account_uuid(&account_uuid, &email_map)?;

    let preview = token_preview(token);
    let keys = open_keys_store()?;
    let row = keys
        .insert_api_key(label, &preview, account_id, token)
        .map_err(|e| format!("insert: {e}"))?;
    Ok(api_summary(row, &email_map))
}

/// Add a `CLAUDE_CODE_OAUTH_TOKEN`. Account tag is required — the user
/// picks the account they ran `claude setup-token` against.
///
/// `async fn` for the same reason as `key_api_add` / the rest of the
/// `key_*` family.
#[tauri::command]
pub async fn key_oauth_add(
    label: String,
    token: String,
    account_uuid: String,
) -> Result<OauthTokenSummaryDto, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    let token = token.trim();
    if !matches!(classify_token(token), Some(KeyPrefix::OauthToken)) {
        return Err(
            "not an OAuth token — expected a value starting with `sk-ant-oat01-`".to_string(),
        );
    }

    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let account_id = parse_account_uuid(&account_uuid, &email_map)?;

    let preview = token_preview(token);
    let keys = open_keys_store()?;
    let row = keys
        .insert_oauth_token(label, &preview, account_id, token)
        .map_err(|e| format!("insert: {e}"))?;
    Ok(oauth_summary(row, &email_map))
}

/// `async fn` — SQLite delete + encrypted-blob wipe. See `key_api_list`.
#[tauri::command]
pub async fn key_api_remove(uuid: String) -> Result<(), String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let keys = open_keys_store()?;
    keys.remove_api_key(id).map_err(|e| format!("{e}"))
}

/// `async fn` — sibling to `key_api_remove`.
#[tauri::command]
pub async fn key_oauth_remove(uuid: String) -> Result<(), String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let keys = open_keys_store()?;
    keys.remove_oauth_token(id).map_err(|e| format!("{e}"))
}

/// Rename an API key. Label is user-owned metadata — resolution and
/// lookup key off `uuid`, never the label, so renames are a pure
/// display-layer change.
#[tauri::command]
pub async fn key_api_rename(uuid: String, label: String) -> Result<(), String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    let keys = open_keys_store()?;
    keys.rename_api_key(id, label).map_err(|e| format!("{e}"))
}

/// Rename an OAuth token. See `key_api_rename`.
#[tauri::command]
pub async fn key_oauth_rename(uuid: String, label: String) -> Result<(), String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    let keys = open_keys_store()?;
    keys.rename_oauth_token(id, label)
        .map_err(|e| format!("{e}"))
}

/// Return the full API-key secret for clipboard copy. Deliberately
/// distinct from `key_api_list` which only returns the preview.
///
/// `async fn` — SQLite read + decrypt. See `key_api_list`.
#[tauri::command]
pub async fn key_api_copy(uuid: String) -> Result<String, String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let keys = open_keys_store()?;
    keys.find_api_secret(id).map_err(|e| format!("{e}"))
}

/// `async fn` — sibling to `key_api_copy`.
#[tauri::command]
pub async fn key_oauth_copy(uuid: String) -> Result<String, String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let keys = open_keys_store()?;
    keys.find_oauth_secret(id).map_err(|e| format!("{e}"))
}

/// Probe an API key against `GET /v1/models`. `Ok(())` = key accepted,
/// `Err(reason)` = rejected / rate-limited / transport. No DB write —
/// the result is ephemeral so a stale cached status can never lie.
#[tauri::command]
pub async fn key_api_probe(uuid: String) -> Result<(), String> {
    use claudepot_core::error::OAuthError;
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let keys = open_keys_store()?;
    let secret = keys.find_api_secret(id).map_err(|e| format!("{e}"))?;
    match claudepot_core::keys::probe_api_key(&secret).await {
        Ok(()) => Ok(()),
        Err(OAuthError::AuthFailed(_)) => Err("rejected (invalid key)".into()),
        Err(OAuthError::RateLimited { retry_after_secs }) => {
            Err(format!("rate-limited (retry in {retry_after_secs}s)"))
        }
        Err(e) => Err(format!("{e}")),
    }
}

/// Return the cached usage snapshot for the account this OAuth token
/// belongs to. Never hits Anthropic — reads only from the in-memory
/// `UsageCache` already populated by `fetch_all_usage` /
/// `refresh_usage_for` on the Accounts side. `None` means the cache
/// has no entry for this account yet (user hasn't opened Accounts,
/// or cache was invalidated); the UI renders that as an empty state
/// rather than triggering a fetch here.
#[tauri::command]
pub async fn key_oauth_usage_cached(
    uuid: String,
    cache: tauri::State<'_, UsageCache>,
) -> Result<Option<crate::dto::AccountUsageDto>, String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;
    let keys = open_keys_store()?;
    let token = keys
        .find_oauth_token(id)
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("oauth token {id} not found"))?;
    let snapshot = cache.peek_cached(token.account_uuid).await;
    Ok(snapshot.as_ref().map(crate::dto::AccountUsageDto::from_response))
}
