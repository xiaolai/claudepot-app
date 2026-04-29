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
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;
use uuid::Uuid;
use zeroize::Zeroize;

/// 30-second self-clear deadline for clipboard payloads written by
/// `key_*_copy*`. Mirrors the legacy JS `CLIPBOARD_CLEAR_MS` so the UX
/// timing didn't change when the policy moved Rust-side.
const CLIPBOARD_CLEAR_MS: u64 = 30_000;

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
    // `<= 0` Expired check. Past-expiry deltas must stay strictly
    // negative — Rust's `/` truncates toward zero, so a token expired
    // by a few hours would otherwise round to 0 and read "expires
    // today" instead of "expired" (audit B8 commands_keys.rs:54).
    // `div_euclid` gives floor division for negative numerators with
    // a positive denominator, which is exactly what we want here.
    let secs_remaining = (expires_at - chrono::Utc::now()).num_seconds();
    let days_remaining = if secs_remaining > 0 {
        (secs_remaining + 86_399) / 86_400
    } else {
        secs_remaining.div_euclid(86_400)
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

/// Wrapped in `tokio::task::spawn_blocking` so the two SQLite opens
/// + the email-map list don't block a Tauri IPC worker (audit B8
/// commands_keys.rs:96). KeysSection mounts this alongside
/// `key_oauth_list` and `account_list` via `Promise.all`, so a
/// blocking call here would serialize the whole mount.
#[tauri::command]
pub async fn key_api_list() -> Result<Vec<ApiKeySummaryDto>, String> {
    tokio::task::spawn_blocking(|| {
        let keys = open_keys_store()?;
        let accounts = open_store()?;
        let email_map = account_email_map(&accounts)?;
        let rows = keys
            .list_api_keys()
            .map_err(|e| format!("list api keys: {e}"))?;
        Ok::<_, String>(
            rows.into_iter()
                .map(|k| api_summary(k, &email_map))
                .collect(),
        )
    })
    .await
    .map_err(|e| format!("blocking task failed: {e}"))?
}

/// Sibling to `key_api_list` — same `spawn_blocking` rationale.
#[tauri::command]
pub async fn key_oauth_list() -> Result<Vec<OauthTokenSummaryDto>, String> {
    tokio::task::spawn_blocking(|| {
        let keys = open_keys_store()?;
        let accounts = open_store()?;
        let email_map = account_email_map(&accounts)?;
        let rows = keys
            .list_oauth_tokens()
            .map_err(|e| format!("list oauth tokens: {e}"))?;
        Ok::<_, String>(
            rows.into_iter()
                .map(|t| oauth_summary(t, &email_map))
                .collect(),
        )
    })
    .await
    .map_err(|e| format!("blocking task failed: {e}"))?
}

/// Add an `ANTHROPIC_API_KEY`. `account_uuid` is required — every key
/// was created under some account, and recording that makes the row
/// findable by account later. The "no account" case isn't a real state
/// we need to model.
///
/// `async fn` — the body opens two SQLite stores and writes the
/// encrypted secret into `keys.db`. Keeping it off the main thread is
/// consistent with the rest of the `key_*` family; see `key_api_list`.
///
/// Secret handling (D-5/6/7): the `token` `String` arrives via the
/// IPC bridge, is trimmed into a fresh owned `String`, and is
/// **zeroized** on every exit path — success and error alike. The
/// originally-deserialized `token` argument also has its bytes
/// scrubbed before drop. `KeyError` Display impl never interpolates
/// the secret (audit `keys/error.rs`), so error paths returning
/// `format!("insert: {e}")` cannot leak it.
#[tauri::command]
pub async fn key_api_add(
    label: String,
    mut token: String,
    account_uuid: String,
) -> Result<ApiKeySummaryDto, String> {
    let result = key_api_add_inner(&label, token.trim(), &account_uuid).await;
    // Scrub the IPC-bridge `String` regardless of outcome. The trimmed
    // slice above is a borrow of these bytes — once `inner` returned,
    // its `String` shadow has been dropped (and zeroized) and the
    // backing buffer is what we must wipe here.
    token.zeroize();
    result
}

async fn key_api_add_inner(
    label: &str,
    token: &str,
    account_uuid: &str,
) -> Result<ApiKeySummaryDto, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    if !matches!(classify_token(token), Some(KeyPrefix::ApiKey)) {
        return Err("not an API key — expected a value starting with `sk-ant-api03-`".to_string());
    }

    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let account_id = parse_account_uuid(account_uuid, &email_map)?;

    // Local owned copy so we can zeroize after persistence — `token`
    // is a borrow into the IPC arg buffer; `key_api_add` (the public
    // wrapper) is responsible for scrubbing that buffer on return.
    let mut secret_buf = token.to_string();
    let preview = token_preview(&secret_buf);
    let keys = open_keys_store()?;
    let outcome = keys
        .insert_api_key(label, &preview, account_id, &secret_buf)
        .map(|row| api_summary(row, &email_map))
        .map_err(|e| format!("insert: {e}"));
    secret_buf.zeroize();
    outcome
}

/// Add a `CLAUDE_CODE_OAUTH_TOKEN`. Account tag is required — the user
/// picks the account they ran `claude setup-token` against.
///
/// `async fn` for the same reason as `key_api_add` / the rest of the
/// `key_*` family. See `key_api_add` for the zeroization contract.
#[tauri::command]
pub async fn key_oauth_add(
    label: String,
    mut token: String,
    account_uuid: String,
) -> Result<OauthTokenSummaryDto, String> {
    let result = key_oauth_add_inner(&label, token.trim(), &account_uuid).await;
    token.zeroize();
    result
}

async fn key_oauth_add_inner(
    label: &str,
    token: &str,
    account_uuid: &str,
) -> Result<OauthTokenSummaryDto, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    if !matches!(classify_token(token), Some(KeyPrefix::OauthToken)) {
        return Err(
            "not an OAuth token — expected a value starting with `sk-ant-oat01-`".to_string(),
        );
    }

    let accounts = open_store()?;
    let email_map = account_email_map(&accounts)?;
    let account_id = parse_account_uuid(account_uuid, &email_map)?;

    let mut secret_buf = token.to_string();
    let preview = token_preview(&secret_buf);
    let keys = open_keys_store()?;
    let outcome = keys
        .insert_oauth_token(label, &preview, account_id, &secret_buf)
        .map(|row| oauth_summary(row, &email_map))
        .map_err(|e| format!("insert: {e}"));
    secret_buf.zeroize();
    outcome
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

/// Receipt returned to the renderer after a successful `key_*_copy*`
/// call. The raw secret never crosses the IPC boundary on this path —
/// only the label + preview the UI already had on hand, plus the
/// timestamp at which Rust will self-clear the clipboard.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KeyCopyReceiptDto {
    pub label: String,
    pub preview: String,
    pub clipboard_clears_at_unix_ms: u64,
}

/// What kind of secret a `key_*_copy*` call should fetch from the
/// store. Drives both DB lookup and the receipt's `kind`-agnostic
/// payload formatting.
#[derive(Clone, Copy)]
enum CopyKind {
    Api,
    Oauth,
    OauthShell,
}

/// How to render the secret bytes that get written to the clipboard.
/// `Raw` is the historical behavior; `Shell` wraps in a paste-ready
/// `CLAUDE_CODE_OAUTH_TOKEN='…' claude` invocation so the user can
/// open a new terminal and switch identities without disturbing their
/// current login (matches CC's env-var precedence in `auth.ts:168`).
fn format_payload(kind: CopyKind, secret: &str) -> String {
    match kind {
        CopyKind::Api | CopyKind::Oauth => secret.to_string(),
        CopyKind::OauthShell => format!("CLAUDE_CODE_OAUTH_TOKEN='{secret}' claude"),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Schedule a 30s clipboard self-clear, gated on the clipboard still
/// holding the exact payload we wrote. If `read_text` returns
/// something else (or fails — permissions, etc.), abort silently:
/// it's better to leave whatever the user copied in between than
/// blind-clobber it.
///
/// The `payload` is moved into the task; the caller must zeroize its
/// own local copy before / after returning to the IPC bridge.
fn schedule_self_clear(app: AppHandle, mut payload: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(CLIPBOARD_CLEAR_MS)).await;
        let still_ours = match app.clipboard().read_text() {
            Ok(s) => s == payload,
            Err(_) => false,
        };
        // Zeroize our copy of the payload regardless of the readback
        // outcome — there is no further legitimate use of these bytes.
        payload.zeroize();
        if still_ours {
            // `clear` writes an empty string; the JS prior art picked
            // the same shape ("better than nothing"). Errors are
            // ignorable: if the user has the screen-recording
            // permission scrub seam open, there's nothing we can do.
            let _ = app.clipboard().clear();
        }
    });
}

/// Shared body of the three `key_*_copy*` commands. Loads the secret
/// inside `spawn_blocking` (SQLite read + decrypt), formats the
/// payload, writes it to the OS clipboard, schedules the 30s
/// self-clear, and returns a `KeyCopyReceiptDto` — the secret bytes
/// never reach the renderer.
async fn key_copy_inner(
    uuid: String,
    kind: CopyKind,
    app: AppHandle,
) -> Result<KeyCopyReceiptDto, String> {
    let id = Uuid::parse_str(&uuid).map_err(|e| format!("bad uuid: {e}"))?;

    // SQLite + decrypt off the IPC worker. Returns the secret + the
    // metadata we need for the receipt — a single round-trip so the
    // store doesn't have to be re-opened on the IPC thread.
    let (mut secret, label, preview) =
        tokio::task::spawn_blocking(move || -> Result<(String, String, String), String> {
            let keys = open_keys_store()?;
            match kind {
                CopyKind::Api => {
                    let row = keys
                        .find_api_key(id)
                        .map_err(|e| format!("{e}"))?
                        .ok_or_else(|| format!("api key {id} not found"))?;
                    let secret = keys.find_api_secret(id).map_err(|e| format!("{e}"))?;
                    Ok((secret, row.label, row.token_preview))
                }
                CopyKind::Oauth | CopyKind::OauthShell => {
                    let row = keys
                        .find_oauth_token(id)
                        .map_err(|e| format!("{e}"))?
                        .ok_or_else(|| format!("oauth token {id} not found"))?;
                    let secret = keys.find_oauth_secret(id).map_err(|e| format!("{e}"))?;
                    Ok((secret, row.label, row.token_preview))
                }
            }
        })
        .await
        .map_err(|e| format!("blocking task failed: {e}"))??;

    let mut payload = format_payload(kind, &secret);
    // The `secret` `String` is no longer needed past this point — every
    // downstream consumer reads `payload`. Scrub before drop so the raw
    // bytes don't sit on the heap waiting for an allocator reuse.
    secret.zeroize();

    // Write to the OS clipboard. On failure, scrub the payload before
    // bubbling up; the renderer never sees these bytes.
    if let Err(e) = app.clipboard().write_text(payload.clone()) {
        payload.zeroize();
        return Err(format!("clipboard: {e}"));
    }

    let clears_at = now_unix_ms() + CLIPBOARD_CLEAR_MS;
    // Hand a *clone* of `payload` to the self-clear task — the task
    // owns its copy and zeroizes it after the readback. Our local
    // copy is scrubbed below.
    schedule_self_clear(app.clone(), payload.clone());
    payload.zeroize();

    Ok(KeyCopyReceiptDto {
        label,
        preview,
        clipboard_clears_at_unix_ms: clears_at,
    })
}

/// Copy an `ANTHROPIC_API_KEY` to the OS clipboard. The secret never
/// returns to JS — the renderer only sees a `KeyCopyReceiptDto` it
/// can toast verbatim. Self-clears after 30s if the clipboard still
/// holds our payload.
#[tauri::command]
pub async fn key_api_copy(uuid: String, app: AppHandle) -> Result<KeyCopyReceiptDto, String> {
    key_copy_inner(uuid, CopyKind::Api, app).await
}

/// Copy a `CLAUDE_CODE_OAUTH_TOKEN` to the OS clipboard. Sibling of
/// `key_api_copy`. See that function for the secret-handling contract.
#[tauri::command]
pub async fn key_oauth_copy(uuid: String, app: AppHandle) -> Result<KeyCopyReceiptDto, String> {
    key_copy_inner(uuid, CopyKind::Oauth, app).await
}

/// Copy a paste-ready POSIX shell invocation that runs CC under this
/// OAuth token: `CLAUDE_CODE_OAUTH_TOKEN='<secret>' claude`. The
/// formatting happens server-side so the raw secret never leaves Rust
/// — the JS layer only ever sees a `KeyCopyReceiptDto`.
#[tauri::command]
pub async fn key_oauth_copy_shell(
    uuid: String,
    app: AppHandle,
) -> Result<KeyCopyReceiptDto, String> {
    key_copy_inner(uuid, CopyKind::OauthShell, app).await
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
    Ok(snapshot
        .as_ref()
        .map(crate::dto::AccountUsageDto::from_response))
}

#[cfg(test)]
mod oauth_summary_days_tests {
    use super::*;
    use chrono::Duration;
    use claudepot_core::keys::OauthToken;

    fn make_token(created_at: chrono::DateTime<chrono::Utc>) -> OauthToken {
        OauthToken {
            uuid: Uuid::new_v4(),
            label: "test".into(),
            token_preview: "sk-ant-oat01-…".into(),
            account_uuid: Uuid::new_v4(),
            created_at,
            last_probed_at: None,
            last_probe_status: None,
        }
    }

    #[test]
    fn oauth_summary_expired_a_few_hours_reads_minus_one_day() {
        // Audit B8 commands_keys.rs:54 — the previous truncating
        // division turned a token expired 5 hours ago into
        // `days_remaining = 0`, which the UI renders as "expires
        // today" instead of "expired". Floor division must yield
        // -1 (or more negative) for any negative remainder.
        let validity = Duration::days(OAUTH_TOKEN_VALIDITY_DAYS);
        let created_at = chrono::Utc::now() - validity - Duration::hours(5);
        let dto = oauth_summary(make_token(created_at), &HashMap::new());
        assert!(
            dto.days_remaining < 0,
            "expected negative days_remaining for expired token, got {}",
            dto.days_remaining
        );
    }

    #[test]
    fn oauth_summary_expired_one_full_day_reads_minus_one_day() {
        let validity = Duration::days(OAUTH_TOKEN_VALIDITY_DAYS);
        let created_at = chrono::Utc::now() - validity - Duration::days(1);
        let dto = oauth_summary(make_token(created_at), &HashMap::new());
        assert!(
            dto.days_remaining <= -1,
            "expected ≤ -1 days_remaining, got {}",
            dto.days_remaining
        );
    }

    #[test]
    fn oauth_summary_fresh_token_reads_positive_days() {
        let created_at = chrono::Utc::now();
        let dto = oauth_summary(make_token(created_at), &HashMap::new());
        assert!(
            dto.days_remaining > 0,
            "expected positive days_remaining for fresh token, got {}",
            dto.days_remaining
        );
    }
}

#[cfg(test)]
mod copy_payload_tests {
    //! Pure-helper tests for the secret-IPC redesign (D-5/6/7). The
    //! happy-path `key_*_copy*` flow needs a live Tauri AppHandle to
    //! exercise (clipboard plugin + tokio runtime), which is too
    //! heavyweight for a unit test — those paths are covered by the
    //! manual scp-to-test-host QA pass documented in CLAUDE.md.
    //! What we *can* lock down here is the pure logic: payload
    //! formatting, deadline computation, receipt shape.

    use super::*;

    #[test]
    fn key_api_copy_format_payload_passes_secret_through_unchanged() {
        // Raw API key copy hands the bytes to the clipboard verbatim.
        let p = format_payload(CopyKind::Api, "sk-ant-api03-abcdef");
        assert_eq!(p, "sk-ant-api03-abcdef");
    }

    #[test]
    fn key_oauth_copy_format_payload_passes_secret_through_unchanged() {
        let p = format_payload(CopyKind::Oauth, "sk-ant-oat01-zzz");
        assert_eq!(p, "sk-ant-oat01-zzz");
    }

    #[test]
    fn key_oauth_copy_shell_format_payload_wraps_in_env_invocation() {
        // The single-quote wrapping is load-bearing for the user's
        // copy-paste flow: `claude` must see the full token as one
        // env-var assignment regardless of any shell-special chars.
        let p = format_payload(CopyKind::OauthShell, "sk-ant-oat01-secret");
        assert_eq!(p, "CLAUDE_CODE_OAUTH_TOKEN='sk-ant-oat01-secret' claude");
    }

    #[test]
    fn key_copy_receipt_does_not_log_token() {
        // The Debug impl is what `tracing!` would interpolate, and
        // what error toasts capture if a higher-level handler
        // formats the receipt by accident. Neither field must
        // expose a substring of the secret bytes — they were never
        // copied into the receipt to begin with, so this is a
        // structural guarantee, not a sanitization step.
        let secret = "sk-ant-oat01-DO_NOT_LEAK_VERY_LONG_TOKEN_VALUE";
        let receipt = KeyCopyReceiptDto {
            label: "ci-token".into(),
            preview: "sk-ant-oat01-DO_…UE".into(),
            clipboard_clears_at_unix_ms: 12345,
        };
        let dbg = format!("{receipt:?}");
        assert!(
            !dbg.contains(secret),
            "receipt Debug impl must not include the secret value: {dbg}"
        );
        // Also assert the JSON shape — this is what crosses the
        // bridge and what the renderer toasts.
        let json = serde_json::to_string(&receipt).unwrap();
        assert!(json.contains("\"label\":\"ci-token\""), "json: {json}");
        assert!(
            json.contains("\"preview\":\"sk-ant-oat01-DO_…UE\""),
            "json: {json}",
        );
        assert!(
            json.contains("\"clipboard_clears_at_unix_ms\":12345"),
            "json: {json}",
        );
        assert!(
            !json.contains(secret),
            "receipt JSON must not include the secret: {json}",
        );
    }

    #[test]
    fn now_unix_ms_is_within_a_minute_of_systemtime() {
        // Sanity check that the deadline math doesn't wrap. The
        // narrow tolerance catches a regression where someone
        // converts `as_millis()` to seconds without noticing.
        let ours = now_unix_ms();
        let theirs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(
            theirs.saturating_sub(ours) < 60_000,
            "now_unix_ms drifted: ours={ours} theirs={theirs}",
        );
    }
}
